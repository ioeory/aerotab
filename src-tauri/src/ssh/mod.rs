//! SSH client built on `russh`.
//!
//! Password + public-key auth, single interactive shell channel, and
//! `ProxyJump`-style multi-hop tunneling via `direct-tcpip` channels.
//! Host-key trust is backed by a persistent
//! [`KnownHosts`](known_hosts::KnownHosts) store (TOFU on first contact,
//! strict match thereafter).

pub mod ephemeral_agent;
pub mod keys;
pub mod known_hosts;
pub mod sftp;
pub mod stats;
pub mod tunnel;
pub mod vault_resolve;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rand::Rng;
use russh::client;
use russh::keys::{key, load_secret_key, PublicKeyBase64};
use russh::{Channel, ChannelId, ChannelMsg, CryptoVec, Disconnect};
use russh_keys::agent::client::AgentClient;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use known_hosts::{KnownHosts, KnownHostsError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthMethod {
    Password {
        secret: String,
    },
    PublicKey {
        key_path: PathBuf,
        passphrase: Option<String>,
    },
    Agent,
    /// Load credentials from the encrypted vault at connect time.
    VaultRef {
        entry_id: String,
        /// Optional second vault entry holding a key passphrase.
        #[serde(default)]
        passphrase_entry_id: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshProfile {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: AuthMethod,
    /// Multi-hop proxy chain. Each entry is a fully-formed SSH profile
    /// describing one bastion. The chain is dialed left-to-right and the
    /// final hop reaches the target described by the outer profile.
    /// Empty = direct dial.
    #[serde(default)]
    pub jump_via: Vec<SshProfile>,
}

#[derive(Debug, thiserror::Error)]
pub enum SshError {
    #[error("connect: {0}")]
    Connect(String),
    #[error("authentication failed")]
    Auth,
    #[error("host key mismatch: {0}")]
    HostKeyMismatch(String),
    #[error("io: {0}")]
    Io(String),
    #[error("channel: {0}")]
    Channel(String),
    #[error("agent auth: {0}")]
    Agent(String),
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

impl From<russh::Error> for SshError {
    fn from(e: russh::Error) -> Self {
        SshError::Io(e.to_string())
    }
}

impl From<KnownHostsError> for SshError {
    fn from(e: KnownHostsError) -> Self {
        match e {
            KnownHostsError::Mismatch { host, .. } => SshError::HostKeyMismatch(host),
            other => SshError::Io(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct X11ForwardOptions {
    pub enabled: bool,
    pub display: Option<String>,
}

/// Persistent host-key handler.
pub struct TrustingClient {
    host_port: String,
    known_hosts: Option<KnownHosts>,
    /// Fallback pin for ephemeral connections (no known_hosts configured):
    /// behaves as the original strict-TOFU did.
    pinned_host_key_b64: Option<String>,
    /// When true, accept inbound X11 channels and bridge to the local display.
    x11_forward: bool,
    #[cfg_attr(not(unix), allow(dead_code))]
    x11_display: Option<String>,
    /// When true, accept OpenSSH agent forwarding channels and bridge them to the local agent.
    agent_forward: bool,
    /// Optional `SSH_AUTH_SOCK` override (ephemeral agent for Direct transfer).
    agent_sock: Option<String>,
    agent_channels: HashMap<ChannelId, AgentForwardChannel>,
}

trait LocalAgentStream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T> LocalAgentStream for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

struct AgentForwardChannel {
    agent: Box<dyn LocalAgentStream>,
    pending: Vec<u8>,
}

impl TrustingClient {
    fn new(
        host_port: String,
        known_hosts: Option<KnownHosts>,
        x11_forward: bool,
        x11_display: Option<String>,
        agent_forward: bool,
        agent_sock: Option<String>,
    ) -> Self {
        Self {
            host_port,
            known_hosts,
            pinned_host_key_b64: None,
            x11_forward,
            x11_display,
            agent_forward,
            agent_sock,
            agent_channels: HashMap::new(),
        }
    }
}

#[async_trait]
impl client::Handler for TrustingClient {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &key::PublicKey,
    ) -> Result<bool, Self::Error> {
        let b64 = server_public_key.public_key_base64();
        let key_type = server_public_key.name().to_string();
        if let Some(store) = &self.known_hosts {
            return match store.verify(&self.host_port, &b64, &key_type) {
                Ok(_) => Ok(true),
                Err(KnownHostsError::Mismatch { .. }) => Ok(false),
                Err(_) => Err(russh::Error::Disconnect),
            };
        }
        // Process-local TOFU fallback.
        match &self.pinned_host_key_b64 {
            Some(pinned) => Ok(pinned == &b64),
            None => {
                self.pinned_host_key_b64 = Some(b64);
                Ok(true)
            }
        }
    }

    async fn server_channel_open_x11(
        &mut self,
        channel: Channel<client::Msg>,
        _originator_address: &str,
        _originator_port: u32,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        if !self.x11_forward {
            return Ok(());
        }
        #[cfg(unix)]
        {
            if let Some(path) = resolve_x11_socket_path(self.x11_display.as_deref()) {
                tokio::spawn(async move {
                    match tokio::net::UnixStream::connect(path).await {
                        Ok(mut unix) => {
                            let mut stream = channel.into_stream();
                            let _ = tokio::io::copy_bidirectional(&mut unix, &mut stream).await;
                        }
                        Err(e) => tracing::warn!("x11 local socket: {e}"),
                    }
                });
            }
        }
        #[cfg(not(unix))]
        {
            let _ = channel;
        }
        Ok(())
    }

    async fn server_channel_open_agent_forward(
        &mut self,
        channel: ChannelId,
        session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        if !self.agent_forward {
            session.close(channel);
            return Ok(());
        }
        match connect_local_agent_stream(self.agent_sock.as_deref()).await {
            Ok(agent) => {
                self.agent_channels.insert(
                    channel,
                    AgentForwardChannel {
                        agent,
                        pending: Vec::new(),
                    },
                );
            }
            Err(e) => {
                tracing::warn!("ssh agent forwarding local agent: {e}");
                session.close(channel);
            }
        }
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        let Some(forward) = self.agent_channels.get_mut(&channel) else {
            return Ok(());
        };
        forward.pending.extend_from_slice(data);
        while let Some(packet_len) = pending_agent_packet_len(&forward.pending) {
            if packet_len > MAX_AGENT_PACKET_SIZE {
                tracing::warn!("ssh agent forwarding packet too large: {packet_len}");
                self.agent_channels.remove(&channel);
                session.close(channel);
                return Ok(());
            }
            if forward.pending.len() < packet_len {
                break;
            }
            let packet = pop_agent_packet(&mut forward.pending).expect("length already checked");
            if let Err(e) = forward.agent.write_all(&packet).await {
                tracing::warn!("ssh agent forwarding write: {e}");
                self.agent_channels.remove(&channel);
                session.close(channel);
                return Ok(());
            }
            if let Err(e) = forward.agent.flush().await {
                tracing::warn!("ssh agent forwarding flush: {e}");
                self.agent_channels.remove(&channel);
                session.close(channel);
                return Ok(());
            }
            match read_agent_packet(forward.agent.as_mut()).await {
                Ok(response) => session.data(channel, CryptoVec::from_slice(&response)),
                Err(e) => {
                    tracing::warn!("ssh agent forwarding read: {e}");
                    self.agent_channels.remove(&channel);
                    session.close(channel);
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        self.agent_channels.remove(&channel);
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        self.agent_channels.remove(&channel);
        Ok(())
    }
}

#[cfg(unix)]
pub fn resolve_x11_socket_path(display_override: Option<&str>) -> Option<String> {
    let display = display_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| std::env::var("DISPLAY").ok().filter(|s| !s.is_empty()))?;
    let path = parse_x11_unix_socket(&display)?;
    if std::path::Path::new(&path).exists() {
        Some(path)
    } else {
        None
    }
}

#[cfg(not(unix))]
pub fn resolve_x11_socket_path(_display_override: Option<&str>) -> Option<String> {
    None
}

#[cfg(unix)]
fn parse_x11_unix_socket(display: &str) -> Option<String> {
    let trimmed = display.trim();
    let colon_idx = trimmed.find(':')?;
    let num_part = &trimmed[colon_idx + 1..];
    let num: u32 = num_part.split('.').next()?.parse().ok()?;
    Some(format!("/tmp/.X11-unix/X{num}"))
}

/// Returns an error when X11 forwarding is enabled but no local X socket is reachable.
pub fn validate_x11_forward(opts: &X11ForwardOptions) -> Result<(), String> {
    if !opts.enabled {
        return Ok(());
    }
    if resolve_x11_socket_path(opts.display.as_deref()).is_some() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        let wayland = std::env::var("WAYLAND_DISPLAY")
            .ok()
            .filter(|s| !s.is_empty())
            .is_some();
        if wayland {
            return Err(
            "X11 forwarding is enabled but no local X display is available. Install and run XWayland (e.g. xorg-xwayland on Arch) or set x11Display in Settings → SSH.".into(),
        );
        }
        Err(
        "X11 forwarding is enabled but DISPLAY is unset or the X socket is missing. Set x11Display in Settings → SSH or start your X server.".into(),
    )
    }
    #[cfg(not(unix))]
    {
        let _ = opts;
        Err("X11 forwarding is only supported on Unix.".into())
    }
}

#[cfg(unix)]
#[allow(dead_code)]
fn local_x11_socket_path() -> Option<String> {
    resolve_x11_socket_path(None)
}

const MAX_AGENT_PACKET_SIZE: usize = 256 * 1024;

fn pending_agent_packet_len(pending: &[u8]) -> Option<usize> {
    if pending.len() < 4 {
        return None;
    }
    let body_len = u32::from_be_bytes([pending[0], pending[1], pending[2], pending[3]]) as usize;
    body_len.checked_add(4)
}

fn pop_agent_packet(pending: &mut Vec<u8>) -> Option<Vec<u8>> {
    let packet_len = pending_agent_packet_len(pending)?;
    if pending.len() < packet_len {
        return None;
    }
    Some(pending.drain(..packet_len).collect())
}

async fn read_agent_packet(agent: &mut dyn LocalAgentStream) -> std::io::Result<Vec<u8>> {
    let mut header = [0_u8; 4];
    agent.read_exact(&mut header).await?;
    let body_len = u32::from_be_bytes(header) as usize;
    let packet_len = body_len.checked_add(4).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "agent packet overflow")
    })?;
    if packet_len > MAX_AGENT_PACKET_SIZE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "agent packet too large",
        ));
    }
    let mut packet = Vec::with_capacity(packet_len);
    packet.extend_from_slice(&header);
    packet.resize(packet_len, 0);
    agent.read_exact(&mut packet[4..]).await?;
    Ok(packet)
}

async fn connect_local_agent_stream(
    sock_override: Option<&str>,
) -> std::io::Result<Box<dyn LocalAgentStream>> {
    #[cfg(unix)]
    {
        let path = match sock_override {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => std::env::var("SSH_AUTH_SOCK").map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "SSH_AUTH_SOCK is not set")
            })?,
        };
        let stream = tokio::net::UnixStream::connect(path).await?;
        Ok(Box::new(stream))
    }

    #[cfg(windows)]
    {
        let pipe = match sock_override {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => std::env::var("SSH_AUTH_SOCK")
                .unwrap_or_else(|_| r"\\.\pipe\openssh-ssh-agent".to_string()),
        };
        let stream = tokio::net::windows::named_pipe::ClientOptions::new().open(pipe)?;
        Ok(Box::new(stream))
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = sock_override;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "system ssh-agent is unavailable on this platform",
        ))
    }
}

