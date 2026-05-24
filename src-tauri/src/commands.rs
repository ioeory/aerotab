//! Wiring layer: registers IPC methods on a [`Dispatcher`] and binds them to
//! the in-process [`SessionManager`].
//!
//! Method surface (v1):
//!
//! | method                  | params                              | result            |
//! |-------------------------|-------------------------------------|-------------------|
//! | `core.version`          | none                                | `{ version }`     |
//! | `core.protocolVersion`  | none                                | `u32`             |
//! | `session.list`          | none                                | `[SessionMeta]`   |
//! | `session.openLocal`     | `{ title?, rows?, cols? }`          | `SessionMeta`     |
//! | `session.write`         | `{ id, data }` (data = base64)      | `null`            |
//! | `session.resize`        | `{ id, rows, cols }`                | `null`            |
//! | `session.close`         | `{ id }`                            | `null`            |
//! | `session.pollOutput`    | `{ id, max_chunks? }`               | `[base64 chunk]`  |
//! | `session.openSerial`    | `{ title?, profile }`               | `SessionMeta`     |
//! | `serial.listPorts`      | none                                | `[string]`        |
//! | `ssh.hostStats`         | `{ profile }`                       | `HostStats`       |
//! | `sftp.open`             | `{ profile, sudo? }`                | `{ id }`          |
//! | `sftp.close`            | `{ id }`                            | `null`            |
//! | `sftp.list`             | `{ id, path }`                      | `[SftpEntry]`     |
//! | `sftp.read`             | `{ id, path }`                      | `{ data: b64 }`   |
//! | `sftp.write`            | `{ id, path, data: b64 }`           | `null`            |
//! | `sftp.stat`             | `{ id, path }`                      | `SftpEntry`       |
//! | `sftp.readChunk`        | `{ id, path, offset, len }`         | `{ data: b64 }`   |
//! | `sftp.writeChunk`       | `{ id, path, offset, data, create }`| `null`            |
//! | `sftp.mkdir`            | `{ id, path }`                      | `null`            |
//! | `sftp.removeFile`       | `{ id, path }`                      | `null`            |
//! | `sftp.removeDir`        | `{ id, path }`                      | `null`            |
//! | `sftp.rename`           | `{ id, from, to }`                  | `null`            |
//! | `sftp.realpath`         | `{ id, path }`                      | `{ path }`        |
//! | `settings.configure`    | `{ path }`                          | `null`            |
//! | `settings.get`          | `{ key }`                           | `{ value }`       |
//! | `settings.set`          | `{ key, value }`                    | `null`            |
//! | `settings.all`          | none                                | `[SettingEntry]`  |
//! | `settings.remove`       | `{ key }`                           | `{ removed }`     |
//! | `settings.reset`        | none                                | `null`            |
//! | `profile.healthCheck`   | `{ ids?, connect? }`                | `[ProfileHealth]` |
//! | `tunnel.open`           | `{ profile, kind, bind_*, target_* }`| `TunnelMeta`      |
//! | `tunnel.close`          | `{ id }`                            | `{ closed }`      |
//! | `tunnel.list`           | none                                | `[TunnelMeta]`    |
//!
//! `session.pollOutput` is a stop-gap poll API for testing without a true
//! event stream; the production transport (Tauri or websocket) will replace
//! it with a server-pushed `session.output` notification.

use std::collections::HashMap;
use std::sync::Arc;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{mpsc, Mutex};
use uuid::Uuid;

use crate::core::session_manager::{SessionId, SessionKind, SessionManager, SessionMeta};
use crate::ipc::{Dispatcher, ErrorCode, RpcError};
use crate::plugins::wasm_host::WasmHost;
use crate::profile::{Profile, ProfileKind, ProfileStore, RemoteDesktopSpec};
use crate::secret;
use crate::serial::{SerialChannel, SerialProfile};
use crate::settings::SettingsStore;
use crate::ssh::known_hosts::KnownHosts;
use crate::ssh::sftp::{Sftp, SftpOpenOptions};
use crate::remote;
use crate::ssh::tunnel::{TunnelKind, TunnelManager, TunnelOpenRequest};
use crate::ssh::{self, SshProfile, SshShell, X11ForwardOptions};
use crate::sync::oauth::{self, OAuthProvider};
use crate::sync::backends::git::GitBackend;
use crate::sync::backends::webdav::WebDavBackend;
use crate::sync::crypto::KdfParams;
use crate::sync::persistence::SledStore;
use crate::sync::{Group, RecordId, SyncEngine};
use crate::terminal::{PtyChannel, PtySize};

