//! Config & session sync.
//!
//! - Self-hosted only; first-party backends: WebDAV and Git.
//! - End-to-end encryption via [`crypto`] (Argon2id + ChaCha20-Poly1305).
//! - Version-vector based conflict resolution.
//!
//! Full implementation lands W10-W12 (plan steps 11a, 12).

pub mod backends;
pub mod crypto;
pub mod engine;
pub mod oauth;
pub mod persistence;
pub mod version_vector;

pub use engine::{Clock, LocalRecord, SyncEngine, SyncStats, SystemClock};
pub use persistence::{SledStore, SyncStore};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use version_vector::{Ordering as VvOrdering, VersionVector};

/// Logical record stored under a [`Group`]. The payload is the AEAD
/// envelope produced by [`crypto::encrypt`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub id: RecordId,
    pub group: Group,
    pub vv: VersionVector,
    /// Unix epoch milliseconds (writer's clock — informational, not used for
    /// conflict resolution; the version vector is authoritative).
    pub updated_at_ms: i64,
    pub schema: u32,
    /// AEAD envelope bytes.
    pub payload: Vec<u8>,
}

impl Record {
    pub const CURRENT_SCHEMA: u32 = 1;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Group {
    Connections,
    Appearance,
    Shortcuts,
    PluginCfg,
    Credentials,
}

impl Group {
    /// Stable, lowercase slug used as a directory / key segment by every
    /// backend and the persistence layer. Do not rename without bumping the
    /// sync protocol version.
    pub fn as_str(self) -> &'static str {
        match self {
            Group::Connections => "connections",
            Group::Appearance => "appearance",
            Group::Shortcuts => "shortcuts",
            Group::PluginCfg => "plugincfg",
            Group::Credentials => "credentials",
        }
    }

    pub fn from_slug(s: &str) -> Option<Self> {
        Some(match s {
            "connections" => Group::Connections,
            "appearance" => Group::Appearance,
            "shortcuts" => Group::Shortcuts,
            "plugincfg" => Group::PluginCfg,
            "credentials" => Group::Credentials,
            _ => return None,
        })
    }

    pub const ALL: [Group; 5] = [
        Group::Connections,
        Group::Appearance,
        Group::Shortcuts,
        Group::PluginCfg,
        Group::Credentials,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RecordId(pub Uuid);

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("transport: {0}")]
    Transport(String),
    #[error("auth failed")]
    Auth,
    #[error("conflict on record {0:?}")]
    Conflict(RecordId),
    #[error("crypto: {0}")]
    Crypto(String),
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

#[async_trait]
pub trait SyncBackend: Send + Sync {
    async fn list(&self, group: Group) -> Result<Vec<RecordId>, SyncError>;
    async fn get(&self, group: Group, id: RecordId) -> Result<Vec<u8>, SyncError>;
    async fn put(&self, group: Group, id: RecordId, blob: &[u8]) -> Result<(), SyncError>;
    async fn delete(&self, group: Group, id: RecordId) -> Result<(), SyncError>;
}

pub async fn init() -> crate::Result<()> {
    tracing::debug!("sync::init (stub)");
    Ok(())
}