fn random_x11_cookie() -> String {
    let mut rng = rand::thread_rng();
    (0..16)
        .map(|_| format!("{:02x}", rng.gen::<u8>()))
        .collect()
}

/// Operations the writer side can send into the forwarder task.
enum ShellOp {
    Data(Vec<u8>),
    Resize { cols: u32, rows: u32 },
    Close,
}

/// A live SSH shell. Mirrors the shape of [`crate::terminal::PtyChannel`].
#[allow(dead_code)]
pub struct SshShell {
    ops_tx: mpsc::Sender<ShellOp>,
    output_rx: Option<mpsc::Receiver<Vec<u8>>>,
    /// Held only so the connection isn't dropped while the shell lives.
    _handle: client::Handle<TrustingClient>,
}

impl SshShell {
    pub fn take_output(&mut self) -> Option<mpsc::Receiver<Vec<u8>>> {
        self.output_rx.take()
    }

    pub async fn write(&self, data: &[u8]) -> Result<(), SshError> {
        self.ops_tx
            .send(ShellOp::Data(data.to_vec()))
            .await
            .map_err(|_| SshError::Channel("forwarder closed".into()))
    }

    pub async fn resize(&self, cols: u32, rows: u32) -> Result<(), SshError> {
        self.ops_tx
            .send(ShellOp::Resize { cols, rows })
            .await
            .map_err(|_| SshError::Channel("forwarder closed".into()))
    }

