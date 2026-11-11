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
//! | `sftp.openForSession`   | `{ session_id, sudo? }`             | `{ id }`          |
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
//! | `sftp.directTransfer`  | `{ transfer_id, source_session_id, dest_session_id, … }` | transfer output |
//! | `sftp.directPreflight` | `{ source_session_id, dest_session_id }` | capability + consent |
//! | `sftp.cancelDirectTransfer` | `{ transfer_id }` | `{ canceled }` |
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::Emitter;
use tokio::sync::{mpsc, Mutex};
use tokio::time::timeout;
use uuid::Uuid;

use crate::core::session_manager::{SessionId, SessionKind, SessionManager, SessionMeta};
use crate::ipc::{Dispatcher, ErrorCode, RpcError};
use crate::native_terminal::{EmbedRectDip, EmbeddedTerminalRegistry};
use crate::plugins::wasm_host::WasmHost;
use crate::profile::{Profile, ProfileKind, ProfileStore, RemoteDesktopSpec};
use crate::remote;
use crate::secret;
use crate::serial::{SerialChannel, SerialProfile};
use crate::settings::SettingsStore;
use crate::ssh::known_hosts::KnownHosts;
use crate::ssh::sftp::{Sftp, SftpKind, SftpOpenOptions};
use crate::ssh::tunnel::{TunnelKind, TunnelManager, TunnelOpenRequest};
use crate::ssh::{self, SshProfile, SshShell, X11ForwardOptions};
use crate::sync::backends::git::GitBackend;
use crate::sync::backends::webdav::WebDavBackend;
use crate::sync::crypto::KdfParams;
use crate::sync::oauth::{self, OAuthProvider};
use crate::sync::persistence::SledStore;
use crate::sync::{BlobStore, Group, OplogDriver, OplogStats, RecordId, SyncEngine, SyncStats};
use crate::terminal::{PtyChannel, PtySize};

static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

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

#[derive(Default)]
pub struct SyncApproval {
    generation: Uuid,
    target: Option<String>,
    binding: Option<String>,
    pending: Option<(Uuid, Vec<Group>, Value)>,
    approved: Option<Vec<Group>>,
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
    /// Serialize configuration, reconciliation and destructive maintenance.
    pub sync_operation: Mutex<()>,
    pub sync_migration: Mutex<Option<(Uuid, String)>>,
    pub sync_approval: Mutex<SyncApproval>,
    pub sync_approval_store: Mutex<Option<SettingsStore>>,
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
    pub notes: Mutex<Option<crate::notes::NotesStore>>,
    pub scripts: Mutex<Option<Arc<crate::scripts::ScriptsService>>>,
    pub mcp_batches: crate::mcp::batch::BatchRuntime,
    pub ai: crate::ai::AiRuntime,
    pub network_resources: Mutex<Option<crate::network_resources::NetworkResourceStore>>,
    pub forwarding: crate::network_resources::ForwardingRuntime,
    /// Most recently configured Git sync backend; retained so the engine
    /// can call `fetch_remote` / `push_remote` outside the abstract
    /// SyncBackend trait.
    pub git_backend: Mutex<Option<GitBackend>>,
    /// Protocol-2 oplog driver (incremental BlobStore sync). When set,
    /// `sync.now` uses append-only ops instead of per-record GET.
    pub oplog: Mutex<Option<Arc<OplogDriver>>>,
    /// Stats from the most recent oplog cycle (bytes / ops / snapshot).
    pub oplog_last: Mutex<Option<OplogStats>>,
    /// Open SFTP sessions, keyed by an opaque per-session id.
    pub sftp_sessions: Mutex<HashMap<Uuid, Arc<Sftp>>>,
    /// Resolved SSH profile for each SFTP session, stored so transfer RPCs
    /// can look up auth info from just the session id.
    pub sftp_profiles: Mutex<HashMap<Uuid, SshProfile>>,
    /// Cancel flags for in-flight backend relay transfers (`transfer_id` → flag).
    pub relay_cancel: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// SSH port-forwarding tunnels (`-L` / `-R` / `-D`).
    pub tunnels: TunnelManager,
    /// Persistent settings store. Configured via `settings.configure`.
    pub settings: Mutex<Option<SettingsStore>>,
    /// Master-password vault (M10). Configured via `vault.configure`.
    pub vault: Mutex<Option<crate::vault::VaultStore>>,
    /// WASM plugin host. Configured via `plugin.configure`.
    pub wasm_host: Arc<WasmHost>,
    /// Native terminal embed registry (in-pane child windows on supported platforms).
    pub embedded_terminals: EmbeddedTerminalRegistry,
    /// Detached native terminal windows (Alacritty / Kitty / Ghostty).
    pub native_terminals: crate::native_terminal::NativeTerminalRegistry,
    /// Native terminal engines (VT state in Rust, binary frames to the webview).
    pub terminal_engines: crate::terminal::engine::TerminalEngineRegistry,
}

/// Set the parent window HWND. Called once from the Tauri desktop shell after
/// the main window is created.
pub fn set_parent_hwnd(hwnd: usize) {
    crate::native_terminal::embed::set_parent_hwnd(hwnd);
}

/// Store the Tauri AppHandle so native-terminal RPCs can access the window.
pub fn set_app_handle(app: tauri::AppHandle) {
    crate::native_terminal::embed::set_app_handle(app.clone());
    let _ = APP_HANDLE.set(app);
}

/// Request the desktop to open one visible SSH tab using its normal connection flow.
/// No backend session is created here, preventing orphan/duplicate sessions.
pub fn mcp_open_terminal(profile_id: &str) -> Result<(), String> {
    let app = APP_HANDLE
        .get()
        .ok_or_else(|| "Visible terminals require the AeroTab desktop application".to_string())?;
    app.emit(
        "aerotab:mcp-open-terminal",
        serde_json::json!({ "profileId": profile_id }),
    )
    .map_err(|error| error.to_string())
}

/// Discriminated union over the channel kinds we know how to drive.
pub enum SessionChannel {
    Local(LocalPty),
    Ssh(SshSessionEntry),
    Serial(SerialSessionEntry),
}

pub struct LocalPty {
    pub channel: PtyChannel,
    /// `Arc` so `session.poll` can recv without holding the global `channels` lock (tab close stays fast).
    pub rx: Arc<Mutex<mpsc::Receiver<Vec<u8>>>>,
}

pub struct SshSessionEntry {
    pub shell: Arc<SshShell>,
    pub rx: Arc<Mutex<mpsc::Receiver<Vec<u8>>>>,
}

