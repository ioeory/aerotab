//! Local persistence for the sync engine.
//!
//! The engine keeps its authoritative state in memory; this module is what
//! makes it survive process restarts. The default implementation uses
//! [`sled`] — a small embedded key/value store — so we don't pull in a
//! database dependency. Tests can substitute an in-memory store.
//!
//! Key encoding: `"<group-slug>/<record-uuid>"` (UTF-8).
//! Value encoding: `serde_json::to_vec(&LocalRecord)`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use super::engine::LocalRecord;
use super::{Group, RecordId, SyncError};

#[async_trait]
pub trait SyncStore: Send + Sync {
    /// Loads every persisted record. Called once on engine startup.
    async fn load_all(&self) -> Result<Vec<((Group, RecordId), LocalRecord)>, SyncError>;

    /// Persists a single record (overwriting any previous version).
    async fn upsert(
        &self,
        group: Group,
        id: RecordId,
        record: &LocalRecord,
    ) -> Result<(), SyncError>;
}

fn key_for(group: Group, id: RecordId) -> String {
    format!("{}/{}", group.as_str(), id.0)
}

fn parse_key(key: &str) -> Option<(Group, RecordId)> {
    let (g, r) = key.split_once('/')?;
    let group = Group::from_slug(g)?;
    let uuid = Uuid::parse_str(r).ok()?;
    Some((group, RecordId(uuid)))
}

fn io_err(e: std::io::Error) -> SyncError {
    SyncError::Transport(format!("io: {e}"))
}

fn sled_err(e: sled::Error) -> SyncError {
    SyncError::Transport(format!("sled: {e}"))
}

fn json_err(e: serde_json::Error) -> SyncError {
    SyncError::Crypto(format!("json: {e}"))
}

/// `sled`-backed implementation. The on-disk database lives at the path
/// supplied to [`SledStore::open`]; the directory is created on demand.
#[derive(Clone)]
pub struct SledStore {
    inner: Arc<SledInner>,
}

struct SledInner {
    db: sled::Db,
    #[allow(dead_code)]
    path: PathBuf,
}

impl SledStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SyncError> {
        let path = path.as_ref().to_path_buf();
        std::fs::create_dir_all(&path).map_err(io_err)?;
        let db = sled::open(&path).map_err(sled_err)?;
        Ok(Self {
            inner: Arc::new(SledInner { db, path }),
        })
    }
}

#[async_trait]
impl SyncStore for SledStore {
    async fn load_all(&self) -> Result<Vec<((Group, RecordId), LocalRecord)>, SyncError> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || -> Result<_, SyncError> {
            let mut out = Vec::new();
            for kv in inner.db.iter() {
                let (k, v) = kv.map_err(sled_err)?;
                let key = std::str::from_utf8(&k)
                    .map_err(|e| SyncError::Crypto(format!("key utf8: {e}")))?;
                let Some((group, id)) = parse_key(key) else {
                    continue;
                };
                let record: LocalRecord = serde_json::from_slice(&v).map_err(json_err)?;
                out.push(((group, id), record));
            }
            Ok(out)
        })
        .await
        .map_err(|e| SyncError::Transport(format!("join: {e}")))?
    }

    async fn upsert(
        &self,
        group: Group,
        id: RecordId,
        record: &LocalRecord,
    ) -> Result<(), SyncError> {
        let key = key_for(group, id);
        let value = serde_json::to_vec(record).map_err(json_err)?;
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || -> Result<(), SyncError> {
            inner
                .db
                .insert(key.as_bytes(), value.as_slice())
                .map_err(sled_err)?;
            inner.db.flush().map_err(sled_err)?;
            Ok(())
        })
        .await
        .map_err(|e| SyncError::Transport(format!("join: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::version_vector::VersionVector;
    use crate::sync::Record;

    fn tmpdir() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("tabby-sled-test-{}", Uuid::new_v4()));
        p
    }

    fn sample_record() -> LocalRecord {
        let mut vv = VersionVector::new();
        vv.bump(Uuid::nil());
        LocalRecord {
            vv,
            schema: Record::CURRENT_SCHEMA,
            updated_at_ms: 42,
            tombstone: false,
            payload: b"hi".to_vec(),
            writer: Uuid::nil(),
        }
    }

    #[tokio::test]
    async fn roundtrip_through_disk() {
        let dir = tmpdir();
        let id = RecordId(Uuid::new_v4());

        {
            let store = SledStore::open(&dir).unwrap();
            store
                .upsert(Group::Connections, id, &sample_record())
                .await
                .unwrap();
        }
        // Re-open: simulate process restart.
        let store = SledStore::open(&dir).unwrap();
        let loaded = store.load_all().await.unwrap();
        assert_eq!(loaded.len(), 1);
        let ((g, lid), rec) = &loaded[0];
        assert_eq!(*g, Group::Connections);
        assert_eq!(*lid, id);
        assert_eq!(rec.payload, b"hi");
        assert_eq!(rec.updated_at_ms, 42);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn malformed_keys_are_skipped() {
        let dir = tmpdir();
        {
            let store = SledStore::open(&dir).unwrap();
            store
                .inner
                .db
                .insert(b"not-a-group/xxxx", &b"junk"[..])
                .unwrap();
            store
                .upsert(
                    Group::Appearance,
                    RecordId(Uuid::new_v4()),
                    &sample_record(),
                )
                .await
                .unwrap();
        }
        let store = SledStore::open(&dir).unwrap();
        let loaded = store.load_all().await.unwrap();
        assert_eq!(loaded.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