    pub async fn close(self) -> Result<(), SshError> {
        let _ = self.ops_tx.send(ShellOp::Close).await;
        self._handle
            .disconnect(Disconnect::ByApplication, "bye", "en")
            .await
            .map_err(SshError::from)
    }

    /// Opens SFTP on the same SSH connection as this shell (no second `connect_authenticated`).
    pub async fn open_sftp(
        &self,
        options: crate::ssh::sftp::SftpOpenOptions,
    ) -> Result<crate::ssh::sftp::Sftp, SshError> {
        crate::ssh::sftp::open_subsystem_on_handle(&self._handle, options).await
    }
}

/// Connects to `profile.host:port`, authenticates, and opens an interactive
/// shell channel with the given PTY size.
pub async fn connect_shell(
    profile: &SshProfile,
    cols: u32,
    rows: u32,
) -> Result<SshShell, SshError> {
    connect_shell_with_known_hosts(
        profile,
        cols,
        rows,
        None,
        None,
        SshTransportSettings::default(),
    )
    .await
}

/// TCP/SSH timing applied to every new SSH dial (terminal, SFTP, tunnels).
#[derive(Debug, Clone)]
pub struct SshTransportSettings {
    /// Client keepalive interval when no data is received from the server.
    pub keepalive_interval: Duration,
    /// Close after this many unanswered keepalives.
    pub keepalive_max: usize,
    /// Drop the connection after this long with no inbound traffic at all.
    pub inactivity_timeout: Duration,
}

