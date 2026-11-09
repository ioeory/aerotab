//! Local-only scripts, independent of settings/Appearance sync. No auth is persisted.
//! Integration: declare `pub mod scripts`, initialize `ScriptsService::new(
//! ScriptsStore::open(data_dir.join("scripts"))?)` once in AppState (Arc is fine).
//! Route scripts.* to `service.dispatch(method, params, &resolver).await`.
//! Resolver MUST load a current SSH profile by stable ID, resolve jump/proxy/Vault
//! references using the same path as session.openSshProfile, and supply the app's
//! KnownHosts and current transport settings. Never accept frontend SSH snapshots.
//! Before profile deletion consult `service.store.references(id)` and reject in-use IDs.
//! On application shutdown await `service.shutdown()`. Do not grant MCP script access
//! implicitly. Definitions, confirmations, task output and runtime state are not synced.

use crate::ssh::{self, known_hosts::KnownHosts, SshProfile, SshTransportSettings};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    future::Future,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio_util::sync::CancellationToken;

const OUTPUT_LIMIT: usize = 256 * 1024;
const MAX_TASKS: usize = 32;
const MAX_ACTIVE: usize = 4;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptInput {
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub profile_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Script {
    #[serde(flatten)]
    pub input: ScriptInput,
    pub version: u64,
    pub updated_at: u64,
}
#[derive(Clone)]
pub struct ScriptsStore {
    db: sled::Db,
    lock: Arc<Mutex<()>>,
}
impl ScriptsStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        Self::from_db(sled::open(path).map_err(|_| "scripts database unavailable")?)
    }
    pub fn temporary() -> Result<Self, String> {
        Self::from_db(
            sled::Config::new()
                .temporary(true)
                .open()
                .map_err(|_| "scripts database unavailable")?,
        )
    }
    fn from_db(db: sled::Db) -> Result<Self, String> {
        Ok(Self {
            db,
            lock: Arc::new(Mutex::new(())),
        })
    }
    pub fn list(&self) -> Result<Vec<Script>, String> {
        self.db
            .iter()
            .map(|entry| {
                let (_, value) = entry.map_err(|_| "scripts read failed")?;
                serde_json::from_slice(&value).map_err(|_| "invalid script record".into())
            })
            .collect()
    }
    pub fn get(&self, id: &str) -> Result<Script, String> {
        let value = self
            .db
            .get(id)
            .map_err(|_| "scripts read failed")?
            .ok_or("script not found")?;
        serde_json::from_slice(&value).map_err(|_| "invalid script record".into())
    }
    pub fn put(&self, mut input: ScriptInput, expected: u64) -> Result<Script, String> {
        let _lock = self.lock.lock().map_err(|_| "scripts lock failed")?;
        input.name = input.name.trim().into();
        if input.id.is_empty()
            || input.id.len() > 128
            || input.name.is_empty()
            || input.name.len() > 256
            || input.command.trim().is_empty()
            || input.command.len() > 64 * 1024
            || input.description.len() > 16 * 1024
            || input.command.contains('\0')
            || input
                .profile_id
                .as_ref()
                .is_some_and(|id| id.is_empty() || id.len() > 128)
        {
            return Err("invalid script fields".into());
        }
        let old = self.db.get(&input.id).map_err(|_| "scripts read failed")?;
        let version = old
            .as_ref()
            .map(|v| serde_json::from_slice::<Script>(v).map(|s| s.version))
            .transpose()
            .map_err(|_| "invalid script record")?
            .unwrap_or(0);
        if expected != version {
            return Err("script version conflict".into());
        }
        let script = Script {
            input,
            version: version.checked_add(1).ok_or("script version overflow")?,
            updated_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        };
        self.db
            .insert(
                script.input.id.as_bytes(),
                serde_json::to_vec(&script).map_err(|_| "script encoding failed")?,
            )
            .map_err(|_| "scripts write failed")?;
        self.db.flush().map_err(|_| "scripts flush failed")?;
        Ok(script)
    }
    pub fn delete(&self, id: &str, expected: u64) -> Result<(), String> {
        let _lock = self.lock.lock().map_err(|_| "scripts lock failed")?;
        if self.get(id)?.version != expected {
            return Err("script version conflict".into());
        }
        self.db.remove(id).map_err(|_| "scripts delete failed")?;
        self.db.flush().map_err(|_| "scripts flush failed")?;
        Ok(())
    }
    pub fn references(&self, id: &str) -> Result<Vec<String>, String> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|s| s.input.profile_id.as_deref() == Some(id))
            .map(|s| s.input.name)
            .collect())
    }
}

