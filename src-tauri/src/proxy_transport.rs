//! Real first-hop transport. Use `client::connect_stream(cfg, dial_proxy(...).await.map_err(|e| SshError::Io(e.to_string()))?, handler)`.
//! SSH-jump resources must resolve their profile ID + Vault refs before `dial_ssh_jump`.
//! Neither this module nor resource definitions alter system proxy settings.
use crate::{
    network_resources::{validate_endpoint, ProxyConfig, ProxyResource},
    ssh::{self, known_hosts::KnownHosts, SshError, SshProfile, SshTransportSettings},
    vault::{EntryKind, VaultStore},
};
use base64::Engine;
use std::{
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use zeroize::Zeroizing;

#[derive(Clone)]
pub struct ResolvedProxy {
    pub resource: ProxyResource,
    pub vault: Option<VaultStore>,
}
impl std::fmt::Debug for ResolvedProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedProxy")
            .field("id", &self.resource.id)
            .finish_non_exhaustive()
    }
}

pub trait ProxyStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> ProxyStream for T {}
pub type BoxProxyStream = Box<dyn ProxyStream>;
fn error(message: &str) -> SshError {
    SshError::Connect(message.into())
}
struct Credentials {
    username: Zeroizing<String>,
    password: Zeroizing<String>,
}
async fn credentials(
    vault: Option<&VaultStore>,
    id: Option<&str>,
) -> Result<Option<Credentials>, SshError> {
    let Some(id) = id else { return Ok(None) };
    let mut entry = vault
        .ok_or_else(|| error("proxy Vault unavailable"))?
        .get(id)
        .await
        .map_err(|_| error("proxy Vault locked or credential missing"))?;
    let password = Zeroizing::new(std::mem::take(&mut entry.secret));
    if entry.kind != EntryKind::Password {
        return Err(error(
            "proxy authentication requires a Vault password entry",
        ));
    }
    let username = Zeroizing::new(
        entry
            .username
            .take()
            .filter(|u| !u.is_empty())
            .ok_or_else(|| error("proxy Vault entry requires username"))?,
    );
    Ok(Some(Credentials { username, password }))
}
/// SOCKS5 supports remote DNS + RFC1929 username/password. HTTP CONNECT uses Basic.
/// These are plaintext proxy transports, not HTTPS proxies: use only trusted local links
/// or a separately secured tunnel. The SSH session *inside* remains host-key verified.
pub async fn dial_proxy(
    proxy: &ProxyResource,
    target_host: &str,
    target_port: u16,
    vault: Option<&VaultStore>,
) -> Result<BoxProxyStream, SshError> {
    proxy
        .validate()
        .map_err(|_| error("invalid proxy definition"))?;
    validate_endpoint(target_host, target_port).map_err(|_| error("invalid proxy target"))?;
    tokio::time::timeout(Duration::from_secs(20), async {
        let (host, port, id, socks) = match &proxy.config {
            ProxyConfig::Socks5 {
                host,
                port,
                vault_entry_id,
            } => (host, *port, vault_entry_id.as_deref(), true),
            ProxyConfig::HttpConnect {
                host,
                port,
                vault_entry_id,
            } => (host, *port, vault_entry_id.as_deref(), false),
            ProxyConfig::SshJump { .. } => {
                return Err(error("resolve SSH jump profile and call dial_ssh_jump"))
            }
        };
        let auth = credentials(vault, id)
            .await
            .map_err(|e| SshError::Io(e.to_string()))?;
        let mut stream = TcpStream::connect((host.as_str(), port))
            .await
            .map_err(|_| error("proxy TCP connection failed"))?;
        if socks {
            socks5(&mut stream, target_host, target_port, auth.as_ref())
                .await
                .map_err(|e| SshError::Io(e.to_string()))?;
        } else {
            http_connect(&mut stream, target_host, target_port, auth.as_ref())
                .await
                .map_err(|e| SshError::Io(e.to_string()))?;
        }
        Ok(Box::new(stream) as BoxProxyStream)
    })
    .await
    .map_err(|_| error("proxy handshake timed out"))?
}
async fn socks5(
    stream: &mut TcpStream,
    host: &str,
    port: u16,
    auth: Option<&Credentials>,
) -> Result<(), SshError> {
    let method = if auth.is_some() { 2 } else { 0 };
    stream
        .write_all(&[5, 1, method])
        .await
        .map_err(|e| SshError::Io(e.to_string()))?;
    let mut selection = [0; 2];
    stream
        .read_exact(&mut selection)
        .await
        .map_err(|e| SshError::Io(e.to_string()))?;
    if selection != [5, method] {
        return Err(error("proxy SOCKS5 authentication method rejected"));
    }
    if let Some(auth) = auth {
        let (u, p) = (auth.username.as_bytes(), auth.password.as_bytes());
        if u.is_empty() || u.len() > 255 || p.is_empty() || p.len() > 255 {
            return Err(error("SOCKS5 credentials must contain 1..255 bytes"));
        }
        let mut packet = Zeroizing::new(Vec::with_capacity(3 + u.len() + p.len()));
        packet.extend_from_slice(&[1, u.len() as u8]);
        packet.extend_from_slice(u);
        packet.push(p.len() as u8);
        packet.extend_from_slice(p);
        stream
            .write_all(&packet)
            .await
            .map_err(|e| SshError::Io(e.to_string()))?;
        let mut reply = [0; 2];
        stream
            .read_exact(&mut reply)
            .await
            .map_err(|e| SshError::Io(e.to_string()))?;
        if reply != [1, 0] {
            return Err(error("proxy authentication rejected"));
        }
    }
    let mut request = vec![5, 1, 0];
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            request.push(1);
            request.extend_from_slice(&ip.octets());
        }
        Ok(std::net::IpAddr::V6(ip)) => {
            request.push(4);
            request.extend_from_slice(&ip.octets());
        }
        Err(_) => {
            request.extend_from_slice(&[3, host.len() as u8]);
            request.extend_from_slice(host.as_bytes());
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    stream
        .write_all(&request)
        .await
        .map_err(|e| SshError::Io(e.to_string()))?;
    let mut header = [0; 4];
    stream
        .read_exact(&mut header)
        .await
        .map_err(|e| SshError::Io(e.to_string()))?;
    if header[0] != 5 || header[1] != 0 || header[2] != 0 {
        return Err(error("proxy SOCKS5 CONNECT rejected"));
    }
    let len = match header[3] {
        1 => 4,
        4 => 16,
        3 => stream
            .read_u8()
            .await
            .map_err(|e| SshError::Io(e.to_string()))? as usize,
        _ => return Err(error("proxy SOCKS5 invalid reply")),
    };
    let mut bound = vec![0; len + 2];
    stream
        .read_exact(&mut bound)
        .await
        .map_err(|e| SshError::Io(e.to_string()))?;
    Ok(())
}
async fn http_connect(
    stream: &mut TcpStream,
    host: &str,
    port: u16,
    auth: Option<&Credentials>,
) -> Result<(), SshError> {
    let authority = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let mut request = Zeroizing::new(format!(
        "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n"
    ));
    if let Some(auth) = auth {
        if auth.username.contains(':') {
            return Err(error("HTTP proxy username cannot contain colon"));
        }
        let raw = Zeroizing::new(format!("{}:{}", &*auth.username, &*auth.password));
        let encoded =
            Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(raw.as_bytes()));
        request.push_str("Proxy-Authorization: Basic ");
        request.push_str(&encoded);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| SshError::Io(e.to_string()))?;
    // Byte-wise bounded header read deliberately does not consume any following SSH banner.
    let mut response = Vec::new();
    while !response.ends_with(b"\r\n\r\n") {
        if response.len() >= 16384 {
            return Err(error("HTTP proxy response header too large"));
        }
        response.push(
            stream
                .read_u8()
                .await
                .map_err(|e| SshError::Io(e.to_string()))?,
        );
    }
    let first = response.split(|b| *b == b'\n').next().unwrap_or_default();
    let line = std::str::from_utf8(first).map_err(|_| error("invalid HTTP proxy response"))?;
    let mut parts = line.split_whitespace();
    let version = parts.next();
    let code = parts.next();
    if !matches!(version, Some("HTTP/1.0" | "HTTP/1.1"))
        || code
            .and_then(|c| c.parse::<u16>().ok())
            .map_or(true, |c| !(200..300).contains(&c))
    {
        return Err(error("HTTP proxy CONNECT rejected"));
    }
    Ok(())
}
/// This uses normal shared SSH auth/host-key verification, never an unchecked socket.
/// The authenticated jump handle stays alive for the lifetime of the returned stream.
pub async fn dial_ssh_jump(
    profile: &SshProfile,
    host: &str,
    port: u16,
    known_hosts: Option<KnownHosts>,
    transport: SshTransportSettings,
) -> Result<BoxProxyStream, SshError> {
    validate_endpoint(host, port).map_err(|_| error("invalid SSH jump target"))?;
    let handle = ssh::connect_authenticated(profile, known_hosts, transport)
        .await
        .map_err(|e| SshError::Io(e.to_string()))?;
    let channel = handle
        .channel_open_direct_tcpip(host, port as u32, "127.0.0.1", 0)
        .await
        .map_err(|e| SshError::Io(e.to_string()))?;
    let (local, mut bridge) = tokio::io::duplex(65536);
    let task = tokio::spawn(async move {
        let _handle = handle;
        let mut remote = channel.into_stream();
        let _ = tokio::io::copy_bidirectional(&mut bridge, &mut remote).await;
    });
    Ok(Box::new(JumpStream {
        stream: local,
        task,
    }))
}
struct JumpStream {
    stream: tokio::io::DuplexStream,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for JumpStream {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl AsyncRead for JumpStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_read(cx, buf)
    }
}
impl AsyncWrite for JumpStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}