impl Default for SshTransportSettings {
    fn default() -> Self {
        Self {
            // Tolerant defaults for high-latency / NAT links.
            keepalive_interval: Duration::from_secs(45),
            keepalive_max: 10,
            inactivity_timeout: Duration::from_secs(60 * 60 * 4),
        }
    }
}

pub fn ssh_config(transport: &SshTransportSettings) -> Arc<client::Config> {
    Arc::new(client::Config {
        inactivity_timeout: Some(transport.inactivity_timeout),
        keepalive_interval: Some(transport.keepalive_interval),
        keepalive_max: transport.keepalive_max,
        ..Default::default()
    })
}

async fn authenticate_custom<H>(
    handle: &mut client::Handle<H>,
    profile: &SshProfile,
) -> Result<(), SshError>
where
    H: client::Handler,
    H::Error: From<russh::Error>,
{
    let authed = match &profile.auth {
        AuthMethod::Password { secret } => handle
            .authenticate_password(&profile.user, secret)
            .await
            .map_err(SshError::from)?,
        AuthMethod::PublicKey {
            key_path,
            passphrase,
        } => {
            authenticate_public_key_with_fallback(handle, profile, key_path, passphrase.as_deref())
                .await?
        }
        AuthMethod::Agent => match authenticate_agent_generic(handle, profile).await {
            Ok(authed) => authed,
            Err(SshError::Agent(e)) => {
                tracing::debug!("ssh agent unavailable ({e}); trying default private keys");
                authenticate_default_public_keys(handle, &profile.user).await?
            }
            Err(e) => return Err(e),
        },
        AuthMethod::VaultRef { .. } => {
            return Err(SshError::Connect(
                "vault auth was not resolved before connect".into(),
            ));
        }
    };
    if !authed {
        Err(SshError::Auth)
    } else {
        Ok(())
    }
}

