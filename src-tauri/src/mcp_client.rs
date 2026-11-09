//! Independent outbound MCP client. Imported configuration is inert until a
//! local user trusts its exact serialized revision. No AeroTab server grants are reused.
use crate::{secret, settings::SettingsStore};
use rmcp::{model::*, service::RunningService, RoleClient, ServiceExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashMap, process::Stdio, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
};

const KEY: &str = "ai.mcpClients";
pub const MAX_OUTPUT: usize = 131072;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "transport", rename_all = "camelCase", deny_unknown_fields)]
pub enum TransportConfig {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    Http {
        url: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClientConfig {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub transport: TransportConfig,
}
#[derive(Default)]
pub struct ClientRuntime {
    connections: Mutex<HashMap<String, Arc<Mutex<Connection>>>>,
}
enum Connection {
    Http(Box<RunningService<RoleClient, ()>>),
    Stdio(Box<StdioConnection>),
}
struct StdioConnection {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    next: u64,
}
impl Drop for StdioConnection {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}
fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}
pub fn validate_url(raw: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(raw).map_err(|_| "Invalid endpoint URL")?;
    let local = matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    );
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !(url.scheme() == "https" || url.scheme() == "http" && local)
    {
        return Err("Use HTTPS (HTTP is allowed only on loopback); credentials/query/fragment are forbidden".into());
    }
    Ok(url)
}
fn validate(config: &ClientConfig) -> Result<(), String> {
    if config.id.is_empty()
        || config.id.len() > 64
        || !config
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("Invalid client ID".into());
    }
    match &config.transport {
        TransportConfig::Http { url } => {
            validate_url(url)?;
        }
        TransportConfig::Stdio { command, args } => {
            if command.is_empty()
                || command.len() > 4096
                || args.len() > 128
                || command.to_lowercase().contains("aerotab")
                || args
                    .iter()
                    .any(|a| a.to_lowercase().contains("aerotab-mcp"))
            {
                return Err("Invalid command or self MCP recursion".into());
            }
        }
    }
    Ok(())
}
pub fn configs(store: &SettingsStore) -> Result<Vec<ClientConfig>, String> {
    serde_json::from_value(store.local_get(KEY).map_err(err)?.unwrap_or(json!([]))).map_err(err)
}
pub fn save(store: &SettingsStore, items: Vec<ClientConfig>) -> Result<(), String> {
    if items.len() > 16 {
        return Err("At most 16 MCP clients".into());
    }
    let mut ids = std::collections::HashSet::new();
    for item in &items {
        validate(item)?;
        if !ids.insert(&item.id) {
            return Err("Duplicate MCP client ID".into());
        }
    }
    store
        .local_set(KEY, &serde_json::to_value(items).map_err(err)?)
        .map_err(err)
}
fn revision(config: &ClientConfig) -> Value {
    serde_json::to_value(config).expect("serializable config")
}
pub fn trusted(store: &SettingsStore, config: &ClientConfig) -> Result<bool, String> {
    Ok(store
        .local_get(&format!("ai.mcpTrust.{}", config.id))
        .map_err(err)?
        == Some(revision(config)))
}
pub fn trust(store: &SettingsStore, config: &ClientConfig, allow: bool) -> Result<(), String> {
    validate(config)?;
    store
        .local_set(
            &format!("ai.mcpTrust.{}", config.id),
            &if allow { revision(config) } else { Value::Null },
        )
        .map_err(err)
}
impl ClientRuntime {
    pub async fn disconnect(&self, id: &str) {
        self.connections.lock().await.remove(id);
    }
    pub async fn disconnect_all(&self) {
        self.connections.lock().await.clear();
    }
    pub async fn connect(
        &self,
        store: &SettingsStore,
        id: &str,
        own_port: Option<u16>,
    ) -> Result<Value, String> {
        let config = configs(store)?
            .into_iter()
            .find(|c| c.id == id)
            .ok_or("Unknown MCP client")?;
        validate(&config)?;
        if !trusted(store, &config)? {
            return Err("Local trust is required before starting this MCP client".into());
        }
        self.disconnect(id).await;
        let connection = match &config.transport {
            TransportConfig::Http { url } => {
                let parsed = validate_url(url)?;
                if matches!(
                    parsed.host_str(),
                    Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
                ) && own_port == parsed.port_or_known_default()
                {
                    return Err("Self MCP recursion is forbidden".into());
                }
                let mut cfg = rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(url.clone());
                if let Ok(key) = secret::get_secret(&format!("ai.mcpAuth.{id}")) {
                    cfg = cfg.auth_header(key);
                }
                let client = reqwest::Client::builder()
                    .redirect(reqwest::redirect::Policy::none())
                    .connect_timeout(Duration::from_secs(10))
                    .build()
                    .map_err(err)?;
                let transport =
                    rmcp::transport::StreamableHttpClientTransport::with_client(client, cfg);
                let service = tokio::time::timeout(Duration::from_secs(15), ().serve(transport))
                    .await
                    .map_err(|_| "MCP initialize timeout")?
                    .map_err(|_| "MCP HTTP initialization failed")?;
                if service
                    .peer_info()
                    .is_some_and(|p| p.server_info.name.to_lowercase().contains("aerotab"))
                {
                    return Err("Self MCP recursion is forbidden".into());
                }
                Connection::Http(Box::new(service))
            }
            TransportConfig::Stdio { command, args } => {
                let mut cmd = Command::new(command);
                cmd.args(args)
                    .env_clear()
                    .env("PATH", std::env::var("PATH").unwrap_or_default())
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .kill_on_drop(true);
                if let Ok(key) = secret::get_secret(&format!("ai.mcpAuth.{id}")) {
                    cmd.env("MCP_AUTH_TOKEN", key);
                }
                let mut child = cmd.spawn().map_err(|_| "MCP child could not be started")?;
                let input = child.stdin.take().ok_or("Missing child stdin")?;
                let output = BufReader::new(child.stdout.take().ok_or("Missing child stdout")?);
                let mut io = StdioConnection {
                    child,
                    input,
                    output,
                    next: 1,
                };
                let info = io.request("initialize", json!({"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"AeroTab AI client","version":crate::CORE_VERSION}})).await?;
                if info["serverInfo"]["name"]
                    .as_str()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains("aerotab")
                {
                    return Err("Self MCP recursion is forbidden".into());
                }
                io.notify("notifications/initialized", json!({})).await?;
                Connection::Stdio(Box::new(io))
            }
        };
        self.connections
            .lock()
            .await
            .insert(id.to_string(), Arc::new(Mutex::new(connection)));
        self.discover(id).await
    }
    pub async fn request(&self, id: &str, method: &str, params: Value) -> Result<Value, String> {
        let conn = self
            .connections
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or("MCP client is disconnected")?;
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            let mut conn = conn.lock().await;
            match &mut *conn {
                Connection::Stdio(io) => io.request(method, params).await,
                Connection::Http(service) => {
                    macro_rules! request {
                        ($name:ident, $ty:ty) => {{
                            let p: $ty = serde_json::from_value(params).map_err(err)?;
                            serde_json::to_value(
                                service.$name(p).await.map_err(|_| "MCP request failed")?,
                            )
                            .map_err(err)
                        }};
                    }
                    match method {
                        "tools/list" => request!(list_tools, Option<PaginatedRequestParam>),
                        "resources/list" => request!(list_resources, Option<PaginatedRequestParam>),
                        "resources/templates/list" => {
                            request!(list_resource_templates, Option<PaginatedRequestParam>)
                        }
                        "prompts/list" => request!(list_prompts, Option<PaginatedRequestParam>),
                        "tools/call" => request!(call_tool, CallToolRequestParam),
                        "resources/read" => request!(read_resource, ReadResourceRequestParam),
                        "prompts/get" => request!(get_prompt, GetPromptRequestParam),
                        _ => Err("MCP method is not allowlisted".into()),
                    }
                }
            }
        })
        .await
        .map_err(|_| "MCP request timeout".to_string())?;
        if result
            .as_ref()
            .is_ok_and(|v| v.to_string().len() > MAX_OUTPUT)
        {
            return Err("MCP output limit exceeded".into());
        }
        result
    }
    pub async fn discover(&self, id: &str) -> Result<Value, String> {
        if !self.connections.lock().await.contains_key(id) {
            return Err("MCP client is disconnected".into());
        }
        let mut out = json!({});
        for (key, method) in [
            ("tools", "tools/list"),
            ("resources", "resources/list"),
            ("resourceTemplates", "resources/templates/list"),
            ("prompts", "prompts/list"),
        ] {
            let mut items = Vec::new();
            let mut cursor: Option<Value> = None;
            for _ in 0..8 {
                let page = self
                    .request(
                        id,
                        method,
                        cursor.as_ref().map_or(json!({}), |c| json!({"cursor":c})),
                    )
                    .await;
                match page {
                    Ok(page) => {
                        if let Some(list) = page[key].as_array() {
                            items.extend(list.clone());
                            if items.len() > 512
                                || serde_json::to_vec(&items).map_err(err)?.len() > MAX_OUTPUT
                            {
                                return Err("MCP discovery output limit exceeded".into());
                            }
                        }
                        cursor = page.get("nextCursor").cloned();
                        if cursor.is_none() {
                            break;
                        }
                    }
                    Err(e) if key == "tools" => return Err(e),
                    Err(_) => break,
                }
            }
            out[key] = json!(items);
        }
        Ok(out)
    }
}
impl StdioConnection {
    async fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        let mut bytes =
            serde_json::to_vec(&json!({"jsonrpc":"2.0","method":method,"params":params}))
                .map_err(err)?;
        bytes.push(b'\n');
        self.input
            .write_all(&bytes)
            .await
            .map_err(|_| "MCP pipe write failed".into())
    }
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next += 1;
        let id = self.next;
        let mut bytes =
            serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
                .map_err(err)?;
        bytes.push(b'\n');
        self.input
            .write_all(&bytes)
            .await
            .map_err(|_| "MCP pipe write failed")?;
        tokio::time::timeout(Duration::from_secs(30), async {
            for _ in 0..128 {
                // fill_buf/consume enforces the byte cap before allocating a complete line.
                let mut line=Vec::new();
                loop { let buf=self.output.fill_buf().await.map_err(err)?; if buf.is_empty() { return Err("MCP child exited".into()); } let count=buf.iter().position(|b| *b==b'\n').map_or(buf.len(), |i| i+1); if line.len()+count>MAX_OUTPUT { return Err("MCP frame limit exceeded".into()); } let done=buf[count-1]==b'\n'; line.extend_from_slice(&buf[..count]); self.output.consume(count); if done { break; } }
                let frame: Value=serde_json::from_slice(&line).map_err(|_| "Invalid MCP frame")?;
                if frame["id"]==id && frame.get("method").is_none() { if frame.get("error").is_some() { return Err("MCP returned a protocol error".into()); } return frame.get("result").cloned().ok_or("Missing MCP result".into()); }
                // Server requests (sampling, elicitation, approval) are never handled.
                if frame.get("method").is_some() && frame.get("id").is_some() { let mut bytes=serde_json::to_vec(&json!({"jsonrpc":"2.0","id":frame["id"],"error":{"code":-32601,"message":"Client requests are not supported"}})).map_err(err)?; bytes.push(b'\n'); self.input.write_all(&bytes).await.map_err(err)?; }
            }
            Err("MCP unsolicited frame limit exceeded".into())
        }).await.map_err(|_| "MCP response timeout".to_string())?
    }
}
#[cfg(test)]
#[path = "mcp_client_integration_tests.rs"]
mod integration_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn url_policy() {
        assert!(validate_url("http://127.0.0.1:1234/mcp").is_ok());
        for u in [
            "http://example.com/mcp",
            "https://user:pass@example.com",
            "https://example.com?key=secret",
            "file:///tmp/a",
        ] {
            assert!(validate_url(u).is_err());
        }
    }
    #[test]
    fn config_is_inert_and_trust_is_revision_bound() {
        let dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let s = SettingsStore::open(&dir).unwrap();
        let mut c = ClientConfig {
            id: "fixture".into(),
            name: "Fixture".into(),
            transport: TransportConfig::Stdio {
                command: "never-executed".into(),
                args: vec![],
            },
        };
        save(&s, vec![c.clone()]).unwrap();
        assert!(!trusted(&s, &c).unwrap());
        trust(&s, &c, true).unwrap();
        assert!(trusted(&s, &c).unwrap());
        c.name = "Changed".into();
        assert!(!trusted(&s, &c).unwrap());
        assert!(s.all().unwrap().is_empty());
    }
}
