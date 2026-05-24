//! Legacy plugin RPC bridge.
//!
//! Each plugin runs as an isolated child process (`node <entrypoint>`) and
//! exchanges newline-delimited JSON-RPC 2.0 frames over stdio with the Rust
//! core. The bridge owns:
//!
//! - the child handle (so it can `kill()` on shutdown),
//! - a writer task that serialises outbound requests onto stdin,
//! - a reader task that parses inbound frames and matches them to pending
//!   request IDs.
//!
//! Crash policy: when the child exits unexpectedly, all pending futures
//! resolve with [`PluginError::ChildExited`]; the supervisor restarts the
//! plugin with exponential backoff (capped at 30 s). Restart logic is owned
//! by the caller — this bridge only reports lifecycle events.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::Arc;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex};

/// Curated priority plugin list for v2 GA.
pub const PRIORITY_PLUGINS: &[&str] = &[
    "docker",
    "sync-config",
    "quick-cmds",
    "save-output",
    "workspace-manager",
    "background",
    "highlight",
];

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("spawn: {0}")]
    Spawn(String),
    #[error("io: {0}")]
    Io(String),
    #[error("plugin returned error code {code}: {message}")]
    Rpc { code: i32, message: String },
    #[error("plugin child exited")]
    ChildExited,
    #[error("response decode: {0}")]
    Decode(String),
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, PluginError>>>>>;

pub struct LegacyBridge {
    stdin: Mutex<ChildStdin>,
    pending: Pending,
    next_id: AtomicU64,
    child: Mutex<Child>,
}

impl LegacyBridge {
    /// Spawn `node <script>` and start the I/O tasks.
    pub async fn spawn(node: &str, script: &str) -> Result<Arc<Self>, PluginError> {
        let mut cmd = Command::new(node);
        cmd.arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| PluginError::Spawn(e.to_string()))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| PluginError::Spawn("no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| PluginError::Spawn("no stdout".into()))?;

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let bridge = Arc::new(Self {
            stdin: Mutex::new(stdin),
            pending: pending.clone(),
            next_id: AtomicU64::new(1),
            child: Mutex::new(child),
        });

        // Reader task.
        let pending_clone = pending.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(_) => break,
                }
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let parsed: serde_json::Result<Value> = serde_json::from_str(trimmed);
                match parsed {
                    Ok(v) => dispatch_incoming(v, &pending_clone).await,
                    Err(_) => {
                        // Garbage line — ignore but keep the loop running.
                        continue;
                    }
                }
            }
            // Child closed stdout: fail every pending request.
            let mut map = pending_clone.lock().await;
            for (_, tx) in map.drain() {
                let _ = tx.send(Err(PluginError::ChildExited));
            }
        });

        Ok(bridge)
    }

    /// Issues a JSON-RPC call and awaits the response.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, PluginError> {
        let id = self.next_id.fetch_add(1, AtomicOrdering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let frame = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let mut wire = serde_json::to_vec(&frame).map_err(|e| PluginError::Io(e.to_string()))?;
        wire.push(b'\n');

        {
            let mut stdin = self.stdin.lock().await;
            stdin
                .write_all(&wire)
                .await
                .map_err(|e| PluginError::Io(e.to_string()))?;
            stdin
                .flush()
                .await
                .map_err(|e| PluginError::Io(e.to_string()))?;
        }

        match rx.await {
            Ok(r) => r,
            Err(_) => Err(PluginError::ChildExited),
        }
    }

    /// Best-effort shutdown.
    pub async fn shutdown(&self) {
        let mut child = self.child.lock().await;
        let _ = child.kill().await;
    }
}

async fn dispatch_incoming(v: Value, pending: &Pending) {
    let Some(id) = v.get("id").and_then(|x| x.as_u64()) else {
        return; // notification — ignored for now
    };
    let mut map = pending.lock().await;
    let Some(tx) = map.remove(&id) else { return };
    if let Some(err) = v.get("error") {
        let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(-32603) as i32;
        let message = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("plugin error")
            .to_string();
        let _ = tx.send(Err(PluginError::Rpc { code, message }));
    } else {
        let result = v.get("result").cloned().unwrap_or(Value::Null);
        let _ = tx.send(Ok(result));
    }
}

impl Default for LegacyBridge {
    fn default() -> Self {
        // Cannot meaningfully default without spawning; provide an inert
        // placeholder for type-level needs (rare).
        unreachable!("use LegacyBridge::spawn")
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    const ECHO_PLUGIN_JS: &str = r#"
process.stdin.setEncoding('utf8');
let buf = '';
process.stdin.on('data', (chunk) => {
  buf += chunk;
  let i;
  while ((i = buf.indexOf('\n')) >= 0) {
    const line = buf.slice(0, i);
    buf = buf.slice(i + 1);
    if (!line.trim()) continue;
    let req;
    try { req = JSON.parse(line); } catch { continue; }
    let resp;
    if (req.method === 'boom') {
      resp = { jsonrpc: '2.0', id: req.id, error: { code: -32000, message: 'kaboom' } };
    } else {
      resp = { jsonrpc: '2.0', id: req.id, result: { method: req.method, echoed: req.params } };
    }
    process.stdout.write(JSON.stringify(resp) + '\n');
  }
});
"#;

    fn write_plugin() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("aerotab-plugin-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("plugin.js");
        std::fs::write(&path, ECHO_PLUGIN_JS).unwrap();
        path
    }

    fn node_available() -> bool {
        std::process::Command::new("node")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn echo_roundtrip() {
        if !node_available() {
            eprintln!("skipping: node not on PATH");
            return;
        }
        let script = write_plugin();
        let bridge = LegacyBridge::spawn("node", script.to_str().unwrap())
            .await
            .unwrap();
        let resp = bridge
            .call("ping", serde_json::json!({"x": 1}))
            .await
            .unwrap();
        assert_eq!(resp["method"], "ping");
        assert_eq!(resp["echoed"]["x"], 1);
        bridge.shutdown().await;
    }

    #[tokio::test]
    async fn rpc_error_propagated() {
        if !node_available() {
            eprintln!("skipping: node not on PATH");
            return;
        }
        let script = write_plugin();
        let bridge = LegacyBridge::spawn("node", script.to_str().unwrap())
            .await
            .unwrap();
        let err = bridge
            .call("boom", serde_json::Value::Null)
            .await
            .unwrap_err();
        match err {
            PluginError::Rpc { code, message } => {
                assert_eq!(code, -32000);
                assert!(message.contains("kaboom"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
        bridge.shutdown().await;
    }
}