async fn authenticate_public_key_with_fallback<H>(
    handle: &mut client::Handle<H>,
    profile: &SshProfile,
    key_path: &std::path::Path,
    passphrase: Option<&str>,
) -> Result<bool, SshError>
where
    H: client::Handler,
    H::Error: From<russh::Error>,
{
    if key_path.is_file() {
        match load_secret_key(key_path, passphrase) {
            Ok(key) => {
                match handle
                    .authenticate_publickey(&profile.user, Arc::new(key))
                    .await
                    .map_err(SshError::from)
                {
                    Ok(true) => return Ok(true),
                    Ok(false) => {}
                    Err(e) => {
                        tracing::debug!(
                            "configured key {} failed: {e}; trying fallbacks",
                            key_path.display()
                        );
                    }
                }
            }
            Err(e) => {
                tracing::debug!(
                    "load configured key {}: {e}; trying fallbacks",
                    key_path.display()
                );
            }
        }
    } else {
        tracing::debug!(
            "configured key missing: {}; trying default keys and ssh-agent",
            key_path.display()
        );
    }

    match authenticate_default_public_keys(handle, &profile.user).await {
        Ok(true) => return Ok(true),
        Ok(false) => {}
        Err(SshError::Agent(e)) => {
            tracing::debug!("default keys failed: {e}; trying ssh-agent");
        }
        Err(e) => return Err(e),
    }

    authenticate_agent_generic(handle, profile).await
}

async fn authenticate_default_public_keys<H>(
    handle: &mut client::Handle<H>,
    user: &str,
) -> Result<bool, SshError>
where
    H: client::Handler,
    H::Error: From<russh::Error>,
{
    let mut last_err: Option<String> = None;
    for key_path in keys::default_ssh_key_candidates() {
        if !key_path.is_file() {
            continue;
        }
        let key = match load_secret_key(&key_path, None) {
            Ok(k) => k,
            Err(e) => {
                last_err = Some(format!("load {}: {e}", key_path.display()));
                continue;
            }
        };
        match handle
            .authenticate_publickey(user, Arc::new(key))
            .await
            .map_err(SshError::from)
        {
            Ok(true) => return Ok(true),
            Ok(false) => {}
            Err(e) => last_err = Some(e.to_string()),
        }
    }
    Err(SshError::Agent(last_err.unwrap_or_else(|| {
        "connect openssh agent failed and no default ~/.ssh private keys worked".into()
    })))
}

async fn authenticate_agent_generic<H>(
    handle: &mut client::Handle<H>,
    profile: &SshProfile,
) -> Result<bool, SshError>
where
    H: client::Handler,
    H::Error: From<russh::Error>,
{
    #[cfg(unix)]
    {
        let agent = AgentClient::connect_env()
            .await
            .map_err(|e| SshError::Agent(e.to_string()))?;
        return authenticate_agent_client_generic(handle, &profile.user, agent).await;
    }

    #[cfg(windows)]
    {
        let pipe = std::env::var("SSH_AUTH_SOCK")
            .unwrap_or_else(|_| r"\\.\pipe\openssh-ssh-agent".to_string());
        let stream = tokio::net::windows::named_pipe::ClientOptions::new()
            .open(pipe)
            .map_err(|e| SshError::Agent(format!("connect openssh agent: {e}")))?;
        let agent = AgentClient::connect(stream);
        return authenticate_agent_client_generic(handle, &profile.user, agent).await;
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = handle;
        let _ = profile;
        Err(SshError::Agent(
            "system ssh-agent is unavailable on this platform".into(),
        ))
    }
}

async fn authenticate_agent_client_generic<H, R>(
    handle: &mut client::Handle<H>,
    user: &str,
    mut agent: AgentClient<R>,
) -> Result<bool, SshError>
where
    H: client::Handler,
    H::Error: From<russh::Error>,
    R: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let identities = agent
        .request_identities()
        .await
        .map_err(|e| SshError::Agent(format!("list identities: {e}")))?;
    if identities.is_empty() {
        return Err(SshError::Agent("agent has no identities".into()));
    }

    let mut last_error = None;
    for identity in identities {
        let (next_agent, result) = handle
            .authenticate_future(user.to_string(), identity, agent)
            .await;
        agent = next_agent;
        match result {
            Ok(true) => return Ok(true),
            Ok(false) => {}
            Err(e) => last_error = Some(e.to_string()),
        }
    }

    if let Some(error) = last_error {
        Err(SshError::Agent(format!("signing failed: {error}")))
    } else {
        Ok(false)
    }
}

