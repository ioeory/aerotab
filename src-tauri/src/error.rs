//! Cross-module error type.
//!
//! All cross-module failures funnel through [`CoreError`]. IPC serializes
//! these into JSON-RPC errors with stable numeric codes (see [`crate::ipc`]).

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("ssh error: {0}")]
    Ssh(String),

    #[error("terminal error: {0}")]
    Terminal(String),

    #[error("serial error: {0}")]
    Serial(String),

    #[error("sync error: {0}")]
    Sync(String),

    #[error("plugin error: {0}")]
    Plugin(String),

    #[error("not implemented: {0}")]
    NotImplemented(&'static str),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, CoreError>;
