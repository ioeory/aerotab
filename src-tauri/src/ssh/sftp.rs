//! SFTP client built on top of an established russh session, via the SFTP
//! subsystem. Wraps [`russh_sftp::client::SftpSession`] in a slim API the
//! IPC layer can call directly.
//!
//! A `SftpSession` is created off the same authenticated `client::Handle`
//! produced by [`super::connect_authenticated`] (so jump-host chains are
//! automatically honoured) — we open a fresh `channel_open_session`,
//! request the `sftp` subsystem on it, then hand the channel stream to
//! `russh-sftp`.

use std::{io::SeekFrom, time::SystemTime};

use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{FileType, OpenFlags};
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use super::known_hosts::KnownHosts;
use super::{connect_authenticated, SshError, SshProfile, SshTransportSettings, TrustingClient};

#[derive(Debug, Clone, Default)]
pub struct SftpOpenOptions {
    pub sudo: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct SftpEntry {
    pub name: String,
    pub kind: SftpKind,
    pub size: u64,
    /// Unix-style mode bits (0o644 etc.). May be zero if the server does not
    /// report them (rare in practice).
    pub mode: u32,
    /// Last-modified time in seconds since the Unix epoch, if known.
    pub mtime: Option<i64>,
}

#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
pub enum SftpKind {
    File,
    Dir,
    Symlink,
    Other,
}

impl From<FileType> for SftpKind {
    fn from(t: FileType) -> Self {
        match t {
            FileType::Dir => SftpKind::Dir,
            FileType::File => SftpKind::File,
            FileType::Symlink => SftpKind::Symlink,
            _ => SftpKind::Other,
        }
    }
}

/// An open SFTP session. Drops automatically when the handle is gone.
pub struct Sftp {
    inner: SftpSession,
    /// Standalone SFTP connections own the SSH link; sessions opened on an existing
    /// terminal reuse that shell's connection (`None` here).
    _owned_handle: Option<russh::client::Handle<TrustingClient>>,
}

/// Open the SFTP subsystem on an already-authenticated SSH handle (no extra TCP dial).
pub async fn open_subsystem_on_handle(
    handle: &russh::client::Handle<TrustingClient>,
    options: SftpOpenOptions,
) -> Result<Sftp, SshError> {
    let channel = handle
        .channel_open_session()
        .await
        .map_err(|e| SshError::Channel(e.to_string()))?;

    if options.sudo {
        let command = "sudo -n sh -c 'if command -v sftp-server >/dev/null 2>&1; then exec sftp-server; elif [ -x /usr/lib/openssh/sftp-server ]; then exec /usr/lib/openssh/sftp-server; elif [ -x /usr/lib/ssh/sftp-server ]; then exec /usr/lib/ssh/sftp-server; else echo sftp-server-not-found >&2; exit 127; fi'";
        channel
            .exec(true, command)
            .await
            .map_err(|e| SshError::Channel(format!("sudo sftp exec: {e}")))?;
    } else {
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| SshError::Channel(format!("sftp subsystem: {e}")))?;
    }

    let inner = SftpSession::new(channel.into_stream()).await.map_err(|e| {
        if options.sudo {
            SshError::Channel(format!(
                "sudo sftp session: {e}; target must allow passwordless sudo for sftp-server"
            ))
        } else {
            SshError::Channel(format!("sftp session: {e}"))
        }
    })?;
    Ok(Sftp {
        inner,
        _owned_handle: None,
    })
}

impl Sftp {
    /// Dials `profile` (honouring jump chains) and opens an SFTP session.
    pub async fn open(
        profile: &SshProfile,
        known_hosts: Option<KnownHosts>,
    ) -> Result<Self, SshError> {
        Self::open_with_options(
            profile,
            known_hosts,
            SftpOpenOptions::default(),
            SshTransportSettings::default(),
        )
        .await
    }

    pub async fn open_with_options(
        profile: &SshProfile,
        known_hosts: Option<KnownHosts>,
        options: SftpOpenOptions,
        transport: SshTransportSettings,
    ) -> Result<Self, SshError> {
        let handle = connect_authenticated(profile, known_hosts, transport).await?;
        let mut sftp = open_subsystem_on_handle(&handle, options).await?;
        sftp._owned_handle = Some(handle);
        Ok(sftp)
    }