/// Dials and authenticates against `profile`, walking `jump_via` left-to-right
/// and tunnelling each subsequent hop through a `direct-tcpip` channel on the
/// previous hop. Returns the final authenticated session handle.
///
/// `known_hosts` is consulted for every hop (each hop's `host:port` is keyed
/// independently).
/// Dials and authenticates against `profile`, walking `jump_via` left-to-right.
/// `make_handler` is invoked per hop; `is_final` is true on the last hop.
pub async fn connect_authenticated_custom<H>(
    profile: &SshProfile,
    _known_hosts: Option<KnownHosts>,
    transport: SshTransportSettings,
    mut make_handler: impl FnMut(&SshProfile, bool) -> H,
) -> Result<client::Handle<H>, SshError>
where
    H: client::Handler + Send + 'static,
    H::Error: From<russh::Error> + Send + std::fmt::Debug + std::fmt::Display,
{
    let mut chain: Vec<&SshProfile> = profile.jump_via.iter().collect();
    chain.push(profile);
    let final_idx = chain.len().saturating_sub(1);

    let mut prev_handle: Option<client::Handle<H>> = None;
    for (idx, hop) in chain.into_iter().enumerate() {
        let is_final = idx == final_idx;
        let handler = make_handler(hop, is_final);
        let cfg = ssh_config(&transport);
        let mut handle = match prev_handle.take() {
            None => client::connect(cfg, (hop.host.as_str(), hop.port), handler)
                .await
                .map_err(|e| SshError::Connect(e.to_string()))?,
            Some(prev) => {
                let channel = prev
                    .channel_open_direct_tcpip(&hop.host, hop.port as u32, "127.0.0.1", 0)
                    .await
                    .map_err(|e| SshError::Channel(format!("jump tcpip: {e}")))?;
                let stream = channel.into_stream();
                client::connect_stream(cfg, stream, handler)
                    .await
                    .map_err(|e| SshError::Connect(format!("jump connect: {e}")))?
            }
        };
        authenticate_custom(&mut handle, hop).await?;
        prev_handle = Some(handle);
    }

    prev_handle.ok_or_else(|| SshError::Connect("empty connection chain".into()))
}

pub async fn connect_authenticated(
    profile: &SshProfile,
    known_hosts: Option<KnownHosts>,
    transport: SshTransportSettings,
) -> Result<client::Handle<TrustingClient>, SshError> {
    let kh = known_hosts.clone();
    connect_authenticated_custom(profile, known_hosts, transport, move |hop, _| {
        TrustingClient::new(
            format!("{}:{}", hop.host, hop.port),
            kh.clone(),
            false,
            None,
            false,
            None,
        )
    })
    .await
}

pub async fn connect_authenticated_with_agent_forwarding(
    profile: &SshProfile,
    known_hosts: Option<KnownHosts>,
    transport: SshTransportSettings,
) -> Result<client::Handle<TrustingClient>, SshError> {
    connect_authenticated_with_agent_forwarding_sock(profile, known_hosts, transport, None).await
}

/// Like [`connect_authenticated_with_agent_forwarding`], but bridges forwarded
/// agent channels to `agent_sock` (ephemeral agent) when set.
pub async fn connect_authenticated_with_agent_forwarding_sock(
    profile: &SshProfile,
    known_hosts: Option<KnownHosts>,
    transport: SshTransportSettings,
    agent_sock: Option<String>,
) -> Result<client::Handle<TrustingClient>, SshError> {
    let kh = known_hosts.clone();
    let sock = agent_sock.clone();
    connect_authenticated_custom(profile, known_hosts, transport, move |hop, is_final| {
        TrustingClient::new(
            format!("{}:{}", hop.host, hop.port),
            kh.clone(),
            false,
            None,
            is_final,
            if is_final { sock.clone() } else { None },
        )
    })
    .await
}