#[derive(Debug, Deserialize)]
struct OpenLocalParams {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    rows: Option<u16>,
    #[serde(default)]
    cols: Option<u16>,
    /// Optional absolute path to a shell executable. When None, the host
    /// default shell is used.
    #[serde(default)]
    shell: Option<String>,
    /// Optional arguments to pass to the shell (e.g. WSL `-d <distro>`).
    #[serde(default)]
    shell_args: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OpenSshParams {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    rows: Option<u16>,
    #[serde(default)]
    cols: Option<u16>,
    profile: SshProfile,
}

#[derive(Debug, Deserialize)]
struct OpenSshProfileParams {
    profile_id: Uuid,
    #[serde(default)]
    rows: Option<u16>,
    #[serde(default)]
    cols: Option<u16>,
}

#[derive(Debug, Deserialize)]
struct IdParam {
    id: Uuid,
}

#[derive(Debug, Deserialize)]
struct WriteParams {
    id: Uuid,
    /// Base64-encoded bytes (so binary data survives the JSON pipe).
    data: String,
}

#[derive(Debug, Deserialize)]
struct WriteManyParams {
    ids: Vec<Uuid>,
    data: String,
}

#[derive(Debug, Deserialize)]
struct ResizeParams {
    id: Uuid,
    rows: u16,
    cols: u16,
}

#[derive(Debug, Deserialize)]
struct PollOutputParams {
    id: Uuid,
    #[serde(default)]
    max_chunks: Option<usize>,
}

#[derive(Debug, Serialize)]
struct OpenLocalResult {
    #[serde(flatten)]
    meta: SessionMeta,
}

/// Process-wide session state.
#[derive(Default)]
pub struct AppState {
    pub session_manager: Mutex<SessionManager>,
    /// All open interactive sessions (local PTYs and SSH shells).
    pub channels: Mutex<HashMap<SessionId, SessionChannel>>,
    /// Configured sync engine, if any. Set via `sync.configureWebdav` /
    /// `sync.configureGit`.
    pub sync: Mutex<Option<Arc<SyncEngine>>>,
    /// Handle for the auto-sync background task, if running.
    pub sync_auto: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Auto-sync interval (ms) recorded by `sync.startAutoSync`.
    pub sync_auto_interval_ms: Mutex<Option<u64>>,
    /// Discriminator of the configured backend: "webdav" or "git". None
    /// when no engine is configured.
    pub sync_kind: Mutex<Option<&'static str>>,
    /// Wall-clock time (unix ms) of the most recent successful `sync.now`
    /// or auto-sync tick. None if no sync has completed yet.
    pub sync_last_ms: Mutex<Option<i64>>,
    /// Persistent host-key store. Configured via `ssh.configure`.
    pub known_hosts: Mutex<Option<KnownHosts>>,
    /// Persistent connection-profile store. Configured via `profile.configure`.
    pub profiles: Mutex<Option<ProfileStore>>,
    /// Most recently configured Git sync backend; retained so the engine
    /// can call `fetch_remote` / `push_remote` outside the abstract
    /// SyncBackend trait.
    pub git_backend: Mutex<Option<GitBackend>>,
    /// Open SFTP sessions, keyed by an opaque per-session id.
    pub sftp_sessions: Mutex<HashMap<Uuid, Arc<Sftp>>>,
    /// SSH port-forwarding tunnels (`-L` / `-R` / `-D`).
    pub tunnels: TunnelManager,
    /// Persistent settings store. Configured via `settings.configure`.
    pub settings: Mutex<Option<SettingsStore>>,
    /// Master-password vault (M10). Configured via `vault.configure`.
    pub vault: Mutex<Option<crate::vault::VaultStore>>,
    /// WASM plugin host. Configured via `plugin.configure`.
    pub wasm_host: Arc<WasmHost>,
}

/// Discriminated union over the channel kinds we know how to drive.
pub enum SessionChannel {
    Local(LocalPty),
    Ssh(SshSessionEntry),
    Serial(SerialSessionEntry),
}

pub struct LocalPty {
    pub channel: PtyChannel,
    pub rx: Mutex<mpsc::Receiver<Vec<u8>>>,
}

pub struct SshSessionEntry {
    pub shell: SshShell,
    pub rx: Mutex<mpsc::Receiver<Vec<u8>>>,
}

pub struct SerialSessionEntry {
    pub channel: SerialChannel,
    pub rx: Mutex<mpsc::Receiver<Vec<u8>>>,
}

impl AppState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

fn invalid_params(msg: impl Into<String>) -> RpcError {
    RpcError::new(ErrorCode::InvalidParams, msg)
}

fn internal(msg: impl Into<String>) -> RpcError {
    RpcError::new(ErrorCode::InternalError, msg)
}

fn session_not_found(id: SessionId) -> RpcError {
    RpcError::new(
        ErrorCode::SessionNotFound,
        format!("session not found: {}", id.0),
    )
}

/// Registers every method on `dispatcher`, capturing `state` by clone.
pub fn register_all(dispatcher: &Dispatcher, state: Arc<AppState>) {
    {
        dispatcher.register("core.version", |_p| async move {
            Ok(json!({ "version": crate::CORE_VERSION }))
        });
    }
    {
        dispatcher.register("core.protocolVersion", |_p| async move {
            Ok(json!(crate::ipc::PROTOCOL_VERSION))
        });
    }
    {
        let st = state.clone();
        dispatcher.register("session.list", move |_p| {
            let st = st.clone();
            async move {
                let sm = st.session_manager.lock().await;
                serde_json::to_value(sm.list()).map_err(|e| internal(e.to_string()))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("session.openLocal", move |params| {
            let st = st.clone();
            async move {
                let p: OpenLocalParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let size = PtySize {
                    rows: p.rows.unwrap_or(24),
                    cols: p.cols.unwrap_or(80),
                };
                let mut channel = if let Some(shell) = p.shell.as_deref() {
                    let mut cmd = portable_pty::CommandBuilder::new(shell);
                    for a in &p.shell_args {
                        cmd.arg(a);
                    }
                    crate::terminal::PtyChannel::spawn(cmd, size)
                        .map_err(|e| internal(e.to_string()))?
                } else {
                    PtyChannel::spawn_default_shell(size).map_err(|e| internal(e.to_string()))?
                };
                let rx = channel
                    .take_output()
                    .ok_or_else(|| internal("no output rx"))?;
                let meta = {
                    let mut sm = st.session_manager.lock().await;
                    sm.open(
                        SessionKind::LocalShell,
                        p.title.unwrap_or_else(|| "shell".into()),
                    )
                };
                st.channels.lock().await.insert(
                    meta.id,
                    SessionChannel::Local(LocalPty {
                        channel,
                        rx: Mutex::new(rx),
                    }),
                );
                serde_json::to_value(OpenLocalResult { meta }).map_err(|e| internal(e.to_string()))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("session.openSsh", move |params| {
            let st = st.clone();
            async move {
                let p: OpenSshParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let rows = p.rows.unwrap_or(24);
                let cols = p.cols.unwrap_or(80);
                let kh = st.known_hosts.lock().await.clone();
                let x11 = load_ssh_x11_options(&st).await;
                let mut shell = ssh::connect_shell_with_known_hosts(
                    &p.profile,
                    cols as u32,
                    rows as u32,
                    kh,
                    Some(x11),
                )
                .await
                .map_err(|e| internal(e.to_string()))?;
                let rx = shell
                    .take_output()
                    .ok_or_else(|| internal("no output rx"))?;
                let meta = {
                    let mut sm = st.session_manager.lock().await;
                    sm.open(
                        SessionKind::Ssh,
                        p.title
                            .unwrap_or_else(|| format!("{}@{}", p.profile.user, p.profile.host)),
                    )
                };
                st.channels.lock().await.insert(
                    meta.id,
                    SessionChannel::Ssh(SshSessionEntry {
                        shell,
                        rx: Mutex::new(rx),
                    }),
                );
                serde_json::to_value(OpenLocalResult { meta }).map_err(|e| internal(e.to_string()))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("session.openSshProfile", move |params| {
            let st = st.clone();
            async move {
                let p: OpenSshProfileParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let store = require_profiles(&st).await?;
                let profile = store
                    .get(p.profile_id)
                    .await
                    .map_err(|e| internal(e.to_string()))?
                    .ok_or_else(|| {
                        invalid_params(format!("profile not found: {}", p.profile_id))
                    })?;
                let rows = p.rows.unwrap_or(24);
                let cols = p.cols.unwrap_or(80);
                let kh = st.known_hosts.lock().await.clone();
                let x11 = load_ssh_x11_options(&st).await;
                let mut shell = match profile.spec {
                    ProfileKind::Ssh { ssh } => {
                        ssh::connect_shell_with_known_hosts(
                            &ssh,
                            cols as u32,
                            rows as u32,
                            kh,
                            Some(x11),
                        )
                        .await
                    }
                    ProfileKind::Rdp { .. } | ProfileKind::Vnc { .. } => {
                        return Err(invalid_params(
                            "profile is remote desktop; use remote.openProfile",
                        ));
                    }
                }
                .map_err(|e| internal(e.to_string()))?;
                let rx = shell
                    .take_output()
                    .ok_or_else(|| internal("no output rx"))?;
                let meta = {
                    let mut sm = st.session_manager.lock().await;
                    sm.open(SessionKind::Ssh, profile.name)
                };
                st.channels.lock().await.insert(
                    meta.id,
                    SessionChannel::Ssh(SshSessionEntry {
                        shell,
                        rx: Mutex::new(rx),
                    }),
                );
                serde_json::to_value(OpenLocalResult { meta }).map_err(|e| internal(e.to_string()))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("session.write", move |params| {
            let st = st.clone();
            async move {
                let p: WriteParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let bytes = BASE64
                    .decode(p.data.as_bytes())
                    .map_err(|e| invalid_params(format!("bad base64: {e}")))?;
                let id = SessionId(p.id);
                let chans = st.channels.lock().await;
                let chan = chans.get(&id).ok_or_else(|| session_not_found(id))?;
                match chan {
                    SessionChannel::Local(pty) => pty
                        .channel
                        .write(&bytes)
                        .map_err(|e| internal(e.to_string()))?,
                    SessionChannel::Ssh(s) => s
                        .shell
                        .write(&bytes)
                        .await
                        .map_err(|e| internal(e.to_string()))?,
                    SessionChannel::Serial(s) => s
                        .channel
                        .write(&bytes)
                        .map_err(|e| internal(e.to_string()))?,
                }
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("session.writeMany", move |params| {
            let st = st.clone();
            async move {
                let p: WriteManyParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                if p.ids.is_empty() {
                    return Ok(json!({ "written": 0 }));
                }
                let bytes = BASE64
                    .decode(p.data.as_bytes())
                    .map_err(|e| invalid_params(format!("bad base64: {e}")))?;
                let mut joins = Vec::new();
                for raw_id in p.ids {
                    let stc = st.clone();
                    let data = bytes.clone();
                    joins.push(tokio::spawn(async move {
                        let id = SessionId(raw_id);
                        let chans = stc.channels.lock().await;
                        let Some(chan) = chans.get(&id) else {
                            return false;
                        };
                        match chan {
                            SessionChannel::Local(pty) => pty.channel.write(&data).is_ok(),
                            SessionChannel::Ssh(s) => s.shell.write(&data).await.is_ok(),
                            SessionChannel::Serial(s) => s.channel.write(&data).is_ok(),
                        }
                    }));
                }
                let mut written = 0usize;
                for join in joins {
                    if join.await.ok() == Some(true) {
                        written += 1;
                    }
                }
                Ok(json!({ "written": written }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("session.resize", move |params| {
            let st = st.clone();
            async move {
                let p: ResizeParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let id = SessionId(p.id);
                let chans = st.channels.lock().await;
                let chan = chans.get(&id).ok_or_else(|| session_not_found(id))?;
                match chan {
                    SessionChannel::Local(pty) => pty
                        .channel
                        .resize(PtySize {
                            rows: p.rows,
                            cols: p.cols,
                        })
                        .map_err(|e| internal(e.to_string()))?,
                    SessionChannel::Ssh(s) => s
                        .shell
                        .resize(p.cols as u32, p.rows as u32)
                        .await
                        .map_err(|e| internal(e.to_string()))?,
                    SessionChannel::Serial(_) => {
                        // Serial has no concept of terminal geometry; ignore.
                    }
                }
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("session.close", move |params| {
            let st = st.clone();
            async move {
                let p: IdParam =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let id = SessionId(p.id);
                let removed = st.channels.lock().await.remove(&id);
                if let Some(chan) = removed {
                    match chan {
                        SessionChannel::Local(mut pty) => {
                            let _ = pty.channel.kill();
                        }
                        SessionChannel::Ssh(s) => {
                            let _ = s.shell.close().await;
                        }
                        SessionChannel::Serial(_) => {
                            // Dropping the channel closes the underlying port and ends the reader thread.
                        }
                    }
                }
                st.session_manager.lock().await.close(id);
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("session.pollOutput", move |params| {
            let st = st.clone();
            async move {
                let p: PollOutputParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let id = SessionId(p.id);
                let max = p.max_chunks.unwrap_or(16);
                let chans = st.channels.lock().await;
                let chan = chans.get(&id).ok_or_else(|| session_not_found(id))?;
                let rx_mtx = match chan {
                    SessionChannel::Local(pty) => &pty.rx,
                    SessionChannel::Ssh(s) => &s.rx,
                    SessionChannel::Serial(s) => &s.rx,
                };
                let mut rx = rx_mtx.lock().await;
                let mut chunks = Vec::new();
                for _ in 0..max {
                    match rx.try_recv() {
                        Ok(c) => chunks.push(BASE64.encode(&c)),
                        Err(_) => break,
                    }
                }
                Ok(Value::Array(
                    chunks.into_iter().map(Value::String).collect(),
                ))
            }
        });
    }
    {
        // Newer poll variant that also reports liveness so the UI can show
        // "process exited" instead of silently sitting on a dead session.
        let st = state.clone();
        dispatcher.register("session.poll", move |params| {
            let st = st.clone();
            async move {
                let p: PollOutputParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let id = SessionId(p.id);
                let max = p.max_chunks.unwrap_or(16);
                let mut chans = st.channels.lock().await;
                let Some(chan) = chans.get_mut(&id) else {
                    return Ok(json!({ "chunks": Value::Array(vec![]), "alive": false }));
                };
                let (rx_mtx, mut alive) = match chan {
                    SessionChannel::Local(pty) => {
                        let alive = matches!(pty.channel.try_wait(), Ok(None));
                        (&pty.rx, alive)
                    }
                    SessionChannel::Ssh(s) => (&s.rx, true),
                    SessionChannel::Serial(s) => (&s.rx, true),
                };
                let mut rx = rx_mtx.lock().await;
                let mut chunks = Vec::new();
                use tokio::sync::mpsc::error::TryRecvError;
                for _ in 0..max {
                    match rx.try_recv() {
                        Ok(c) => chunks.push(BASE64.encode(&c)),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            alive = false;
                            break;
                        }
                    }
                }
                Ok(json!({
                    "chunks": chunks,
                    "alive": alive,
                }))
            }
        });
    }
    {
        dispatcher.register("serial.listPorts", |_p| async move {
            serde_json::to_value(crate::serial::list_ports()).map_err(|e| internal(e.to_string()))
        });
    }
    {
        let st = state.clone();
        dispatcher.register("session.openSerial", move |params| {
            let st = st.clone();
            async move {
                #[derive(Debug, Deserialize)]
                struct OpenSerialParams {
                    #[serde(default)]
                    title: Option<String>,
                    profile: SerialProfile,
                }
                let p: OpenSerialParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let mut channel =
                    SerialChannel::open(&p.profile).map_err(|e| internal(e.to_string()))?;
                let rx = channel
                    .take_output()
                    .ok_or_else(|| internal("no output rx"))?;
                let meta = {
                    let mut sm = st.session_manager.lock().await;
                    sm.open(
                        SessionKind::Serial,
                        p.title.unwrap_or_else(|| p.profile.port.clone()),
                    )
                };
                st.channels.lock().await.insert(
                    meta.id,
                    SessionChannel::Serial(SerialSessionEntry {
                        channel,
                        rx: Mutex::new(rx),
                    }),
                );
                serde_json::to_value(OpenLocalResult { meta }).map_err(|e| internal(e.to_string()))
            }
        });
    }
    register_secret(dispatcher, state.clone());
    register_profiles(dispatcher, state.clone());
    register_known_hosts(dispatcher, state.clone());
    register_ssh_stats(dispatcher, state.clone());
    register_sftp(dispatcher, state.clone());
    register_tunnel(dispatcher, state.clone());
    register_oauth(dispatcher, state.clone());
    register_remote(dispatcher, state.clone());
    register_settings(dispatcher, state.clone());
    register_vault(dispatcher, state.clone());
    register_plugins(dispatcher, state.clone());
    register_sync(dispatcher, state);
}

#[derive(Debug, Deserialize)]
struct ConfigureWebDavParams {
    base_url: String,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    password: Option<String>,
    /// Master password for E2E encryption.
    master_password: String,
    /// Optional device id override (for tests). Random UUID otherwise.
    #[serde(default)]
    device_id: Option<Uuid>,
    /// If true, use cheap Argon2id params (tests / smoke).
    #[serde(default)]
    test_cheap_kdf: bool,
    /// Optional sled state directory. If provided, local state survives
    /// process restart.
    #[serde(default)]
    state_dir: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConfigureGitParams {
    repo_path: String,
    master_password: String,
    #[serde(default)]
    device_id: Option<Uuid>,
    #[serde(default)]
    test_cheap_kdf: bool,
    #[serde(default)]
    state_dir: Option<String>,
    #[serde(default)]
    author_name: Option<String>,
    #[serde(default)]
    author_email: Option<String>,
    /// Optional remote URL. If supplied, sync.now will fetch+push.
    #[serde(default)]
    remote_url: Option<String>,
    #[serde(default = "default_remote_name")]
    remote_name: String,
    #[serde(default = "default_remote_branch")]
    remote_branch: String,
    #[serde(default)]
    remote_user: Option<String>,
    #[serde(default)]
    remote_password: Option<String>,
    #[serde(default)]
    remote_ssh_key: Option<String>,
    #[serde(default)]
    remote_ssh_passphrase: Option<String>,
    /// `github` or `gitlab` — load access token from OS keyring.
    #[serde(default)]
    oauth_provider: Option<String>,
}

fn default_remote_name() -> String {
    "origin".into()
}
fn default_remote_branch() -> String {
    "master".into()
}

#[derive(Debug, Deserialize)]
struct StartAutoSyncParams {
    /// Tick interval in milliseconds. Must be > 0.
    interval_ms: u64,
    /// Optional selected sync groups. Empty means all groups.
    #[serde(default)]
    groups: Vec<Group>,
}

#[derive(Debug, Default, Deserialize)]
struct SyncNowParams {
    /// Optional selected sync groups. Empty means all groups.
    #[serde(default)]
    groups: Vec<Group>,
}

fn selected_sync_groups(groups: Vec<Group>) -> Vec<Group> {
    let mut out = Vec::new();
    for group in groups {
        if !out.contains(&group) {
            out.push(group);
        }
    }
    if out.is_empty() {
        Group::ALL.to_vec()
    } else {
        out
    }
}

#[derive(Debug, Deserialize)]
struct SyncRecordParams {
    group: Group,
    id: Uuid,
}

#[derive(Debug, Deserialize)]
struct SyncPutParams {
    group: Group,
    id: Uuid,
    /// Base64-encoded payload bytes.
    data: String,
}

#[derive(Debug, Deserialize)]
struct SyncListParams {
    group: Group,
}

async fn require_engine(st: &AppState) -> Result<Arc<SyncEngine>, RpcError> {
    st.sync
        .lock()
        .await
        .clone()
        .ok_or_else(|| RpcError::new(ErrorCode::InvalidParams, "sync not configured"))
}

// --- secret.* -------------------------------------------------------------

fn register_secret(dispatcher: &Dispatcher, _state: Arc<AppState>) {
    #[derive(Debug, Deserialize)]
    struct AcctSecret {
        #[serde(default)]
        account: Option<String>,
        secret: String,
    }
    #[derive(Debug, Deserialize)]
    struct Acct {
        #[serde(default)]
        account: Option<String>,
    }

    dispatcher.register("secret.setMaster", |params| async move {
        let p: AcctSecret =
            serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
        secret::set_master(p.account.as_deref(), &p.secret).map_err(|e| internal(e.to_string()))?;
        Ok(Value::Null)
    });
    dispatcher.register("secret.getMaster", |params| async move {
        let p: Acct = serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
        let s = secret::get_master(p.account.as_deref()).map_err(|e| internal(e.to_string()))?;
        Ok(json!({ "secret": s }))
    });
    dispatcher.register("secret.hasMaster", |params| async move {
        let p: Acct = serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
        Ok(json!({ "has": secret::has_master(p.account.as_deref()) }))
    });
    dispatcher.register("secret.clearMaster", |params| async move {
        let p: Acct = serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
        secret::clear_master(p.account.as_deref()).map_err(|e| internal(e.to_string()))?;
        Ok(Value::Null)
    });
}

// --- profile.* ------------------------------------------------------------

async fn require_profiles(state: &AppState) -> Result<ProfileStore, RpcError> {
    state
        .profiles
        .lock()
        .await
        .clone()
        .ok_or_else(|| RpcError::new(ErrorCode::InvalidParams, "profile store not configured"))
}

fn register_profiles(dispatcher: &Dispatcher, state: Arc<AppState>) {
    #[derive(Debug, Deserialize)]
    struct ConfigureParams {
        path: String,
    }
    #[derive(Debug, Deserialize)]
    struct IdParams {
        id: Uuid,
    }
    #[derive(Debug, Deserialize)]
    struct HealthCheckParams {
        #[serde(default)]
        ids: Vec<Uuid>,
        #[serde(default)]
        connect: bool,
    }

    {
        let st = state.clone();
        dispatcher.register("profile.configure", move |params| {
            let st = st.clone();
            async move {
                let p: ConfigureParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let store = ProfileStore::open(&p.path).map_err(|e| internal(e.to_string()))?;
                *st.profiles.lock().await = Some(store);
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("profile.list", move |_params| {
            let st = st.clone();
            async move {
                let store = require_profiles(&st).await?;
                let list = store.list().await.map_err(|e| internal(e.to_string()))?;
                serde_json::to_value(list).map_err(|e| internal(e.to_string()))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("profile.get", move |params| {
            let st = st.clone();
            async move {
                let p: IdParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let store = require_profiles(&st).await?;
                let v = store.get(p.id).await.map_err(|e| internal(e.to_string()))?;
                serde_json::to_value(v).map_err(|e| internal(e.to_string()))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("profile.upsert", move |params| {
            let st = st.clone();
            async move {
                let profile: Profile =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let store = require_profiles(&st).await?;
                store
                    .upsert(profile)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("profile.delete", move |params| {
            let st = st.clone();
            async move {
                let p: IdParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let store = require_profiles(&st).await?;
                let removed = store
                    .delete(p.id)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                Ok(json!({ "removed": removed }))
            }
        });
    }
    {
        // M2 — built-in shells + ~/.ssh/config import. Stateless; doesn't
        // require the profile store to be configured.
        dispatcher.register("profile.discover", move |_params| async move {
            let shells = crate::shell_detect::detect();
            let ssh_config = crate::ssh_config::load_default();
            Ok(json!({
                "shells": shells,
                "sshConfig": ssh_config,
            }))
        });
    }
    {
        let st = state.clone();
        dispatcher.register("profile.healthCheck", move |params| {
            let st = st.clone();
            async move {
                let p: HealthCheckParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let store = require_profiles(&st).await?;
                let mut profiles = store.list().await.map_err(|e| internal(e.to_string()))?;
                if !p.ids.is_empty() {
                    profiles.retain(|profile| p.ids.contains(&profile.id));
                }
                let kh = st.known_hosts.lock().await.clone();
                let results = crate::profile_health::check_profiles(profiles, kh, p.connect).await;
                serde_json::to_value(results).map_err(|e| internal(e.to_string()))
            }
        });
    }
}

// --- ssh.knownHosts.* -----------------------------------------------------

fn register_known_hosts(dispatcher: &Dispatcher, state: Arc<AppState>) {
    #[derive(Debug, Deserialize)]
    struct ConfigureParams {
        dir: String,
    }
    #[derive(Debug, Deserialize)]
    struct HostParams {
        host: String,
    }

    {
        let st = state.clone();
        dispatcher.register("ssh.knownHosts.configure", move |params| {
            let st = st.clone();
            async move {
                let p: ConfigureParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let kh = KnownHosts::open(&p.dir).map_err(|e| internal(e.to_string()))?;
                *st.known_hosts.lock().await = Some(kh);
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("ssh.knownHosts.list", move |_params| {
            let st = st.clone();
            async move {
                let kh = match st.known_hosts.lock().await.clone() {
                    Some(k) => k,
                    None => return Ok(Value::Array(vec![])),
                };
                let entries = kh
                    .list()
                    .into_iter()
                    .map(|(host, key)| {
                        json!({
                            "host": host,
                            "key_type": key.key_type,
                            "key_b64": key.key_b64,
                        })
                    })
                    .collect::<Vec<_>>();
                Ok(Value::Array(entries))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("ssh.knownHosts.remove", move |params| {
            let st = st.clone();
            async move {
                let p: HostParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let kh = st
                    .known_hosts
                    .lock()
                    .await
                    .clone()
                    .ok_or_else(|| invalid_params("known_hosts not configured"))?;
                let removed = kh.remove(&p.host).map_err(|e| internal(e.to_string()))?;
                Ok(json!({ "removed": removed }))
            }
        });
    }
}

// --- ssh.hostStats -------------------------------------------------------

#[derive(Debug, Deserialize)]
struct HostStatsParams {
    profile: SshProfile,
}

fn register_ssh_stats(dispatcher: &Dispatcher, state: Arc<AppState>) {
    let st = state.clone();
    dispatcher.register("ssh.hostStats", move |params| {
        let st = st.clone();
        async move {
            let p: HostStatsParams =
                serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
            let kh = st.known_hosts.lock().await.clone();
            let stats = ssh::stats::probe_host_stats(&p.profile, kh)
                .await
                .map_err(|e| internal(e.to_string()))?;
            serde_json::to_value(stats).map_err(|e| internal(e.to_string()))
        }
    });
}

// --- sftp.* ---------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct SftpOpenParams {
    profile: SshProfile,
    #[serde(default)]
    sudo: bool,
}

#[derive(Debug, Deserialize)]
struct SftpIdParams {
    id: Uuid,
}

#[derive(Debug, Deserialize)]
struct SftpListParams {
    id: Uuid,
    path: String,
}

#[derive(Debug, Deserialize)]
struct SftpReadParams {
    id: Uuid,
    path: String,
}

#[derive(Debug, Deserialize)]
struct SftpWriteParams {
    id: Uuid,
    path: String,
    /// Base64-encoded bytes.
    data: String,
}

#[derive(Debug, Deserialize)]
struct SftpReadChunkParams {
    id: Uuid,
    path: String,
    offset: u64,
    len: u32,
}

#[derive(Debug, Deserialize)]
struct SftpWriteChunkParams {
    id: Uuid,
    path: String,
    offset: u64,
    /// Base64-encoded bytes.
    data: String,
    #[serde(default)]
    create: bool,
}

#[derive(Debug, Deserialize)]
struct SftpPathParams {
    id: Uuid,
    path: String,
}

#[derive(Debug, Deserialize)]
struct SftpRenameParams {
    id: Uuid,
    from: String,
    to: String,
}

async fn require_sftp(state: &AppState, id: Uuid) -> Result<Arc<Sftp>, RpcError> {
    state
        .sftp_sessions
        .lock()
        .await
        .get(&id)
        .cloned()
        .ok_or_else(|| RpcError::new(ErrorCode::SessionNotFound, format!("sftp session {id}")))
}

fn register_sftp(dispatcher: &Dispatcher, state: Arc<AppState>) {
    {
        let st = state.clone();
        dispatcher.register("sftp.open", move |params| {
            let st = st.clone();
            async move {
                let p: SftpOpenParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let kh = st.known_hosts.lock().await.clone();
                let sftp =
                    Sftp::open_with_options(&p.profile, kh, SftpOpenOptions { sudo: p.sudo })
                        .await
                        .map_err(|e| internal(e.to_string()))?;
                let id = Uuid::new_v4();
                st.sftp_sessions.lock().await.insert(id, Arc::new(sftp));
                Ok(json!({ "id": id }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sftp.close", move |params| {
            let st = st.clone();
            async move {
                let p: SftpIdParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                st.sftp_sessions.lock().await.remove(&p.id);
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sftp.list", move |params| {
            let st = st.clone();
            async move {
                let p: SftpListParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let sftp = require_sftp(&st, p.id).await?;
                let entries = sftp
                    .read_dir(&p.path)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                serde_json::to_value(entries).map_err(|e| internal(e.to_string()))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sftp.read", move |params| {
            let st = st.clone();
            async move {
                let p: SftpReadParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let sftp = require_sftp(&st, p.id).await?;
                let bytes = sftp
                    .read_file(&p.path)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                Ok(json!({ "data": BASE64.encode(&bytes) }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sftp.write", move |params| {
            let st = st.clone();
            async move {
                let p: SftpWriteParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let sftp = require_sftp(&st, p.id).await?;
                let bytes = BASE64
                    .decode(p.data.as_bytes())
                    .map_err(|e| invalid_params(format!("bad base64: {e}")))?;
                sftp.write_file(&p.path, &bytes)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sftp.stat", move |params| {
            let st = st.clone();
            async move {
                let p: SftpPathParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let sftp = require_sftp(&st, p.id).await?;
                let entry = sftp
                    .stat(&p.path)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                serde_json::to_value(entry).map_err(|e| internal(e.to_string()))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sftp.readChunk", move |params| {
            let st = st.clone();
            async move {
                let p: SftpReadChunkParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let sftp = require_sftp(&st, p.id).await?;
                let bytes = sftp
                    .read_file_chunk(&p.path, p.offset, p.len)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                Ok(json!({ "data": BASE64.encode(&bytes) }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sftp.writeChunk", move |params| {
            let st = st.clone();
            async move {
                let p: SftpWriteChunkParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let sftp = require_sftp(&st, p.id).await?;
                let bytes = BASE64
                    .decode(p.data.as_bytes())
                    .map_err(|e| invalid_params(format!("bad base64: {e}")))?;
                sftp.write_file_chunk(&p.path, p.offset, &bytes, p.create)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sftp.mkdir", move |params| {
            let st = st.clone();
            async move {
                let p: SftpPathParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let sftp = require_sftp(&st, p.id).await?;
                sftp.mkdir(&p.path)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sftp.removeFile", move |params| {
            let st = st.clone();
            async move {
                let p: SftpPathParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let sftp = require_sftp(&st, p.id).await?;
                sftp.remove_file(&p.path)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sftp.removeDir", move |params| {
            let st = st.clone();
            async move {
                let p: SftpPathParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let sftp = require_sftp(&st, p.id).await?;
                sftp.remove_dir(&p.path)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sftp.rename", move |params| {
            let st = st.clone();
            async move {
                let p: SftpRenameParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let sftp = require_sftp(&st, p.id).await?;
                sftp.rename(&p.from, &p.to)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sftp.realpath", move |params| {
            let st = st.clone();
            async move {
                let p: SftpPathParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let sftp = require_sftp(&st, p.id).await?;
                let real = sftp
                    .canonicalize(&p.path)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                Ok(json!({ "path": real }))
            }
        });
    }
}

async fn load_ssh_x11_options(st: &AppState) -> X11ForwardOptions {
    let Ok(store) = require_settings(st).await else {
        return X11ForwardOptions::default();
    };
    let Ok(entry) = store.get("ssh") else {
        return X11ForwardOptions::default();
    };
    let Some(value) = entry else {
        return X11ForwardOptions::default();
    };
    let Ok(v) = serde_json::from_value::<serde_json::Value>(value) else {
        return X11ForwardOptions::default();
    };
    X11ForwardOptions {
        enabled: v
            .get("x11Forwarding")
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
    }
}

fn parse_oauth_provider(raw: &str) -> Result<OAuthProvider, RpcError> {
    match raw.to_ascii_lowercase().as_str() {
        "github" => Ok(OAuthProvider::Github),
        "gitlab" => Ok(OAuthProvider::Gitlab),
        other => Err(invalid_params(format!("unknown oauth provider: {other}"))),
    }
}

fn oauth_https_credentials(provider: OAuthProvider, token: String) -> (String, String) {
    match provider {
        OAuthProvider::Github => ("x-access-token".into(), token),
        OAuthProvider::Gitlab => ("oauth2".into(), token),
    }
}

async fn resolve_ssh_profile(st: &AppState, profile_id: Uuid) -> Result<SshProfile, RpcError> {
    let store = require_profiles(st).await?;
    let profile = store
        .get(profile_id)
        .await
        .map_err(|e| internal(e.to_string()))?
        .ok_or_else(|| invalid_params(format!("profile not found: {profile_id}")))?;
    match profile.spec {
        ProfileKind::Ssh { ssh } => Ok(ssh),
        _ => Err(invalid_params("profile is not SSH")),
    }
}

async fn open_remote_desktop(
    st: &AppState,
    kind: &str,
    spec: RemoteDesktopSpec,
) -> Result<Value, RpcError> {
    let kind_lc = kind.to_ascii_lowercase();
    let mut local_port = if spec.local_bind_port > 0 {
        spec.local_bind_port
    } else {
        0
    };
    let mut tunnel_id = None;
    if let Some(pid) = spec.ssh_profile_id {
        let ssh = resolve_ssh_profile(st, pid).await?;
        let kh = st.known_hosts.lock().await.clone();
        let bind_port = if local_port > 0 {
            local_port
        } else {
            0
        };
        let meta = st
            .tunnels
            .open(
                TunnelOpenRequest {
                    profile: ssh,
                    kind: TunnelKind::Local,
                    bind_host: "127.0.0.1".into(),
                    bind_port,
                    target_host: spec.host.clone(),
                    target_port: spec.port,
                },
                kh,
            )
            .await
            .map_err(|e| internal(e.to_string()))?;
        local_port = meta.bind_port;
        tunnel_id = Some(meta.id);
    } else if local_port == 0 {
        local_port = spec.port;
    }
    let address = if tunnel_id.is_some() {
        format!("127.0.0.1:{local_port}")
    } else {
        format!("{}:{}", spec.host, spec.port)
    };
    remote::launch_viewer(&kind_lc, &address).map_err(|e| internal(e.to_string()))?;
    Ok(json!({
        "local_address": address,
        "tunnel_id": tunnel_id,
    }))
}

// --- tunnel.* ------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TunnelOpenParams {
    profile: SshProfile,
    kind: TunnelKind,
    bind_host: String,
    bind_port: u16,
    target_host: String,
    target_port: u16,
}

#[derive(Debug, Deserialize)]
struct TunnelIdParams {
    id: Uuid,
}

fn register_tunnel(dispatcher: &Dispatcher, state: Arc<AppState>) {
    {
        let st = state.clone();
        dispatcher.register("tunnel.open", move |params| {
            let st = st.clone();
            async move {
                let p: TunnelOpenParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let kh = st.known_hosts.lock().await.clone();
                let req = TunnelOpenRequest {
                    profile: p.profile,
                    kind: p.kind,
                    bind_host: p.bind_host,
                    bind_port: p.bind_port,
                    target_host: p.target_host,
                    target_port: p.target_port,
                };
                let meta = st
                    .tunnels
                    .open(req, kh)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                serde_json::to_value(meta).map_err(|e| internal(e.to_string()))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("tunnel.close", move |params| {
            let st = st.clone();
            async move {
                let p: TunnelIdParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let removed = st.tunnels.close(p.id).await;
                Ok(json!({ "closed": removed }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("tunnel.list", move |_params| {
            let st = st.clone();
            async move {
                let list = st.tunnels.list().await;
                serde_json::to_value(list).map_err(|e| internal(e.to_string()))
            }
        });
    }
}

// --- sync.oauth.* --------------------------------------------------------

#[derive(Debug, Deserialize)]
struct OAuthDeviceStartParams {
    provider: String,
    client_id: String,
    #[serde(default)]
    gitlab_base_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthDevicePollParams {
    provider: String,
    client_id: String,
    device_code: String,
    #[serde(default)]
    gitlab_base_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthProviderParams {
    provider: String,
}

fn register_oauth(dispatcher: &Dispatcher, _state: Arc<AppState>) {
    dispatcher.register("sync.oauthDeviceStart", move |params| async move {
        let p: OAuthDeviceStartParams =
            serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
        let prov = parse_oauth_provider(&p.provider)?;
        let start = oauth::device_start(prov, &p.client_id, p.gitlab_base_url.as_deref())
            .await
            .map_err(|e| internal(e.to_string()))?;
        serde_json::to_value(start).map_err(|e| internal(e.to_string()))
    });
    dispatcher.register("sync.oauthDevicePoll", move |params| async move {
        let p: OAuthDevicePollParams =
            serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
        let prov = parse_oauth_provider(&p.provider)?;
        match oauth::device_poll(
            prov,
            &p.client_id,
            &p.device_code,
            p.gitlab_base_url.as_deref(),
        )
        .await
        {
            Ok(token) => Ok(json!({ "status": "ok", "token_len": token.len() })),
            Err(oauth::OAuthError::Pending) => Ok(json!({ "status": "pending" })),
            Err(oauth::OAuthError::SlowDown) => Ok(json!({ "status": "slow_down" })),
            Err(e) => Err(internal(e.to_string())),
        }
    });
    dispatcher.register("sync.oauthStatus", move |params| async move {
        let p: OAuthProviderParams =
            serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
        let prov = parse_oauth_provider(&p.provider)?;
        let connected = oauth::load_token(prov)
            .map_err(|e| internal(e.to_string()))?
            .is_some();
        serde_json::to_value(oauth::OAuthStatus { provider: prov, connected })
            .map_err(|e| internal(e.to_string()))
    });
    dispatcher.register("sync.oauthClear", move |params| async move {
        let p: OAuthProviderParams =
            serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
        let prov = parse_oauth_provider(&p.provider)?;
        oauth::clear_token(prov)
            .map_err(|e| internal(e.to_string()))?;
        Ok(json!({ "cleared": true }))
    });
}

// --- remote.* ------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RemoteOpenParams {
    kind: String,
    host: String,
    port: u16,
    #[serde(default)]
    ssh_profile_id: Option<Uuid>,
    #[serde(default)]
    local_bind_port: u16,
}

#[derive(Debug, Deserialize)]
struct RemoteOpenProfileParams {
    profile_id: Uuid,
}

fn register_remote(dispatcher: &Dispatcher, state: Arc<AppState>) {
    {
        let st = state.clone();
        dispatcher.register("remote.open", move |params| {
            let st = st.clone();
            async move {
                let p: RemoteOpenParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                open_remote_desktop(
                    &st,
                    &p.kind,
                    RemoteDesktopSpec {
                        host: p.host,
                        port: p.port,
                        ssh_profile_id: p.ssh_profile_id,
                        local_bind_port: p.local_bind_port,
                    },
                )
                .await
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("remote.openProfile", move |params| {
            let st = st.clone();
            async move {
                let p: RemoteOpenProfileParams = serde_json::from_value(params)
                    .map_err(|e| invalid_params(e.to_string()))?;
                let store = require_profiles(&st).await?;
                let profile = store
                    .get(p.profile_id)
                    .await
                    .map_err(|e| internal(e.to_string()))?
                    .ok_or_else(|| {
                        invalid_params(format!("profile not found: {}", p.profile_id))
                    })?;
                match profile.spec {
                    ProfileKind::Rdp { rdp } => open_remote_desktop(&st, "rdp", rdp).await,
                    ProfileKind::Vnc { spec } => open_remote_desktop(&st, "vnc", spec).await,
                    ProfileKind::Ssh { .. } => {
                        Err(invalid_params("profile is SSH; use session.openSshProfile"))
                    }
                }
            }
        });
    }
}

// --- settings.* ----------------------------------------------------------

async fn require_settings(state: &AppState) -> Result<SettingsStore, RpcError> {
    state
        .settings
        .lock()
        .await
        .clone()
        .ok_or_else(|| RpcError::new(ErrorCode::InvalidParams, "settings not configured"))
}

fn register_settings(dispatcher: &Dispatcher, state: Arc<AppState>) {
    #[derive(Debug, Deserialize)]
    struct ConfigureParams {
        path: String,
    }
    #[derive(Debug, Deserialize)]
    struct KeyParams {
        key: String,
    }
    #[derive(Debug, Deserialize)]
    struct SetParams {
        key: String,
        value: Value,
    }

    {
        let st = state.clone();
        dispatcher.register("settings.configure", move |params| {
            let st = st.clone();
            async move {
                let p: ConfigureParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let store = SettingsStore::open(&p.path).map_err(|e| internal(e.to_string()))?;
                *st.settings.lock().await = Some(store);
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("settings.get", move |params| {
            let st = st.clone();
            async move {
                let p: KeyParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let store = require_settings(&st).await?;
                let v = store.get(&p.key).map_err(|e| internal(e.to_string()))?;
                Ok(json!({ "value": v }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("settings.set", move |params| {
            let st = st.clone();
            async move {
                let p: SetParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let store = require_settings(&st).await?;
                store
                    .set(&p.key, &p.value)
                    .map_err(|e| internal(e.to_string()))?;
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("settings.all", move |_params| {
            let st = st.clone();
            async move {
                let store = require_settings(&st).await?;
                let entries = store.all().map_err(|e| internal(e.to_string()))?;
                serde_json::to_value(entries).map_err(|e| internal(e.to_string()))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("settings.remove", move |params| {
            let st = st.clone();
            async move {
                let p: KeyParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let store = require_settings(&st).await?;
                let removed = store.remove(&p.key).map_err(|e| internal(e.to_string()))?;
                Ok(json!({ "removed": removed }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("settings.reset", move |_params| {
            let st = st.clone();
            async move {
                let store = require_settings(&st).await?;
                store.reset().map_err(|e| internal(e.to_string()))?;
                Ok(Value::Null)
            }
        });
    }
    // --- M8 — full-document YAML editor ---
    {
        let st = state.clone();
        dispatcher.register("settings.dumpYaml", move |_params| {
            let st = st.clone();
            async move {
                let store = require_settings(&st).await?;
                let entries = store.all().map_err(|e| internal(e.to_string()))?;
                let mut map = serde_yaml::Mapping::new();
                for e in entries {
                    let v: serde_yaml::Value =
                        serde_yaml::to_value(&e.value).map_err(|err| internal(err.to_string()))?;
                    map.insert(serde_yaml::Value::String(e.key), v);
                }
                let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(map))
                    .map_err(|err| internal(err.to_string()))?;
                Ok(json!({ "yaml": yaml }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("settings.loadYaml", move |params| {
            let st = st.clone();
            async move {
                #[derive(Deserialize)]
                struct P {
                    yaml: String,
                    #[serde(default)]
                    replace: bool,
                }
                let p: P =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let parsed: serde_yaml::Value = serde_yaml::from_str(&p.yaml)
                    .map_err(|e| invalid_params(format!("yaml parse: {e}")))?;
                let map = match parsed {
                    serde_yaml::Value::Mapping(m) => m,
                    serde_yaml::Value::Null => serde_yaml::Mapping::new(),
                    _ => return Err(invalid_params("top-level YAML must be a mapping")),
                };
                let store = require_settings(&st).await?;
                if p.replace {
                    store.reset().map_err(|e| internal(e.to_string()))?;
                }
                let mut written = 0usize;
                for (k, v) in map {
                    let key = match k {
                        serde_yaml::Value::String(s) => s,
                        other => serde_yaml::to_string(&other)
                            .map_err(|e| invalid_params(e.to_string()))?
                            .trim()
                            .to_string(),
                    };
                    let json_v: Value =
                        serde_json::to_value(v).map_err(|e| internal(e.to_string()))?;
                    store
                        .set(&key, &json_v)
                        .map_err(|e| internal(e.to_string()))?;
                    written += 1;
                }
                Ok(json!({ "written": written }))
            }
        });
    }
}

// --- plugin.* ---

#[derive(Debug, Deserialize)]
struct PluginConfigureParams {
    /// Directory containing `*.wasm` plugin bundles.
    path: String,
}

#[derive(Debug, Deserialize)]
struct PluginInvokeParams {
    name: String,
    command: String,
    #[serde(default)]
    args: String,
}

#[derive(Debug, Deserialize)]
struct PluginLoadParams {
    /// Path to a single `.wasm` file to load on demand.
    path: String,
}

fn register_plugins(dispatcher: &Dispatcher, state: Arc<AppState>) {
    {
        let st = state.clone();
        dispatcher.register("plugin.configure", move |params| {
            let st = st.clone();
            async move {
                let p: PluginConfigureParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let n = st
                    .wasm_host
                    .load_dir(std::path::Path::new(&p.path))
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                Ok(json!({ "loaded": n }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("plugin.list", move |_params| {
            let st = st.clone();
            async move {
                let list = st.wasm_host.list().await;
                Ok(json!(list))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("plugin.invoke", move |params| {
            let st = st.clone();
            async move {
                let p: PluginInvokeParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let r = st
                    .wasm_host
                    .invoke(&p.name, &p.command, &p.args)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                Ok(json!({ "result": r }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("plugin.reload", move |_params| {
            let st = st.clone();
            async move {
                let n = st
                    .wasm_host
                    .reload()
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                Ok(json!({ "loaded": n }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("plugin.load", move |params| {
            let st = st.clone();
            async move {
                let p: PluginLoadParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let name = st
                    .wasm_host
                    .load_file(std::path::Path::new(&p.path))
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                Ok(json!({ "name": name }))
            }
        });
    }
}

fn register_sync(dispatcher: &Dispatcher, state: Arc<AppState>) {
    {
        let st = state.clone();
        dispatcher.register("sync.configureWebdav", move |params| {
            let st = st.clone();
            async move {
                let p: ConfigureWebDavParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let backend = Arc::new(WebDavBackend::with_auth(p.base_url, p.user, p.password));
                let params_kdf = if p.test_cheap_kdf {
                    KdfParams::test_cheap()
                } else {
                    KdfParams::DEFAULT
                };
                let device_id = p.device_id.unwrap_or_else(Uuid::new_v4);
                let mut engine = SyncEngine::new(
                    device_id,
                    p.master_password.into_bytes(),
                    params_kdf,
                    backend,
                );
                if let Some(dir) = p.state_dir {
                    let store =
                        Arc::new(SledStore::open(&dir).map_err(|e| internal(e.to_string()))?);
                    engine = engine
                        .with_store(store)
                        .await
                        .map_err(|e| internal(e.to_string()))?;
                }
                stop_auto(&st).await;
                *st.sync.lock().await = Some(Arc::new(engine));
                *st.sync_kind.lock().await = Some("webdav");
                *st.git_backend.lock().await = None;
                Ok(json!({ "device_id": device_id }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sync.configureGit", move |params| {
            let st = st.clone();
            async move {
                let p: ConfigureGitParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let mut backend =
                    GitBackend::open_or_init(&p.repo_path).map_err(|e| internal(e.to_string()))?;
                if let (Some(name), Some(email)) = (p.author_name, p.author_email) {
                    backend = backend.with_author(name, email);
                }
                if let Some(url) = p.remote_url.as_ref() {
                    backend = backend
                        .with_remote(&p.remote_name, url, &p.remote_branch)
                        .map_err(|e| internal(e.to_string()))?;
                    if let Some(ref provider) = p.oauth_provider {
                        let prov = parse_oauth_provider(provider)?;
                        if let Some(token) = oauth::load_token(prov)
                            .map_err(|e| internal(e.to_string()))?
                        {
                            let (user, pass) = oauth_https_credentials(prov, token);
                            backend = backend.with_https_auth(user, pass);
                        } else {
                            return Err(invalid_params(
                                "OAuth token missing; complete device sign-in first",
                            ));
                        }
                    } else if let (Some(u), Some(pw)) = (p.remote_user, p.remote_password) {
                        backend = backend.with_https_auth(u, pw);
                    }
                    if let Some(key) = p.remote_ssh_key {
                        backend = backend.with_ssh_key(key, p.remote_ssh_passphrase);
                    }
                }
                *st.git_backend.lock().await = Some(backend.clone());
                let backend_dyn: Arc<dyn crate::sync::SyncBackend> = Arc::new(backend);
                let params_kdf = if p.test_cheap_kdf {
                    KdfParams::test_cheap()
                } else {
                    KdfParams::DEFAULT
                };
                let device_id = p.device_id.unwrap_or_else(Uuid::new_v4);
                let mut engine = SyncEngine::new(
                    device_id,
                    p.master_password.into_bytes(),
                    params_kdf,
                    backend_dyn,
                );
                if let Some(dir) = p.state_dir {
                    let store =
                        Arc::new(SledStore::open(&dir).map_err(|e| internal(e.to_string()))?);
                    engine = engine
                        .with_store(store)
                        .await
                        .map_err(|e| internal(e.to_string()))?;
                }
                stop_auto(&st).await;
                *st.sync.lock().await = Some(Arc::new(engine));
                *st.sync_kind.lock().await = Some("git");
                Ok(json!({ "device_id": device_id }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sync.put", move |params| {
            let st = st.clone();
            async move {
                let p: SyncPutParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let bytes = BASE64
                    .decode(p.data.as_bytes())
                    .map_err(|e| invalid_params(format!("bad base64: {e}")))?;
                let eng = require_engine(&st).await?;
                eng.put_local(p.group, RecordId(p.id), bytes)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sync.delete", move |params| {
            let st = st.clone();
            async move {
                let p: SyncRecordParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let eng = require_engine(&st).await?;
                eng.delete_local(p.group, RecordId(p.id))
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sync.get", move |params| {
            let st = st.clone();
            async move {
                let p: SyncRecordParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let eng = require_engine(&st).await?;
                match eng.get_local(p.group, RecordId(p.id)).await {
                    Some(bytes) => Ok(json!({ "data": BASE64.encode(&bytes) })),
                    None => Ok(Value::Null),
                }
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sync.list", move |params| {
            let st = st.clone();
            async move {
                let p: SyncListParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let eng = require_engine(&st).await?;
                let ids = eng.list_local(p.group).await;
                Ok(Value::Array(ids.into_iter().map(|r| json!(r.0)).collect()))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sync.now", move |params| {
            let st = st.clone();
            async move {
                let p: SyncNowParams = if params.is_null() {
                    SyncNowParams::default()
                } else {
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?
                };
                let groups = selected_sync_groups(p.groups);
                let eng = require_engine(&st).await?;
                let git = st.git_backend.lock().await.clone();
                // Pull remote commits first so we reconcile against the
                // latest known state.
                if let Some(g) = &git {
                    g.fetch_remote()
                        .await
                        .map_err(|e| internal(e.to_string()))?;
                }
                let results = eng
                    .sync_groups(groups)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                if let Some(g) = &git {
                    g.push_remote().await.map_err(|e| internal(e.to_string()))?;
                }
                *st.sync_last_ms.lock().await = Some(now_ms());
                let obj: serde_json::Map<String, Value> = results
                    .into_iter()
                    .map(|(g, s)| {
                        let key = format!("{g:?}");
                        let v = serde_json::to_value(s).unwrap_or(Value::Null);
                        (key, v)
                    })
                    .collect();
                Ok(Value::Object(obj))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sync.startAutoSync", move |params| {
            let st = st.clone();
            async move {
                let p: StartAutoSyncParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                if p.interval_ms == 0 {
                    return Err(invalid_params("interval_ms must be > 0"));
                }
                let eng = require_engine(&st).await?;
                stop_auto(&st).await;
                let interval = std::time::Duration::from_millis(p.interval_ms);
                let groups = selected_sync_groups(p.groups);
                let st_tick = st.clone();
                let handle = tokio::spawn(async move {
                    let mut ticker = tokio::time::interval(interval);
                    // Skip the immediate first tick to avoid double-sync if
                    // the caller just configured the engine.
                    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                    ticker.tick().await;
                    loop {
                        ticker.tick().await;
                        match eng.sync_groups(groups.clone()).await {
                            Ok(_) => {
                                *st_tick.sync_last_ms.lock().await = Some(now_ms());
                            }
                            Err(e) => tracing::warn!(error = %e, "auto-sync tick failed"),
                        }
                    }
                });
                *st.sync_auto.lock().await = Some(handle);
                *st.sync_auto_interval_ms.lock().await = Some(p.interval_ms);
                Ok(json!({ "interval_ms": p.interval_ms }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sync.stopAutoSync", move |_params| {
            let st = st.clone();
            async move {
                let was_running = stop_auto(&st).await;
                Ok(json!({ "stopped": was_running }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sync.status", move |_params| {
            let st = st.clone();
            async move {
                let kind = *st.sync_kind.lock().await;
                let configured = st.sync.lock().await.is_some();
                let device_id = st
                    .sync
                    .lock()
                    .await
                    .as_ref()
                    .map(|e| e.device_id().to_string());
                let last_sync_ms = *st.sync_last_ms.lock().await;
                let auto_interval_ms = *st.sync_auto_interval_ms.lock().await;
                Ok(json!({
                    "configured": configured,
                    "kind": kind,
                    "deviceId": device_id,
                    "lastSyncMs": last_sync_ms,
                    "autoIntervalMs": auto_interval_ms,
                }))
            }
        });
    }
}

/// Cancels any running auto-sync task. Returns true if one was active.
async fn stop_auto(st: &AppState) -> bool {
    let was_running = if let Some(h) = st.sync_auto.lock().await.take() {
        h.abort();
        true
    } else {
        false
    };
    *st.sync_auto_interval_ms.lock().await = None;
    was_running
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// --- vault.* (M10) ---

async fn require_vault(state: &Arc<AppState>) -> Result<crate::vault::VaultStore, RpcError> {
    state
        .vault
        .lock()
        .await
        .clone()
        .ok_or_else(|| internal("vault store not configured"))
}

fn register_vault(dispatcher: &Dispatcher, state: Arc<AppState>) {
    {
        let st = state.clone();
        dispatcher.register("vault.configure", move |params| {
            let st = st.clone();
            async move {
                #[derive(Deserialize)]
                struct P {
                    path: String,
                }
                let p: P =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let store =
                    crate::vault::VaultStore::open(&p.path).map_err(|e| internal(e.to_string()))?;
                *st.vault.lock().await = Some(store);
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("vault.status", move |_p| {
            let st = st.clone();
            async move {
                let store = match st.vault.lock().await.clone() {
                    Some(s) => s,
                    None => {
                        return Ok(
                            json!({ "configured": false, "initialized": false, "unlocked": false }),
                        )
                    }
                };
                let initialized = store
                    .is_initialized()
                    .map_err(|e| internal(e.to_string()))?;
                let unlocked = store.is_unlocked().await;
                Ok(json!({ "configured": true, "initialized": initialized, "unlocked": unlocked }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("vault.initialize", move |params| {
            let st = st.clone();
            async move {
                #[derive(Deserialize)]
                struct P {
                    password: String,
                }
                let p: P =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let store = require_vault(&st).await?;
                store
                    .initialize(p.password.as_bytes())
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("vault.unlock", move |params| {
            let st = st.clone();
            async move {
                #[derive(Deserialize)]
                struct P {
                    password: String,
                }
                let p: P =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let store = require_vault(&st).await?;
                store
                    .unlock(p.password.as_bytes())
                    .await
                    .map_err(|e| match e {
                        crate::vault::VaultError::BadPassword => invalid_params("bad password"),
                        other => internal(other.to_string()),
                    })?;
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("vault.lock", move |_p| {
            let st = st.clone();
            async move {
                let store = require_vault(&st).await?;
                store.lock().await;
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("vault.list", move |_p| {
            let st = st.clone();
            async move {
                let store = require_vault(&st).await?;
                let list = store.list().await.map_err(|e| internal(e.to_string()))?;
                serde_json::to_value(list).map_err(|e| internal(e.to_string()))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("vault.get", move |params| {
            let st = st.clone();
            async move {
                #[derive(Deserialize)]
                struct P {
                    id: String,
                }
                let p: P =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let store = require_vault(&st).await?;
                let entry = store
                    .get(&p.id)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                serde_json::to_value(entry).map_err(|e| internal(e.to_string()))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("vault.put", move |params| {
            let st = st.clone();
            async move {
                let entry: crate::vault::VaultEntry =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let store = require_vault(&st).await?;
                let saved = store
                    .put(entry)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                serde_json::to_value(saved).map_err(|e| internal(e.to_string()))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("vault.remove", move |params| {
            let st = st.clone();
            async move {
                #[derive(Deserialize)]
                struct P {
                    id: String,
                }
                let p: P =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let store = require_vault(&st).await?;
                let removed = store
                    .remove(&p.id)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                Ok(json!({ "removed": removed }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("vault.changePassword", move |params| {
            let st = st.clone();
            async move {
                #[derive(Deserialize)]
                struct P {
                    old: String,
                    new: String,
                }
                let p: P =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let store = require_vault(&st).await?;
                store
                    .change_password(p.old.as_bytes(), p.new.as_bytes())
                    .await
                    .map_err(|e| match e {
                        crate::vault::VaultError::BadPassword => invalid_params("bad password"),
                        other => internal(other.to_string()),
                    })?;
                Ok(Value::Null)
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::Request;

    fn req(method: &str, params: Value) -> Request {
        Request {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: method.into(),
            params,
        }
    }

    #[tokio::test]
    async fn core_version_works() {
        let d = Dispatcher::new();
        register_all(&d, AppState::new());
        let r = d.dispatch(req("core.version", json!(null))).await;
        assert!(r.result.unwrap().get("version").is_some());
    }

    #[tokio::test]
    async fn core_protocol_version_returns_const() {
        let d = Dispatcher::new();
        register_all(&d, AppState::new());
        let r = d.dispatch(req("core.protocolVersion", json!(null))).await;
        assert_eq!(r.result.unwrap(), json!(crate::ipc::PROTOCOL_VERSION));
    }

    #[tokio::test]
    async fn serial_list_ports_returns_array() {
        let d = Dispatcher::new();
        register_all(&d, AppState::new());
        let r = d.dispatch(req("serial.listPorts", json!(null))).await;
        assert!(r.result.unwrap().is_array());
    }

    #[tokio::test]
    async fn session_list_starts_empty() {
        let d = Dispatcher::new();
        register_all(&d, AppState::new());
        let r = d.dispatch(req("session.list", json!(null))).await;
        assert_eq!(r.result.unwrap(), json!([]));
    }

    #[tokio::test]
    async fn sync_get_requires_configuration() {
        let d = Dispatcher::new();
        register_all(&d, AppState::new());
        let r = d
            .dispatch(req(
                "sync.get",
                json!({"group": "Connections", "id": Uuid::nil()}),
            ))
            .await;
        let err = r.error.expect("must error when sync not configured");
        assert!(err.message.contains("sync not configured"));
    }

    #[tokio::test]
    async fn sync_put_get_list_roundtrip_without_remote() {
        // Configure with a bogus WebDAV URL; we never call sync.now, so the
        // backend is never hit — exercises the local read/write path only.
        let d = Dispatcher::new();
        register_all(&d, AppState::new());
        let r = d
            .dispatch(req(
                "sync.configureWebdav",
                json!({
                    "base_url": "http://127.0.0.1:1/dav",
                    "master_password": "pw",
                    "test_cheap_kdf": true,
                }),
            ))
            .await;
        assert!(r.error.is_none(), "{:?}", r.error);

        let rid = Uuid::new_v4();
        let payload = BASE64.encode(b"connection-blob");
        let r = d
            .dispatch(req(
                "sync.put",
                json!({"group": "Connections", "id": rid, "data": payload}),
            ))
            .await;
        assert!(r.error.is_none(), "{:?}", r.error);

        let r = d
            .dispatch(req("sync.list", json!({"group": "Connections"})))
            .await;
        let ids = r.result.unwrap();
        assert_eq!(ids.as_array().unwrap().len(), 1);

        let r = d
            .dispatch(req("sync.get", json!({"group": "Connections", "id": rid})))
            .await;
        let got = r.result.unwrap();
        assert_eq!(got["data"], json!(BASE64.encode(b"connection-blob")));

        let r = d
            .dispatch(req(
                "sync.delete",
                json!({"group": "Connections", "id": rid}),
            ))
            .await;
        assert!(r.error.is_none());

        let r = d
            .dispatch(req("sync.get", json!({"group": "Connections", "id": rid})))
            .await;
        assert_eq!(r.result.unwrap(), Value::Null);
    }

    #[tokio::test]
    async fn sync_configure_git_creates_repo() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("aerotab-cmd-git-{}", Uuid::new_v4()));
        let d = Dispatcher::new();
        register_all(&d, AppState::new());
        let r = d
            .dispatch(req(
                "sync.configureGit",
                json!({
                    "repo_path": dir.to_str().unwrap(),
                    "master_password": "pw",
                    "test_cheap_kdf": true,
                }),
            ))
            .await;
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(dir.join(".git").exists());

        // Local put + sync.now should populate the working tree.
        let rid = Uuid::new_v4();
        let payload = BASE64.encode(b"hello");
        let r = d
            .dispatch(req(
                "sync.put",
                json!({"group": "Connections", "id": rid, "data": payload}),
            ))
            .await;
        assert!(r.error.is_none(), "{:?}", r.error);
        let r = d.dispatch(req("sync.now", json!(null))).await;
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(dir.join("connections").join(format!("{rid}.bin")).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn sync_persistence_survives_restart() {
        let mut state_dir = std::env::temp_dir();
        state_dir.push(format!("aerotab-cmd-state-{}", Uuid::new_v4()));
        let device = Uuid::new_v4();

        // First "process" — write a record.
        {
            let d = Dispatcher::new();
            register_all(&d, AppState::new());
            let r = d
                .dispatch(req(
                    "sync.configureWebdav",
                    json!({
                        "base_url": "http://127.0.0.1:1/dav",
                        "master_password": "pw",
                        "test_cheap_kdf": true,
                        "device_id": device,
                        "state_dir": state_dir.to_str().unwrap(),
                    }),
                ))
                .await;
            assert!(r.error.is_none(), "{:?}", r.error);
            let r = d
                .dispatch(req(
                    "sync.put",
                    json!({
                        "group": "Appearance",
                        "id": Uuid::nil(),
                        "data": BASE64.encode(b"persisted"),
                    }),
                ))
                .await;
            assert!(r.error.is_none());
        }
        // Second "process" — re-open and read.
        {
            let d = Dispatcher::new();
            register_all(&d, AppState::new());
            let r = d
                .dispatch(req(
                    "sync.configureWebdav",
                    json!({
                        "base_url": "http://127.0.0.1:1/dav",
                        "master_password": "pw",
                        "test_cheap_kdf": true,
                        "device_id": device,
                        "state_dir": state_dir.to_str().unwrap(),
                    }),
                ))
                .await;
            assert!(r.error.is_none(), "{:?}", r.error);
            let r = d
                .dispatch(req(
                    "sync.get",
                    json!({"group": "Appearance", "id": Uuid::nil()}),
                ))
                .await;
            let got = r.result.unwrap();
            assert_eq!(got["data"], json!(BASE64.encode(b"persisted")));
        }
        let _ = std::fs::remove_dir_all(&state_dir);
    }

    #[tokio::test]
    async fn auto_sync_start_then_stop() {
        let d = Dispatcher::new();
        register_all(&d, AppState::new());

        // Start without configuration -> error.
        let r = d
            .dispatch(req("sync.startAutoSync", json!({"interval_ms": 1000})))
            .await;
        assert!(r.error.is_some());

        // Configure something local-only.
        let r = d
            .dispatch(req(
                "sync.configureWebdav",
                json!({
                    "base_url": "http://127.0.0.1:1/dav",
                    "master_password": "pw",
                    "test_cheap_kdf": true,
                }),
            ))
            .await;
        assert!(r.error.is_none());

        let r = d
            .dispatch(req("sync.startAutoSync", json!({"interval_ms": 60000})))
            .await;
        assert!(r.error.is_none(), "{:?}", r.error);

        let r = d.dispatch(req("sync.stopAutoSync", json!(null))).await;
        assert_eq!(r.result.unwrap(), json!({"stopped": true}));

        // Second stop is a no-op.
        let r = d.dispatch(req("sync.stopAutoSync", json!(null))).await;
        assert_eq!(r.result.unwrap(), json!({"stopped": false}));
    }

    #[tokio::test]
    async fn open_ssh_with_bad_host_reports_error() {
        let d = Dispatcher::new();
        register_all(&d, AppState::new());
        // Port 1 has no SSH server; connect will fail. The handler should
        // surface an `InternalError` rather than crash.
        let r = d
            .dispatch(req(
                "session.openSsh",
                json!({
                    "rows": 24,
                    "cols": 80,
                    "profile": {
                        "host": "127.0.0.1",
                        "port": 1,
                        "user": "nobody",
                        "auth": { "Password": { "secret": "x" } },
                        "jump_via": [],
                    },
                }),
            ))
            .await;
        let err = r.error.expect("ssh connect must fail");
        assert_eq!(err.code, ErrorCode::InternalError as i32);

        // No session should have been registered.
        let r = d.dispatch(req("session.list", json!(null))).await;
        assert_eq!(r.result.unwrap(), json!([]));
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn open_write_poll_close_local_shell() {
        let d = Dispatcher::new();
        register_all(&d, AppState::new());
        let r = d
            .dispatch(req(
                "session.openLocal",
                json!({"title": "t", "rows": 24, "cols": 80}),
            ))
            .await;
        let id_val = r.result.unwrap();
        let id = id_val["id"].as_str().unwrap().to_string();

        // Write a command + newline.
        let cmd = BASE64.encode("echo aerotab-rpc\n".as_bytes());
        let _ = d
            .dispatch(req("session.write", json!({"id": id, "data": cmd})))
            .await;

        // Poll output until we see the echoed text or timeout.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut all = String::new();
        while std::time::Instant::now() < deadline {
            let r = d
                .dispatch(req(
                    "session.pollOutput",
                    json!({"id": id, "max_chunks": 32}),
                ))
                .await;
            for chunk in r.result.unwrap().as_array().unwrap() {
                let bytes = BASE64.decode(chunk.as_str().unwrap()).unwrap();
                all.push_str(&String::from_utf8_lossy(&bytes));
            }
            if all.contains("aerotab-rpc") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(all.contains("aerotab-rpc"), "did not see echo, got {all:?}");

        // Close.
        let r = d.dispatch(req("session.close", json!({"id": id}))).await;
        assert!(r.error.is_none());
    }
}
