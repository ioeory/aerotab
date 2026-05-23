//! SSH port forwarding (`-L` / `-R` / `-D`) on top of the shared russh client.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use russh::client;
use russh::keys::key;
use russh::{Channel, Disconnect};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::known_hosts::KnownHosts;
use super::{connect_authenticated_custom, SshError, SshProfile, TrustingClient};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TunnelKind {
    Local,
    Remote,
    Dynamic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelOpenRequest {
    pub profile: SshProfile,
    pub kind: TunnelKind,
    pub bind_host: String,
    pub bind_port: u16,
    pub target_host: String,
    pub target_port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelMeta {
    pub id: Uuid,
    pub kind: TunnelKind,
    pub bind_host: String,
    pub bind_port: u16,
    pub target_host: String,
    pub target_port: u16,
    pub ssh_host: String,
    pub ssh_user: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

struct RunningTunnel {
    meta: TunnelMeta,
    shutdown_tx: watch::Sender<()>,
    #[allow(dead_code)]
    task: JoinHandle<()>,
}

pub struct TunnelManager {
    tunnels: Mutex<HashMap<Uuid, RunningTunnel>>,
}

impl Default for TunnelManager {
    fn default() -> Self {
        Self {
            tunnels: Mutex::new(HashMap::new()),
        }
    }
}

impl TunnelManager {
    pub async fn open(
        &self,
        req: TunnelOpenRequest,
        known_hosts: Option<KnownHosts>,
    ) -> Result<TunnelMeta, SshError> {
        let id = Uuid::new_v4();
        let (shutdown_tx, shutdown_rx) = watch::channel(());

        let ssh_host = req.profile.host.clone();
        let ssh_user = req.profile.user.clone();

        let mut meta = TunnelMeta {
            id,
            kind: req.kind,
            bind_host: req.bind_host.clone(),
            bind_port: req.bind_port,
            target_host: req.target_host.clone(),
            target_port: req.target_port,
            ssh_host,
            ssh_user,
            status: "running".into(),
            error: None,
        };

        let task = match req.kind {
            TunnelKind::Local => {
                spawn_local_tunnel(req, known_hosts, meta.clone(), shutdown_rx).await?
            }
            TunnelKind::Remote => {
                spawn_remote_tunnel(req, known_hosts, &mut meta, shutdown_rx).await?
            }
            TunnelKind::Dynamic => {
                spawn_dynamic_tunnel(req, known_hosts, meta.clone(), shutdown_rx).await?
            }
        };

        let running = RunningTunnel {
            meta: meta.clone(),
            shutdown_tx,
            task,
        };
        self.tunnels.lock().await.insert(id, running);
        Ok(meta)
    }

    pub async fn close(&self, id: Uuid) -> bool {
        if let Some(t) = self.tunnels.lock().await.remove(&id) {
            let _ = t.shutdown_tx.send(());
            t.task.abort();
            true
        } else {
            false
        }
    }

    pub async fn list(&self) -> Vec<TunnelMeta> {
        self.tunnels
            .lock()
            .await
            .values()
            .map(|t| t.meta.clone())
            .collect()
    }
}

/// Handler used for `-R` on the final SSH hop: bridges server-initiated forwards to a local socket.
struct ForwardingClient {
    trusting: TrustingClient,
    local_target: Option<(String, u16)>,
}

#[async_trait]
impl client::Handler for ForwardingClient {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &key::PublicKey,
    ) -> Result<bool, Self::Error> {
        self.trusting.check_server_key(server_public_key).await
    }

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<client::Msg>,
        connected_address: &str,
        connected_port: u32,
        originator_address: &str,
        originator_port: u32,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        let Some((host, port)) = self.local_target.clone() else {
            tracing::debug!(
                "ignore remote forward on jump hop ({connected_address}:{connected_port} from {originator_address}:{originator_port})"
            );
            return Ok(());
        };
        match TcpStream::connect((host.as_str(), port)).await {
            Ok(tcp) => {
                tokio::spawn(async move {
                    bridge_tcp_channel(tcp, channel).await;
                });
            }
            Err(e) => {
                tracing::warn!("tunnel local target {host}:{port}: {e}");
            }
        }
        Ok(())
    }
}

async fn bridge_tcp_channel(mut tcp: TcpStream, channel: Channel<client::Msg>) {
    let mut stream = channel.into_stream();
    let _ = tokio::io::copy_bidirectional(&mut tcp, &mut stream).await;
}

async fn bridge_direct<H>(
    handle: Arc<Mutex<client::Handle<H>>>,
    remote_host: String,
    remote_port: u16,
    tcp: TcpStream,
) where
    H: client::Handler + Send + 'static,
    H::Error: From<russh::Error> + Send + std::fmt::Debug + std::fmt::Display,
{
    let channel = {
        let h = handle.lock().await;
        h.channel_open_direct_tcpip(&remote_host, remote_port as u32, "127.0.0.1", 0)
            .await
    };
    match channel {
        Ok(ch) => bridge_tcp_channel(tcp, ch).await,
        Err(e) => tracing::warn!("tunnel direct-tcpip {remote_host}:{remote_port}: {e}"),
    }
}

fn parse_bind_addr(host: &str, port: u16) -> Result<SocketAddr, SshError> {
    let host = if host.is_empty() || host == "*" {
        "127.0.0.1"
    } else {
        host
    };
    format!("{host}:{port}")
        .parse()
        .map_err(|e| SshError::Connect(format!("bind address: {e}")))
}

async fn spawn_local_tunnel(
    req: TunnelOpenRequest,
    known_hosts: Option<KnownHosts>,
    meta: TunnelMeta,
    mut shutdown: watch::Receiver<()>,
) -> Result<JoinHandle<()>, SshError> {
    let bind = parse_bind_addr(&req.bind_host, req.bind_port)?;
    let remote_host = req.target_host.clone();
    let remote_port = req.target_port;
    let profile = req.profile;

    let kh = known_hosts.clone();
    let handle = connect_authenticated_custom(&profile, known_hosts, move |hop, _| TrustingClient {
        host_port: format!("{}:{}", hop.host, hop.port),
        known_hosts: kh.clone(),
        pinned_host_key_b64: None,
        x11_forward: false,
    })
    .await?;
    let handle = Arc::new(Mutex::new(handle));

    Ok(tokio::spawn(async move {
        let listener = match TcpListener::bind(bind).await {
            Ok(l) => l,
            Err(e) => {
                tracing::error!("tunnel local bind: {e}");
                return;
            }
        };
        loop {
            tokio::select! {
                _ = shutdown.changed() => break,
                accept = listener.accept() => {
                    let Ok((tcp, _)) = accept else { continue };
                    let h = handle.clone();
                    let rh = remote_host.clone();
                    tokio::spawn(async move {
                        bridge_direct(h, rh, remote_port, tcp).await;
                    });
                }
            }
        }
        let h = handle.lock().await;
        let _ = h
            .disconnect(Disconnect::ByApplication, "tunnel closed", "en")
            .await;
        let _ = meta;
    }))
}

async fn spawn_remote_tunnel(
    req: TunnelOpenRequest,
    known_hosts: Option<KnownHosts>,
    meta: &mut TunnelMeta,
    mut shutdown: watch::Receiver<()>,
) -> Result<JoinHandle<()>, SshError> {
    let remote_bind_host = if req.bind_host.is_empty() {
        "127.0.0.1".to_string()
    } else {
        req.bind_host.clone()
    };
    let remote_bind_port = req.bind_port;
    let local_host = if req.target_host.is_empty() {
        "127.0.0.1".to_string()
    } else {
        req.target_host.clone()
    };
    let local_port = req.target_port;
    let profile = req.profile;
    let kh = known_hosts.clone();

    let mut handle = connect_authenticated_custom(&profile, known_hosts, |hop, is_final| {
        let trusting = TrustingClient {
            host_port: format!("{}:{}", hop.host, hop.port),
            known_hosts: kh.clone(),
            pinned_host_key_b64: None,
            x11_forward: false,
        };
        ForwardingClient {
            trusting,
            local_target: if is_final {
                Some((local_host.clone(), local_port))
            } else {
                None
            },
        }
    })
    .await?;

    let bound_port = handle
        .tcpip_forward(&remote_bind_host, remote_bind_port as u32)
        .await
        .map_err(|e| SshError::Channel(format!("tcpip_forward: {e}")))?;
    if remote_bind_port == 0 {
        meta.bind_port = bound_port as u16;
    }

    let cancel_addr = remote_bind_host.clone();
    Ok(tokio::spawn(async move {
        let _ = shutdown.changed().await;
        let _ = handle
            .cancel_tcpip_forward(&cancel_addr, bound_port)
            .await;
        let _ = handle
            .disconnect(Disconnect::ByApplication, "tunnel closed", "en")
            .await;
    }))
}

async fn spawn_dynamic_tunnel(
    req: TunnelOpenRequest,
    known_hosts: Option<KnownHosts>,
    meta: TunnelMeta,
    mut shutdown: watch::Receiver<()>,
) -> Result<JoinHandle<()>, SshError> {
    let bind = parse_bind_addr(&req.bind_host, req.bind_port)?;
    let profile = req.profile;

    let kh = known_hosts.clone();
    let handle = connect_authenticated_custom(&profile, known_hosts, move |hop, _| TrustingClient {
        host_port: format!("{}:{}", hop.host, hop.port),
        known_hosts: kh.clone(),
        pinned_host_key_b64: None,
        x11_forward: false,
    })
    .await?;
    let handle = Arc::new(Mutex::new(handle));

    Ok(tokio::spawn(async move {
        let listener = match TcpListener::bind(bind).await {
            Ok(l) => l,
            Err(e) => {
                tracing::error!("tunnel dynamic bind: {e}");
                return;
            }
        };
        loop {
            tokio::select! {
                _ = shutdown.changed() => break,
                accept = listener.accept() => {
                    let Ok((tcp, _)) = accept else { continue };
                    let h = handle.clone();
                    tokio::spawn(async move {
                        if let Err(e) = socks5_relay(h, tcp).await {
                            tracing::debug!("socks5: {e}");
                        }
                    });
                }
            }
        }
        let h = handle.lock().await;
        let _ = h
            .disconnect(Disconnect::ByApplication, "tunnel closed", "en")
            .await;
        let _ = meta;
    }))
}

async fn socks5_relay<H>(
    handle: Arc<Mutex<client::Handle<H>>>,
    mut client: TcpStream,
) -> Result<(), SshError>
where
    H: client::Handler + Send + 'static,
    H::Error: From<russh::Error> + Send + std::fmt::Debug + std::fmt::Display,
{
    let mut buf = [0u8; 512];
    let n = client
        .read(&mut buf)
        .await
        .map_err(|e| SshError::Io(e.to_string()))?;
    if n < 2 || buf[0] != 5 {
        return Err(SshError::Channel("invalid socks greeting".into()));
    }
    client
        .write_all(&[5, 0])
        .await
        .map_err(|e| SshError::Io(e.to_string()))?;

    let n = client
        .read(&mut buf)
        .await
        .map_err(|e| SshError::Io(e.to_string()))?;
    if n < 7 || buf[0] != 5 || buf[1] != 1 {
        return Err(SshError::Channel("socks CONNECT required".into()));
    }
    let atyp = buf[3];
    let (host, port, consumed) = parse_socks_addr(&buf[4..n], atyp)?;
    if 4 + consumed != n {
        // ignore extra bytes for MVP
    }

    // success reply: VER REP RSV ATYP BND.ADDR BND.PORT
    client
        .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
        .map_err(|e| SshError::Io(e.to_string()))?;

    bridge_direct(handle, host, port, client).await;
    Ok(())
}

fn parse_socks_addr(buf: &[u8], atyp: u8) -> Result<(String, u16, usize), SshError> {
    match atyp {
        1 => {
            if buf.len() < 6 {
                return Err(SshError::Channel("socks ipv4 too short".into()));
            }
            let host = format!("{}.{}.{}.{}", buf[0], buf[1], buf[2], buf[3]);
            let port = u16::from_be_bytes([buf[4], buf[5]]);
            Ok((host, port, 6))
        }
        3 => {
            if buf.is_empty() {
                return Err(SshError::Channel("socks domain too short".into()));
            }
            let len = buf[0] as usize;
            if buf.len() < 1 + len + 2 {
                return Err(SshError::Channel("socks domain truncated".into()));
            }
            let host = String::from_utf8_lossy(&buf[1..1 + len]).into_owned();
            let port = u16::from_be_bytes([buf[1 + len], buf[1 + len + 1]]);
            Ok((host, port, 1 + len + 2))
        }
        4 => {
            if buf.len() < 18 {
                return Err(SshError::Channel("socks ipv6 too short".into()));
            }
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&buf[0..16]);
            let host = format!(
                "{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}",
                u16::from_be_bytes(octets[0..2].try_into().unwrap()),
                u16::from_be_bytes(octets[2..4].try_into().unwrap()),
                u16::from_be_bytes(octets[4..6].try_into().unwrap()),
                u16::from_be_bytes(octets[6..8].try_into().unwrap()),
                u16::from_be_bytes(octets[8..10].try_into().unwrap()),
                u16::from_be_bytes(octets[10..12].try_into().unwrap()),
                u16::from_be_bytes(octets[12..14].try_into().unwrap()),
                u16::from_be_bytes(octets[14..16].try_into().unwrap()),
            );
            let port = u16::from_be_bytes([buf[16], buf[17]]);
            Ok((host, port, 18))
        }
        _ => Err(SshError::Channel(format!("unsupported socks ATYP {atyp}"))),
    }
}