    pub async fn read_dir(&self, path: &str) -> Result<Vec<SftpEntry>, SshError> {
        let mut out = Vec::new();
        let dir = self
            .inner
            .read_dir(path)
            .await
            .map_err(|e| SshError::Channel(format!("readdir: {e}")))?;
        for entry in dir {
            let meta = entry.metadata();
            out.push(SftpEntry {
                name: entry.file_name(),
                kind: meta.file_type().into(),
                size: meta.len(),
                mode: meta.permissions.unwrap_or(0),
                mtime: meta.mtime.map(|m| m as i64),
            });
        }
        Ok(out)
    }

    pub async fn read_file(&self, path: &str) -> Result<Vec<u8>, SshError> {
        self.inner
            .read(path)
            .await
            .map_err(|e| SshError::Channel(format!("read: {e}")))
    }

    pub async fn stat(&self, path: &str) -> Result<SftpEntry, SshError> {
        let meta = self
            .inner
            .metadata(path)
            .await
            .map_err(|e| SshError::Channel(format!("stat: {e}")))?;
        Ok(SftpEntry {
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
            kind: meta.file_type().into(),
            size: meta.len(),
            mode: meta.permissions.unwrap_or(0),
            mtime: meta.mtime.map(|m| m as i64),
        })
    }

    pub async fn read_file_chunk(
        &self,
        path: &str,
        offset: u64,
        len: u32,
    ) -> Result<Vec<u8>, SshError> {
        let mut file = self
            .inner
            .open(path)
            .await
            .map_err(|e| SshError::Channel(format!("open read: {e}")))?;
        file.seek(SeekFrom::Start(offset))
            .await
            .map_err(|e| SshError::Channel(format!("seek read: {e}")))?;
        let mut buf = vec![0; len as usize];
        let n = file
            .read(&mut buf)
            .await
            .map_err(|e| SshError::Channel(format!("read chunk: {e}")))?;
        buf.truncate(n);
        Ok(buf)
    }

    pub async fn write_file(&self, path: &str, data: &[u8]) -> Result<(), SshError> {
        self.inner
            .write(path, data)
            .await
            .map_err(|e| SshError::Channel(format!("write: {e}")))
    }

    pub async fn write_file_chunk(
        &self,
        path: &str,
        offset: u64,
        data: &[u8],
        create: bool,
    ) -> Result<(), SshError> {
        let mut file = if create {
            self.inner
                .create(path)
                .await
                .map_err(|e| SshError::Channel(format!("create write: {e}")))?
        } else {
            self.inner
                .open_with_flags(path, OpenFlags::WRITE)
                .await
                .map_err(|e| SshError::Channel(format!("open write: {e}")))?
        };
        file.seek(SeekFrom::Start(offset))
            .await
            .map_err(|e| SshError::Channel(format!("seek write: {e}")))?;
        file.write_all(data)
            .await
            .map_err(|e| SshError::Channel(format!("write chunk: {e}")))?;
        file.shutdown()
            .await
            .map_err(|e| SshError::Channel(format!("close write: {e}")))?;
        Ok(())
    }

    pub async fn mkdir(&self, path: &str) -> Result<(), SshError> {
        self.inner
            .create_dir(path)
            .await
            .map_err(|e| SshError::Channel(format!("mkdir: {e}")))
    }

    pub async fn remove_file(&self, path: &str) -> Result<(), SshError> {
        self.inner
            .remove_file(path)
            .await
            .map_err(|e| SshError::Channel(format!("rm: {e}")))
    }

    pub async fn remove_dir(&self, path: &str) -> Result<(), SshError> {
        self.inner
            .remove_dir(path)
            .await
            .map_err(|e| SshError::Channel(format!("rmdir: {e}")))
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<(), SshError> {
        self.inner
            .rename(from, to)
            .await
            .map_err(|e| SshError::Channel(format!("rename: {e}")))
    }

    pub async fn canonicalize(&self, path: &str) -> Result<String, SshError> {
        self.inner
            .canonicalize(path)
            .await
            .map_err(|e| SshError::Channel(format!("realpath: {e}")))
    }
}

#[allow(dead_code)]
fn _system_time_into_unix(t: SystemTime) -> Option<i64> {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}