pub struct SerialSessionEntry {
    pub channel: SerialChannel,
    pub rx: Arc<Mutex<mpsc::Receiver<Vec<u8>>>>,
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

/// Merge PTY/SSH packets from one poll burst so binary protocols (e.g. trzsz) are
/// less likely to be split across frontend processing calls.
fn coalesce_poll_output_chunks(raw_chunks: Vec<Vec<u8>>) -> Vec<String> {
    if raw_chunks.is_empty() {
        return Vec::new();
    }
    if raw_chunks.len() == 1 {
        return vec![BASE64.encode(&raw_chunks[0])];
    }
    let total: usize = raw_chunks.iter().map(|c| c.len()).sum();
    let mut merged = Vec::with_capacity(total);
    for chunk in raw_chunks {
        merged.extend_from_slice(&chunk);
    }
    vec![BASE64.encode(&merged)]
}

/// Registers every method on `dispatcher`, capturing `state` by clone.
pub fn register_all(dispatcher: &Dispatcher, state: Arc<AppState>) {
    crate::ai::register(dispatcher, state.clone());
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
        dispatcher.register("desktop.sessionInfo", move |params| async move {
            #[derive(Deserialize, Default)]
            struct P {
                #[serde(default)]
                display: Option<String>,
            }
            let p: P = if params.is_null() {
                P::default()
            } else {
                serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?
            };
            let info = crate::desktop_session::desktop_session_info(p.display.as_deref());
            serde_json::to_value(info).map_err(|e| internal(e.to_string()))
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
                    let cmd = crate::terminal::build_shell_command(shell, &p.shell_args);
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
                        rx: Arc::new(Mutex::new(rx)),
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
                ssh::validate_x11_forward(&x11).map_err(invalid_params)?;
                let transport = load_ssh_transport_settings(&st).await;
                let profile = materialize_ssh_profile(&st, p.profile).await?;
                let mut shell = ssh::connect_shell_with_known_hosts(
                    &profile,
                    cols as u32,
                    rows as u32,
                    kh,
                    Some(x11),
                    transport,
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
                            .unwrap_or_else(|| format!("{}@{}", profile.user, profile.host)),
                    )
                };
                st.channels.lock().await.insert(
                    meta.id,
                    SessionChannel::Ssh(SshSessionEntry {
                        shell: Arc::new(shell),
                        rx: Arc::new(Mutex::new(rx)),
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
                let profile = load_effective_catalog(&st)
                    .await?
                    .into_iter()
                    .find(|profile| profile.id == p.profile_id)
                    .ok_or_else(|| {
                        invalid_params(format!("profile not found: {}", p.profile_id))
                    })?;
                let rows = p.rows.unwrap_or(24);
                let cols = p.cols.unwrap_or(80);
                let kh = st.known_hosts.lock().await.clone();
                let x11 = load_ssh_x11_options(&st).await;
                ssh::validate_x11_forward(&x11).map_err(invalid_params)?;
                let transport = load_ssh_transport_settings(&st).await;
                let mut shell = match profile.spec {
                    ProfileKind::Ssh { ssh } => {
                        let ssh = materialize_ssh_profile(&st, ssh).await?;
                        ssh::connect_shell_with_known_hosts(
                            &ssh,
                            cols as u32,
                            rows as u32,
                            kh,
                            Some(x11),
                            transport,
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
                        shell: Arc::new(shell),
                        rx: Arc::new(Mutex::new(rx)),
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
                st.terminal_engines.remove(p.id);
                let removed = st.channels.lock().await.remove(&id);
                if let Some(chan) = removed {
                    match chan {
                        SessionChannel::Local(mut pty) => {
                            let _ = pty.channel.kill();
                        }
                        SessionChannel::Ssh(s) => {
                            if let Ok(shell) = Arc::try_unwrap(s.shell) {
                                let _ = shell.close().await;
                            }
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
                let rx = {
                    let chans = st.channels.lock().await;
                    let chan = chans.get(&id).ok_or_else(|| session_not_found(id))?;
                    match chan {
                        SessionChannel::Local(pty) => pty.rx.clone(),
                        SessionChannel::Ssh(s) => s.rx.clone(),
                        SessionChannel::Serial(s) => s.rx.clone(),
                    }
                };
                let mut rx = rx.lock().await;
                let mut raw_chunks: Vec<Vec<u8>> = Vec::new();
                for _ in 0..max {
                    match rx.try_recv() {
                        Ok(c) => raw_chunks.push(c),
                        Err(_) => break,
                    }
                }
                let chunks: Vec<String> = coalesce_poll_output_chunks(raw_chunks);
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
                // Output owned by a native engine: frames go through the
                // terminal channel instead; never block on the shared receiver.
                if let Some(engine) = st.terminal_engines.get(p.id) {
                    return Ok(json!({
                        "chunks": Value::Array(vec![]),
                        "alive": engine.is_alive(),
                        "native": true,
                    }));
                }
                let (rx, mut alive) = {
                    let mut chans = st.channels.lock().await;
                    let Some(chan) = chans.get_mut(&id) else {
                        return Ok(json!({ "chunks": Value::Array(vec![]), "alive": false }));
                    };
                    match chan {
                        SessionChannel::Local(pty) => {
                            let alive = matches!(pty.channel.try_wait(), Ok(None));
                            (pty.rx.clone(), alive)
                        }
                        SessionChannel::Ssh(s) => (s.rx.clone(), true),
                        SessionChannel::Serial(s) => (s.rx.clone(), true),
                    }
                };
                let mut rx = rx.lock().await;
                let mut raw_chunks: Vec<Vec<u8>> = Vec::new();
                use tokio::sync::mpsc::error::TryRecvError;
                for _ in 0..max {
                    match rx.try_recv() {
                        Ok(c) => raw_chunks.push(c),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            alive = false;
                            break;
                        }
                    }
                }
                let chunks = coalesce_poll_output_chunks(raw_chunks);
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
                        rx: Arc::new(Mutex::new(rx)),
                    }),
                );
                serde_json::to_value(OpenLocalResult { meta }).map_err(|e| internal(e.to_string()))
            }
        });
    }
    register_secret(dispatcher, state.clone());
    register_profiles(dispatcher, state.clone());
    register_resources(dispatcher, state.clone());
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
    #[serde(default = "default_sync_protocol")]
    protocol: u32,
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
    #[serde(default = "default_sync_protocol")]
    protocol: u32,
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
    /// SSH port when remote uses SSH (default 22). Embedded in `ssh://` remote URL.
    #[serde(default)]
    remote_ssh_port: Option<u16>,
    /// `ssh` | `https` — explicit transport from UI (avoids mis-detecting SSH as HTTPS).
    #[serde(default)]
    remote_auth: Option<String>,
    /// `github` or `gitlab` — load access token from OS keyring.
    #[serde(default)]
    oauth_provider: Option<String>,
}

fn default_sync_protocol() -> u32 {
    3
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
    #[serde(default)]
    preview_token: Option<Uuid>,
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
        Group::ALL
            .into_iter()
            .filter(|g| *g != Group::Notes)
            .collect()
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

fn register_resources(dispatcher: &Dispatcher, state: Arc<AppState>) {
    crate::mcp::batch::register_rpc(dispatcher, state.clone());
    {
        let st = state.clone();
        dispatcher.register("notes.editState", move |params| {
            let st = st.clone();
            async move {
                let owner = params["owner"]
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() < 128)
                    .ok_or_else(|| invalid_params("editor owner required"))?;
                let dirty = params["dirty"]
                    .as_bool()
                    .ok_or_else(|| invalid_params("dirty required"))?;
                let store = st
                    .notes
                    .lock()
                    .await
                    .clone()
                    .ok_or_else(|| internal("notes store unavailable"))?;
                store
                    .edit_state(owner.to_string(), dirty)
                    .map_err(|e| internal(e.to_string()))?;
                Ok(json!({"paused":store.sync_paused()}))
            }
        });
    }
    for method in [
        "notes.list",
        "notes.put",
        "notes.delete",
        "notes.deleteImpact",
        "notes.putFolder",
    ] {
        let st = state.clone();
        dispatcher.register(method, move |params| {
            let st = st.clone();
            async move {
                let _operation = st.sync_operation.lock().await;
                let store = st
                    .notes
                    .lock()
                    .await
                    .clone()
                    .ok_or_else(|| internal("notes store not configured"))?;
                let ids = require_profiles(&st)
                    .await?
                    .list()
                    .await
                    .map_err(|e| internal(e.to_string()))?
                    .into_iter()
                    .map(|p| p.id.to_string())
                    .collect::<Vec<_>>();
                crate::notes::dispatch(&store, method, params, &ids).map_err(invalid_params)
            }
        });
    }
    for method in [
        "scripts.list",
        "scripts.get",
        "scripts.put",
        "scripts.delete",
        "scripts.preview",
        "scripts.prepare",
        "scripts.run",
        "scripts.tasks",
        "scripts.status",
        "scripts.cancel",
    ] {
        let st = state.clone();
        dispatcher.register(method, move |params| {
            let st = st.clone();
            async move {
                let service = st
                    .scripts
                    .lock()
                    .await
                    .clone()
                    .ok_or_else(|| internal("scripts store not configured"))?;
                let resolve = |id: String| {
                    let st = st.clone();
                    async move {
                        let id =
                            Uuid::parse_str(&id).map_err(|_| "invalid profile id".to_string())?;
                        let profile = resolve_ssh_profile(&st, id).await.map_err(|e| e.message)?;
                        let known_hosts = st
                            .known_hosts
                            .lock()
                            .await
                            .clone()
                            .ok_or_else(|| "SSH trust store not configured".to_string())?;
                        Ok(crate::scripts::ResolvedTarget {
                            profile,
                            known_hosts,
                            transport: load_ssh_transport_settings(&st).await,
                        })
                    }
                };
                service
                    .dispatch(method, params, &resolve)
                    .await
                    .map_err(invalid_params)
            }
        });
    }
    for method in [
        "proxy.list",
        "proxy.save",
        "proxy.delete",
        "forwarding.list",
        "forwarding.save",
        "forwarding.delete",
        "forwarding.start",
        "forwarding.stop",
        "forwarding.running",
    ] {
        let st = state.clone();
        dispatcher.register(method, move |params: Value| {
            let st = st.clone();
            async move {
                let store = st.network_resources.lock().await.clone().ok_or_else(|| internal("network resource store not configured"))?;
                let id = || -> Result<Uuid, RpcError> { serde_json::from_value(params.get("id").cloned().unwrap_or(Value::Null)).map_err(|e| invalid_params(e.to_string())) };
                match method {
                    "proxy.list" => serde_json::to_value(store.list_proxies().map_err(|e| internal(e.to_string()))?).map_err(|e| internal(e.to_string())),
                    "forwarding.list" => serde_json::to_value(store.list_rules().map_err(|e| internal(e.to_string()))?).map_err(|e| internal(e.to_string())),
                    "proxy.save" => {
                        let resource: crate::network_resources::ProxyResource = serde_json::from_value(params.get("resource").cloned().unwrap_or(Value::Null)).map_err(|e| invalid_params(e.to_string()))?;
                        let profiles = require_profiles(&st).await?.list().await.map_err(|e| internal(e.to_string()))?;
                        let mut proxies = store.list_proxies().map_err(|e| internal(e.to_string()))?;
                        proxies.retain(|p| p.id != resource.id);
                        proxies.push(resource.clone());
                        let catalog = crate::profile_inheritance::merge_catalog(&profiles, &load_group_defaults(&st).await?).map_err(invalid_params)?;
                        let mut affected = crate::profile_inheritance::proxy_dependents_ids(&catalog, resource.id);
                        if let crate::network_resources::ProxyConfig::SshJump { profile_id } = resource.config {
                            if !catalog.iter().any(|p| p.id == profile_id && matches!(p.spec, ProfileKind::Ssh { .. })) { return Err(invalid_params("proxy target is not a saved SSH profile")); }
                            affected.insert(profile_id);
                        }
                        crate::profile_inheritance::validate_affected(&catalog, &proxies, affected).map_err(invalid_params)?;
                        store.save_proxy(resource).map_err(|e| invalid_params(e.to_string()))?;
                        Ok(Value::Null)
                    }
                    "proxy.delete" => {
                        let id = id()?;
                        let mut profiles = require_profiles(&st).await?.list().await.map_err(|e| internal(e.to_string()))?;
                        profiles.extend(load_effective_catalog(&st).await?);
                        let mut deps = crate::profile_inheritance::proxy_dependents(&profiles, id);
                        deps.extend(crate::profile_inheritance::group_dependents(&load_group_defaults(&st).await?, "proxy_id", &id.to_string()));
                        Ok(json!({"removed": store.delete_proxy(id, &deps).map_err(|e| invalid_params(e.to_string()))?}))
                    }
                    "forwarding.save" => {
                        let rule: crate::network_resources::ForwardingRule = serde_json::from_value(params.get("rule").cloned().unwrap_or(Value::Null)).map_err(|e| invalid_params(e.to_string()))?;
                        resolve_ssh_profile(&st, rule.profile_id).await?;
                        store.save_rule(rule).map_err(|e| invalid_params(e.to_string()))?;
                        Ok(Value::Null)
                    }
                    "forwarding.delete" => {
                        let id = id()?;
                        Ok(json!({"removed": store.delete_rule(id, &st.forwarding.dependents(id).await).map_err(|e| invalid_params(e.to_string()))?}))
                    }
                    "forwarding.start" => {
                        let rule = store.get_rule(id()?).map_err(|e| internal(e.to_string()))?.ok_or_else(|| invalid_params("forwarding rule not found"))?;
                        let profile = resolve_ssh_profile(&st, rule.profile_id).await?;
                        let kh = st.known_hosts.lock().await.clone();
                        serde_json::to_value(st.forwarding.start(&rule, profile, &st.tunnels, kh).await.map_err(|e| internal(e.to_string()))?).map_err(|e| internal(e.to_string()))
                    }
                    "forwarding.stop" => Ok(json!({"stopped": st.forwarding.stop(id()?, &st.tunnels).await})),
                    "forwarding.running" => serde_json::to_value(st.forwarding.running(&st.tunnels).await).map_err(|e| internal(e.to_string())),
                    _ => unreachable!(),
                }
            }
        });
    }
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
                let dir = std::path::Path::new(&p.path)
                    .parent()
                    .ok_or_else(|| invalid_params("invalid profile path"))?;
                *st.notes.lock().await = Some(
                    crate::notes::NotesStore::open(dir.join("notes"))
                        .map_err(|e| internal(e.to_string()))?,
                );
                *st.scripts.lock().await = Some(Arc::new(crate::scripts::ScriptsService::new(
                    crate::scripts::ScriptsStore::open(dir.join("scripts"))
                        .map_err(|e| internal(e.to_string()))?,
                )));
                *st.network_resources.lock().await = Some(
                    crate::network_resources::NetworkResourceStore::open(dir)
                        .map_err(|e| internal(e.to_string()))?,
                );
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
                let Some(store) = st.profiles.lock().await.clone() else {
                    tracing::warn!("profile.list: store not configured");
                    return Ok(json!([]));
                };
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
        dispatcher.register("profile.effective", move |params| {
            let st = st.clone();
            async move {
                let p: IdParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let store = require_profiles(&st).await?;
                let raw = store
                    .get(p.id)
                    .await
                    .map_err(|e| internal(e.to_string()))?
                    .ok_or_else(|| invalid_params("profile not found"))?;
                let defaults = load_group_defaults(&st).await?;
                serde_json::to_value(
                    crate::profile_inheritance::effective_profile(&raw, &defaults)
                        .map_err(invalid_params)?,
                )
                .map_err(|e| internal(e.to_string()))
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
                let mut catalog = store.list().await.map_err(|e| internal(e.to_string()))?;
                catalog.retain(|p| p.id != profile.id);
                catalog.push(profile.clone());
                let effective = crate::profile_inheritance::merge_catalog(
                    &catalog,
                    &load_group_defaults(&st).await?,
                )
                .map_err(invalid_params)?;
                validate_affected_profiles(
                    &st,
                    &effective,
                    std::collections::HashSet::from([profile.id]),
                )
                .await?;
                if let ProfileKind::Ssh { ssh } = &profile.spec {
                    if let Some(id) = ssh
                        .proxy_id
                        .filter(|_| !ssh.inherit.iter().any(|field| field == "proxy_id"))
                    {
                        let network = st
                            .network_resources
                            .lock()
                            .await
                            .clone()
                            .ok_or_else(|| internal("proxy store not configured"))?;
                        if network
                            .get_proxy(id)
                            .map_err(|e| internal(e.to_string()))?
                            .is_none()
                        {
                            return Err(invalid_params("proxy resource not found"));
                        }
                    }
                }
                store
                    .upsert_synced(profile)
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
                let mut dependencies = crate::profile_inheritance::group_dependents(
                    &load_group_defaults(&st).await?,
                    "jump",
                    &p.id.to_string(),
                );
                dependencies.extend(crate::profile::profile_dependents(
                    &load_effective_catalog(&st).await?,
                    p.id,
                ));
                if let Some(notes) = st.notes.lock().await.clone() {
                    dependencies.extend(
                        notes
                            .profile_references(&p.id.to_string())
                            .map_err(|e| internal(e.to_string()))?,
                    );
                }
                if let Some(scripts) = st.scripts.lock().await.clone() {
                    dependencies.extend(
                        scripts
                            .store
                            .references(&p.id.to_string())
                            .map_err(|e| internal(e.to_string()))?,
                    );
                }
                if let Some(network) = st.network_resources.lock().await.clone() {
                    dependencies.extend(
                        network
                            .profile_dependents(p.id)
                            .map_err(|e| internal(e.to_string()))?,
                    );
                }
                if !dependencies.is_empty() {
                    return Err(invalid_params(format!(
                        "profile is referenced by: {}",
                        dependencies.join(", ")
                    )));
                }
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
            use std::time::Duration;
            let result = tokio::time::timeout(
                Duration::from_secs(8),
                tokio::task::spawn_blocking(|| {
                    let shells = crate::shell_detect::detect();
                    let ssh_config = crate::ssh_config::load_default();
                    (shells, ssh_config)
                }),
            )
            .await
            .map_err(|_| internal("profile.discover timed out (check ~/.ssh/config is readable)"))?
            .map_err(|e| internal(format!("profile.discover join: {e}")))?;
            let (shells, ssh_config) = result;
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
                let mut profiles = require_profiles(&st)
                    .await?
                    .list()
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                if !p.ids.is_empty() {
                    profiles.retain(|profile| p.ids.contains(&profile.id));
                }
                let defaults = load_group_defaults(&st).await?;
                let catalog = crate::profile_inheritance::merge_catalog(&profiles, &defaults)
                    .map_err(invalid_params)?;
                let all_catalog = load_effective_catalog(&st).await?;
                let proxies = load_proxies(&st).await?;
                let kh = st.known_hosts.lock().await.clone();
                let vault = st.vault.lock().await.clone();
                let mut results = Vec::new();
                for mut profile in catalog {
                    let preparation = async {
                        if let ProfileKind::Ssh { ssh } = &mut profile.spec {
                            crate::profile_inheritance::validate_ssh_graph(
                                ssh,
                                &all_catalog,
                                &proxies,
                                Some(profile.id),
                            )
                            .map_err(invalid_params)?;
                            if p.connect {
                                *ssh = materialize_ssh_profile(&st, ssh.clone()).await?;
                            }
                        }
                        Ok::<_, RpcError>(())
                    }
                    .await;
                    if preparation.is_err() {
                        use crate::profile_health::{
                            HealthStatus, ProfileHealthCheck, ProfileHealthResult,
                        };
                        results.push(ProfileHealthResult {
                            id: profile.id,
                            name: profile.name,
                            status: HealthStatus::Error,
                            checks: vec![ProfileHealthCheck {
                                name: "Configuration".into(),
                                status: HealthStatus::Error,
                                message: "Invalid SSH inheritance or resource reference".into(),
                            }],
                        });
                    } else {
                        results.extend(
                            crate::profile_health::check_profiles(
                                vec![profile],
                                kh.clone(),
                                p.connect,
                                vault.clone(),
                            )
                            .await,
                        );
                    }
                }
                serde_json::to_value(results).map_err(|e| internal(e.to_string()))
            }
        });
    }
    register_profile_import(dispatcher, state.clone());
}

fn register_profile_import(dispatcher: &Dispatcher, state: Arc<AppState>) {
    use crate::import::{
        apply_import_items, import_detect, load_import_preview, mark_duplicates,
        ImportApplyItemInput, ImportPreviewResult,
    };
    use crate::profile::Profile;
    use crate::ssh::AuthMethod;

    #[derive(Debug, Deserialize)]
    struct ImportSourceParams {
        source: String,
    }

    #[derive(Debug, Deserialize)]
    struct ImportPreviewParams {
        source: String,
        #[serde(default)]
        path: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ImportApplyItem {
        source_id: String,
        #[serde(default)]
        overwrite: bool,
        #[serde(default)]
        user: Option<String>,
        #[serde(default)]
        auth: Option<AuthMethod>,
        #[serde(default)]
        duplicate_of: Option<Uuid>,
        #[serde(default)]
        profile: Option<Profile>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ImportApplyParams {
        source: String,
        #[serde(default)]
        path: Option<String>,
        items: Vec<ImportApplyItem>,
    }

    fn load_preview(source: &str, path: Option<&str>) -> Result<ImportPreviewResult, RpcError> {
        load_import_preview(source, path).map_err(invalid_params)
    }

    {
        dispatcher.register("profile.importDetect", move |params| async move {
            let p: ImportSourceParams =
                serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
            let result = import_detect(&p.source).map_err(invalid_params)?;
            serde_json::to_value(result).map_err(|e| internal(e.to_string()))
        });
    }
    {
        let st = state.clone();
        dispatcher.register("profile.importPreview", move |params| {
            let st = st.clone();
            async move {
                let p: ImportPreviewParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let mut preview = load_preview(&p.source, p.path.as_deref())?;
                if let Ok(store) = require_profiles(&st).await {
                    if let Ok(existing) = store.list().await {
                        mark_duplicates(&mut preview.candidates, &existing);
                        preview.stats = crate::import::preview_stats(&preview.candidates);
                    }
                }
                serde_json::to_value(preview).map_err(|e| internal(e.to_string()))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("profile.importApply", move |params| {
            let st = st.clone();
            async move {
                let p: ImportApplyParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let store = require_profiles(&st).await?;
                let mut preview = load_preview(&p.source, p.path.as_deref())?;
                let existing = store.list().await.map_err(|e| internal(e.to_string()))?;
                mark_duplicates(&mut preview.candidates, &existing);
                let by_id: std::collections::HashMap<String, _> = preview
                    .candidates
                    .into_iter()
                    .map(|c| (c.source_id.clone(), c))
                    .collect();
                let inputs: Vec<ImportApplyItemInput> = p
                    .items
                    .into_iter()
                    .map(|item| ImportApplyItemInput {
                        source_id: item.source_id,
                        overwrite: item.overwrite,
                        user: item.user,
                        auth: item.auth,
                        duplicate_of: item.duplicate_of,
                        profile: item.profile,
                    })
                    .collect();
                let result = apply_import_items(&store, &existing, &by_id, inputs).await;
                Ok(json!(result))
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
    #[serde(default)]
    profile: Option<SshProfile>,
    #[serde(default)]
    profile_id: Option<Uuid>,
}

fn register_ssh_stats(dispatcher: &Dispatcher, state: Arc<AppState>) {
    let st = state.clone();
    dispatcher.register("ssh.hostStats", move |params| {
        let st = st.clone();
        async move {
            let p: HostStatsParams =
                serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
            let profile = match (p.profile_id, p.profile) {
                (Some(id), _) => resolve_ssh_profile(&st, id).await?,
                (None, Some(profile)) => materialize_ssh_profile(&st, profile).await?,
                _ => return Err(invalid_params("profile_id or profile required")),
            };
            let kh = st.known_hosts.lock().await.clone();
            let stats = ssh::stats::probe_host_stats(&profile, kh)
                .await
                .map_err(|e| internal(e.to_string()))?;
            serde_json::to_value(stats).map_err(|e| internal(e.to_string()))
        }
    });
}

// --- sftp.* ---------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct SftpOpenParams {
    #[serde(default)]
    profile: Option<SshProfile>,
    #[serde(default)]
    profile_id: Option<Uuid>,
    #[serde(default)]
    sudo: bool,
}

#[derive(Debug, Deserialize)]
struct SftpOpenForSessionParams {
    session_id: String,
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

#[derive(Debug, Deserialize)]
struct SftpDirectTransferParams {
    transfer_id: String,
    source_session_id: Uuid,
    dest_session_id: Uuid,
    source_path: String,
    kind: crate::remote_transfer::DirectTransferKind,
    dest_path: String,
    /// When true, caller already obtained (or remembered) agent-forward consent.
    #[serde(default)]
    consent_granted: bool,
}

#[derive(Debug, Deserialize)]
struct SftpDirectPreflightParams {
    source_session_id: Uuid,
    dest_session_id: Uuid,
    /// When true, run a short source-side tool probe (`rsync`/`scp`).
    #[serde(default = "default_true")]
    probe_tools: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct SftpRememberDirectConsentParams {
    source_host_key: String,
}

#[derive(Debug, Deserialize)]
struct SftpRelayTransferParams {
    transfer_id: String,
    source_session_id: Uuid,
    dest_session_id: Uuid,
    source_path: String,
    source_kind: SftpKind,
    dest_path: String,
}

#[derive(Debug, Deserialize)]
struct SftpCancelRelayParams {
    transfer_id: String,
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

async fn require_sftp_profile(state: &AppState, id: Uuid) -> Result<SshProfile, RpcError> {
    state
        .sftp_profiles
        .lock()
        .await
        .get(&id)
        .cloned()
        .ok_or_else(|| {
            RpcError::new(
                ErrorCode::SessionNotFound,
                format!("sftp session profile {id}"),
            )
        })
}

const RELAY_TRANSFER_CHUNK_SIZE: u32 = 1024 * 1024;
const SFTP_CHUNK_IO_TIMEOUT: Duration = Duration::from_secs(120);

fn relay_transfer_canceled(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|flag| flag.load(Ordering::Relaxed))
}

async fn sftp_read_chunk_timed(
    sftp: &Sftp,
    path: &str,
    offset: u64,
    len: u32,
) -> Result<Vec<u8>, RpcError> {
    timeout(
        SFTP_CHUNK_IO_TIMEOUT,
        sftp.read_file_chunk(path, offset, len),
    )
    .await
    .map_err(|_| {
        internal(format!(
            "SFTP read timed out after {}s at offset {offset}",
            SFTP_CHUNK_IO_TIMEOUT.as_secs()
        ))
    })?
    .map_err(|e| internal(e.to_string()))
}

async fn sftp_write_chunk_timed(
    sftp: &Sftp,
    path: &str,
    offset: u64,
    data: &[u8],
    create: bool,
) -> Result<(), RpcError> {
    timeout(
        SFTP_CHUNK_IO_TIMEOUT,
        sftp.write_file_chunk(path, offset, data, create),
    )
    .await
    .map_err(|_| {
        internal(format!(
            "SFTP write timed out after {}s at offset {offset}",
            SFTP_CHUNK_IO_TIMEOUT.as_secs()
        ))
    })?
    .map_err(|e| internal(e.to_string()))
}

fn normalize_remote_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "." {
        return ".".into();
    }
    let absolute = trimmed.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in trimmed.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            parts.pop();
        } else {
            parts.push(part);
        }
    }
    let joined = parts.join("/");
    if absolute {
        if joined.is_empty() {
            "/".into()
        } else {
            format!("/{joined}")
        }
    } else if joined.is_empty() {
        ".".into()
    } else {
        joined
    }
}

fn join_remote_path(base: &str, name: &str) -> String {
    let b = if base.is_empty() { "." } else { base };
    if b == "/" {
        format!("/{name}")
    } else if b.ends_with('/') {
        format!("{b}{name}")
    } else {
        format!("{b}/{name}")
    }
}

fn parent_remote_path(path: &str) -> String {
    let normalized = normalize_remote_path(path);
    if normalized == "/" || normalized == "." {
        return normalized;
    }
    let Some(idx) = normalized.rfind('/') else {
        return ".".into();
    };
    if idx == 0 {
        "/".into()
    } else {
        normalized[..idx].into()
    }
}

#[derive(Clone, Serialize)]
struct RelayTransferProgressEvent {
    transfer_id: String,
    path: String,
    offset: u64,
    total: u64,
}

#[derive(Clone, Serialize)]
struct RelayTransferFileEvent {
    transfer_id: String,
    path: String,
    total: u64,
}

fn emit_relay_progress(transfer_id: &str, path: &str, offset: u64, total: u64) {
    if let Some(app) = APP_HANDLE.get() {
        let _ = app.emit(
            "transfer:relay-progress",
            RelayTransferProgressEvent {
                transfer_id: transfer_id.to_string(),
                path: path.to_string(),
                offset,
                total,
            },
        );
    } else {
        tracing::warn!("relay progress event dropped: AppHandle not set");
    }
}

fn emit_relay_file(transfer_id: &str, path: &str, total: u64) {
    if let Some(app) = APP_HANDLE.get() {
        let _ = app.emit(
            "transfer:relay-file",
            RelayTransferFileEvent {
                transfer_id: transfer_id.to_string(),
                path: path.to_string(),
                total,
            },
        );
    } else {
        tracing::warn!("relay file event dropped: AppHandle not set");
    }
}

#[derive(Clone, Serialize)]
struct DirectTransferHeartbeatEvent {
    transfer_id: String,
}

#[derive(Clone, Serialize)]
struct DirectTransferProgressEvent {
    transfer_id: String,
    percent: Option<u8>,
    transferred_hint: Option<u64>,
}

fn emit_direct_heartbeat(transfer_id: &str) {
    if let Some(app) = APP_HANDLE.get() {
        let _ = app.emit(
            "transfer:direct-heartbeat",
            DirectTransferHeartbeatEvent {
                transfer_id: transfer_id.to_string(),
            },
        );
    }
}

fn emit_direct_progress(transfer_id: &str, percent: Option<u8>, transferred_hint: Option<u64>) {
    if let Some(app) = APP_HANDLE.get() {
        let _ = app.emit(
            "transfer:direct-progress",
            DirectTransferProgressEvent {
                transfer_id: transfer_id.to_string(),
                percent,
                transferred_hint,
            },
        );
    }
}

const DIRECT_CONSENT_SETTINGS_KEY: &str = "sftp.directAgentConsent";

fn load_direct_consent_hosts(store: &crate::settings::SettingsStore) -> Vec<String> {
    match store.get(DIRECT_CONSENT_SETTINGS_KEY) {
        Ok(Some(value)) => value
            .get("hosts")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn remember_direct_consent_host(
    store: &crate::settings::SettingsStore,
    host_key: &str,
) -> Result<(), RpcError> {
    let mut hosts = load_direct_consent_hosts(store);
    if !hosts.iter().any(|h| h == host_key) {
        hosts.push(host_key.to_string());
    }
    let value = json!({ "hosts": hosts });
    store
        .set(DIRECT_CONSENT_SETTINGS_KEY, &value)
        .map_err(|e| internal(e.to_string()))
}

async fn prepare_direct_agent_source(
    dest_profile: &SshProfile,
) -> Result<crate::ssh::AgentSource, RpcError> {
    use crate::ssh::ephemeral_agent::{
        dest_auth_supports_direct, dest_key_for_direct, InMemoryAgent,
    };
    use crate::ssh::AgentSource;

    if !dest_auth_supports_direct(&dest_profile.auth) {
        return Err(invalid_params(
            "destination auth does not support Direct (agent-forward); use Relay",
        ));
    }
    match dest_key_for_direct(&dest_profile.auth).map_err(|e| internal(e.to_string()))? {
        None => Ok(AgentSource::System),
        Some(key) => {
            let agent = InMemoryAgent::with_key(&key)
                .await
                .map_err(|e| internal(e.to_string()))?;
            Ok(AgentSource::InMemory(agent))
        }
    }
}

async fn ensure_remote_dir(sftp: &Sftp, path: &str) -> Result<(), RpcError> {
    let normalized = normalize_remote_path(path);
    if normalized == "/" || normalized == "." {
        return Ok(());
    }
    let absolute = normalized.starts_with('/');
    let segments: Vec<&str> = normalized.split('/').filter(|s| !s.is_empty()).collect();
    let mut cursor = if absolute {
        "/".to_string()
    } else {
        ".".to_string()
    };
    for segment in segments {
        cursor = join_remote_path(&cursor, segment);
        match sftp.stat(&cursor).await {
            Ok(entry) if entry.kind == SftpKind::Dir => {}
            Ok(_) => return Err(internal(format!("{cursor} exists and is not a directory"))),
            Err(_) => {
                if let Err(e) = sftp.mkdir(&cursor).await {
                    match sftp.stat(&cursor).await {
                        Ok(entry) if entry.kind == SftpKind::Dir => {}
                        _ => return Err(internal(e.to_string())),
                    }
                }
            }
        }
    }
    Ok(())
}

async fn relay_copy_file(
    transfer_id: &str,
    src: &Sftp,
    src_path: &str,
    dest: &Sftp,
    dest_path: &str,
    size: u64,
    cancel: Option<&AtomicBool>,
) -> Result<u64, RpcError> {
    ensure_remote_dir(dest, &parent_remote_path(dest_path)).await?;
    if relay_transfer_canceled(cancel) {
        return Err(internal("transfer canceled"));
    }
    if size == 0 {
        sftp_write_chunk_timed(dest, dest_path, 0, &[], true).await?;
        emit_relay_progress(transfer_id, dest_path, 0, 0);
        return Ok(0);
    }
    let mut offset = 0_u64;
    while offset < size {
        if relay_transfer_canceled(cancel) {
            return Err(internal("transfer canceled"));
        }
        let len = RELAY_TRANSFER_CHUNK_SIZE.min((size - offset) as u32);
        let bytes = sftp_read_chunk_timed(src, src_path, offset, len).await?;
        if bytes.is_empty() {
            break;
        }
        sftp_write_chunk_timed(dest, dest_path, offset, &bytes, offset == 0).await?;
        offset += bytes.len() as u64;
        emit_relay_progress(transfer_id, dest_path, offset, size);
    }
    Ok(offset)
}

async fn relay_copy_dir(
    transfer_id: &str,
    src: &Sftp,
    src_path: &str,
    dest: &Sftp,
    dest_path: &str,
    cancel: Option<&AtomicBool>,
) -> Result<u64, RpcError> {
    if relay_transfer_canceled(cancel) {
        return Err(internal("transfer canceled"));
    }
    ensure_remote_dir(dest, dest_path).await?;
    let entries = src
        .read_dir(src_path)
        .await
        .map_err(|e| internal(e.to_string()))?;
    let mut copied = 0_u64;
    for entry in entries {
        if relay_transfer_canceled(cancel) {
            return Err(internal("transfer canceled"));
        }
        let child_src = join_remote_path(src_path, &entry.name);
        let child_dest = join_remote_path(dest_path, &entry.name);
        match entry.kind {
            SftpKind::Dir => {
                copied += Box::pin(relay_copy_dir(
                    transfer_id,
                    src,
                    &child_src,
                    dest,
                    &child_dest,
                    cancel,
                ))
                .await?
            }
            SftpKind::File => {
                emit_relay_file(transfer_id, &child_dest, entry.size);
                copied += relay_copy_file(
                    transfer_id,
                    src,
                    &child_src,
                    dest,
                    &child_dest,
                    entry.size,
                    cancel,
                )
                .await?
            }
            _ => {}
        }
    }
    Ok(copied)
}

fn register_sftp(dispatcher: &Dispatcher, state: Arc<AppState>) {
    {
        let st = state.clone();
        dispatcher.register("sftp.open", move |params| {
            let st = st.clone();
            async move {
                let p: SftpOpenParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let profile = match (p.profile_id, p.profile) {
                    (Some(id), _) => resolve_ssh_profile(&st, id).await?,
                    (None, Some(profile)) => materialize_ssh_profile(&st, profile).await?,
                    _ => return Err(invalid_params("profile_id or profile required")),
                };
                let kh = st.known_hosts.lock().await.clone();
                let transport = load_ssh_transport_settings(&st).await;
                let sftp = Sftp::open_with_options(
                    &profile,
                    kh,
                    SftpOpenOptions { sudo: p.sudo },
                    transport,
                )
                .await
                .map_err(|e| internal(e.to_string()))?;
                let id = Uuid::new_v4();
                st.sftp_sessions.lock().await.insert(id, Arc::new(sftp));
                st.sftp_profiles.lock().await.insert(id, profile);
                Ok(json!({ "id": id }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sftp.openForSession", move |params| {
            let st = st.clone();
            async move {
                let p: SftpOpenForSessionParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let sid = Uuid::parse_str(&p.session_id)
                    .map_err(|e| invalid_params(format!("invalid session_id: {e}")))?;
                let session_id = SessionId(sid);
                let shell = {
                    let chans = st.channels.lock().await;
                    let Some(SessionChannel::Ssh(entry)) = chans.get(&session_id) else {
                        return Err(invalid_params(
                            "session is not an open SSH terminal; use sftp.open instead",
                        ));
                    };
                    entry.shell.clone()
                };
                let sftp = shell
                    .open_sftp(SftpOpenOptions { sudo: p.sudo })
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
                st.sftp_profiles.lock().await.remove(&p.id);
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
                let entries = match sftp.read_dir(&p.path).await {
                    Ok(entries) => entries,
                    Err(e) => {
                        let msg = e.to_string();
                        if is_dead_sftp_channel(&msg) {
                            st.sftp_sessions.lock().await.remove(&p.id);
                        }
                        return Err(internal(msg));
                    }
                };
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
                let bytes = sftp_read_chunk_timed(&sftp, &p.path, p.offset, p.len).await?;
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
                sftp_write_chunk_timed(&sftp, &p.path, p.offset, &bytes, p.create).await?;
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

    {
        let st = state.clone();
        dispatcher.register("sftp.directPreflight", move |params| {
            let st = st.clone();
            async move {
                let p: SftpDirectPreflightParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let source_profile = require_sftp_profile(&st, p.source_session_id).await?;
                let dest_profile = require_sftp_profile(&st, p.dest_session_id).await?;
                let source_host_key =
                    crate::remote_transfer::source_host_consent_key(&source_profile);
                let mut reasons = Vec::new();
                let dest_auth_ok =
                    crate::ssh::ephemeral_agent::dest_auth_supports_direct(&dest_profile.auth);
                if !dest_auth_ok {
                    reasons.push("dest_auth_unsupported".into());
                }
                let consented = if let Ok(store) = require_settings(&st).await {
                    load_direct_consent_hosts(&store)
                        .iter()
                        .any(|h| h == &source_host_key)
                } else {
                    false
                };
                let needs_consent = !consented;
                let mut has_rsync = false;
                let mut has_scp = false;
                if p.probe_tools && dest_auth_ok {
                    let kh = st.known_hosts.lock().await.clone();
                    let transport = load_ssh_transport_settings(&st).await;
                    match crate::remote_transfer::probe_source_transfer_tools(
                        &source_profile,
                        kh,
                        transport,
                    )
                    .await
                    {
                        Ok((r, s)) => {
                            has_rsync = r;
                            has_scp = s;
                            if !has_rsync && !has_scp {
                                reasons.push("missing_rsync_or_scp".into());
                            }
                        }
                        Err(e) => {
                            reasons.push(format!("source_probe_failed:{e}"));
                        }
                    }
                }
                let can_direct = dest_auth_ok && (has_rsync || has_scp || !p.probe_tools);
                Ok(json!({
                    "ok": can_direct && reasons.is_empty(),
                    "can_direct": can_direct,
                    "needs_consent": needs_consent,
                    "source_host_key": source_host_key,
                    "has_rsync": has_rsync,
                    "has_scp": has_scp,
                    "dest_auth_ok": dest_auth_ok,
                    "reasons": reasons,
                }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sftp.rememberDirectConsent", move |params| {
            let st = st.clone();
            async move {
                let p: SftpRememberDirectConsentParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                if p.source_host_key.trim().is_empty() {
                    return Err(invalid_params("source_host_key is required"));
                }
                let store = require_settings(&st).await?;
                remember_direct_consent_host(&store, p.source_host_key.trim())?;
                Ok(json!({ "remembered": true }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sftp.directTransfer", move |params| {
            let st = st.clone();
            async move {
                let p: SftpDirectTransferParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                if p.transfer_id.trim().is_empty() {
                    return Err(invalid_params("transfer_id is required"));
                }
                let source_profile = require_sftp_profile(&st, p.source_session_id).await?;
                let dest_profile = require_sftp_profile(&st, p.dest_session_id).await?;
                let source_host_key =
                    crate::remote_transfer::source_host_consent_key(&source_profile);
                let consented = if let Ok(store) = require_settings(&st).await {
                    load_direct_consent_hosts(&store)
                        .iter()
                        .any(|h| h == &source_host_key)
                } else {
                    false
                };
                if !consented && !p.consent_granted {
                    return Err(invalid_params(
                        "agent-forward consent required for Direct transfer on this source host",
                    ));
                }
                let target = crate::remote_transfer::DirectTransferTarget {
                    user: dest_profile.user.clone(),
                    host: dest_profile.host.clone(),
                    port: dest_profile.port,
                    path: p.dest_path,
                };
                let kh = st.known_hosts.lock().await.clone();
                let transport = load_ssh_transport_settings(&st).await;
                let agent_source = prepare_direct_agent_source(&dest_profile).await?;
                let cancel = Arc::new(AtomicBool::new(false));
                st.relay_cancel
                    .lock()
                    .await
                    .insert(p.transfer_id.clone(), cancel.clone());
                let transfer_id = p.transfer_id.clone();
                let on_heartbeat: crate::remote_transfer::DirectHeartbeatFn =
                    Arc::new(move || emit_direct_heartbeat(&transfer_id));
                let transfer_id_progress = p.transfer_id.clone();
                let on_progress: crate::remote_transfer::DirectProgressFn = Arc::new(move |hint| {
                    emit_direct_progress(
                        &transfer_id_progress,
                        hint.percent,
                        hint.transferred_hint,
                    );
                });
                let result = crate::remote_transfer::run_direct_transfer(
                    &source_profile,
                    &p.source_path,
                    p.kind,
                    &target,
                    kh,
                    transport,
                    agent_source,
                    Some(cancel),
                    Some(on_heartbeat),
                    Some(on_progress),
                )
                .await;
                st.relay_cancel.lock().await.remove(&p.transfer_id);
                let output = result.map_err(|e| {
                    if e.to_string().contains("canceled") {
                        RpcError::new(ErrorCode::InternalError, "direct transfer canceled")
                    } else {
                        internal(e.to_string())
                    }
                })?;
                serde_json::to_value(output).map_err(|e| internal(e.to_string()))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sftp.relayTransfer", move |params| {
            let st = st.clone();
            async move {
                let p: SftpRelayTransferParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let cancel = Arc::new(AtomicBool::new(false));
                st.relay_cancel
                    .lock()
                    .await
                    .insert(p.transfer_id.clone(), cancel.clone());
                let result: Result<Value, RpcError> = async {
                    let src = require_sftp(&st, p.source_session_id).await?;
                    let dest = require_sftp(&st, p.dest_session_id).await?;
                    let cancel_ref = cancel.as_ref();
                    let copied = match p.source_kind {
                        SftpKind::Dir => {
                            relay_copy_dir(
                                &p.transfer_id,
                                &src,
                                &p.source_path,
                                &dest,
                                &p.dest_path,
                                Some(cancel_ref),
                            )
                            .await?
                        }
                        SftpKind::File => {
                            let stat = src
                                .stat(&p.source_path)
                                .await
                                .map_err(|e| internal(e.to_string()))?;
                            emit_relay_file(&p.transfer_id, &p.dest_path, stat.size);
                            relay_copy_file(
                                &p.transfer_id,
                                &src,
                                &p.source_path,
                                &dest,
                                &p.dest_path,
                                stat.size,
                                Some(cancel_ref),
                            )
                            .await?
                        }
                        _ => {
                            return Err(invalid_params(
                                "relay transfer only supports files and directories",
                            ))
                        }
                    };
                    Ok(json!({ "method": "relay", "bytes": copied }))
                }
                .await;
                st.relay_cancel.lock().await.remove(&p.transfer_id);
                result
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sftp.cancelRelayTransfer", move |params| {
            let st = st.clone();
            async move {
                let p: SftpCancelRelayParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let canceled = st
                    .relay_cancel
                    .lock()
                    .await
                    .get(&p.transfer_id)
                    .is_some_and(|flag| {
                        flag.store(true, Ordering::Relaxed);
                        true
                    });
                Ok(json!({ "canceled": canceled }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sftp.cancelDirectTransfer", move |params| {
            let st = st.clone();
            async move {
                let p: SftpCancelRelayParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let canceled = st
                    .relay_cancel
                    .lock()
                    .await
                    .get(&p.transfer_id)
                    .is_some_and(|flag| {
                        flag.store(true, Ordering::Relaxed);
                        true
                    });
                Ok(json!({ "canceled": canceled }))
            }
        });
    }
}

fn is_dead_sftp_channel(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("session closed")
        || m.contains("channel closed")
        || m.contains("connection reset")
        || m.contains("broken pipe")
        || m.contains("not connected")
}

async fn load_ssh_transport_settings(st: &AppState) -> ssh::SshTransportSettings {
    let Ok(store) = require_settings(st).await else {
        return ssh::SshTransportSettings::default();
    };
    let Ok(entry) = store.get("ssh") else {
        return ssh::SshTransportSettings::default();
    };
    let Some(value) = entry else {
        return ssh::SshTransportSettings::default();
    };
    let Ok(v) = serde_json::from_value::<serde_json::Value>(value) else {
        return ssh::SshTransportSettings::default();
    };
    let mut out = ssh::SshTransportSettings {
        // Missing legacy settings retain auto; malformed values fail closed.
        agent: v
            .get("agent")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .unwrap_or(Some(ssh::AgentPreference::None))
            .unwrap_or_default(),
        ..Default::default()
    };
    let interval_secs = v
        .get("keepaliveInterval")
        .and_then(|x| x.as_u64())
        .unwrap_or(45)
        .clamp(15, 600);
    let keepalive_max = v
        .get("keepaliveCountMax")
        .and_then(|x| x.as_u64())
        .unwrap_or(10)
        .clamp(3, 30) as usize;
    let server_alive = v
        .get("serverAliveInterval")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let interval_secs = if server_alive > 0 {
        interval_secs.min(server_alive).max(15)
    } else {
        interval_secs
    };
    out.keepalive_interval = std::time::Duration::from_secs(interval_secs);
    out.keepalive_max = keepalive_max;
    // Allow long idle SFTP browsing sessions on high-latency links.
    out.inactivity_timeout = std::time::Duration::from_secs(60 * 60 * 6);
    out
}

#[cfg(test)]
mod ssh_agent_settings_tests {
    use super::*;

    #[tokio::test]
    async fn saved_agent_preference_reaches_transport() {
        let dir = std::env::temp_dir().join(format!("aerotab-agent-settings-{}", Uuid::new_v4()));
        let state = AppState::new();
        let store = SettingsStore::open(&dir).unwrap();
        *state.settings.lock().await = Some(store.clone());
        for preference in ["auto", "system", "pageant", "none"] {
            store.set("ssh", &json!({"agent":preference})).unwrap();
            let transport = load_ssh_transport_settings(&state).await;
            assert_eq!(serde_json::to_value(transport.agent).unwrap(), preference);
        }
        drop(state);
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
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
        display: v
            .get("x11Display")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
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

async fn load_group_defaults(
    st: &AppState,
) -> Result<crate::profile_inheritance::GroupDefaults, RpcError> {
    let Some(store) = st.settings.lock().await.clone() else {
        return Ok(Default::default());
    };
    let Some(value) = store
        .get(crate::profile_inheritance::SETTINGS_KEY)
        .map_err(|e| internal(e.to_string()))?
    else {
        return Ok(Default::default());
    };
    crate::profile_inheritance::parse_defaults(&value).map_err(invalid_params)
}

async fn load_proxies(
    st: &AppState,
) -> Result<Vec<crate::network_resources::ProxyResource>, RpcError> {
    match st.network_resources.lock().await.clone() {
        Some(store) => store.list_proxies().map_err(|e| internal(e.to_string())),
        None => Ok(vec![]),
    }
}

async fn validate_affected_profiles(
    st: &AppState,
    catalog: &[Profile],
    affected: std::collections::HashSet<Uuid>,
) -> Result<(), RpcError> {
    crate::profile_inheritance::validate_affected(catalog, &load_proxies(st).await?, affected)
        .map_err(invalid_params)
}

async fn load_effective_catalog(st: &AppState) -> Result<Vec<Profile>, RpcError> {
    let profiles = match st.profiles.lock().await.clone() {
        Some(store) => store.list().await.map_err(|e| internal(e.to_string()))?,
        None => vec![],
    };
    crate::profile_inheritance::merge_catalog(&profiles, &load_group_defaults(st).await?)
        .map_err(invalid_params)
}

async fn materialize_ssh_profile(
    st: &AppState,
    mut ssh: SshProfile,
) -> Result<SshProfile, RpcError> {
    let profiles = load_effective_catalog(st).await?;
    crate::profile_inheritance::validate_ssh_graph(&ssh, &profiles, &load_proxies(st).await?, None)
        .map_err(invalid_params)?;
    let network = st.network_resources.lock().await.clone();
    let vault = st.vault.lock().await.clone();
    fn attach(
        ssh: &mut SshProfile,
        profiles: &[Profile],
        network: Option<&crate::network_resources::NetworkResourceStore>,
        vault: &Option<crate::vault::VaultStore>,
        path: &mut Vec<Uuid>,
    ) -> Result<(), RpcError> {
        if path.len() > 32 {
            return Err(invalid_params("proxy chain exceeds 32 levels"));
        }
        if let Some(id) = ssh.proxy_id {
            if path.contains(&id) {
                return Err(invalid_params("proxy cycle"));
            }
            let proxy = network
                .ok_or_else(|| internal("proxy store not configured"))?
                .get_proxy(id)
                .map_err(|e| internal(e.to_string()))?
                .ok_or_else(|| invalid_params("proxy resource not found"))?;
            path.push(id);
            match proxy.config {
                crate::network_resources::ProxyConfig::SshJump { profile_id } => {
                    let p = profiles
                        .iter()
                        .find(|p| p.id == profile_id)
                        .ok_or_else(|| invalid_params("proxy SSH profile not found"))?;
                    let ProfileKind::Ssh { ssh: hop } = &p.spec else {
                        return Err(invalid_params("proxy target is not SSH"));
                    };
                    let hop = crate::profile::resolve_jump_references(hop, profiles, &mut vec![])
                        .map_err(|e| invalid_params(e.to_string()))?;
                    if !ssh.jump_profile_ids.is_empty() || !ssh.jump_via.is_empty() {
                        return Err(invalid_params(
                            "SSH proxy and jump chain cannot be combined",
                        ));
                    }
                    ssh.jump_via.push(hop);
                }
                _ => {
                    ssh.resolved_proxy = Some(Arc::new(crate::proxy_transport::ResolvedProxy {
                        resource: proxy,
                        vault: vault.clone(),
                    }))
                }
            }
            path.pop();
            ssh.proxy_id = None;
        }
        let mut flat = Vec::new();
        for mut hop in std::mem::take(&mut ssh.jump_via) {
            attach(&mut hop, profiles, network, vault, path)?;
            flat.append(&mut hop.jump_via);
            flat.push(hop);
        }
        ssh.jump_via = flat;
        Ok(())
    }
    ssh = crate::profile::resolve_jump_references(&ssh, &profiles, &mut vec![])
        .map_err(|e| invalid_params(e.to_string()))?;
    attach(&mut ssh, &profiles, network.as_ref(), &vault, &mut vec![])?;
    ssh::vault_resolve::resolve_profile_vault_auth(vault.as_ref(), &mut ssh)
        .await
        .map_err(|e| internal(e.to_string()))?;
    Ok(ssh)
}

async fn resolve_ssh_profile(st: &AppState, profile_id: Uuid) -> Result<SshProfile, RpcError> {
    let profile = load_effective_catalog(st)
        .await?
        .into_iter()
        .find(|p| p.id == profile_id)
        .ok_or_else(|| invalid_params(format!("profile not found: {profile_id}")))?;
    match profile.spec {
        ProfileKind::Ssh { ssh } => materialize_ssh_profile(st, ssh).await,
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
        let bind_port = if local_port > 0 { local_port } else { 0 };
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
    #[serde(default)]
    profile_id: Option<Uuid>,
    #[serde(default)]
    profile: Option<SshProfile>,
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
                let profile = match (p.profile_id, p.profile) {
                    (Some(id), _) => resolve_ssh_profile(&st, id).await?,
                    (None, Some(profile)) => materialize_ssh_profile(&st, profile).await?,
                    _ => return Err(invalid_params("profile_id or profile required")),
                };
                let req = TunnelOpenRequest {
                    profile,
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
        serde_json::to_value(oauth::OAuthStatus {
            provider: prov,
            connected,
        })
        .map_err(|e| internal(e.to_string()))
    });
    dispatcher.register("sync.oauthClear", move |params| async move {
        let p: OAuthProviderParams =
            serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
        let prov = parse_oauth_provider(&p.provider)?;
        oauth::clear_token(prov).map_err(|e| internal(e.to_string()))?;
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
                let p: RemoteOpenProfileParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let profile = load_effective_catalog(&st)
                    .await?
                    .into_iter()
                    .find(|profile| profile.id == p.profile_id)
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
                if p.key == "ai" {
                    crate::ai::migrate(&store).map_err(internal)?;
                }
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
                if p.key == "ai" || p.key.starts_with("ai.") {
                    return Err(invalid_params(
                        "AI configuration must use device-local ai.config / keyring RPCs",
                    ));
                }
                let store = require_settings(&st).await?;
                if p.key == crate::profile_inheritance::SETTINGS_KEY {
                    let defaults = crate::profile_inheritance::parse_defaults(&p.value)
                        .map_err(invalid_params)?;
                    let profiles = match st.profiles.lock().await.clone() {
                        Some(store) => store.list().await.map_err(|e| internal(e.to_string()))?,
                        None => vec![],
                    };
                    let previous = load_group_defaults(&st).await?;
                    let changed = defaults
                        .iter()
                        .map(|(path, fields)| {
                            let changed_fields = fields
                                .iter()
                                .filter(|(field, value)| {
                                    previous.get(path).and_then(|old| old.get(*field))
                                        != Some(*value)
                                })
                                .map(|(field, value)| (field.clone(), value.clone()))
                                .collect();
                            (path.clone(), changed_fields)
                        })
                        .collect();
                    let effective = crate::profile_inheritance::merge_catalog(&profiles, &defaults)
                        .map_err(invalid_params)?;
                    crate::profile_inheritance::validate_default_references(
                        &changed,
                        &effective,
                        &load_proxies(&st).await?,
                    )
                    .map_err(invalid_params)?;
                    let old = crate::profile_inheritance::merge_catalog(&profiles, &previous)
                        .map_err(invalid_params)?;
                    let affected = effective
                        .iter()
                        .zip(&old)
                        .filter(|(new, old)| {
                            serde_json::to_value(new).ok() != serde_json::to_value(old).ok()
                        })
                        .map(|(p, _)| p.id)
                        .collect();
                    validate_affected_profiles(&st, &effective, affected).await?;
                }
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
                let mut entries = store.all().map_err(|e| internal(e.to_string()))?;
                for entry in &mut entries {
                    if entry.key == "ai" {
                        if let Some(value) = entry.value.as_object_mut() {
                            value.remove("apiKey");
                        }
                    }
                }
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

#[path = "sync/cloud_commands.rs"]
mod cloud_commands;
#[path = "sync/maintenance_commands.rs"]
mod maintenance_commands;
#[path = "sync/projection.rs"]
mod projection;

fn register_sync(dispatcher: &Dispatcher, state: Arc<AppState>) {
    cloud_commands::register(dispatcher, state.clone());
    maintenance_commands::register(dispatcher, state.clone());
    {
        let st = state.clone();
        dispatcher.register("sync.configureWebdav", move |params| {
            let st = st.clone();
            async move {
                let _operation = st.sync_operation.lock().await;
                let p: ConfigureWebDavParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let target = format!(
                    "webdav:{:?}:{:?}:{:?}",
                    p.base_url,
                    p.user,
                    p.password
                        .as_ref()
                        .map(|s| Uuid::new_v5(&Uuid::NAMESPACE_OID, s.as_bytes()))
                );
                let backend = Arc::new(WebDavBackend::with_auth(p.base_url, p.user, p.password));
                let blobs: Arc<dyn BlobStore> = backend.clone();
                let params_kdf = if p.test_cheap_kdf {
                    KdfParams::test_cheap()
                } else {
                    KdfParams::DEFAULT
                };
                let device_id = if let Some(dir) = p.state_dir.as_ref() {
                    crate::sync::persistence::device_identity(
                        std::path::Path::new(dir),
                        p.device_id,
                    )
                    .map_err(|e| internal(e.to_string()))?
                } else if let Some(settings) = st.settings.lock().await.as_ref() {
                    let existing = settings
                        .local_get("sync.deviceId")
                        .map_err(|e| internal(e.to_string()))?;
                    if let Some(value) = existing {
                        serde_json::from_value(value)
                            .map_err(|e| internal(format!("invalid device identity: {e}")))?
                    } else {
                        let id = p.device_id.unwrap_or_else(Uuid::new_v4);
                        settings
                            .local_set("sync.deviceId", &json!(id))
                            .map_err(|e| internal(e.to_string()))?;
                        id
                    }
                } else {
                    p.device_id.unwrap_or_else(Uuid::new_v4)
                };
                let password = p.master_password.into_bytes();
                let mut engine = SyncEngine::new(device_id, password.clone(), params_kdf, backend);
                if let Some(dir) = p.state_dir.as_ref() {
                    let store =
                        Arc::new(SledStore::open(dir).map_err(|e| internal(e.to_string()))?);
                    engine = engine
                        .with_store(store)
                        .await
                        .map_err(|e| internal(e.to_string()))?;
                }
                if p.state_dir.is_none() {
                    if let Some(settings) = st.settings.lock().await.clone() {
                        engine = engine
                            .with_store(Arc::new(crate::sync::persistence::SettingsSyncStore(
                                settings,
                            )))
                            .await
                            .map_err(|e| internal(e.to_string()))?;
                    }
                }
                let mut driver = OplogDriver::new(blobs, device_id, password.clone(), params_kdf)
                    .with_protocol(p.protocol)
                    .map_err(|e| invalid_params(e.to_string()))?
                    .with_target(&target);
                if let Some(dir) = p.state_dir.as_ref() {
                    driver = driver
                        .with_state_file(std::path::Path::new(dir).join("oplog.json"))
                        .await
                        .map_err(|e| internal(e.to_string()))?;
                }
                if p.state_dir.is_none() {
                    if let Some(settings) = st.settings.lock().await.clone() {
                        driver = driver
                            .with_settings(settings)
                            .map_err(|e| internal(e.to_string()))?;
                    }
                }
                let oplog = Arc::new(driver);
                // Configuration is local only; never publish a snapshot before
                // fetching and reconciling the remote.
                stop_auto(&st).await;
                configure_sync_approval(&st, &target, p.state_dir.as_deref(), &oplog).await?;
                *st.sync.lock().await = Some(Arc::new(engine));
                *st.sync_kind.lock().await = Some("webdav");
                *st.git_backend.lock().await = None;
                *st.oplog.lock().await = Some(oplog);
                *st.oplog_last.lock().await = None;
                Ok(json!({ "device_id": device_id, "protocol": p.protocol }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sync.configureGit", move |params| {
            let st = st.clone();
            async move {
                let _operation = st.sync_operation.lock().await;
                let p: ConfigureGitParams =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let mut backend =
                    GitBackend::open_or_init(&p.repo_path).map_err(|e| internal(e.to_string()))?;
                if let (Some(name), Some(email)) = (p.author_name, p.author_email) {
                    backend = backend.with_author(name, email);
                }
                if let Some(url) = p.remote_url.as_ref() {
                    use crate::sync::backends::git::{align_git_remote_url, GitRemoteTransport};
                    let transport = match p.remote_auth.as_deref() {
                        Some("ssh") => GitRemoteTransport::Ssh,
                        Some("https") => GitRemoteTransport::Https,
                        _ if p.oauth_provider.is_some()
                            || p.remote_password.as_ref().is_some_and(|s| !s.is_empty()) =>
                        {
                            GitRemoteTransport::Https
                        }
                        _ if p.remote_ssh_key.is_some() => GitRemoteTransport::Ssh,
                        _ => GitRemoteTransport::Unspecified,
                    };
                    let url = align_git_remote_url(url, transport, p.remote_ssh_port);
                    backend = backend
                        .with_remote(&p.remote_name, &url, &p.remote_branch)
                        .map_err(|e| internal(e.to_string()))?;
                    if let Some(ref provider) = p.oauth_provider {
                        let prov = parse_oauth_provider(provider)?;
                        if let Some(token) =
                            oauth::load_token(prov).map_err(|e| internal(e.to_string()))?
                        {
                            let (user, pass) = oauth_https_credentials(prov, token);
                            backend = backend.with_https_auth(user, pass);
                        } else {
                            return Err(invalid_params(
                                "OAuth token missing; complete device sign-in first",
                            ));
                        }
                    } else if let Some(pw) = p.remote_password.filter(|s| !s.is_empty()) {
                        let user = p
                            .remote_user
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| "oauth2".into());
                        backend = backend.with_https_auth(user, pw);
                    }
                    if let Some(key) = p.remote_ssh_key.filter(|s| !s.is_empty()) {
                        if transport == GitRemoteTransport::Ssh {
                            backend = backend.with_ssh_key(key, p.remote_ssh_passphrase);
                        }
                    }
                }
                let configured_backend = backend.clone();
                let target = backend.target_identity();
                let blobs: Arc<dyn BlobStore> = Arc::new(backend.clone());
                let backend_dyn: Arc<dyn crate::sync::SyncBackend> = Arc::new(backend);
                let params_kdf = if p.test_cheap_kdf {
                    KdfParams::test_cheap()
                } else {
                    KdfParams::DEFAULT
                };
                let device_id = if let Some(dir) = p.state_dir.as_ref() {
                    crate::sync::persistence::device_identity(
                        std::path::Path::new(dir),
                        p.device_id,
                    )
                    .map_err(|e| internal(e.to_string()))?
                } else if let Some(settings) = st.settings.lock().await.as_ref() {
                    let existing = settings
                        .local_get("sync.deviceId")
                        .map_err(|e| internal(e.to_string()))?;
                    if let Some(value) = existing {
                        serde_json::from_value(value)
                            .map_err(|e| internal(format!("invalid device identity: {e}")))?
                    } else {
                        let id = p.device_id.unwrap_or_else(Uuid::new_v4);
                        settings
                            .local_set("sync.deviceId", &json!(id))
                            .map_err(|e| internal(e.to_string()))?;
                        id
                    }
                } else {
                    p.device_id.unwrap_or_else(Uuid::new_v4)
                };
                let password = p.master_password.into_bytes();
                let mut engine =
                    SyncEngine::new(device_id, password.clone(), params_kdf, backend_dyn);
                if let Some(dir) = p.state_dir.as_ref() {
                    let store =
                        Arc::new(SledStore::open(dir).map_err(|e| internal(e.to_string()))?);
                    engine = engine
                        .with_store(store)
                        .await
                        .map_err(|e| internal(e.to_string()))?;
                }
                if p.state_dir.is_none() {
                    if let Some(settings) = st.settings.lock().await.clone() {
                        engine = engine
                            .with_store(Arc::new(crate::sync::persistence::SettingsSyncStore(
                                settings,
                            )))
                            .await
                            .map_err(|e| internal(e.to_string()))?;
                    }
                }
                let mut driver = OplogDriver::new(blobs, device_id, password.clone(), params_kdf)
                    .with_protocol(p.protocol)
                    .map_err(|e| invalid_params(e.to_string()))?
                    .with_target(&target);
                if let Some(dir) = p.state_dir.as_ref() {
                    driver = driver
                        .with_state_file(std::path::Path::new(dir).join("oplog.json"))
                        .await
                        .map_err(|e| internal(e.to_string()))?;
                }
                if p.state_dir.is_none() {
                    if let Some(settings) = st.settings.lock().await.clone() {
                        driver = driver
                            .with_settings(settings)
                            .map_err(|e| internal(e.to_string()))?;
                    }
                }
                let oplog = Arc::new(driver);
                // Git acknowledgement happens only after commit + remote push.
                let oplog = Arc::new(
                    Arc::try_unwrap(oplog)
                        .map_err(|_| internal("oplog shared during configure"))?
                        .defer_upload_confirmation(),
                );
                // Snapshot publication belongs to an explicit reconciled cycle.
                stop_auto(&st).await;
                configure_sync_approval(&st, &target, p.state_dir.as_deref(), &oplog).await?;
                *st.sync.lock().await = Some(Arc::new(engine));
                *st.sync_kind.lock().await = Some("git");
                *st.git_backend.lock().await = Some(configured_backend);
                *st.oplog.lock().await = Some(oplog);
                *st.oplog_last.lock().await = None;
                Ok(json!({ "device_id": device_id, "protocol": p.protocol }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sync.put", move |params| {
            let st = st.clone();
            async move {
                let _operation = st.sync_operation.lock().await;
                st.sync_approval.lock().await.pending = None;
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
                let _operation = st.sync_operation.lock().await;
                st.sync_approval.lock().await.pending = None;
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
        dispatcher.register("sync.ensureVaultUnlock", move |params| {
            let st = st.clone();
            async move {
                #[derive(Deserialize)]
                struct EnsureVaultUnlockParams {
                    password: Option<String>,
                    account: Option<String>,
                }
                let p: EnsureVaultUnlockParams = if params.is_null() {
                    EnsureVaultUnlockParams {
                        password: None,
                        account: None,
                    }
                } else {
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?
                };
                let vault = match st.vault.lock().await.clone() {
                    Some(v) => v,
                    None => {
                        return Ok(json!({
                            "configured": false,
                            "initialized": false,
                            "unlocked": false,
                        }));
                    }
                };
                let settings = require_settings(&st).await.ok();
                let initialized = vault
                    .is_initialized()
                    .map_err(|e| internal(e.to_string()))?;
                if !initialized {
                    return Ok(json!({
                        "configured": true,
                        "initialized": false,
                        "unlocked": false,
                    }));
                }
                let unlocked = if vault.is_unlocked().await {
                    true
                } else {
                    crate::sync::bridge::try_unlock_vault_for_sync(
                        &vault,
                        settings.as_ref(),
                        p.password.as_deref(),
                        p.account.as_deref(),
                    )
                    .await
                    .map_err(|e| internal(e.to_string()))?
                };
                Ok(json!({
                    "configured": true,
                    "initialized": true,
                    "unlocked": unlocked,
                }))
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
                let (results, bridge_notes) =
                    match run_sync_now_cycle(&st, &groups, p.preview_token).await {
                        Ok(v) => {
                            let at_ms = now_ms();
                            *st.sync_last_ms.lock().await = Some(at_ms);
                            record_sync_history_ok(&st, "manual", &groups, &v.0, at_ms).await;
                            v
                        }
                        Err(e) => {
                            record_sync_history_err(&st, "manual", &groups, &e).await;
                            return Err(e);
                        }
                    };
                let mut obj: serde_json::Map<String, Value> = results
                    .into_iter()
                    .map(|(g, s)| {
                        let key = format!("{g:?}");
                        let v = serde_json::to_value(s).unwrap_or(Value::Null);
                        (key, v)
                    })
                    .collect();
                obj.insert(
                    "_bridge".into(),
                    serde_json::to_value(bridge_notes).unwrap_or(Value::Null),
                );
                if let Some(meta) = *st.oplog_last.lock().await {
                    obj.insert(
                        "_oplog".into(),
                        serde_json::to_value(meta).unwrap_or(Value::Null),
                    );
                }
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
                require_engine(&st).await?;
                require_completed_sync_setup(&st, &selected_sync_groups(p.groups.clone()), None)
                    .await?;
                stop_auto(&st).await;
                let interval = std::time::Duration::from_millis(p.interval_ms);
                let groups = selected_sync_groups(p.groups);
                let st_immediate = st.clone();
                let groups_now = groups.clone();
                // Run one cycle immediately so users see activity without waiting a full interval.
                match run_sync_now_cycle(&st_immediate, &groups_now, None).await {
                    Ok((results, _)) => {
                        let at_ms = now_ms();
                        *st_immediate.sync_last_ms.lock().await = Some(at_ms);
                        record_sync_history_ok(&st_immediate, "auto", &groups_now, &results, at_ms)
                            .await;
                    }
                    Err(e) => {
                        tracing::warn!(error = ?e, "auto-sync initial cycle failed");
                        record_sync_history_err(&st_immediate, "auto", &groups_now, &e).await;
                    }
                }
                let st_tick = st.clone();
                let handle = tokio::spawn(async move {
                    let mut ticker = tokio::time::interval(interval);
                    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                    // `interval.tick()` is ready immediately; skip it because we already
                    // ran one cycle synchronously above.
                    ticker.tick().await;
                    loop {
                        ticker.tick().await;
                        match run_sync_now_cycle(&st_tick, &groups, None).await {
                            Ok((results, _)) => {
                                let at_ms = now_ms();
                                *st_tick.sync_last_ms.lock().await = Some(at_ms);
                                record_sync_history_ok(&st_tick, "auto", &groups, &results, at_ms)
                                    .await;
                            }
                            Err(e) => {
                                tracing::warn!(error = ?e, "auto-sync tick failed");
                                record_sync_history_err(&st_tick, "auto", &groups, &e).await;
                            }
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
                let last_sync_ms = latest_successful_sync_ms(&st).await;
                let auto_interval_ms = *st.sync_auto_interval_ms.lock().await;
                let auto_running = st.sync_auto.lock().await.is_some();
                let protocol = st.oplog.lock().await.as_ref().map(|d|d.protocol()).unwrap_or(1);
                let cursors = if let Some(oplog) = st.oplog.lock().await.as_ref() {
                    Some(
                        oplog
                            .cursors_snapshot()
                            .await
                            .into_iter()
                            .map(|(k, v)| (k.to_string(), v))
                            .collect::<std::collections::BTreeMap<_, _>>(),
                    )
                } else {
                    None
                };
                let own_seq = st.oplog.lock().await.as_ref().map(|o| o.own_seq());
                let last_oplog = *st.oplog_last.lock().await;
                Ok(json!({
                    "configured": configured,
                    "kind": kind,
                    "deviceId": device_id,
                    "lastSyncMs": last_sync_ms,
                    "autoIntervalMs": auto_interval_ms,
                    "autoRunning": auto_running,
                    "notesPausedDirty": st.notes.lock().await.as_ref().is_some_and(|s|s.sync_paused()),
                    "protocol": protocol,
                    "cursors": cursors,
                    "ownSeq": own_seq,
                    "lastOplog": last_oplog,
                }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sync.capacity", move |_params| {
            let st = st.clone();
            async move {
                let _operation = st.sync_operation.lock().await;
                if let Some(git) = st.git_backend.lock().await.clone() {
                    return serde_json::to_value(git.capacity().await.map_err(|e| internal(e.to_string()))?).map_err(|e| internal(e.to_string()));
                }
                let driver = st.oplog.lock().await.clone().ok_or_else(|| invalid_params("sync not configured"))?;
                let files = driver.blobs().list_blobs("").await.map_err(|e| internal(e.to_string()))?;
                let total: u64 = files.iter().map(|f| f.size).sum();
                let ops: Vec<_> = files.iter().filter(|f| f.path.contains("/ops/")).collect();
                let snaps: Vec<_> = files.iter().filter(|f| f.path.contains("/snapshots/")).collect();
                Ok(json!({"worktreeBytes": total, "gitBytes": null, "totalBytes": total,
                    "opBytes": ops.iter().map(|f| f.size).sum::<u64>(), "opFiles": ops.len(),
                    "snapshotBytes": snaps.iter().map(|f| f.size).sum::<u64>(), "snapshotFiles": snaps.len(),
                    "commits": null, "historyRewriteAllowed": false, "scope": "remote listed payload bytes"}))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sync.rewriteHistory", move |params| {
            let st = st.clone();
            async move {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P { confirmation: String, expected_oid: String }
                let p: P = serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                if p.confirmation != "REWRITE SYNC HISTORY" { return Err(invalid_params("explicit history rewrite confirmation required")); }
                let _operation = st.sync_operation.lock().await;
                let git = st.git_backend.lock().await.clone().ok_or_else(|| invalid_params("Git backend required"))?;
                let before = git.capacity().await.map_err(|e| internal(e.to_string()))?;
                if !before.history_rewrite_allowed || before.head_oid.as_deref() != Some(p.expected_oid.as_str()) {
                    return Err(invalid_params("branch or expected OID changed; refresh capacity and reconcile first"));
                }
                // No implicit fetch: the confirmed local tip must be the fetched,
                // reconciled remote tip, and push checks its live advertised OID.
                git.squash_orphan("sync: explicit history maintenance").await.map_err(|e| internal(e.to_string()))?;
                Ok(json!({"rewritten": true, "capacity": git.capacity().await.map_err(|e| internal(e.to_string()))?}))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sync.compact", move |params| {
            let st = st.clone();
            async move {
                let _operation = st.sync_operation.lock().await;
                #[derive(Deserialize, Default)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    #[serde(default)]
                    groups: Option<Vec<String>>,
                }
                let p: P = if params.is_null() {
                    P::default()
                } else {
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?
                };
                projection::recover_projection(&st).await?;
                let eng = require_engine(&st).await?;
                let staged = eng.preview_copy().await;
                let oplog = st
                    .oplog
                    .lock()
                    .await
                    .clone()
                    .ok_or_else(|| invalid_params("oplog not configured"))?;
                let groups = match p.groups {
                    Some(names) => names
                        .into_iter()
                        .filter_map(|n| {
                            Group::from_slug(&n.to_ascii_lowercase()).or(match n.as_str() {
                                "Connections" => Some(Group::Connections),
                                "Appearance" => Some(Group::Appearance),
                                "Shortcuts" => Some(Group::Shortcuts),
                                "PluginCfg" => Some(Group::PluginCfg),
                                "Credentials" => Some(Group::Credentials),
                                "Notes" => Some(Group::Notes),
                                _ => None,
                            })
                        })
                        .collect::<Vec<_>>(),
                    None => Group::ALL.to_vec(),
                };
                if groups.is_empty() {
                    return Err(invalid_params("no valid groups"));
                }
                if oplog.protocol() != 3 {
                    return Err(invalid_params(
                        "explicit migration to v3 required for compaction",
                    ));
                }
                require_completed_sync_setup(&st, &groups, None).await?;
                if groups.contains(&Group::Notes)
                    && st
                        .notes
                        .lock()
                        .await
                        .as_ref()
                        .is_some_and(|n| n.sync_paused())
                {
                    return Err(invalid_params("save Notes drafts before compaction"));
                }
                if let Some(git) = st.git_backend.lock().await.clone() {
                    if git
                        .fetch_outcome()
                        .await
                        .map_err(|e| internal(e.to_string()))?
                        .discarded_local_batches
                    {
                        oplog
                            .reconcile_transport_change()
                            .await
                            .map_err(|e| internal(e.to_string()))?;
                    }
                }
                if let (Ok(profiles), Ok(settings)) =
                    (require_profiles(&st).await, require_settings(&st).await)
                {
                    crate::sync::bridge::export_locals(
                        &profiles,
                        &settings,
                        st.vault.lock().await.as_ref(),
                        &st.wasm_host.list().await,
                        &staged,
                        &groups,
                    )
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                }
                if groups.contains(&Group::Notes) {
                    if let Some(notes) = st.notes.lock().await.as_ref() {
                        crate::sync::bridge::preview_notes(notes, &staged)
                            .await
                            .map_err(|e| internal(e.to_string()))?;
                    }
                }
                let (candidate, cursors) = oplog
                    .maintenance_candidate(&staged, &groups)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                projection::commit_projection(&st, &eng, &candidate, &groups, false).await?;
                oplog
                    .accept_maintenance_coverage(cursors)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                let eng = require_engine(&st).await?;
                let ok = oplog
                    .compact_now(&eng, &groups)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                if let Some(git) = st.git_backend.lock().await.clone() {
                    git.commit_cycle("sync: v3 compaction")
                        .await
                        .map_err(|e| internal(e.to_string()))?;
                    git.push_remote()
                        .await
                        .map_err(|e| internal(e.to_string()))?;
                    oplog
                        .confirm_uploaded()
                        .await
                        .map_err(|e| internal(e.to_string()))?;
                }
                Ok(json!({"compacted":ok}))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sync.purgeV1", move |_params| {
            let st = st.clone();
            async move {
                let _operation = st.sync_operation.lock().await;
                let engine=require_engine(&st).await?;
                let driver=st.oplog.lock().await.clone().ok_or_else(||invalid_params("sync not configured"))?;
                if driver.protocol()!=3 {return Err(invalid_params("verified v3 migration required"));}
                let blobs=driver.blobs();
                // Compare each legacy payload/tombstone with a fresh engine
                // reconstructed from durable v3 snapshots and operation logs.
                let durable=driver.empty_engine();
                let recovered=driver.readonly_v3_engine(&durable).await.map_err(|e|internal(e.to_string()))?;
                let mut verified=Vec::new();
                for group in Group::ALL {
                    let records:std::collections::HashMap<_,_>=recovered.snapshot_group(group).await.into_iter().collect();
                    for blob in blobs.list_blobs(&format!("{}/",group.as_str())).await.map_err(|e|internal(e.to_string()))? {
                        let Some(id)=blob.path.strip_prefix(&format!("{}/",group.as_str())).and_then(|s|s.strip_suffix(".bin")).and_then(|s|Uuid::parse_str(s).ok()).map(crate::sync::RecordId) else {continue;};
                        let bytes=blobs.get_blob(&blob.path).await.map_err(|e|internal(e.to_string()))?;
                        let old=engine.decode_legacy(&bytes).map_err(|e|internal(e.to_string()))?;
                        let new=records.get(&id).ok_or_else(||invalid_params("legacy record absent from durable v3 recovery"))?;
                        if old.payload!=new.payload || old.tombstone!=new.tombstone {return Err(invalid_params("legacy payload/tombstone differs from durable v3; no files deleted"));}
                        verified.push((blob.path,bytes));
                    }
                }
                for (path,bytes) in &verified {if blobs.get_blob(path).await.map_err(|e|internal(e.to_string()))?!=*bytes {return Err(invalid_params("legacy changed during verification; retry"));}}
                for (path,_) in &verified {blobs.delete_blob(path).await.map_err(|e|internal(e.to_string()))?;}
                if let Some(git)=st.git_backend.lock().await.clone() {git.commit_cycle("sync: verified legacy purge (history retained)").await.map_err(|e|internal(e.to_string()))?;git.push_remote().await.map_err(|e|internal(e.to_string()))?;}
                Ok(json!({"deleted":verified.len(),"gitHistoryUnchanged":true}))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sync.history", move |_params| {
            let st = st.clone();
            async move {
                let store = require_settings(&st).await?;
                let list =
                    crate::sync::history::load(&store).map_err(|e| internal(e.to_string()))?;
                serde_json::to_value(list).map_err(|e| internal(e.to_string()))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sync.historyClear", move |_params| {
            let st = st.clone();
            async move {
                let store = require_settings(&st).await?;
                crate::sync::history::clear(&store).map_err(|e| internal(e.to_string()))?;
                Ok(Value::Null)
            }
        });
    }
}

async fn record_sync_history_ok(
    st: &AppState,
    trigger: &str,
    groups: &[Group],
    results: &[(Group, SyncStats)],
    at_ms: i64,
) {
    if let Ok(store) = require_settings(st).await {
        if let Err(e) =
            crate::sync::history::append_success_at(&store, trigger, groups, results, at_ms)
        {
            tracing::warn!(error = %e, "sync history append failed");
        }
    }
}

async fn latest_successful_sync_ms(st: &AppState) -> Option<i64> {
    if let Some(ms) = *st.sync_last_ms.lock().await {
        return Some(ms);
    }
    let store = require_settings(st).await.ok()?;
    crate::sync::history::latest_success_ms(&store)
        .ok()
        .flatten()
}

async fn record_sync_history_err(st: &AppState, trigger: &str, groups: &[Group], err: &RpcError) {
    if let Ok(store) = require_settings(st).await {
        if let Err(e) =
            crate::sync::history::append_failure(&store, trigger, groups, err.message.clone())
        {
            tracing::warn!(error = %e, "sync history append failed");
        }
    }
}

/// Export locals → fetch → reconcile → import → push (Git).
async fn invalidate_sync_approval(st: &AppState) {
    if let Some(store) = st.sync_approval_store.lock().await.as_ref() {
        let _ = store.local_set("sync.approval", &Value::Null);
    }
    *st.sync_approval.lock().await = SyncApproval {
        generation: Uuid::new_v4(),
        ..Default::default()
    };
}
async fn configure_sync_approval(
    st: &AppState,
    target: &str,
    state_dir: Option<&str>,
    driver: &OplogDriver,
) -> Result<(), RpcError> {
    let target = Uuid::new_v5(&Uuid::NAMESPACE_URL, target.as_bytes()).to_string();
    let target = target.as_str();
    let store = if let Some(settings) = st.settings.lock().await.clone() {
        Some(settings)
    } else if let Some(dir) = state_dir {
        Some(
            SettingsStore::open(std::path::Path::new(dir).join("approval-local"))
                .map_err(|e| internal(e.to_string()))?,
        )
    } else {
        None
    };
    let saved = store
        .as_ref()
        .map(|s| {
            s.local_get("sync.approval")
                .map_err(|e| internal(e.to_string()))
        })
        .transpose()?
        .flatten();
    let binding = Some(driver.approval_binding());
    let blocked = driver
        .maintenance_journal()
        .map_err(|e| internal(e.to_string()))?
        .is_some_and(|v| !v.is_null());
    let approved = saved
        .as_ref()
        .filter(|v| {
            !blocked && v["schema"] == 2 && v["target"] == target && v["binding"] == json!(binding)
        })
        .and_then(|v| serde_json::from_value::<Vec<Group>>(v["groups"].clone()).ok());
    // A successful switch destroys old consent; returning to an old target must
    // not resurrect it. Caller cannot provide an approval in settings.sync.
    if approved.is_none() {
        if let Some(s) = &store {
            s.local_set("sync.approval", &Value::Null)
                .map_err(|e| internal(e.to_string()))?;
        }
    }
    *st.sync_approval_store.lock().await = store;
    *st.sync_approval.lock().await = SyncApproval {
        generation: Uuid::new_v4(),
        target: Some(target.to_string()),
        binding,
        approved,
        pending: None,
    };
    Ok(())
}
async fn require_completed_sync_setup(
    st: &AppState,
    groups: &[Group],
    token: Option<Uuid>,
) -> Result<(), RpcError> {
    let binding = st.oplog.lock().await.as_ref().map(|d| d.approval_binding());
    let mut approval = st.sync_approval.lock().await;
    if approval.binding != binding {
        approval.approved = None;
        approval.pending = None;
        approval.binding = binding.clone();
        approval.generation = Uuid::new_v4();
    }
    if let Some(token) = token {
        if let Some((expected, selected, result)) = &approval.pending {
            if let Some(digest) = result["engineDigest"].as_str() {
                if require_engine(st)
                    .await?
                    .preview_digest()
                    .await
                    .map_err(|e| internal(e.to_string()))?
                    != digest
                {
                    return Err(invalid_params(
                        "engine changed since preview; preview again",
                    ));
                }
            }
            if groups.contains(&Group::Notes) {
                let store = st
                    .notes
                    .lock()
                    .await
                    .clone()
                    .ok_or_else(|| internal("notes store unavailable"))?;
                if store.sync_paused()
                    || result["notesRevision"].as_u64()
                        != Some(
                            store
                                .sync_snapshot()
                                .map_err(|e| internal(e.to_string()))?
                                .2,
                        )
                {
                    return Err(invalid_params(
                        "Notes changed since preview; save drafts and preview again",
                    ));
                }
            }
            if *expected == token && selected == groups {
                if let (Some(store), Some(target)) = (
                    st.sync_approval_store.lock().await.as_ref(),
                    approval.target.as_ref(),
                ) {
                    store
                        .local_set(
                            "sync.approval",
                            &json!({"schema":2,"target":target,"binding":binding,"groups":groups}),
                        )
                        .map_err(|e| internal(e.to_string()))?;
                }
                approval.approved = Some(groups.to_vec());
                approval.pending = None;
                return Ok(());
            }
        }
        return Err(invalid_params("invalid or stale merge preview token"));
    }
    if approval.approved.as_deref() == Some(groups) {
        return Ok(());
    }
    Err(invalid_params(
        "complete setup and confirm the merge preview for this configuration and groups",
    ))
}

async fn run_sync_now_cycle(
    st: &AppState,
    groups: &[crate::sync::Group],
    preview_token: Option<Uuid>,
) -> Result<
    (
        Vec<(crate::sync::Group, crate::sync::SyncStats)>,
        crate::sync::bridge::BridgeNotes,
    ),
    RpcError,
> {
    let _operation = st.sync_operation.lock().await;
    projection::recover_projection(st).await?;
    if st
        .oplog
        .lock()
        .await
        .as_ref()
        .is_some_and(|d| d.protocol() == 2)
    {
        return Err(invalid_params(
            "v2 is import-only; preview and confirm v3 migration",
        ));
    }
    require_completed_sync_setup(st, groups, preview_token).await?;
    let notes_paused = st
        .notes
        .lock()
        .await
        .as_ref()
        .is_some_and(|store| store.sync_paused());
    // Preserve the configured/approved group set; only defer Notes this tick.
    // Saving the draft clears ownership and Notes rejoins the next tick.
    let effective: Vec<_> = groups
        .iter()
        .copied()
        .filter(|g| *g != Group::Notes || !notes_paused)
        .collect();
    let groups = effective.as_slice();
    let current_engine = require_engine(st).await?;
    let eng = current_engine.as_ref();
    let profiles = require_profiles(st).await.ok();
    let settings = require_settings(st).await.ok();
    let vault = st.vault.lock().await.clone();
    let plugins = st.wasm_host.list().await;
    let notes_store = st.notes.lock().await.clone();
    let notes_revision = if groups.contains(&Group::Notes) {
        let store = notes_store
            .as_ref()
            .ok_or_else(|| internal("notes store unavailable"))?;
        Some(
            crate::sync::bridge::export_notes(store, eng)
                .await
                .map_err(|e| internal(e.to_string()))?,
        )
    } else {
        None
    };
    // Locked credentials must not enter reconciliation (including previously cached
    // plaintext records). Other approved groups can continue syncing normally.
    let credentials_requested = groups.contains(&Group::Credentials);
    let credentials_initialized = match vault.as_ref() {
        Some(vault) => vault
            .is_initialized()
            .map_err(|e| internal(e.to_string()))?,
        None => false,
    };
    let credentials_unlocked = match vault.as_ref() {
        Some(vault) => vault.is_unlocked().await,
        None => false,
    };
    let effective: Vec<_> = groups
        .iter()
        .copied()
        .filter(|g| *g != Group::Credentials || (credentials_initialized && credentials_unlocked))
        .collect();
    let groups = effective.as_slice();
    let mut bridge_notes = crate::sync::bridge::BridgeNotes::default();
    if let (Some(profiles), Some(settings)) = (&profiles, &settings) {
        bridge_notes = crate::sync::bridge::export_locals(
            profiles,
            settings,
            vault.as_ref(),
            &plugins,
            eng,
            groups,
        )
        .await
        .map_err(|e| internal(e.to_string()))?;
    }
    bridge_notes.credentials_skipped_locked |=
        credentials_requested && credentials_initialized && !credentials_unlocked;
    bridge_notes.credentials_skipped_uninitialized |=
        credentials_requested && !credentials_initialized;
    let git = st.git_backend.lock().await.clone();
    if let Some(g) = &git {
        let changed = g
            .fetch_outcome()
            .await
            .map_err(|e| internal(e.to_string()))?;
        if changed.discarded_local_batches {
            if let Some(oplog) = st.oplog.lock().await.clone() {
                oplog
                    .reconcile_transport_change()
                    .await
                    .map_err(|e| internal(e.to_string()))?;
            }
        }
    }
    let results = if let Some(oplog) = st.oplog.lock().await.clone() {
        let (results, meta) = oplog
            .cycle(eng, groups)
            .await
            .map_err(|e| internal(e.to_string()))?;
        *st.oplog_last.lock().await = Some(meta);
        results
    } else {
        *st.oplog_last.lock().await = None;
        eng.sync_groups(groups.to_vec())
            .await
            .map_err(|e| internal(e.to_string()))?
    };
    if let (Some(profiles), Some(settings)) = (&profiles, &settings) {
        crate::sync::bridge::import_locals(profiles, settings, vault.as_ref(), eng, groups)
            .await
            .map_err(|e| internal(e.to_string()))?;
    }
    if let (Some(store), Some(revision)) = (notes_store.as_ref(), notes_revision) {
        crate::sync::bridge::import_notes(store, eng, revision)
            .await
            .map_err(|e| internal(e.to_string()))?;
    }
    bridge_notes.notes_paused_dirty = notes_paused;
    if let Some(g) = &git {
        // One commit per cycle covering every record put/delete in this tick.
        let device = eng.device_id();
        let msg = format!("sync({device}): cycle");
        g.commit_cycle(&msg)
            .await
            .map_err(|e| internal(e.to_string()))?;
        g.push_remote().await.map_err(|e| internal(e.to_string()))?;
        if let Some(oplog) = st.oplog.lock().await.clone() {
            oplog
                .confirm_uploaded()
                .await
                .map_err(|e| internal(e.to_string()))?;
        }
    }
    Ok((results, bridge_notes))
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
                // Finish an in-flight credential cycle before acknowledging manual lock.
                let _operation = st.sync_operation.lock().await;
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
                let profile_store = st.profiles.lock().await.clone();
                if let Some(profiles) = profile_store {
                    let mut list = profiles.list().await.map_err(|e| internal(e.to_string()))?;
                    list.extend(load_effective_catalog(&st).await?);
                    let mut dependencies = crate::profile::vault_dependents(&list, &p.id);
                    dependencies.extend(crate::profile_inheritance::group_dependents(
                        &load_group_defaults(&st).await?,
                        "auth",
                        &p.id,
                    ));
                    if let Some(network) = st.network_resources.lock().await.clone() {
                        dependencies.extend(
                            network
                                .vault_dependents(&p.id)
                                .map_err(|e| internal(e.to_string()))?,
                        );
                    }
                    if !dependencies.is_empty() {
                        return Err(invalid_params(format!(
                            "vault entry {} is used by {}",
                            p.id,
                            dependencies.join(", ")
                        )));
                    }
                }
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
    // -----------------------------------------------------------
    // nativeTerminal.*
    // -----------------------------------------------------------
    {
        let st = state.clone();
        dispatcher.register("nativeTerminal.embedCapabilities", move |_p| {
            let st = st.clone();
            async move {
                let caps = st.embedded_terminals.embed_capabilities();
                serde_json::to_value(caps).map_err(|e| internal(e.to_string()))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("nativeTerminal.spawn", move |params| {
            let st = st.clone();
            async move {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    #[serde(default)]
                    program: Option<String>,
                    #[serde(default)]
                    title: String,
                    #[serde(default)]
                    argv: Vec<String>,
                    #[serde(default)]
                    mode: Option<String>,
                }
                let p: P =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                use crate::native_terminal::NativeSpawnMode;
                let mode = match p.mode.as_deref() {
                    Some("embed") => NativeSpawnMode::Embed,
                    _ => NativeSpawnMode::Detached,
                };
                let result = st
                    .native_terminals
                    .spawn(p.program.as_deref(), &p.title, &p.argv, mode)
                    .map_err(|e| internal(e.to_string()))?;
                serde_json::to_value(result).map_err(|e| internal(e.to_string()))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("nativeTerminal.embedStart", move |params| {
            let st = st.clone();
            async move {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    #[serde(default)]
                    program: Option<String>,
                    #[serde(default)]
                    title: String,
                    #[serde(default)]
                    argv: Vec<String>,
                    rect: EmbedRectDip,
                }
                let p: P =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let result = st
                    .embedded_terminals
                    .embed_start(p.program.as_deref(), &p.title, &p.argv, &p.rect)
                    .map_err(|e| internal(e.to_string()))?;
                serde_json::to_value(result).map_err(|e| internal(e.to_string()))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("nativeTerminal.embedSyncGeometry", move |params| {
            let st = st.clone();
            async move {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    instance_id: String,
                    rect: EmbedRectDip,
                }
                let p: P =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                st.embedded_terminals
                    .embed_sync_geometry(&p.instance_id, &p.rect)
                    .map_err(|e| internal(e.to_string()))?;
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("nativeTerminal.embedEnd", move |params| {
            let st = st.clone();
            async move {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    instance_id: String,
                }
                let p: P =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let removed = st
                    .embedded_terminals
                    .embed_end(&p.instance_id)
                    .map_err(|e| internal(e.to_string()))?;
                Ok(json!({ "removed": removed }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("nativeTerminal.embedCleanup", move |_p| {
            let st = st.clone();
            async move {
                st.embedded_terminals.embed_cleanup();
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("nativeTerminal.embedList", move |_p| {
            let st = st.clone();
            async move {
                let list = st.embedded_terminals.embed_list();
                serde_json::to_value(list).map_err(|e| internal(e.to_string()))
            }
        });
    }
    // -----------------------------------------------------------
    // terminal.* — native terminal engine (alacritty_terminal in Rust)
    // -----------------------------------------------------------
    register_terminal_engine(dispatcher, state.clone());
}

/// Session output receiver + a writer for engine replies (DA / CPR …).
async fn session_output_and_writer(
    st: &Arc<AppState>,
    id: SessionId,
) -> Result<
    (
        crate::terminal::engine::SharedOutput,
        mpsc::UnboundedSender<Vec<u8>>,
    ),
    RpcError,
> {
    let rx = {
        let chans = st.channels.lock().await;
        let chan = chans.get(&id).ok_or_else(|| session_not_found(id))?;
        match chan {
            SessionChannel::Local(pty) => pty.rx.clone(),
            SessionChannel::Ssh(s) => s.rx.clone(),
            SessionChannel::Serial(s) => s.rx.clone(),
        }
    };
    let (tx, mut reply_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let st2 = st.clone();
    tokio::spawn(async move {
        while let Some(bytes) = reply_rx.recv().await {
            let chans = st2.channels.lock().await;
            let Some(chan) = chans.get(&id) else { break };
            let ok = match chan {
                SessionChannel::Local(pty) => pty.channel.write(&bytes).is_ok(),
                SessionChannel::Ssh(s) => s.shell.write(&bytes).await.is_ok(),
                SessionChannel::Serial(s) => s.channel.write(&bytes).is_ok(),
            };
            if !ok {
                break;
            }
        }
    });
    Ok((rx, tx))
}

fn require_term_engine(
    st: &AppState,
    id: Uuid,
) -> Result<Arc<crate::terminal::engine::TerminalEngine>, RpcError> {
    st.terminal_engines
        .get(id)
        .ok_or_else(|| RpcError::new(ErrorCode::SessionNotFound, format!("no engine for {id}")))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EngineIdParam {
    id: Uuid,
}

fn register_terminal_engine(dispatcher: &Dispatcher, state: Arc<AppState>) {
    use crate::terminal::engine::{
        CellSide, EngineConfig, ScrollCmd, SearchDirection, SelectionKind, TerminalEngine,
    };
    {
        let st = state.clone();
        dispatcher.register("terminal.attach", move |params| {
            let st = st.clone();
            async move {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    id: Uuid,
                    #[serde(default)]
                    cols: Option<u16>,
                    #[serde(default)]
                    rows: Option<u16>,
                    #[serde(default)]
                    scrollback: Option<usize>,
                    #[serde(default)]
                    transfer_detect: bool,
                }
                let p: P =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let sid = SessionId(p.id);
                if let Some(existing) = st.terminal_engines.get(p.id) {
                    if let (Some(c), Some(r)) = (p.cols, p.rows) {
                        existing.resize(c, r, 0, 0);
                    }
                    existing.set_transfer_detect(p.transfer_detect);
                    return Ok(
                        json!({ "attached": true, "alive": existing.is_alive(), "reused": true }),
                    );
                }
                let (rx, tx) = session_output_and_writer(&st, sid).await?;
                let engine = TerminalEngine::new(
                    EngineConfig {
                        cols: p.cols.unwrap_or(80),
                        rows: p.rows.unwrap_or(24),
                        scrollback: p.scrollback.unwrap_or(10_000).clamp(0, 200_000),
                    },
                    tx,
                );
                engine.set_transfer_detect(p.transfer_detect);
                engine.start(rx);
                st.terminal_engines.insert(p.id, engine.clone());
                Ok(json!({ "attached": true, "alive": engine.is_alive(), "reused": false }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("terminal.detach", move |params| {
            let st = st.clone();
            async move {
                let p: EngineIdParam =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let removed = st.terminal_engines.remove(p.id).is_some();
                Ok(json!({ "removed": removed }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("terminal.resize", move |params| {
            let st = st.clone();
            async move {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    id: Uuid,
                    cols: u16,
                    rows: u16,
                    #[serde(default)]
                    cell_width: u16,
                    #[serde(default)]
                    cell_height: u16,
                }
                let p: P =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let engine = require_term_engine(&st, p.id)?;
                engine.resize(p.cols, p.rows, p.cell_width, p.cell_height);
                let id = SessionId(p.id);
                let chans = st.channels.lock().await;
                if let Some(chan) = chans.get(&id) {
                    match chan {
                        SessionChannel::Local(pty) => {
                            let _ = pty.channel.resize(PtySize {
                                rows: p.rows,
                                cols: p.cols,
                            });
                        }
                        SessionChannel::Ssh(s) => {
                            let _ = s.shell.resize(p.cols as u32, p.rows as u32).await;
                        }
                        SessionChannel::Serial(_) => {}
                    }
                }
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("terminal.scroll", move |params| {
            let st = st.clone();
            async move {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    id: Uuid,
                    cmd: ScrollCmd,
                }
                let p: P =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                require_term_engine(&st, p.id)?.scroll(p.cmd);
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("terminal.snapshot", move |params| {
            let st = st.clone();
            async move {
                let p: EngineIdParam =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let engine = require_term_engine(&st, p.id)?;
                engine.request_full();
                Ok(json!({ "alive": engine.is_alive() }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("terminal.snapshotEnvelope", move |params| {
            let st = st.clone();
            async move {
                let p: EngineIdParam =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let snapshot = require_term_engine(&st, p.id)?.native_snapshot_envelope();
                // Out-of-band envelope v1; embedded binary frame remains v1.
                Ok(json!({ "version": 1, "frameVersion": 1, "outputSeq": snapshot.output_seq.to_string(), "bytes": snapshot.bytes }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("terminal.exportText", move |params| {
            let st = st.clone();
            async move {
                let p: EngineIdParam =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let engine = require_term_engine(&st, p.id)?;
                Ok(json!({ "text": engine.buffer_text(), "alive": engine.is_alive() }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("terminal.setVisible", move |params| {
            let st = st.clone();
            async move {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    id: Uuid,
                    visible: bool,
                }
                let p: P =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                require_term_engine(&st, p.id)?.set_visible(p.visible);
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("terminal.selectionStart", move |params| {
            let st = st.clone();
            async move {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    id: Uuid,
                    col: usize,
                    line: i32,
                    #[serde(default = "default_sel_kind")]
                    kind: SelectionKind,
                    #[serde(default = "default_side")]
                    side: CellSide,
                }
                fn default_sel_kind() -> SelectionKind {
                    SelectionKind::Simple
                }
                fn default_side() -> CellSide {
                    CellSide::Left
                }
                let p: P =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                require_term_engine(&st, p.id)?.selection_start(p.col, p.line, p.kind, p.side);
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("terminal.selectionUpdate", move |params| {
            let st = st.clone();
            async move {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    id: Uuid,
                    col: usize,
                    line: i32,
                    #[serde(default = "default_side")]
                    side: CellSide,
                }
                fn default_side() -> CellSide {
                    CellSide::Right
                }
                let p: P =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                require_term_engine(&st, p.id)?.selection_update(p.col, p.line, p.side);
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("terminal.selectionClear", move |params| {
            let st = st.clone();
            async move {
                let p: EngineIdParam =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                require_term_engine(&st, p.id)?.selection_clear();
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("terminal.selectAll", move |params| {
            let st = st.clone();
            async move {
                let p: EngineIdParam =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                require_term_engine(&st, p.id)?.select_all();
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("terminal.selectionText", move |params| {
            let st = st.clone();
            async move {
                let p: EngineIdParam =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let text = require_term_engine(&st, p.id)?.selection_text();
                Ok(json!({ "text": text }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("terminal.search", move |params| {
            let st = st.clone();
            async move {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    id: Uuid,
                    query: String,
                    #[serde(default)]
                    regex: bool,
                    #[serde(default)]
                    case_sensitive: bool,
                    #[serde(default = "default_dir")]
                    direction: SearchDirection,
                }
                fn default_dir() -> SearchDirection {
                    SearchDirection::Next
                }
                let p: P =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let hit = require_term_engine(&st, p.id)?
                    .search(&p.query, p.regex, p.case_sensitive, p.direction)
                    .map_err(invalid_params)?;
                Ok(json!({ "hit": hit }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("terminal.hyperlinkAt", move |params| {
            let st = st.clone();
            async move {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    id: Uuid,
                    col: usize,
                    line: i32,
                }
                let p: P =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let url = require_term_engine(&st, p.id)?.hyperlink_at(p.col, p.line);
                Ok(json!({ "url": url }))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("terminal.setRawBypass", move |params| {
            let st = st.clone();
            async move {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    id: Uuid,
                    on: bool,
                }
                let p: P =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                require_term_engine(&st, p.id)?.set_raw_bypass(p.on);
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("terminal.setTransferDetect", move |params| {
            let st = st.clone();
            async move {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    id: Uuid,
                    on: bool,
                }
                let p: P =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                require_term_engine(&st, p.id)?.set_transfer_detect(p.on);
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("terminal.feed", move |params| {
            let st = st.clone();
            async move {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    id: Uuid,
                    data: String, // base64
                }
                let p: P =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let bytes = BASE64
                    .decode(p.data.as_bytes())
                    .map_err(|e| invalid_params(format!("bad base64: {e}")))?;
                require_term_engine(&st, p.id)?.feed(&bytes);
                Ok(Value::Null)
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("terminal.text", move |params| {
            let st = st.clone();
            async move {
                let p: EngineIdParam =
                    serde_json::from_value(params).map_err(|e| invalid_params(e.to_string()))?;
                let engine = require_term_engine(&st, p.id)?;
                let (cols, rows) = engine.dims();
                Ok(json!({
                    "text": engine.viewport_text(),
                    "cols": cols,
                    "rows": rows,
                    "alive": engine.is_alive(),
                    "mode": engine.mode_bits(),
                    "title": engine.title(),
                }))
            }
        });
    }
}

// MCP adapter is appended below to avoid overlapping concurrent command edits.
#[cfg(test)]
#[allow(clippy::items_after_test_module)]
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
    async fn low_level_put_delete_invalidate_pending_but_keep_completed_approval() {
        let root = std::env::temp_dir().join(Uuid::new_v4().to_string());
        let st = AppState::new();
        let d = Dispatcher::new();
        register_all(&d, st.clone());
        assert!(d
            .dispatch(req(
                "sync.configureGit",
                json!({"repo_path":root,"master_password":"pw","test_cheap_kdf":true})
            ))
            .await
            .error
            .is_none());
        let groups = vec![Group::Appearance];
        let id = Uuid::new_v4();
        for method in ["sync.put", "sync.delete"] {
            let preview = d
                .dispatch(req("sync.preview", json!({"groups":groups})))
                .await
                .result
                .unwrap();
            let token = serde_json::from_value(preview["previewToken"].clone()).unwrap();
            let r = d
                .dispatch(req(
                    method,
                    json!({"group":"Appearance","id":id,"data":BASE64.encode(b"changed")}),
                ))
                .await;
            assert!(r.error.is_none());
            assert!(require_completed_sync_setup(&st, &groups, Some(token))
                .await
                .is_err());
        }
        let preview = d
            .dispatch(req("sync.preview", json!({"groups":groups})))
            .await
            .result
            .unwrap();
        let token = serde_json::from_value(preview["previewToken"].clone()).unwrap();
        require_completed_sync_setup(&st, &groups, Some(token))
            .await
            .unwrap();
        d.dispatch(req(
            "sync.put",
            json!({"group":"Appearance","id":id,"data":BASE64.encode(b"new")}),
        ))
        .await;
        assert!(require_completed_sync_setup(&st, &groups, None)
            .await
            .is_ok());
    }
    #[tokio::test]
    async fn approved_target_survives_reconfigure_restart_but_not_new_target() {
        let root = std::env::temp_dir().join(Uuid::new_v4().to_string());
        let settings = SettingsStore::open(root.join("settings")).unwrap();
        let groups = vec![Group::Appearance];
        let state = AppState::new();
        *state.settings.lock().await = Some(settings.clone());
        let d = Dispatcher::new();
        register_all(&d, state.clone());
        let args =
            json!({"repo_path":root.join("git"),"master_password":"pw","test_cheap_kdf":true});
        assert!(d
            .dispatch(req("sync.configureGit", args.clone()))
            .await
            .error
            .is_none());
        let preview = d
            .dispatch(req("sync.preview", json!({"groups":groups})))
            .await
            .result
            .unwrap();
        let token = serde_json::from_value(preview["previewToken"].clone()).unwrap();
        require_completed_sync_setup(&state, &groups, Some(token))
            .await
            .unwrap();
        let restarted = AppState::new();
        *restarted.settings.lock().await = Some(settings);
        let d = Dispatcher::new();
        register_all(&d, restarted.clone());
        assert!(d
            .dispatch(req("sync.configureGit", args))
            .await
            .error
            .is_none());
        assert!(
            require_completed_sync_setup(&restarted, &groups, None)
                .await
                .is_ok(),
            "same target approval must survive restart"
        );
        assert!(
            require_completed_sync_setup(&restarted, &[Group::Notes], None)
                .await
                .is_err()
        );
        assert!(d
            .dispatch(req(
                "sync.configureGit",
                json!({"repo_path":root.join("other"),"master_password":"pw","test_cheap_kdf":true})
            ))
            .await
            .error
            .is_none());
        assert!(require_completed_sync_setup(&restarted, &groups, None)
            .await
            .is_err());
        assert!(d
            .dispatch(req(
                "sync.configureGit",
                json!({"repo_path":root.join("git"),"master_password":"pw","test_cheap_kdf":true})
            ))
            .await
            .error
            .is_none());
        assert!(
            require_completed_sync_setup(&restarted, &groups, None)
                .await
                .is_err(),
            "switching back cannot resurrect old consent"
        );
    }

    #[tokio::test]
    async fn sync_approval_is_server_owned_group_bound_and_invalidated() {
        let st = AppState::new();
        let groups = vec![Group::Connections];
        assert!(require_completed_sync_setup(&st, &groups, None)
            .await
            .is_err());
        let token = Uuid::new_v4();
        st.sync_approval.lock().await.pending =
            Some((token, groups.clone(), json!({"publish":100})));
        assert!(
            require_completed_sync_setup(&st, &groups, Some(Uuid::new_v4()))
                .await
                .is_err()
        );
        assert!(
            require_completed_sync_setup(&st, &[Group::Appearance], Some(token))
                .await
                .is_err()
        );
        require_completed_sync_setup(&st, &groups, Some(token))
            .await
            .unwrap();
        require_completed_sync_setup(&st, &groups, None)
            .await
            .unwrap();
        invalidate_sync_approval(&st).await;
        assert!(require_completed_sync_setup(&st, &groups, None)
            .await
            .is_err());
        assert!(require_completed_sync_setup(&st, &groups, Some(token))
            .await
            .is_err());
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
    async fn sync_reconfigure_preserves_device_identity() {
        let dir = std::env::temp_dir().join(format!("aerotab-identity-{}", Uuid::new_v4()));
        let d = Dispatcher::new();
        register_all(&d, AppState::new());
        let args = json!({"repo_path": dir.join("repo"), "state_dir": dir.join("state"), "master_password": "pw", "test_cheap_kdf": true});
        let response = d.dispatch(req("sync.configureGit", args.clone())).await;
        assert!(response.error.is_none(), "{:?}", response.error);
        let first = response.result.unwrap();
        drop(d);
        let d = Dispatcher::new();
        register_all(&d, AppState::new());
        let second = d
            .dispatch(req("sync.configureGit", args))
            .await
            .result
            .unwrap();
        assert_eq!(first["device_id"], second["device_id"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn compact_refuses_unproven_full_group_coverage() {
        let root = std::env::temp_dir().join(format!("aerotab-coverage-{}", Uuid::new_v4()));
        let d = Dispatcher::new();
        register_all(&d, AppState::new());
        assert!(d
            .dispatch(req(
                "sync.configureGit",
                json!({"repo_path": root, "master_password": "pw", "test_cheap_kdf": true})
            ))
            .await
            .error
            .is_none());
        let response = d.dispatch(req("sync.compact", json!({"groups": ["Connections", "Appearance", "Shortcuts", "PluginCfg", "Credentials"]}))).await;
        assert!(
            response.error.is_some(),
            "Group::ALL is not proof of reconciled coverage"
        );
        assert!(!root.join("v2/snapshots/latest").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn session_only_locked_credentials_skip_cycle_but_other_groups_sync() {
        let root = std::env::temp_dir().join(format!("aerotab-session-sync-{}", Uuid::new_v4()));
        let state = AppState::new();
        let d = Dispatcher::new();
        register_all(&d, state.clone());
        let vault =
            crate::vault::VaultStore::open_with_params(root.join("vault"), KdfParams::test_cheap())
                .unwrap();
        vault.initialize(b"pw").await.unwrap();
        *state.vault.lock().await = Some(vault.clone());
        let configured = d.dispatch(req("sync.configureGit", json!({"repo_path":root.join("git"),"master_password":"sync-password","test_cheap_kdf":true}))).await;
        assert!(configured.error.is_none(), "{:?}", configured.error);
        let groups = vec![Group::Appearance, Group::Credentials];
        let preview = d
            .dispatch(req("sync.preview", json!({"groups":groups})))
            .await;
        assert!(preview.error.is_none(), "{:?}", preview.error);
        let token =
            serde_json::from_value(preview.result.unwrap()["previewToken"].clone()).unwrap();
        require_completed_sync_setup(&state, &groups, Some(token))
            .await
            .unwrap();
        assert!(d
            .dispatch(req("vault.lock", json!({})))
            .await
            .error
            .is_none());
        let refused = d
            .dispatch(req(
                "sync.ensureVaultUnlock",
                json!({"account":"sync.master"}),
            ))
            .await;
        assert!(
            refused.error.is_some(),
            "no-password compatibility RPC must reject locked vault"
        );
        let engine = require_engine(&state).await.unwrap();
        let cached_id = RecordId(Uuid::new_v4());
        engine
            .put_local(
                Group::Credentials,
                cached_id,
                b"cached plaintext must not upload".to_vec(),
            )
            .await
            .unwrap();
        let (results, bridge) = run_sync_now_cycle(&state, &groups, None).await.unwrap();
        assert!(bridge.credentials_skipped_locked);
        assert!(engine
            .snapshot_group(Group::Credentials)
            .await
            .iter()
            .any(|(id, record)| *id == cached_id && !record.tombstone));
        engine
            .delete_local(Group::Credentials, cached_id)
            .await
            .unwrap();
        assert!(results.iter().any(|(g, _)| *g == Group::Appearance));
        assert!(results.iter().all(|(g, _)| *g != Group::Credentials));
        assert!(!vault.is_unlocked().await);
        assert!(d
            .dispatch(req("sync.ensureVaultUnlock", json!({"password":"pw"})))
            .await
            .error
            .is_none());
        let (results, bridge) = run_sync_now_cycle(&state, &groups, None).await.unwrap();
        assert!(!bridge.credentials_skipped_locked);
        assert!(results.iter().any(|(g, _)| *g == Group::Credentials));
    }

    #[tokio::test]
    async fn notes_preview_revision_and_dirty_auto_group_gate() {
        let root = std::env::temp_dir().join(Uuid::new_v4().to_string());
        let state = AppState::new();
        let d = Dispatcher::new();
        register_all(&d, state.clone());
        let store = crate::notes::NotesStore::open(root.join("notes")).unwrap();
        *state.notes.lock().await = Some(store.clone());
        let r = d
            .dispatch(req(
                "sync.configureGit",
                json!({"repo_path":root.join("git"),"master_password":"pw","test_cheap_kdf":true}),
            ))
            .await;
        assert!(r.error.is_none(), "{:?}", r.error);
        let groups = vec![Group::Appearance, Group::Notes];
        let preview = d
            .dispatch(req("sync.preview", json!({"groups":groups})))
            .await;
        assert!(preview.error.is_none(), "{:?}", preview.error);
        let token: Uuid =
            serde_json::from_value(preview.result.unwrap()["previewToken"].clone()).unwrap();
        crate::notes::dispatch(&store,"notes.put",json!({"note":{"id":"n","parent_id":null,"title":"N","markdown":"local"},"expectedVersion":0}),&[]).unwrap();
        assert!(require_completed_sync_setup(&state, &groups, Some(token))
            .await
            .is_err());
        let preview = d
            .dispatch(req("sync.preview", json!({"groups":groups})))
            .await;
        let token: Uuid =
            serde_json::from_value(preview.result.unwrap()["previewToken"].clone()).unwrap();
        require_completed_sync_setup(&state, &groups, Some(token))
            .await
            .unwrap();
        let r = d
            .dispatch(req(
                "notes.editState",
                json!({"owner":"editor","dirty":true}),
            ))
            .await;
        assert!(r.error.is_none());
        let (results, bridge) = run_sync_now_cycle(&state, &groups, None).await.unwrap();
        assert!(bridge.notes_paused_dirty);
        assert!(results.iter().any(|(g, _)| *g == Group::Appearance));
        assert!(results.iter().all(|(g, _)| *g != Group::Notes));
        assert!(
            require_engine(&state)
                .await
                .unwrap()
                .list_local(Group::Notes)
                .await
                .is_empty(),
            "preview must not consume persisted export baseline"
        );
        d.dispatch(req(
            "notes.editState",
            json!({"owner":"editor","dirty":false}),
        ))
        .await;
        let (results, bridge) = run_sync_now_cycle(&state, &groups, None).await.unwrap();
        assert!(!bridge.notes_paused_dirty);
        assert!(results.iter().any(|(g, _)| *g == Group::Notes));
        assert_eq!(store.sync_snapshot().unwrap().0.len(), 1);
    }

    #[tokio::test]
    async fn failed_reconfigure_preserves_backend_generation() {
        let root = std::env::temp_dir().join(format!("aerotab-generation-{}", Uuid::new_v4()));
        let state = AppState::new();
        let d = Dispatcher::new();
        register_all(&d, state.clone());
        let args =
            json!({"repo_path": root.join("a"), "master_password": "pw", "test_cheap_kdf": true});
        assert!(d
            .dispatch(req("sync.configureGit", args))
            .await
            .error
            .is_none());
        let bad = root.join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("device-id"), "invalid UUID").unwrap();
        let args = json!({"repo_path": root.join("b"), "state_dir": bad, "master_password": "pw", "test_cheap_kdf": true});
        assert!(d
            .dispatch(req("sync.configureGit", args))
            .await
            .error
            .is_some());
        let git = state.git_backend.lock().await.clone().unwrap();
        git.put_blob("generation.bin", b"A").await.unwrap();
        assert!(root.join("a/generation.bin").exists());
        assert!(!root.join("b/generation.bin").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn rejoin_committed_identity_rejects_old_approval_before_rpc_cleanup() {
        let root = std::env::temp_dir().join(Uuid::new_v4().to_string());
        let st = AppState::new();
        let d = Dispatcher::new();
        register_all(&d, st.clone());
        let s = SettingsStore::open(root.join("s")).unwrap();
        *st.settings.lock().await = Some(s.clone());
        assert!(d
            .dispatch(req(
                "sync.configureGit",
                json!({"repo_path":root.join("git"),"master_password":"pw","test_cheap_kdf":true})
            ))
            .await
            .error
            .is_none());
        let groups = vec![Group::Appearance];
        let p = d
            .dispatch(req("sync.preview", json!({"groups":groups})))
            .await
            .result
            .unwrap();
        assert!(d
            .dispatch(req(
                "sync.now",
                json!({"groups":groups,"preview_token":p["previewToken"]})
            ))
            .await
            .error
            .is_none());
        let old = st.oplog.lock().await.clone().unwrap();
        let next = old.rejoin_successor(Uuid::new_v4()).await.unwrap();
        *st.oplog.lock().await = Some(Arc::new(next));
        assert!(
            require_completed_sync_setup(&st, &groups, None)
                .await
                .is_err(),
            "in-memory approval must be identity bound before RPC invalidate"
        );
        let restarted = AppState::new();
        *restarted.settings.lock().await = Some(s);
        let rd = Dispatcher::new();
        register_all(&rd, restarted.clone());
        assert!(rd
            .dispatch(req(
                "sync.configureGit",
                json!({"repo_path":root.join("git"),"master_password":"pw","test_cheap_kdf":true})
            ))
            .await
            .error
            .is_none());
        assert!(
            require_completed_sync_setup(&restarted, &groups, None)
                .await
                .is_err(),
            "persisted approval must not survive rejoin"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn pending_migration_restart_requires_preview_even_after_recovery() {
        let root = std::env::temp_dir().join(Uuid::new_v4().to_string());
        let st = AppState::new();
        let d = Dispatcher::new();
        register_all(&d, st.clone());
        let s = SettingsStore::open(root.join("s")).unwrap();
        *st.settings.lock().await = Some(s.clone());
        assert!(d
            .dispatch(req(
                "sync.configureGit",
                json!({"repo_path":root.join("git"),"master_password":"pw","test_cheap_kdf":true})
            ))
            .await
            .error
            .is_none());
        let groups = vec![Group::Appearance];
        let p = d
            .dispatch(req("sync.preview", json!({"groups":groups})))
            .await
            .result
            .unwrap();
        assert!(d
            .dispatch(req(
                "sync.now",
                json!({"groups":groups,"preview_token":p["previewToken"]})
            ))
            .await
            .error
            .is_none());
        let live = require_engine(&st).await.unwrap();
        s.local_set("sync.testProjectionFault", &json!("journal"))
            .unwrap();
        assert!(projection::commit_projection(
            &st,
            &live,
            &live.preview_copy().await,
            &groups,
            true
        )
        .await
        .is_err());
        let restarted = AppState::new();
        *restarted.settings.lock().await = Some(s);
        let rd = Dispatcher::new();
        register_all(&rd, restarted.clone());
        assert!(rd
            .dispatch(req(
                "sync.configureGit",
                json!({"repo_path":root.join("git"),"master_password":"pw","test_cheap_kdf":true})
            ))
            .await
            .error
            .is_none());
        projection::recover_projection(&restarted).await.unwrap();
        assert!(require_completed_sync_setup(&restarted, &groups, None)
            .await
            .is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn cancel_current_c_dominates_known_profile_tombstone_and_appearance_b() {
        for group in [Group::Connections, Group::Appearance] {
            let root = std::env::temp_dir().join(Uuid::new_v4().to_string());
            let st = AppState::new();
            let d = Dispatcher::new();
            register_all(&d, st.clone());
            let s = SettingsStore::open(root.join("s")).unwrap();
            *st.settings.lock().await = Some(s.clone());
            let profiles = ProfileStore::open(root.join("p")).unwrap();
            *st.profiles.lock().await = Some(profiles.clone());
            let id = Uuid::new_v4();
            let profile=serde_json::from_value(json!({"id":id,"name":"A","kind":"ssh","ssh":{"host":"host","port":22,"user":"u","auth":"Agent"}})).unwrap();
            profiles.upsert(profile).await.unwrap();
            s.set("terminal", &json!({"fontSize":12})).unwrap();
            assert!(d.dispatch(req("sync.configureGit",json!({"repo_path":root.join("git"),"master_password":"pw","test_cheap_kdf":true}))).await.error.is_none());
            let groups = vec![group];
            let p = d
                .dispatch(req("sync.preview", json!({"groups":groups})))
                .await
                .result
                .unwrap();
            assert!(d
                .dispatch(req(
                    "sync.now",
                    json!({"groups":groups,"preview_token":p["previewToken"]})
                ))
                .await
                .error
                .is_none());
            let live = require_engine(&st).await.unwrap();
            let remote = live.preview_copy().await;
            if group == Group::Connections {
                remote
                    .delete_local(group, crate::sync::RecordId(id))
                    .await
                    .unwrap();
            } else {
                s.set("terminal", &json!({"fontSize":29})).unwrap();
                projection::export_candidate(&st, &remote, &groups)
                    .await
                    .unwrap();
                s.set("terminal", &json!({"fontSize":12})).unwrap();
            }
            let writer = OplogDriver::new(
                st.oplog.lock().await.as_ref().unwrap().blobs(),
                Uuid::new_v4(),
                b"pw",
                KdfParams::test_cheap(),
            )
            .with_protocol(3)
            .unwrap();
            writer.cycle(&remote, &groups).await.unwrap();
            let (candidate, _) = st
                .oplog
                .lock()
                .await
                .as_ref()
                .unwrap()
                .maintenance_candidate(&live, &groups)
                .await
                .unwrap();
            s.local_set("sync.testProjectionFault", &json!("journal"))
                .unwrap();
            assert!(
                projection::commit_projection(&st, &live, &candidate, &groups, false)
                    .await
                    .is_err()
            );
            if group == Group::Connections {
                let mut p = profiles.list().await.unwrap().remove(0);
                p.name = "C".into();
                profiles.upsert(p).await.unwrap();
            } else {
                s.set("terminal", &json!({"fontSize":40})).unwrap();
            }
            assert!(d
                .dispatch(req(
                    "sync.maintenanceCancel",
                    json!({"confirm":"CANCEL MAINTENANCE KEEP LOCAL"})
                ))
                .await
                .error
                .is_none());
            let p = d
                .dispatch(req("sync.preview", json!({"groups":groups})))
                .await
                .result
                .unwrap();
            let r = d
                .dispatch(req(
                    "sync.now",
                    json!({"groups":groups,"preview_token":p["previewToken"]}),
                ))
                .await;
            assert!(r.error.is_none(), "{:?}", r.error);
            if group == Group::Connections {
                assert_eq!(profiles.list().await.unwrap()[0].name, "C");
            } else {
                assert_eq!(s.get("terminal").unwrap().unwrap()["fontSize"], 40);
            }
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[tokio::test]
    async fn cancelled_migration_keeps_protocol_and_current_store_and_allows_retry() {
        let root = std::env::temp_dir().join(Uuid::new_v4().to_string());
        let st = AppState::new();
        let d = Dispatcher::new();
        register_all(&d, st.clone());
        let s = SettingsStore::open(root.join("s")).unwrap();
        *st.settings.lock().await = Some(s.clone());
        *st.profiles.lock().await = Some(ProfileStore::open(root.join("p")).unwrap());
        s.set("terminal", &json!({"fontSize":12})).unwrap();
        assert!(d.dispatch(req("sync.configureGit",json!({"repo_path":root.join("git"),"master_password":"pw","protocol":2,"test_cheap_kdf":true}))).await.error.is_none());
        let live = require_engine(&st).await.unwrap();
        let candidate = live.preview_copy().await;
        projection::export_candidate(&st, &candidate, &[Group::Appearance])
            .await
            .unwrap();
        s.local_set("sync.testProjectionFault", &json!("journal"))
            .unwrap();
        assert!(
            projection::commit_projection(&st, &live, &candidate, &[Group::Appearance], true)
                .await
                .is_err()
        );
        s.set("terminal", &json!({"fontSize":40})).unwrap();
        assert!(d
            .dispatch(req(
                "sync.maintenanceCancel",
                json!({"confirm":"CANCEL MAINTENANCE KEEP LOCAL"})
            ))
            .await
            .error
            .is_none());
        assert_eq!(st.oplog.lock().await.as_ref().unwrap().protocol(), 2);
        assert_eq!(s.get("terminal").unwrap().unwrap()["fontSize"], 40);
        assert!(d
            .dispatch(req("sync.migrationPreview", json!({})))
            .await
            .error
            .is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn failed_notes_maintenance_cancel_then_sync_preserves_c_and_remote_b() {
        let root = std::env::temp_dir().join(Uuid::new_v4().to_string());
        let st = AppState::new();
        let d = Dispatcher::new();
        register_all(&d, st.clone());
        let s = SettingsStore::open(root.join("s")).unwrap();
        *st.settings.lock().await = Some(s.clone());
        *st.profiles.lock().await = Some(ProfileStore::open(root.join("p")).unwrap());
        let notes = crate::notes::NotesStore::open(root.join("notes")).unwrap();
        *st.notes.lock().await = Some(notes.clone());
        crate::notes::dispatch(&notes,"notes.put",json!({"note":{"id":"n","parent_id":null,"title":"A","markdown":"A","tags":[],"profile_ids":[]},"expectedVersion":0}),&[]).unwrap();
        assert!(d
            .dispatch(req(
                "sync.configureGit",
                json!({"repo_path":root.join("git"),"master_password":"pw","test_cheap_kdf":true})
            ))
            .await
            .error
            .is_none());
        let groups = vec![Group::Notes];
        let p = d
            .dispatch(req("sync.preview", json!({"groups":groups})))
            .await
            .result
            .unwrap();
        assert!(d
            .dispatch(req(
                "sync.now",
                json!({"groups":groups,"preview_token":p["previewToken"]})
            ))
            .await
            .error
            .is_none());
        let live = require_engine(&st).await.unwrap();
        let remote = live.preview_copy().await;
        let (mut entities, _, _) = notes.sync_snapshot().unwrap();
        if let crate::notes::SyncEntity::Note(n) = &mut entities[0] {
            n.markdown = "B".into();
            n.version = 0;
            n.updated_at = 0;
        }
        let entity = entities.remove(0);
        remote
            .put_local(
                Group::Notes,
                entity.record_id(),
                serde_json::to_vec(&entity).unwrap(),
            )
            .await
            .unwrap();
        let writer = OplogDriver::new(
            st.oplog.lock().await.as_ref().unwrap().blobs(),
            Uuid::new_v4(),
            b"pw",
            KdfParams::test_cheap(),
        )
        .with_protocol(3)
        .unwrap();
        writer.cycle(&remote, &groups).await.unwrap();
        let (candidate, _) = st
            .oplog
            .lock()
            .await
            .as_ref()
            .unwrap()
            .maintenance_candidate(&live, &groups)
            .await
            .unwrap();
        s.local_set("sync.testProjectionFault", &json!("journal"))
            .unwrap();
        assert!(
            projection::commit_projection(&st, &live, &candidate, &groups, false)
                .await
                .is_err()
        );
        let (mut entities, _, rev) = notes.sync_snapshot().unwrap();
        if let crate::notes::SyncEntity::Note(n) = &mut entities[0] {
            n.markdown = "C".into();
        }
        notes.apply_sync(entities, vec![], rev).unwrap();
        assert!(projection::recover_projection(&st).await.is_err());
        let digest = live.preview_digest().await.unwrap();
        assert!(d
            .dispatch(req("sync.maintenanceCancel", json!({"confirm":"wrong"})))
            .await
            .error
            .is_some());
        assert_eq!(
            require_engine(&st)
                .await
                .unwrap()
                .preview_digest()
                .await
                .unwrap(),
            digest
        );
        assert!(d
            .dispatch(req(
                "sync.maintenanceCancel",
                json!({"confirm":"CANCEL MAINTENANCE KEEP LOCAL"})
            ))
            .await
            .error
            .is_none());
        assert!(require_completed_sync_setup(&st, &groups, None)
            .await
            .is_err());
        let p = d
            .dispatch(req("sync.preview", json!({"groups":groups})))
            .await
            .result
            .unwrap();
        let r = d
            .dispatch(req(
                "sync.now",
                json!({"groups":groups,"preview_token":p["previewToken"]}),
            ))
            .await;
        assert!(r.error.is_none(), "{:?}", r.error);
        let text = serde_json::to_string(&notes.sync_snapshot().unwrap().0).unwrap();
        assert!(text.contains("C"), "local C lost: {text}");
        assert!(text.contains("B"), "remote B lost: {text}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn maintenance_credentials_hash_guards_edits_deletes_and_locked_resolution() {
        for delete in [false, true] {
            let root = std::env::temp_dir().join(Uuid::new_v4().to_string());
            let st = AppState::new();
            let d = Dispatcher::new();
            register_all(&d, st.clone());
            let s = SettingsStore::open(root.join("s")).unwrap();
            *st.settings.lock().await = Some(s.clone());
            *st.profiles.lock().await = Some(ProfileStore::open(root.join("p")).unwrap());
            let vault = crate::vault::VaultStore::open_with_params(
                root.join("vault"),
                KdfParams::test_cheap(),
            )
            .unwrap();
            vault.initialize(b"vpw").await.unwrap();
            let entry: crate::vault::VaultEntry = serde_json::from_value(
                json!({"id":"e","label":"E","kind":"password","secret":"A"}),
            )
            .unwrap();
            vault.put(entry.clone()).await.unwrap();
            *st.vault.lock().await = Some(vault.clone());
            assert!(d.dispatch(req("sync.configureGit",json!({"repo_path":root.join("git"),"master_password":"pw","test_cheap_kdf":true}))).await.error.is_none());
            let live = require_engine(&st).await.unwrap();
            let candidate = live.preview_copy().await;
            projection::export_candidate(&st, &candidate, &[Group::Credentials])
                .await
                .unwrap();
            let mut b = entry.clone();
            b.secret = "B".into();
            vault.put(b).await.unwrap();
            projection::export_candidate(&st, &candidate, &[Group::Credentials])
                .await
                .unwrap();
            let writer = OplogDriver::new(
                st.oplog.lock().await.as_ref().unwrap().blobs(),
                Uuid::new_v4(),
                b"pw",
                KdfParams::test_cheap(),
            )
            .with_protocol(3)
            .unwrap();
            writer
                .cycle(&candidate, &[Group::Credentials])
                .await
                .unwrap();
            vault.put(entry.clone()).await.unwrap();
            s.local_set("sync.testProjectionFault", &json!("journal"))
                .unwrap();
            assert!(projection::commit_projection(
                &st,
                &live,
                &candidate,
                &[Group::Credentials],
                false
            )
            .await
            .is_err());
            let j = st
                .oplog
                .lock()
                .await
                .as_ref()
                .unwrap()
                .maintenance_journal()
                .unwrap()
                .unwrap();
            assert!(j["baseline"]["credentialsHash"].is_string());
            assert!(j["baseline"].get("secret").is_none());
            if delete {
                vault.remove("e").await.unwrap();
            } else {
                let mut e = entry;
                e.secret = "C".into();
                vault.put(e).await.unwrap();
            }
            assert!(projection::recover_projection(&st).await.is_err());
            let status = d
                .dispatch(req("sync.maintenanceStatus", json!({})))
                .await
                .result
                .unwrap();
            assert_eq!(status["checkpointChanged"], true);
            assert!(!status.to_string().contains("secret"));
            vault.lock().await;
            let before = require_engine(&st)
                .await
                .unwrap()
                .preview_digest()
                .await
                .unwrap();
            assert!(d
                .dispatch(req(
                    "sync.maintenanceCancel",
                    json!({"confirm":"CANCEL MAINTENANCE KEEP LOCAL"})
                ))
                .await
                .error
                .is_some());
            assert_eq!(
                require_engine(&st)
                    .await
                    .unwrap()
                    .preview_digest()
                    .await
                    .unwrap(),
                before
            );
            vault.unlock(b"vpw").await.unwrap();
            assert!(d
                .dispatch(req(
                    "sync.maintenanceCancel",
                    json!({"confirm":"CANCEL MAINTENANCE KEEP LOCAL"})
                ))
                .await
                .error
                .is_none());
            let groups = vec![Group::Credentials];
            let p = d
                .dispatch(req("sync.preview", json!({"groups":groups})))
                .await
                .result
                .unwrap();
            let r = d
                .dispatch(req(
                    "sync.now",
                    json!({"groups":groups,"preview_token":p["previewToken"]}),
                ))
                .await;
            assert!(r.error.is_none(), "{:?}", r.error);
            if delete {
                assert!(vault.list().await.unwrap().is_empty());
            } else {
                assert_eq!(vault.get("e").await.unwrap().secret, "C");
            }
            let inspected = d
                .dispatch(req("sync.recoveryInspect", json!({})))
                .await
                .result
                .unwrap();
            assert_eq!(inspected["available"], true);
            assert!(!inspected.to_string().contains("secret"));
            let exported = d
                .dispatch(req("sync.recoveryExport", json!({})))
                .await
                .result
                .unwrap();
            assert_eq!(exported["encrypted"], true);
            let r = d
                .dispatch(req(
                    "sync.recoveryRestore",
                    json!({"confirm":"RESTORE LOCAL RECOVERY"}),
                ))
                .await;
            assert!(r.error.is_none(), "{:?}", r.error);
            assert!(require_completed_sync_setup(&st, &groups, None)
                .await
                .is_err());
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[tokio::test]
    async fn recovery_stops_for_post_crash_profile_and_notes_tombstones() {
        for group in [Group::Connections, Group::Notes] {
            let root = std::env::temp_dir().join(Uuid::new_v4().to_string());
            let st = AppState::new();
            let d = Dispatcher::new();
            register_all(&d, st.clone());
            let s = SettingsStore::open(root.join("s")).unwrap();
            *st.settings.lock().await = Some(s.clone());
            let profiles = ProfileStore::open(root.join("p")).unwrap();
            *st.profiles.lock().await = Some(profiles.clone());
            let notes = crate::notes::NotesStore::open(root.join("notes")).unwrap();
            *st.notes.lock().await = Some(notes.clone());
            let id = Uuid::new_v4();
            if group == Group::Connections {
                let p=serde_json::from_value(json!({"id":id,"name":"A","kind":"ssh","ssh":{"host":"host","port":22,"user":"u","auth":"Agent"}})).unwrap();
                profiles.upsert(p).await.unwrap();
            } else {
                crate::notes::dispatch(&notes,"notes.put",json!({"note":{"id":"n","parent_id":null,"title":"A","markdown":"A","tags":[],"profile_ids":[]},"expectedVersion":0}),&[]).unwrap();
            }
            assert!(d.dispatch(req("sync.configureGit",json!({"repo_path":root.join("git"),"master_password":"pw","test_cheap_kdf":true}))).await.error.is_none());
            let live = require_engine(&st).await.unwrap();
            let candidate = live.preview_copy().await;
            projection::export_candidate(&st, &candidate, &[group])
                .await
                .unwrap();
            s.local_set("sync.testProjectionFault", &json!("journal"))
                .unwrap();
            assert!(
                projection::commit_projection(&st, &live, &candidate, &[group], false)
                    .await
                    .is_err()
            );
            if group == Group::Connections {
                let mut edited = profiles.list().await.unwrap().remove(0);
                edited.name = "C".into();
                profiles.upsert(edited).await.unwrap();
                assert!(projection::recover_projection(&st).await.is_err());
                assert_eq!(profiles.list().await.unwrap()[0].name, "C");
                profiles.delete(id).await.unwrap();
            } else {
                let (mut entities, _, rev) = notes.sync_snapshot().unwrap();
                if let crate::notes::SyncEntity::Note(note) = &mut entities[0] {
                    note.markdown = "C".into();
                }
                notes.apply_sync(entities, vec![], rev).unwrap();
                assert!(projection::recover_projection(&st).await.is_err());
                let rev = notes.sync_snapshot().unwrap().2;
                notes.apply_sync(vec![], vec!["n".into()], rev).unwrap();
            }
            assert!(
                projection::recover_projection(&st).await.is_err(),
                "post-crash deletion must not resurrect"
            );
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[tokio::test]
    async fn recovery_preserves_post_crash_settings_edit_and_delete() {
        for delete in [false, true] {
            let root = std::env::temp_dir().join(Uuid::new_v4().to_string());
            let st = AppState::new();
            let d = Dispatcher::new();
            register_all(&d, st.clone());
            let s = SettingsStore::open(root.join("s")).unwrap();
            s.set("terminal", &json!({"fontSize":12})).unwrap();
            *st.settings.lock().await = Some(s.clone());
            *st.profiles.lock().await = Some(ProfileStore::open(root.join("p")).unwrap());
            assert!(d.dispatch(req("sync.configureGit",json!({"repo_path":root.join("git"),"master_password":"pw","test_cheap_kdf":true}))).await.error.is_none());
            let live = require_engine(&st).await.unwrap();
            let c = live.preview_copy().await;
            projection::export_candidate(&st, &c, &[Group::Appearance])
                .await
                .unwrap();
            s.local_set("sync.testProjectionFault", &json!("journal"))
                .unwrap();
            assert!(
                projection::commit_projection(&st, &live, &c, &[Group::Appearance], false)
                    .await
                    .is_err()
            );
            if delete {
                s.remove("terminal").unwrap();
            } else {
                s.set("terminal", &json!({"fontSize":40})).unwrap();
            }
            let before = s.get("terminal").unwrap();
            assert!(
                projection::recover_projection(&st).await.is_err(),
                "must stop rather than overwrite post-crash C"
            );
            assert_eq!(s.get("terminal").unwrap(), before);
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[tokio::test]
    async fn projection_rejects_invalid_record_without_polluting_live_or_stores() {
        let root = std::env::temp_dir().join(format!("aerotab-project-reject-{}", Uuid::new_v4()));
        let state = AppState::new();
        let d = Dispatcher::new();
        register_all(&d, state.clone());
        let settings = SettingsStore::open(root.join("settings")).unwrap();
        settings.set("terminal", &json!({"fontSize":12})).unwrap();
        *state.settings.lock().await = Some(settings.clone());
        *state.profiles.lock().await = Some(ProfileStore::open(root.join("profiles")).unwrap());
        assert!(d
            .dispatch(req(
                "sync.configureGit",
                json!({"repo_path":root.join("git"),"master_password":"pw","test_cheap_kdf":true})
            ))
            .await
            .error
            .is_none());
        let live = require_engine(&state).await.unwrap();
        let before = live.preview_digest().await.unwrap();
        let candidate = live.preview_copy().await;
        candidate
            .put_local(
                Group::Connections,
                crate::sync::RecordId(Uuid::new_v4()),
                b"invalid".to_vec(),
            )
            .await
            .unwrap();
        assert!(
            projection::commit_projection(&state, &live, &candidate, &Group::ALL, true)
                .await
                .is_err()
        );
        assert_eq!(
            require_engine(&state)
                .await
                .unwrap()
                .preview_digest()
                .await
                .unwrap(),
            before
        );
        assert_eq!(settings.get("terminal").unwrap().unwrap()["fontSize"], 12);
        assert!(state
            .oplog
            .lock()
            .await
            .as_ref()
            .unwrap()
            .maintenance_journal()
            .unwrap()
            .is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn projection_journal_recovers_every_commit_boundary_before_export() {
        for step in ["journal", "projection", "persistence", "install"] {
            let root =
                std::env::temp_dir().join(format!("aerotab-project-fault-{}", Uuid::new_v4()));
            let state = AppState::new();
            let d = Dispatcher::new();
            register_all(&d, state.clone());
            let settings = SettingsStore::open(root.join("settings")).unwrap();
            settings.set("terminal", &json!({"fontSize":12})).unwrap();
            *state.settings.lock().await = Some(settings.clone());
            let profiles = ProfileStore::open(root.join("profiles")).unwrap();
            *state.profiles.lock().await = Some(profiles.clone());
            assert!(d.dispatch(req("sync.configureGit",json!({"repo_path":root.join("git"),"master_password":"pw","test_cheap_kdf":true}))).await.error.is_none());
            let live = require_engine(&state).await.unwrap();
            let candidate = live.preview_copy().await;
            let remote = SettingsStore::open(root.join("remote")).unwrap();
            remote.set("terminal", &json!({"fontSize":29})).unwrap();
            crate::sync::bridge::export_locals(
                &profiles,
                &remote,
                None,
                &[],
                &candidate,
                &[Group::Appearance],
            )
            .await
            .unwrap();
            settings
                .local_set("sync.testProjectionFault", &json!(step))
                .unwrap();
            assert!(projection::commit_projection(
                &state,
                &live,
                &candidate,
                &[Group::Appearance],
                false
            )
            .await
            .is_err());
            settings
                .local_set("sync.testProjectionFault", &Value::Null)
                .unwrap();
            // Recreate AppState and driver: exercise durable restart recovery,
            // not merely a retry against the already-installed candidate.
            let restarted = AppState::new();
            *restarted.settings.lock().await = Some(settings.clone());
            *restarted.profiles.lock().await = Some(profiles.clone());
            let rd = Dispatcher::new();
            register_all(&rd, restarted.clone());
            assert!(rd.dispatch(req("sync.configureGit",json!({"repo_path":root.join("git"),"master_password":"pw","test_cheap_kdf":true}))).await.error.is_none());
            projection::recover_projection(&restarted).await.unwrap();
            assert_eq!(settings.get("terminal").unwrap().unwrap()["fontSize"], 29);
            let recovered = require_engine(&restarted).await.unwrap();
            let digest = recovered.preview_digest().await.unwrap();
            crate::sync::bridge::export_locals(
                &profiles,
                &settings,
                None,
                &[],
                &recovered,
                &[Group::Appearance],
            )
            .await
            .unwrap();
            assert_eq!(
                recovered.preview_digest().await.unwrap(),
                digest,
                "boundary {step} must not export stale A"
            );
            let p = rd
                .dispatch(req("sync.preview", json!({"groups":["Appearance"]})))
                .await
                .result
                .unwrap();
            let r = rd
                .dispatch(req(
                    "sync.now",
                    json!({"groups":["Appearance"],"preview_token":p["previewToken"]}),
                ))
                .await;
            assert!(r.error.is_none(), "{step}: {:?}", r.error);
            assert_eq!(settings.get("terminal").unwrap().unwrap()["fontSize"], 29);
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[tokio::test]
    async fn compact_projects_pulled_setting_before_following_export() {
        let root =
            std::env::temp_dir().join(format!("aerotab-compact-projection-{}", Uuid::new_v4()));
        let state = AppState::new();
        let d = Dispatcher::new();
        register_all(&d, state.clone());
        let settings = SettingsStore::open(root.join("settings")).unwrap();
        settings.set("terminal", &json!({"fontSize":12})).unwrap();
        *state.settings.lock().await = Some(settings.clone());
        let profiles = ProfileStore::open(root.join("profiles")).unwrap();
        *state.profiles.lock().await = Some(profiles.clone());
        assert!(d
            .dispatch(req(
                "sync.configureGit",
                json!({"repo_path":root.join("git"),"master_password":"pw","test_cheap_kdf":true})
            ))
            .await
            .error
            .is_none());
        let groups = vec![Group::Appearance];
        let p = d
            .dispatch(req("sync.preview", json!({"groups":groups})))
            .await
            .result
            .unwrap();
        assert!(d
            .dispatch(req(
                "sync.now",
                json!({"groups":groups,"preview_token":p["previewToken"]})
            ))
            .await
            .error
            .is_none());
        let remote = require_engine(&state).await.unwrap().preview_copy().await;
        let rs = SettingsStore::open(root.join("remote")).unwrap();
        rs.set("terminal", &json!({"fontSize":29})).unwrap();
        crate::sync::bridge::export_locals(&profiles, &rs, None, &[], &remote, &groups)
            .await
            .unwrap();
        let driver = OplogDriver::new(
            state.oplog.lock().await.as_ref().unwrap().blobs(),
            Uuid::new_v4(),
            b"pw",
            KdfParams::test_cheap(),
        )
        .with_protocol(3)
        .unwrap();
        driver.cycle(&remote, &groups).await.unwrap();
        let r = d
            .dispatch(req("sync.compact", json!({"groups":groups})))
            .await;
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(
            settings.get("terminal").unwrap().unwrap()["fontSize"],
            29,
            "compact must import B into app store"
        );
        let r = d.dispatch(req("sync.now", json!({"groups":groups}))).await;
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(settings.get("terminal").unwrap().unwrap()["fontSize"], 29);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn migration_projects_legacy_remote_setting_before_next_export() {
        let root = std::env::temp_dir().join(format!("aerotab-projection-{}", Uuid::new_v4()));
        let state = AppState::new();
        let d = Dispatcher::new();
        register_all(&d, state.clone());
        let settings = SettingsStore::open(root.join("settings")).unwrap();
        settings.set("terminal", &json!({"fontSize":12})).unwrap();
        *state.settings.lock().await = Some(settings.clone());
        *state.profiles.lock().await = Some(ProfileStore::open(root.join("profiles")).unwrap());
        assert!(d.dispatch(req("sync.configureGit",json!({"repo_path":root.join("git"),"master_password":"pw","test_cheap_kdf":true,"protocol":2}))).await.error.is_none());
        let current = require_engine(&state).await.unwrap();
        let profiles = state.profiles.lock().await.clone().unwrap();
        crate::sync::bridge::export_locals(
            &profiles,
            &settings,
            None,
            &[],
            &current,
            &[Group::Appearance],
        )
        .await
        .unwrap();
        let remote = current.preview_copy().await;
        let remote_settings = SettingsStore::open(root.join("remote-settings")).unwrap();
        remote_settings
            .set("terminal", &json!({"fontSize":29}))
            .unwrap();
        let profiles = state.profiles.lock().await.clone().unwrap();
        crate::sync::bridge::export_locals(
            &profiles,
            &remote_settings,
            None,
            &[],
            &remote,
            &[Group::Appearance],
        )
        .await
        .unwrap();
        let writer = OplogDriver::new(
            state.oplog.lock().await.as_ref().unwrap().blobs(),
            Uuid::new_v4(),
            b"pw",
            KdfParams::test_cheap(),
        );
        writer.cycle(&remote, &[Group::Appearance]).await.unwrap();
        let p = d
            .dispatch(req("sync.migrationPreview", json!({})))
            .await
            .result
            .unwrap();
        let r = d
            .dispatch(req(
                "sync.migrateV3",
                json!({"previewToken":p["previewToken"],"confirm":"MIGRATE TO V3"}),
            ))
            .await;
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(
            settings.get("terminal").unwrap().unwrap()["fontSize"],
            29,
            "migration must project B, not leave A for next export"
        );
        let before = require_engine(&state)
            .await
            .unwrap()
            .preview_digest()
            .await
            .unwrap();
        crate::sync::bridge::export_locals(
            &profiles,
            &settings,
            None,
            &[],
            &require_engine(&state).await.unwrap(),
            &[Group::Appearance],
        )
        .await
        .unwrap();
        assert_eq!(
            require_engine(&state)
                .await
                .unwrap()
                .preview_digest()
                .await
                .unwrap(),
            before,
            "next export must not bump old A over B"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn explicit_v3_migration_preserves_legacy_and_enables_approved_compact() {
        let dir = std::env::temp_dir().join(format!("aerotab-migrate-{}", Uuid::new_v4()));
        let state = AppState::new();
        let d = Dispatcher::new();
        register_all(&d, state.clone());
        assert!(d
            .dispatch(req(
                "sync.configureGit",
                json!({"repo_path":dir,"master_password":"pw","test_cheap_kdf":true,"protocol":2})
            ))
            .await
            .error
            .is_none());
        state
            .git_backend
            .lock()
            .await
            .as_ref()
            .unwrap()
            .put_blob("v2/retained", b"keep")
            .await
            .unwrap();
        let preview = d.dispatch(req("sync.migrationPreview", json!({}))).await;
        assert!(preview.error.is_none(), "{:?}", preview.error);
        let token = preview.result.unwrap()["previewToken"].clone();
        assert!(d
            .dispatch(req(
                "sync.migrateV3",
                json!({"previewToken":token,"confirm":"MIGRATE TO V3"})
            ))
            .await
            .error
            .is_none());
        assert_eq!(state.oplog.lock().await.as_ref().unwrap().protocol(), 3);
        assert_eq!(std::fs::read(dir.join("v2/retained")).unwrap(), b"keep");
        let groups = vec![Group::Connections];
        let preview = d
            .dispatch(req("sync.preview", json!({"groups":groups})))
            .await;
        assert!(preview.error.is_none(), "{:?}", preview.error);
        let r = d
            .dispatch(req(
                "sync.now",
                json!({"groups":groups,"preview_token":preview.result.unwrap()["previewToken"]}),
            ))
            .await;
        assert!(r.error.is_none(), "{:?}", r.error);
        let r = d
            .dispatch(req("sync.compact", json!({"groups":groups})))
            .await;
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(r.result.unwrap()["compacted"], true);
        assert!(dir.join("v3/snapshots").is_dir());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn v1_purge_compares_payload_and_tombstone_against_durable_v3() {
        let dir = std::env::temp_dir().join(format!("aerotab-v1-proof-{}", Uuid::new_v4()));
        let state = AppState::new();
        let d = Dispatcher::new();
        register_all(&d, state.clone());
        assert!(d
            .dispatch(req(
                "sync.configureGit",
                json!({"repo_path":dir,"master_password":"pw","test_cheap_kdf":true})
            ))
            .await
            .error
            .is_none());
        let id = crate::sync::RecordId(Uuid::new_v4());
        let engine = require_engine(&state).await.unwrap();
        engine
            .put_local(Group::Connections, id, b"verified".to_vec())
            .await
            .unwrap();
        let record = engine
            .snapshot_group(Group::Connections)
            .await
            .pop()
            .unwrap()
            .1;
        let wire = serde_json::to_vec(&record).unwrap();
        let encrypted =
            crate::sync::crypto::encrypt(b"pw", &wire, KdfParams::test_cheap()).unwrap();
        let git = state.git_backend.lock().await.clone().unwrap();
        let path = format!("connections/{}.bin", id.0);
        git.put_blob(&path, &encrypted).await.unwrap();
        let fail = d.dispatch(req("sync.purgeV1", json!({}))).await;
        assert!(fail.error.is_some());
        assert!(dir.join(&path).exists());
        let preview = d
            .dispatch(req("sync.preview", json!({"groups":["Connections"]})))
            .await
            .result
            .unwrap();
        assert!(d
            .dispatch(req(
                "sync.now",
                json!({"groups":["Connections"],"preview_token":preview["previewToken"]})
            ))
            .await
            .error
            .is_none());
        let success = d.dispatch(req("sync.purgeV1", json!({}))).await;
        assert!(success.error.is_none(), "{:?}", success.error);
        assert_eq!(success.result.unwrap()["deleted"], 1);
        assert!(!dir.join(&path).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn capacity_rpc_and_history_confirmation_are_enforced() {
        let dir = std::env::temp_dir().join(format!("aerotab-maintenance-{}", Uuid::new_v4()));
        let d = Dispatcher::new();
        register_all(&d, AppState::new());
        let response = d
            .dispatch(req(
                "sync.configureGit",
                json!({"repo_path": dir, "master_password": "pw", "test_cheap_kdf": true}),
            ))
            .await;
        assert!(response.error.is_none(), "{:?}", response.error);
        let response = d.dispatch(req("sync.capacity", json!({}))).await;
        assert!(response.error.is_none(), "{:?}", response.error);
        let capacity = response.result.unwrap();
        assert!(capacity["gitBytes"].as_u64().unwrap() > 0);
        assert_eq!(capacity["worktreeBytes"], 0);
        let response = d
            .dispatch(req(
                "sync.rewriteHistory",
                json!({"confirmation": "yes", "expectedOid": "invalid"}),
            ))
            .await;
        assert!(response.error.is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn purge_v1_refuses_without_verified_migration() {
        let dir = std::env::temp_dir().join(format!("aerotab-purge-{}", Uuid::new_v4()));
        let d = Dispatcher::new();
        let state = AppState::new();
        register_all(&d, state.clone());
        let r = d
            .dispatch(req(
                "sync.configureGit",
                json!({"repo_path": dir, "master_password": "pw", "test_cheap_kdf": true}),
            ))
            .await;
        assert!(r.error.is_none(), "{:?}", r.error);
        state
            .git_backend
            .lock()
            .await
            .as_ref()
            .unwrap()
            .put_blob(
                &format!("connections/{}.bin", Uuid::new_v4()),
                b"unverifiable legacy envelope",
            )
            .await
            .unwrap();
        let r = d.dispatch(req("sync.purgeV1", Value::Null)).await;
        assert!(
            r.error.is_some(),
            "must verify migration before legacy deletion"
        );
        let _ = std::fs::remove_dir_all(dir);
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
        let preview = d
            .dispatch(req("sync.preview", json!(null)))
            .await
            .result
            .unwrap();
        let r = d
            .dispatch(req(
                "sync.now",
                json!({"preview_token":preview["previewToken"]}),
            ))
            .await;
        assert!(r.error.is_none(), "{:?}", r.error);
        // Protocol 3 writes encrypted ops under v3/devices/<id>/ops/, not
        // legacy <group>/<uuid>.bin blobs.
        let v2 = dir.join("v3").join("devices");
        assert!(
            v2.is_dir(),
            "expected protocol-3 oplog layout under {}",
            v2.display()
        );
        let mut ops = 0usize;
        if let Ok(devs) = std::fs::read_dir(&v2) {
            for dev in devs.flatten() {
                let ops_dir = dev.path().join("ops");
                if let Ok(files) = std::fs::read_dir(ops_dir) {
                    ops += files.count();
                }
            }
        }
        assert!(ops >= 1, "expected at least one op batch for {rid}");
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
    async fn sync_status_uses_persisted_success_history_after_restart() {
        let mut settings_dir = std::env::temp_dir();
        settings_dir.push(format!("aerotab-cmd-settings-{}", Uuid::new_v4()));
        let history_ms = 1_789_123_456_000_i64;

        // Seed local-only syncHistory before the settings store is opened by RPC.
        {
            let store = crate::settings::SettingsStore::open(&settings_dir).unwrap();
            crate::sync::history::append_success_at(
                &store,
                "manual",
                &[Group::Connections],
                &[(
                    Group::Connections,
                    crate::sync::SyncStats {
                        pushed: 1,
                        ..Default::default()
                    },
                )],
                history_ms,
            )
            .unwrap();
        }

        let d = Dispatcher::new();
        register_all(&d, AppState::new());
        let r = d
            .dispatch(req(
                "settings.configure",
                json!({ "path": settings_dir.to_str().unwrap() }),
            ))
            .await;
        assert!(r.error.is_none(), "{:?}", r.error);
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

        let r = d.dispatch(req("sync.status", json!(null))).await;
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(r.result.unwrap()["lastSyncMs"], json!(history_ms));

        let _ = std::fs::remove_dir_all(&settings_dir);
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
        let sync_dir = std::env::temp_dir().join(format!("aerotab-auto-{}", Uuid::new_v4()));
        let r = d
            .dispatch(req(
                "sync.configureGit",
                json!({
                    "repo_path": sync_dir.to_str().unwrap(),
                    "master_password": "pw",
                    "test_cheap_kdf": true,
                }),
            ))
            .await;
        assert!(r.error.is_none());

        let preview = d
            .dispatch(req("sync.preview", json!(null)))
            .await
            .result
            .unwrap();
        let approved = d
            .dispatch(req(
                "sync.now",
                json!({"preview_token":preview["previewToken"]}),
            ))
            .await;
        assert!(approved.error.is_none());
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
    async fn group_defaults_effective_rpc_and_secret_rejection() {
        let d = Dispatcher::new();
        register_all(&d, AppState::new());
        let dir = std::env::temp_dir().join(format!("group-defaults-{}", Uuid::new_v4()));
        for (method, path) in [
            ("profile.configure", dir.join("profiles.sled")),
            ("settings.configure", dir.clone()),
        ] {
            assert!(d
                .dispatch(req(method, json!({"path":path})))
                .await
                .error
                .is_none());
        }
        let id = Uuid::new_v4();
        let raw = json!({"id":id,"name":"host","group":"team/prod","kind":"ssh","ssh":{"host":"localhost","port":22,"user":"local","auth":"Agent","inherit":["user","port","proxy_id","jump"]}});
        assert!(d.dispatch(req("profile.upsert", raw)).await.error.is_none());
        let defaults = json!({"team":{"user":"parent","port":2222},"team/prod":{"user":"child","proxy_id":null,"jump_profile_ids":[]}});
        assert!(d
            .dispatch(req(
                "settings.set",
                json!({"key":"profile.groupDefaults","value":defaults})
            ))
            .await
            .error
            .is_none());
        let result = d.dispatch(req("profile.effective", json!({"id":id}))).await;
        assert!(result.error.is_none(), "{:?}", result.error);
        let value = result.result.unwrap();
        assert_eq!(value["profile"]["ssh"]["user"], "child");
        assert_eq!(value["profile"]["ssh"]["port"], 2222);
        assert_eq!(value["sources"]["user"], "team/prod");
        assert_eq!(value["sources"]["port"], "team");
        assert_eq!(
            d.dispatch(req("profile.get", json!({"id":id})))
                .await
                .result
                .unwrap()["ssh"]["user"],
            "local"
        );
        for auth in [
            json!({"Password":{"secret":"do-not-store"}}),
            json!({"PublicKey":{"key_path":"key","passphrase":"do-not-store"}}),
        ] {
            assert!(d
                .dispatch(req(
                    "settings.set",
                    json!({"key":"profile.groupDefaults","value":{"team":{"auth":auth}}})
                ))
                .await
                .error
                .is_some());
        }
    }

    async fn inheritance_review_state() -> (
        Arc<AppState>,
        Dispatcher,
        ProfileStore,
        SettingsStore,
        crate::network_resources::NetworkResourceStore,
    ) {
        let state = AppState::new();
        let dir = std::env::temp_dir().join(format!("inherit-review-{}", Uuid::new_v4()));
        let profiles = ProfileStore::open(dir.join("profiles")).unwrap();
        let settings = SettingsStore::open(&dir).unwrap();
        let network = crate::network_resources::NetworkResourceStore::open(&dir).unwrap();
        *state.profiles.lock().await = Some(profiles.clone());
        *state.settings.lock().await = Some(settings.clone());
        *state.network_resources.lock().await = Some(network.clone());
        let dispatcher = Dispatcher::new();
        register_all(&dispatcher, state.clone());
        (state, dispatcher, profiles, settings, network)
    }

    fn review_profile(id: Uuid, ssh: Value) -> Profile {
        serde_json::from_value(json!({"id":id,"name":"host","group":"team","kind":"ssh","ssh":ssh}))
            .unwrap()
    }

    #[tokio::test]
    async fn inheritance_review_unrelated_broken_nodes_do_not_block_connections_or_deletion() {
        let (state, d, profiles, settings, _) = inheritance_review_state().await;
        let good = Uuid::new_v4();
        let broken = Uuid::new_v4();
        let base = json!({"host":"host","port":22,"user":"local","auth":"Agent"});
        profiles
            .upsert(review_profile(good, base.clone()))
            .await
            .unwrap();
        for extra in [
            json!({"jump_profile_ids":[Uuid::new_v4()]}),
            json!({"proxy_id":Uuid::new_v4()}),
            json!({"inherit":["unknown"]}),
        ] {
            let mut raw = base.clone();
            raw.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            profiles
                .upsert_synced(review_profile(broken, raw))
                .await
                .unwrap();
            assert!(resolve_ssh_profile(&state, good).await.is_ok());
            assert!(
                materialize_ssh_profile(&state, serde_json::from_value(base.clone()).unwrap())
                    .await
                    .is_ok()
            );
            assert!(d
                .dispatch(req(
                    "profile.upsert",
                    serde_json::to_value(review_profile(good, base.clone())).unwrap()
                ))
                .await
                .error
                .is_none());
            assert!(d
                .dispatch(req(
                    "settings.set",
                    json!({"key":"profile.groupDefaults","value":{"other":{"user":"new"}}})
                ))
                .await
                .error
                .is_none());
            let health = d
                .dispatch(req("profile.healthCheck", json!({"ids":[good, broken]})))
                .await;
            assert!(health.error.is_none(), "{:?}", health.error);
            let health = health.result.unwrap();
            assert_eq!(health.as_array().unwrap().len(), 2);
            assert_eq!(
                health
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|p| p["id"] == json!(broken))
                    .unwrap()["status"],
                "error"
            );
            assert!(d
                .dispatch(req("profile.delete", json!({"id":broken})))
                .await
                .error
                .is_none());
        }
        settings
            .set(
                "profile.groupDefaults",
                &json!({"unused":{"jump_profile_ids":[Uuid::new_v4()]}}),
            )
            .unwrap();
        assert!(resolve_ssh_profile(&state, good).await.is_ok());
        assert!(d.dispatch(req("settings.set", json!({"key":"profile.groupDefaults","value":{"unused":settings.get("profile.groupDefaults").unwrap().unwrap()["unused"],"other":{"user":"changed"}}}))).await.error.is_none());
    }

    #[tokio::test]
    async fn inheritance_review_mutations_validate_reverse_dependencies_and_changed_defaults_only()
    {
        let (_, d, profiles, settings, _) = inheritance_review_state().await;
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let base = json!({"host":"host","port":22,"user":"u","auth":"Agent"});
        profiles
            .upsert(review_profile(a, base.clone()))
            .await
            .unwrap();
        let mut dependent = base.clone();
        dependent["jump_profile_ids"] = json!([a]);
        profiles.upsert(review_profile(b, dependent)).await.unwrap();
        let mut cyclic = base.clone();
        cyclic["jump_profile_ids"] = json!([b]);
        assert!(d
            .dispatch(req(
                "profile.upsert",
                serde_json::to_value(review_profile(a, cyclic)).unwrap()
            ))
            .await
            .error
            .is_some());
        settings
            .set(
                "profile.groupDefaults",
                &json!({"unused":{"jump_profile_ids":[Uuid::new_v4()]}}),
            )
            .unwrap();
        let mut defaults = settings.get("profile.groupDefaults").unwrap().unwrap();
        defaults["unused"]["user"] = json!("updated");
        assert!(d
            .dispatch(req(
                "settings.set",
                json!({"key":"profile.groupDefaults","value":defaults})
            ))
            .await
            .error
            .is_none());
        let mut inherited = base;
        inherited["inherit"] = json!(["jump"]);
        profiles.upsert(review_profile(a, inherited)).await.unwrap();
        assert!(d
            .dispatch(req(
                "settings.set",
                json!({"key":"profile.groupDefaults","value":{"team":{"jump_profile_ids":[b]}}})
            ))
            .await
            .error
            .is_some());
    }

    #[tokio::test]
    async fn inheritance_review_proxy_delete_protects_hidden_raw_and_inline_fallbacks() {
        let (_, d, profiles, settings, network) = inheritance_review_state().await;
        let proxy_id = Uuid::new_v4();
        network
            .save_proxy(crate::network_resources::ProxyResource {
                id: proxy_id,
                name: "proxy".into(),
                config: crate::network_resources::ProxyConfig::Socks5 {
                    host: "localhost".into(),
                    port: 1080,
                    vault_entry_id: None,
                },
            })
            .unwrap();
        let mut ssh = json!({"host":"host","port":22,"user":"local","auth":"Agent","proxy_id":proxy_id,"inherit":["proxy_id"]});
        let id = Uuid::new_v4();
        settings
            .set("profile.groupDefaults", &json!({"team":{"proxy_id":null}}))
            .unwrap();
        profiles
            .upsert(review_profile(id, ssh.clone()))
            .await
            .unwrap();
        assert!(d
            .dispatch(req("proxy.delete", json!({"id":proxy_id})))
            .await
            .error
            .is_some());
        ssh.as_object_mut().unwrap().remove("proxy_id");
        ssh["jump_via"] =
            json!([{ "host":"inline","port":22,"user":"u","auth":"Agent","proxy_id":proxy_id }]);
        profiles.upsert(review_profile(id, ssh)).await.unwrap();
        assert!(d
            .dispatch(req("proxy.delete", json!({"id":proxy_id})))
            .await
            .error
            .is_some());
    }

    #[tokio::test]
    async fn inheritance_review_proxy_ssh_hop_flattens_inherited_jump_chain() {
        let (state, _, profiles, settings, network) = inheritance_review_state().await;
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let t = Uuid::new_v4();
        let proxy = Uuid::new_v4();
        for (id, host, inherit) in [(a, "a", vec![]), (b, "b", vec!["jump"])] {
            profiles
                .upsert(review_profile(
                    id,
                    json!({"host":host,"port":22,"user":"u","auth":"Agent","inherit":inherit}),
                ))
                .await
                .unwrap();
        }
        settings
            .set(
                "profile.groupDefaults",
                &json!({"team":{"jump_profile_ids":[a]}}),
            )
            .unwrap();
        network
            .save_proxy(crate::network_resources::ProxyResource {
                id: proxy,
                name: "hop".into(),
                config: crate::network_resources::ProxyConfig::SshJump { profile_id: b },
            })
            .unwrap();
        profiles
            .upsert(review_profile(
                t,
                json!({"host":"t","port":22,"user":"u","auth":"Agent","proxy_id":proxy}),
            ))
            .await
            .unwrap();
        let resolved = resolve_ssh_profile(&state, t).await.unwrap();
        assert_eq!(
            resolved
                .jump_via
                .iter()
                .map(|s| s.host.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert!(resolved
            .jump_via
            .iter()
            .all(|s| s.jump_via.is_empty() && s.proxy_id.is_none()));
    }

    #[tokio::test]
    async fn inherited_catalog_reaches_jumps_and_inline_profiles_remain_independent() {
        let state = AppState::new();
        let dir = std::env::temp_dir().join(format!("inherit-catalog-{}", Uuid::new_v4()));
        let profiles = ProfileStore::open(dir.join("profiles")).unwrap();
        let settings = SettingsStore::open(&dir).unwrap();
        *state.profiles.lock().await = Some(profiles.clone());
        *state.settings.lock().await = Some(settings.clone());
        let jump_id = Uuid::new_v4();
        let target_id = Uuid::new_v4();
        for (id, group, inherit, jumps) in [
            (jump_id, "bastions", vec!["user", "port"], vec![]),
            (target_id, "targets", vec!["auth"], vec![jump_id]),
        ] {
            profiles.upsert(serde_json::from_value(json!({"id":id,"name":"host","group":group,"kind":"ssh","ssh":{"host":"host","port":22,"user":"local","auth":"Agent","inherit":inherit,"jump_profile_ids":jumps}})).unwrap()).await.unwrap();
        }
        settings
            .set(
                "profile.groupDefaults",
                &json!({"bastions":{"user":"effective-user","port":2200}}),
            )
            .unwrap();
        let resolved = resolve_ssh_profile(&state, target_id).await.unwrap();
        assert_eq!(resolved.jump_via[0].user, "effective-user");
        assert_eq!(resolved.jump_via[0].port, 2200);
        settings
            .set(
                "profile.groupDefaults",
                &json!({"bastions":{"user":"latest","port":2300}}),
            )
            .unwrap();
        assert_eq!(
            resolve_ssh_profile(&state, target_id)
                .await
                .unwrap()
                .jump_via[0]
                .user,
            "latest"
        );
        let inline: SshProfile = serde_json::from_value(json!({"host":"host","port":22,"user":"local","auth":"Agent","inherit":["user","port"]})).unwrap();
        let inline = materialize_ssh_profile(&state, inline).await.unwrap();
        assert_eq!(inline.user, "local");
        assert_eq!(inline.port, 22);
    }

    #[tokio::test]
    async fn group_reference_validation_and_deletion_protection() {
        let d = Dispatcher::new();
        let state = AppState::new();
        register_all(&d, state.clone());
        let dir = std::env::temp_dir().join(format!("inherit-refs-{}", Uuid::new_v4()));
        let profiles = ProfileStore::open(dir.join("profiles")).unwrap();
        let settings = SettingsStore::open(&dir).unwrap();
        let network = crate::network_resources::NetworkResourceStore::open(&dir).unwrap();
        *state.profiles.lock().await = Some(profiles);
        *state.settings.lock().await = Some(settings);
        *state.network_resources.lock().await = Some(network.clone());
        let id = Uuid::new_v4();
        let raw = json!({"id":id,"name":"host","group":"team","kind":"ssh","ssh":{"host":"host","port":22,"user":"local","auth":"Agent","inherit":["jump","proxy_id"]}});
        assert!(d.dispatch(req("profile.upsert", raw)).await.error.is_none());
        for value in [
            json!({"team":{"jump_profile_ids":[id]}}),
            json!({"unused":{"jump_profile_ids":[Uuid::new_v4()]}}),
            json!({"unused":{"proxy_id":Uuid::new_v4()}}),
        ] {
            assert!(d
                .dispatch(req(
                    "settings.set",
                    json!({"key":"profile.groupDefaults","value":value})
                ))
                .await
                .error
                .is_some());
        }
        let proxy_id = Uuid::new_v4();
        network
            .save_proxy(crate::network_resources::ProxyResource {
                id: proxy_id,
                name: "proxy".into(),
                config: crate::network_resources::ProxyConfig::SshJump { profile_id: id },
            })
            .unwrap();
        assert!(d
            .dispatch(req(
                "settings.set",
                json!({"key":"profile.groupDefaults","value":{"team":{"proxy_id":proxy_id}}})
            ))
            .await
            .error
            .is_some());
        assert!(d.dispatch(req("settings.set", json!({"key":"profile.groupDefaults","value":{"unused":{"jump_profile_ids":[id],"proxy_id":proxy_id}}}))).await.error.is_none());
        assert!(d
            .dispatch(req("profile.delete", json!({"id":id})))
            .await
            .error
            .is_some());
        assert!(d
            .dispatch(req("proxy.delete", json!({"id":proxy_id})))
            .await
            .error
            .is_some());
    }

    #[tokio::test]
    async fn resource_methods_use_configured_stores_and_protect_dependencies() {
        let d = Dispatcher::new();
        register_all(&d, AppState::new());
        let dir = std::env::temp_dir().join(format!("resources-rpc-{}", Uuid::new_v4()));
        let configured = d
            .dispatch(req(
                "profile.configure",
                json!({"path":dir.join("profiles.sled")}),
            ))
            .await;
        assert!(configured.error.is_none(), "{:?}", configured.error);
        let id = Uuid::new_v4();
        assert!(d.dispatch(req("profile.upsert", json!({"id":id,"name":"host","kind":"ssh","ssh":{"host":"localhost","port":22,"user":"u","auth":"Agent"}}))).await.error.is_none());
        for method in [
            "notes.list",
            "scripts.list",
            "proxy.list",
            "forwarding.list",
            "forwarding.running",
        ] {
            let result = d.dispatch(req(method, json!({}))).await;
            assert!(result.error.is_none(), "{method}: {:?}", result.error);
        }
        let note = json!({"id":"note","parent_id":null,"title":"dependent","markdown":"safe","tags":[],"profile_ids":[id],"version":0,"updated_at":0});
        let saved = d
            .dispatch(req("notes.put", json!({"note":note,"expectedVersion":0})))
            .await;
        assert!(saved.error.is_none(), "{:?}", saved.error);
        assert!(d
            .dispatch(req("profile.delete", json!({"id":id})))
            .await
            .error
            .is_some());
        assert!(d
            .dispatch(req("ssh.hostStats", json!({"profile_id":Uuid::new_v4()})))
            .await
            .error
            .unwrap()
            .message
            .contains("profile not found"));
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

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn native_engine_attach_renders_output_and_blocks_poll() {
        let d = Dispatcher::new();
        let state = AppState::new();
        register_all(&d, state.clone());
        let r = d
            .dispatch(req(
                "session.openLocal",
                json!({"title": "t", "rows": 10, "cols": 60}),
            ))
            .await;
        let id = r.result.unwrap()["id"].as_str().unwrap().to_string();

        let r = d
            .dispatch(req(
                "terminal.attach",
                json!({"id": id, "cols": 60, "rows": 10, "scrollback": 500}),
            ))
            .await;
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(r.result.unwrap()["reused"], json!(false));

        let cmd = BASE64.encode("echo native-engine-ok\n".as_bytes());
        let _ = d
            .dispatch(req("session.write", json!({"id": id, "data": cmd})))
            .await;

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut text = String::new();
        while std::time::Instant::now() < deadline {
            let r = d.dispatch(req("terminal.text", json!({"id": id}))).await;
            text = r.result.unwrap()["text"].as_str().unwrap().to_string();
            if text.contains("native-engine-ok") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(text.contains("native-engine-ok"), "viewport: {text:?}");

        let snapshot = d
            .dispatch(req("terminal.snapshotEnvelope", json!({"id": id})))
            .await;
        assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
        let snapshot = snapshot.result.unwrap();
        assert_eq!(snapshot["version"], json!(1));
        assert_eq!(snapshot["frameVersion"], json!(1));
        assert!(
            snapshot["outputSeq"]
                .as_str()
                .unwrap()
                .parse::<u64>()
                .unwrap()
                > 0
        );
        assert_eq!(snapshot["bytes"][0], json!(1));
        assert_eq!(snapshot["bytes"][1], json!(0));
        // Read-only envelope leaves the singleton attached and legacy poll blocked.
        assert!(state
            .terminal_engines
            .get(Uuid::parse_str(&id).unwrap())
            .is_some());

        // Legacy poll must not block or drain the engine's receiver.
        let r = d
            .dispatch(req("session.poll", json!({"id": id, "max_chunks": 4})))
            .await;
        let v = r.result.unwrap();
        assert_eq!(v["native"], json!(true));
        assert_eq!(v["chunks"], json!([]));

        // Search + selection round-trip through RPC.
        let r = d
            .dispatch(req(
                "terminal.search",
                json!({"id": id, "query": "engine-ok", "direction": "next"}),
            ))
            .await;
        assert!(r.result.unwrap()["hit"].is_object());
        let r = d
            .dispatch(req("terminal.selectionText", json!({"id": id})))
            .await;
        assert_eq!(r.result.unwrap()["text"], json!("engine-ok"));

        let r = d.dispatch(req("session.close", json!({"id": id}))).await;
        assert!(r.error.is_none());
        assert!(state.terminal_engines.is_empty());
    }

    #[test]
    fn import_apply_item_accepts_camel_case_source_id() {
        use serde::Deserialize;

        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ImportApplyItem {
            source_id: String,
            #[serde(default)]
            overwrite: bool,
            #[serde(default)]
            duplicate_of: Option<uuid::Uuid>,
        }

        let item: ImportApplyItem = serde_json::from_value(json!({
            "sourceId": "windterm-uuid-1",
            "overwrite": true,
            "duplicateOf": "550e8400-e29b-41d4-a716-446655440000"
        }))
        .unwrap();
        assert_eq!(item.source_id, "windterm-uuid-1");
        assert!(item.overwrite);
        assert!(item.duplicate_of.is_some());
    }

    #[test]
    fn import_apply_item_deserializes_vault_auth_and_profile() {
        use crate::profile::Profile;
        use crate::ssh::AuthMethod;

        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ImportApplyItem {
            source_id: String,
            #[serde(default)]
            overwrite: bool,
            #[serde(default)]
            user: Option<String>,
            #[serde(default)]
            auth: Option<AuthMethod>,
            #[serde(default)]
            duplicate_of: Option<uuid::Uuid>,
            #[serde(default)]
            profile: Option<Profile>,
        }

        let item: ImportApplyItem = serde_json::from_value(json!({
            "sourceId": "uuid-passbolt",
            "overwrite": true,
            "duplicateOf": "550e8400-e29b-41d4-a716-446655440000",
            "user": "devops",
            "auth": { "VaultRef": { "entry_id": "doocom-devops-privkey" } },
            "profile": {
                "schemaVersion": 1,
                "id": "11111111-1111-1111-1111-111111111111",
                "name": "Doocom-Passbolt",
                "group": "Doocom",
                "tags": [],
                "favorite": false,
                "kind": "ssh",
                "ssh": {
                    "host": "10.0.0.8",
                    "port": 22,
                    "user": "devops",
                    "auth": { "VaultRef": { "entry_id": "doocom-devops-privkey" } },
                    "jump_via": []
                }
            }
        }))
        .unwrap();
        assert_eq!(item.source_id, "uuid-passbolt");
        assert!(item.overwrite);
        assert!(item.duplicate_of.is_some());
        assert_eq!(item.user.as_deref(), Some("devops"));
        assert!(matches!(item.auth, Some(AuthMethod::VaultRef { .. })));
        assert!(item.profile.is_some());
    }
}

/// MCP uses a separate authenticated SSH exec connection; it never writes into
/// a shell or consumes session output. Dropping the channel cancels collection.
pub async fn mcp_run_command(
    state: &AppState,
    profile_id: Uuid,
    command: &str,
    timeout_ms: u64,
    max_bytes: usize,
) -> Result<Value, String> {
    if command.is_empty()
        || command.len() > 16384
        || command.contains('\0')
        || !(1..=120000).contains(&timeout_ms)
        || !(1..=1048576).contains(&max_bytes)
    {
        return Err("invalid command, timeout or output limit".into());
    }
    let profile = resolve_ssh_profile(state, profile_id)
        .await
        .map_err(|_| "unable to resolve SSH profile".to_string())?;
    let known_hosts = state.known_hosts.lock().await.clone();
    if known_hosts.is_none() {
        return Err("known-hosts store must be configured before MCP exec".into());
    }
    let transport = load_ssh_transport_settings(state).await;
    let operation = async {
        let handle = ssh::connect_authenticated(&profile, known_hosts, transport)
            .await
            .map_err(|_| "SSH connection or authentication failed".to_string())?;
        let mut channel = handle
            .channel_open_session()
            .await
            .map_err(|_| "unable to open SSH exec channel".to_string())?;
        channel
            .exec(true, command)
            .await
            .map_err(|_| "SSH exec request failed".to_string())?;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut status = None;
        let mut truncated = false;
        while let Some(message) = channel.wait().await {
            match message {
                russh::ChannelMsg::Data { ref data }
                | russh::ChannelMsg::ExtendedData { ref data, .. } => {
                    let available = max_bytes.saturating_sub(stdout.len() + stderr.len());
                    let take = available.min(data.len());
                    if matches!(message, russh::ChannelMsg::Data { .. }) {
                        stdout.extend_from_slice(&data[..take]);
                    } else {
                        stderr.extend_from_slice(&data[..take]);
                    }
                    if take < data.len() {
                        truncated = true;
                        break;
                    }
                }
                russh::ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                russh::ChannelMsg::Close => break,
                _ => {}
            }
        }
        let _ = channel.close().await;
        let _ = handle
            .disconnect(russh::Disconnect::ByApplication, "MCP exec complete", "en")
            .await;
        Ok(
            json!({"stdout": String::from_utf8_lossy(&stdout), "stderr": String::from_utf8_lossy(&stderr), "exit_status":status,"truncated":truncated}),
        )
    };
    tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), operation)
        .await
        .map_err(|_| "SSH exec timed out; remote command may still be running".to_string())?
}
