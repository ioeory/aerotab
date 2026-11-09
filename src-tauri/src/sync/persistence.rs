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

// `async_trait` adds a bare `#[must_use]` to required methods returning boxed futures.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait SyncStore: Send + Sync {
    async fn replace_all(
        &self,
        _records: Vec<((Group, RecordId), LocalRecord)>,
    ) -> Result<(), SyncError> {
        Err(SyncError::Transport("atomic replace unsupported".into()))
    }
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

/// Read or atomically create a device identity outside the remote working tree.
/// Caller serializes configuration; malformed metadata is never silently reset.
pub fn atomic_local_write(path: &Path, bytes: &[u8]) -> Result<(), SyncError> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io_err)?;
    }
    let tmp = path.with_extension("transaction.tmp");
    let mut file = std::fs::File::create(&tmp).map_err(io_err)?;
    file.write_all(bytes).map_err(io_err)?;
    file.sync_all().map_err(io_err)?;
    std::fs::rename(tmp, path).map_err(io_err)?;
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        std::fs::File::open(dir)
            .and_then(|f| f.sync_all())
            .map_err(io_err)?;
    }
    Ok(())
}

pub fn recover_rejoin(path: &Path) -> Result<(), SyncError> {
    let marker = path.join("rejoin-pending.json");
    let bytes = match std::fs::read(&marker) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(io_err(e)),
    };
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(json_err)?;
    let id: Uuid = serde_json::from_value(value["deviceId"].clone()).map_err(json_err)?;
    atomic_local_write(
        &path.join("oplog.json"),
        &serde_json::to_vec(&value["state"]).map_err(json_err)?,
    )?;
    atomic_local_write(&path.join("device-id"), id.to_string().as_bytes())?;
    std::fs::remove_file(marker).map_err(io_err)
}

pub fn device_identity(path: &Path, requested: Option<Uuid>) -> Result<Uuid, SyncError> {
    std::fs::create_dir_all(path).map_err(io_err)?;
    recover_rejoin(path)?;
    let file = path.join("device-id");
    match std::fs::read_to_string(&file) {
        Ok(value) => {
            let id = Uuid::parse_str(value.trim()).map_err(|e| SyncError::Crypto(e.to_string()))?;
            if requested.is_some_and(|r| r != id) {
                return Err(SyncError::Transport(
                    "device id override conflicts with persisted identity".into(),
                ));
            }
            Ok(id)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let id = requested.unwrap_or_else(Uuid::new_v4);
            std::fs::write(&file, id.to_string()).map_err(io_err)?;
            Ok(id)
        }
        Err(e) => Err(io_err(e)),
    }
}

/// Local-only settings tree adapter, used when no explicit state directory exists.
/// Records and oplog progress share the same durable local database.
pub struct SettingsSyncStore(pub crate::settings::SettingsStore);

#[async_trait]
impl SyncStore for SettingsSyncStore {
    async fn replace_all(
        &self,
        records: Vec<((Group, RecordId), LocalRecord)>,
    ) -> Result<(), SyncError> {
        self.0
            .local_set(
                "sync.records",
                &serde_json::to_value(records).map_err(json_err)?,
            )
            .map_err(|e| SyncError::Transport(e.to_string()))
    }
    async fn load_all(&self) -> Result<Vec<((Group, RecordId), LocalRecord)>, SyncError> {
        let value = self
            .0
            .local_get("sync.records")
            .map_err(|e| SyncError::Transport(e.to_string()))?;
        value
            .map(|v| serde_json::from_value(v).map_err(json_err))
            .unwrap_or_else(|| Ok(Vec::new()))
    }
    async fn upsert(
        &self,
        group: Group,
        id: RecordId,
        record: &LocalRecord,
    ) -> Result<(), SyncError> {
        let mut records = self.load_all().await?;
        records.retain(|(key, _)| *key != (group, id));
        records.push(((group, id), record.clone()));
        self.0
            .local_set(
                "sync.records",
                &serde_json::to_value(records).map_err(json_err)?,
            )
            .map_err(|e| SyncError::Transport(e.to_string()))
    }
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
    async fn replace_all(
        &self,
        records: Vec<((Group, RecordId), LocalRecord)>,
    ) -> Result<(), SyncError> {
        let mut batch = sled::Batch::default();
        for kv in self.inner.db.iter() {
            let (key, _) = kv.map_err(sled_err)?;
            if std::str::from_utf8(&key).ok().and_then(parse_key).is_some() {
                batch.remove(key);
            }
        }
        for ((g, id), r) in records {
            batch.insert(
                key_for(g, id).as_bytes(),
                serde_json::to_vec(&r).map_err(json_err)?,
            );
        }
        self.inner.db.apply_batch(batch).map_err(sled_err)?;
        self.inner.db.flush().map_err(sled_err)?;
        Ok(())
    }
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

    #[test]
    fn rejoin_rollforward_recovers_after_each_identity_bundle_write() {
        for stage in 0..3 {
            let root = tmpdir();
            std::fs::create_dir_all(&root).unwrap();
            let old = Uuid::new_v4();
            let new = Uuid::new_v4();
            atomic_local_write(&root.join("device-id"), old.to_string().as_bytes()).unwrap();
            let state = serde_json::json!({"deviceId":new,"target":"test","ownSeq":0,"cursors":{},"lastPushed":[]});
            atomic_local_write(
                &root.join("rejoin-pending.json"),
                &serde_json::to_vec(&serde_json::json!({"deviceId":new,"state":state})).unwrap(),
            )
            .unwrap();
            if stage >= 1 {
                atomic_local_write(
                    &root.join("oplog.json"),
                    &serde_json::to_vec(&state).unwrap(),
                )
                .unwrap();
            }
            if stage >= 2 {
                atomic_local_write(&root.join("device-id"), new.to_string().as_bytes()).unwrap();
            }
            assert_eq!(device_identity(&root, None).unwrap(), new);
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(
                    &std::fs::read(root.join("oplog.json")).unwrap()
                )
                .unwrap()["deviceId"],
                new.to_string()
            );
            assert!(!root.join("rejoin-pending.json").exists());
            let _ = std::fs::remove_dir_all(root);
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