pub struct ResolvedTarget {
    pub profile: SshProfile,
    pub known_hosts: KnownHosts,
    pub transport: SshTransportSettings,
}
fn target(profile: &SshProfile) -> String {
    format!("{}@{}:{}", profile.user, profile.host, profile.port)
}
fn route(profile: &SshProfile) -> Vec<String> {
    let mut result: Vec<_> = profile.jump_via.iter().flat_map(route).collect();
    result.push(target(profile));
    result
}
struct Confirmation {
    script: Script,
    profile_id: String,
    route: Vec<String>,
    expires: Instant,
    timeout: Duration,
}
#[derive(Clone, Debug, Serialize)]
pub struct TaskState {
    pub id: String,
    pub script_id: String,
    pub profile_id: String,
    pub target: String,
    pub status: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<u32>,
    pub error: Option<String>,
    pub truncated: bool,
}
struct Task {
    state: Arc<Mutex<TaskState>>,
    cancel: CancellationToken,
    done: CancellationToken,
}
#[derive(Clone)]
pub struct ScriptsService {
    pub store: ScriptsStore,
    confirmations: Arc<Mutex<HashMap<String, Confirmation>>>,
    tasks: Arc<Mutex<HashMap<String, Arc<Task>>>>,
}
impl ScriptsService {
    pub fn new(store: ScriptsStore) -> Self {
        Self {
            store,
            confirmations: Arc::default(),
            tasks: Arc::default(),
        }
    }
    pub async fn dispatch<F, Fut>(
        &self,
        method: &str,
        params: Value,
        resolve: &F,
    ) -> Result<Value, String>
    where
        F: Fn(String) -> Fut + Sync,
        Fut: Future<Output = Result<ResolvedTarget, String>> + Send,
    {
        let text = |key: &str| {
            params
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("missing {key}"))
        };
        let version = || {
            params
                .get("expectedVersion")
                .and_then(Value::as_u64)
                .ok_or_else(|| "missing expectedVersion".to_string())
        };
        match method {
            "scripts.list" => serde_json::to_value(self.store.list()?)
                .map_err(|_| "script encoding failed".into()),
            "scripts.get" => serde_json::to_value(self.store.get(&text("id")?)?)
                .map_err(|_| "script encoding failed".into()),
            "scripts.put" => {
                let input: ScriptInput =
                    serde_json::from_value(params.get("script").cloned().ok_or("missing script")?)
                        .map_err(|_| "invalid script fields")?;
                if let Some(id) = &input.profile_id {
                    resolve(id.clone()).await?;
                }
                serde_json::to_value(self.store.put(input, version()?)?)
                    .map_err(|_| "script encoding failed".into())
            }
            "scripts.delete" => {
                self.store.delete(&text("id")?, version()?)?;
                Ok(json!(true))
            }
            "scripts.preview" => {
                let script = self.store.get(&text("id")?)?;
                let profile_id = text("profile_id")?;
                let resolved = resolve(profile_id.clone()).await?;
                Ok(
                    json!({"script_name":script.input.name,"profile_id":profile_id,"target":target(&resolved.profile)}),
                )
            }
            "scripts.prepare" => {
                let script = self.store.get(&text("id")?)?;
                let profile_id = text("profile_id")?;
                let resolved = resolve(profile_id.clone()).await?;
                let seconds = params
                    .get("timeout_seconds")
                    .and_then(Value::as_u64)
                    .unwrap_or(30);
                if !(1..=300).contains(&seconds) {
                    return Err("timeout must be 1..300 seconds".into());
                }
                let token = uuid::Uuid::new_v4().to_string();
                let mut confirmations = self
                    .confirmations
                    .lock()
                    .map_err(|_| "scripts lock failed")?;
                confirmations.retain(|_, c| c.expires > Instant::now());
                if confirmations.len() >= 32 {
                    return Err("too many pending confirmations".into());
                }
                let response = json!({"token":token,"script_name":script.input.name,"command":script.input.command,"profile_id":profile_id,"target":target(&resolved.profile),"timeout_seconds":seconds,"output_limit":OUTPUT_LIMIT});
                confirmations.insert(
                    token,
                    Confirmation {
                        script,
                        profile_id,
                        route: route(&resolved.profile),
                        expires: Instant::now() + Duration::from_secs(60),
                        timeout: Duration::from_secs(seconds),
                    },
                );
                Ok(response)
            }
            "scripts.run" => {
                let confirmation = self
                    .confirmations
                    .lock()
                    .map_err(|_| "scripts lock failed")?
                    .remove(&text("token")?)
                    .ok_or("confirmation missing or already consumed")?;
                if confirmation.expires <= Instant::now() {
                    return Err("confirmation expired; confirm again".into());
                }
                let resolved = resolve(confirmation.profile_id.clone()).await?;
                let current = self.store.get(&confirmation.script.input.id)?;
                if current.version != confirmation.script.version
                    || route(&resolved.profile) != confirmation.route
                {
                    return Err("script or target changed; confirm again".into());
                }
                let id = uuid::Uuid::new_v4().to_string();
                let state = TaskState {
                    id: id.clone(),
                    script_id: current.input.id,
                    profile_id: confirmation.profile_id,
                    target: target(&resolved.profile),
                    status: "running".into(),
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: None,
                    error: None,
                    truncated: false,
                };
                let response = json!(state);
                let task = Arc::new(Task {
                    state: Arc::new(Mutex::new(state)),
                    cancel: CancellationToken::new(),
                    done: CancellationToken::new(),
                });
                {
                    let mut tasks = self.tasks.lock().map_err(|_| "scripts lock failed")?;
                    if tasks.values().filter(|t| !t.done.is_cancelled()).count() >= MAX_ACTIVE {
                        return Err("too many active script tasks".into());
                    }
                    if tasks.len() >= MAX_TASKS {
                        if let Some(old) = tasks
                            .iter()
                            .find(|(_, t)| t.done.is_cancelled())
                            .map(|(id, _)| id.clone())
                        {
                            tasks.remove(&old);
                        }
                    }
                    tasks.insert(id, task.clone());
                }
                tokio::spawn(async move {
                    run_task(task, resolved, current.input.command, confirmation.timeout).await;
                });
                Ok(response)
            }
            "scripts.tasks" => {
                let tasks = self.tasks.lock().map_err(|_| "scripts lock failed")?;
                let states: Result<Vec<_>, _> = tasks
                    .values()
                    .map(|task| {
                        task.state
                            .lock()
                            .map(|state| state.clone())
                            .map_err(|_| "scripts lock failed")
                    })
                    .collect();
                Ok(json!(states?))
            }
            "scripts.status" | "scripts.cancel" => {
                let task = self
                    .tasks
                    .lock()
                    .map_err(|_| "scripts lock failed")?
                    .get(&text("id")?)
                    .cloned()
                    .ok_or("script task not found")?;
                if method == "scripts.cancel" {
                    task.cancel.cancel();
                    task.done.cancelled().await;
                }
                let state = task
                    .state
                    .lock()
                    .map_err(|_| "scripts lock failed")?
                    .clone();
                Ok(json!(state))
            }
            _ => Err("unknown scripts method".into()),
        }
    }
    pub async fn shutdown(&self) {
        let tasks: Vec<_> = self
            .tasks
            .lock()
            .expect("scripts lock")
            .values()
            .cloned()
            .collect();
        for task in &tasks {
            task.cancel.cancel();
        }
        for task in tasks {
            task.done.cancelled().await;
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum StopReason {
    Cancelled,
    TimedOut,
    Failed,
}
pub async fn controlled<T>(
    future: impl Future<Output = Result<T, String>>,
    cancel: CancellationToken,
    timeout: Duration,
) -> Result<T, StopReason> {
    tokio::select! { biased; _ = cancel.cancelled() => Err(StopReason::Cancelled), result = tokio::time::timeout(timeout,future) => match result { Ok(Ok(value)) => Ok(value), Ok(Err(_)) => Err(StopReason::Failed), Err(_) => Err(StopReason::TimedOut) } }
}
pub struct BoundedOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    limit: usize,
}
impl BoundedOutput {
    pub fn new(limit: usize) -> Self {
        Self {
            stdout: Vec::new(),
            stderr: Vec::new(),
            limit,
        }
    }
    pub fn push(&mut self, stderr: bool, bytes: &[u8]) -> bool {
        let remaining = self
            .limit
            .saturating_sub(self.stdout.len() + self.stderr.len());
        let destination = if stderr {
            &mut self.stderr
        } else {
            &mut self.stdout
        };
        destination.extend_from_slice(&bytes[..remaining.min(bytes.len())]);
        bytes.len() > remaining
    }
}
async fn run_task(task: Arc<Task>, resolved: ResolvedTarget, command: String, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    let connect = async {
        ssh::connect_authenticated(
            &resolved.profile,
            Some(resolved.known_hosts),
            resolved.transport,
        )
        .await
        .map_err(|_| "SSH connection failed".into())
    };
    let result = controlled(connect, task.cancel.clone(), timeout).await;
    let outcome = match result {
        Err(reason) => Err(reason),
        Ok(handle) => {
            let execution = async {
                let mut channel = handle
                    .channel_open_session()
                    .await
                    .map_err(|_| "exec channel failed".to_string())?;
                channel
                    .exec(true, command.as_bytes())
                    .await
                    .map_err(|_| "exec request failed".to_string())?;
                // No PTY, shell injection into an existing session, agent forwarding or stdin.
                channel
                    .eof()
                    .await
                    .map_err(|_| "exec stdin close failed".to_string())?;
                let mut output = BoundedOutput::new(OUTPUT_LIMIT);
                let mut exit_code = None;
                let mut truncated = false;
                while let Some(message) = channel.wait().await {
                    match message {
                        russh::ChannelMsg::Data { data } => truncated = output.push(false, &data),
                        russh::ChannelMsg::ExtendedData { data, .. } => {
                            truncated = output.push(true, &data)
                        }
                        russh::ChannelMsg::ExitStatus { exit_status } => {
                            exit_code = Some(exit_status)
                        }
                        russh::ChannelMsg::Failure => return Err("exec rejected".into()),
                        russh::ChannelMsg::Close => break,
                        _ => {}
                    }
                    {
                        let mut state = task
                            .state
                            .lock()
                            .map_err(|_| "scripts lock failed".to_string())?;
                        state.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                        state.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                        state.exit_code = exit_code;
                        state.truncated = truncated;
                    }
                    if truncated {
                        break;
                    }
                }
                Ok((truncated, exit_code))
            };
            let result = controlled(
                execution,
                task.cancel.clone(),
                deadline.saturating_duration_since(tokio::time::Instant::now()),
            )
            .await;
            // Disconnect the dedicated connection for every outcome. Closing SSH cannot
            // guarantee termination of a remote command that deliberately daemonizes.
            let _ = tokio::time::timeout(
                Duration::from_secs(2),
                handle.disconnect(russh::Disconnect::ByApplication, "script finished", "en"),
            )
            .await;
            result
        }
    };
    if let Ok(mut state) = task.state.lock() {
        state.status = match outcome {
            Ok((true, _)) => "output_limit",
            Ok((false, Some(_))) => "completed",
            Ok((false, None)) => {
                state.error = Some("Connection closed without exit status".into());
                "failed"
            }
            Err(StopReason::Cancelled) => "cancelled",
            Err(StopReason::TimedOut) => "timed_out",
            Err(StopReason::Failed) => {
                state.error = Some(
                    "SSH connection or exec failed; check host trust and authentication".into(),
                );
                "failed"
            }
        }
        .into();
    }
    task.done.cancel();
}
