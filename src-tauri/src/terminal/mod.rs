//! PTY-backed terminal channel.
//!
//! A [`PtyChannel`] owns a child process attached to a pseudo-terminal. The
//! reader thread runs blocking I/O off the async runtime and forwards bytes
//! into a [`tokio::sync::mpsc`] channel. Writes go in through an
//! [`std::io::Write`] handle protected by a [`std::sync::Mutex`].

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize as PortablePtySize};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PtySize {
    pub rows: u16,
    pub cols: u16,
}

impl Default for PtySize {
    fn default() -> Self {
        Self { rows: 24, cols: 80 }
    }
}

impl From<PtySize> for PortablePtySize {
    fn from(s: PtySize) -> Self {
        PortablePtySize {
            rows: s.rows,
            cols: s.cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("pty open: {0}")]
    Open(String),
    #[error("spawn: {0}")]
    Spawn(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("resize: {0}")]
    Resize(String),
    #[error("channel closed")]
    Closed,
}

pub struct PtyChannel {
    #[allow(dead_code)]
    master: Box<dyn MasterPty + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    output_rx: Option<mpsc::Receiver<Vec<u8>>>,
}

impl PtyChannel {
    pub fn spawn_default_shell(size: PtySize) -> Result<Self, PtyError> {
        let program = default_shell();
        let cmd = CommandBuilder::new(program);
        Self::spawn(cmd, size)
    }

    pub fn spawn(cmd: CommandBuilder, size: PtySize) -> Result<Self, PtyError> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(size.into())
            .map_err(|e| PtyError::Open(e.to_string()))?;
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| PtyError::Spawn(e.to_string()))?;
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::Open(e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| PtyError::Open(e.to_string()))?;

        let (tx, rx) = mpsc::channel::<Vec<u8>>(64);
        std::thread::Builder::new()
            .name("pty-reader".into())
            .spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if tx.blocking_send(buf[..n].to_vec()).is_err() {
                                break;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
            })
            .map_err(PtyError::Io)?;

        Ok(Self {
            master: pair.master,
            writer: Arc::new(Mutex::new(writer)),
            child,
            output_rx: Some(rx),
        })
    }

    pub fn take_output(&mut self) -> Option<mpsc::Receiver<Vec<u8>>> {
        self.output_rx.take()
    }

    pub fn write(&self, data: &[u8]) -> Result<(), PtyError> {
        let mut w = self.writer.lock().map_err(|_| PtyError::Closed)?;
        w.write_all(data)?;
        w.flush()?;
        Ok(())
    }

    pub fn resize(&self, size: PtySize) -> Result<(), PtyError> {
        self.master
            .resize(size.into())
            .map_err(|e| PtyError::Resize(e.to_string()))
    }

    pub fn kill(&mut self) -> Result<(), PtyError> {
        self.child
            .kill()
            .map_err(|e| PtyError::Spawn(e.to_string()))
    }

    pub fn try_wait(&mut self) -> Result<Option<portable_pty::ExitStatus>, PtyError> {
        self.child
            .try_wait()
            .map_err(|e| PtyError::Spawn(e.to_string()))
    }
}

#[cfg(unix)]
fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
}

#[cfg(windows)]
fn default_shell() -> String {
    std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into())
}

pub async fn init() -> crate::Result<()> {
    tracing::debug!("terminal::init");
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn spawn_echo_and_read() {
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.args(["-c", "printf hello-tabby"]);
        let mut ch = PtyChannel::spawn(cmd, PtySize::default()).unwrap();
        let mut rx = ch.take_output().unwrap();
        let mut collected = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            tokio::select! {
                v = rx.recv() => match v {
                    Some(chunk) => collected.extend_from_slice(&chunk),
                    None => break,
                },
                _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {
                    if std::time::Instant::now() >= deadline { break; }
                }
            }
        }
        let s = String::from_utf8_lossy(&collected);
        assert!(s.contains("hello-tabby"), "got {s:?}");
    }

    #[test]
    fn resize_does_not_error() {
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.args(["-c", "sleep 0.2"]);
        let ch = PtyChannel::spawn(cmd, PtySize::default()).unwrap();
        ch.resize(PtySize {
            rows: 40,
            cols: 120,
        })
        .unwrap();
    }
}