/// Same as [`connect_shell`] but with a persistent host-key store.
pub async fn connect_shell_with_known_hosts(
    profile: &SshProfile,
    cols: u32,
    rows: u32,
    known_hosts: Option<KnownHosts>,
    x11: Option<X11ForwardOptions>,
    transport: SshTransportSettings,
) -> Result<SshShell, SshError> {
    let x11_enabled = x11.as_ref().is_some_and(|o| o.enabled);
    let x11_display = x11
        .as_ref()
        .and_then(|o| if o.enabled { o.display.clone() } else { None });
    let kh = known_hosts.clone();
    let handle =
        connect_authenticated_custom(profile, known_hosts, transport, move |hop, is_final| {
            TrustingClient::new(
                format!("{}:{}", hop.host, hop.port),
                kh.clone(),
                is_final && x11_enabled,
                if is_final { x11_display.clone() } else { None },
                false,
                None,
            )
        })
        .await?;

    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|e| SshError::Channel(e.to_string()))?;
    channel
        .request_pty(true, "xterm-256color", cols, rows, 0, 0, &[])
        .await
        .map_err(|e| SshError::Channel(e.to_string()))?;
    if x11_enabled {
        let cookie = random_x11_cookie();
        channel
            .request_x11(true, false, "MIT-MAGIC-COOKIE-1", &cookie, 0)
            .await
            .map_err(|e| SshError::Channel(format!("x11 forward: {e}")))?;
    }
    channel
        .request_shell(true)
        .await
        .map_err(|e| SshError::Channel(e.to_string()))?;

    let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>(64);
    let (ops_tx, mut ops_rx) = mpsc::channel::<ShellOp>(64);

    tokio::spawn(async move {
        loop {
            tokio::select! {
                op = ops_rx.recv() => match op {
                    Some(ShellOp::Data(d)) => {
                        if channel.data(&d[..]).await.is_err() { break; }
                    }
                    Some(ShellOp::Resize { cols, rows }) => {
                        let _ = channel.window_change(cols, rows, 0, 0).await;
                    }
                    Some(ShellOp::Close) | None => {
                        let _ = channel.eof().await;
                        break;
                    }
                },
                msg = channel.wait() => match msg {
                    Some(ChannelMsg::Data { ref data })
                        | Some(ChannelMsg::ExtendedData { ref data, ext: _ })
                        if out_tx.send(data.to_vec()).await.is_err() => break,
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => break,
                    _ => {}
                }
            }
        }
    });

    Ok(SshShell {
        ops_tx,
        output_rx: Some(out_rx),
        _handle: handle,
    })
}

pub async fn init() -> crate::Result<()> {
    tracing::debug!("ssh::init");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_packet_parser_extracts_complete_packets_and_keeps_remainder() {
        let mut pending = Vec::new();
        pending.extend_from_slice(&[0, 0, 0, 2, 11, 22, 0, 0, 0, 3, 33]);

        let packet = pop_agent_packet(&mut pending).expect("first packet should be complete");

        assert_eq!(packet, vec![0, 0, 0, 2, 11, 22]);
        assert_eq!(pending, vec![0, 0, 0, 3, 33]);
        assert!(pop_agent_packet(&mut pending).is_none());
    }

    #[tokio::test]
    async fn agent_auth_connects_before_auth() {
        // Use a port no SSH server is listening on so we never actually dial.
        // jump_via is empty, so the function will proceed to the connect step
        // and we expect a Connect error rather than an agent error (since
        // the auth method check happens after connecting).
        let p = SshProfile {
            host: "127.0.0.1".into(),
            port: 1, // reserved, will fail connect
            user: "x".into(),
            auth: AuthMethod::Agent,
            jump_via: vec![],
        };
        let err = match connect_shell(&p, 80, 24).await {
            Ok(_) => panic!("unexpected ok"),
            Err(e) => e,
        };
        assert!(matches!(err, SshError::Connect(_)), "got {err:?}");
    }

    #[cfg(unix)]
    #[test]
    fn parse_x11_unix_socket_local_display() {
        assert_eq!(
            super::parse_x11_unix_socket(":0").as_deref(),
            Some("/tmp/.X11-unix/X0")
        );
        assert_eq!(
            super::parse_x11_unix_socket(":1.0").as_deref(),
            Some("/tmp/.X11-unix/X1")
        );
        assert!(super::parse_x11_unix_socket("").is_none());
    }

    #[test]
    fn validate_x11_forward_skips_when_disabled() {
        let opts = X11ForwardOptions::default();
        assert!(super::validate_x11_forward(&opts).is_ok());
    }
}
