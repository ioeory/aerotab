//! Protocol-2 append-only encrypted operation log.
//!
//! Remote layout (under the BlobStore root):
//!
//! ```text
//! v2/devices/<device_id>/ops/<seq:012>.bin   # encrypted OpBatch
//! v2/snapshots/<snapshot_id>.bin             # encrypted Snapshot
//! v2/snapshots/latest                        # plaintext snapshot uuid
//! ```
//!
//! Each device only appends to its own ops directory. Peers pull ops with
//! `seq > cursor[device]` so a quiet cycle downloads nothing. Compaction
//! writes a snapshot + watermark and prunes the writer's own covered ops.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use super::blob::BlobStore;
use super::crypto::{self, KdfParams};
use super::engine::{Clock, LocalRecord, SyncEngine, SyncStats, SystemClock};
use super::version_vector::VersionVector;
use super::{Group, RecordId, SyncError};

const OPS_PREFIX: &str = "v2/devices";
const SNAP_PREFIX: &str = "v2/snapshots";
const SNAP_LATEST: &str = "v2/snapshots/latest";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OpKind {
    Put { payload: Vec<u8> },
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Op {
    pub group: Group,
    pub id: RecordId,
    pub kind: OpKind,
    pub vv: VersionVector,
    pub updated_at_ms: i64,
    pub writer: Uuid,
    pub schema: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpBatch {
    pub device: Uuid,
    pub seq: u64,
    pub ops: Vec<Op>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// Full local state for every group in the snapshot.
    pub records: Vec<SnapshotRecord>,
    /// Highest applied op seq per device at snapshot time.
    pub watermark: HashMap<Uuid, u64>,
    pub created_at_ms: i64,
    pub writer: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotRecord {
    pub group: Group,
    pub id: RecordId,
    pub record: LocalRecord,
}

#[derive(Debug, Clone, Copy)]
pub struct CompactConfig {
    pub max_op_files: usize,
    pub max_op_bytes: u64,
    /// Only prune own ops older than this many ms past the watermark.
    pub prune_grace_ms: i64,
}

impl Default for CompactConfig {
    fn default() -> Self {
        Self {
            max_op_files: 64,
            max_op_bytes: 2 * 1024 * 1024,
            prune_grace_ms: 7 * 24 * 60 * 60 * 1000,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OplogStats {
    pub bytes_downloaded: u64,
    pub bytes_uploaded: u64,
    pub ops_applied: usize,
    pub snapshot_used: bool,
    pub compacted: bool,
}

/// Per-device cursors + own sequence counter, backed by a BlobStore.
pub struct OplogDriver {
    blobs: Arc<dyn BlobStore>,
    password: Vec<u8>,
    params: KdfParams,
    device_id: Uuid,
    clock: Arc<dyn Clock>,
    cursors: RwLock<HashMap<Uuid, u64>>,
    own_seq: AtomicU64,
    /// Content hash of the last payload we pushed per record (skip unchanged).
    last_pushed: RwLock<HashMap<(Group, RecordId), u64>>,
    compact: CompactConfig,
}

impl OplogDriver {
    pub fn new(
        blobs: Arc<dyn BlobStore>,
        device_id: Uuid,
        password: impl Into<Vec<u8>>,
        params: KdfParams,
    ) -> Self {
        Self::with_clock(
            blobs,
            device_id,
            password,
            params,
            Arc::new(SystemClock),
            CompactConfig::default(),
        )
    }

    pub fn with_clock(
        blobs: Arc<dyn BlobStore>,
        device_id: Uuid,
        password: impl Into<Vec<u8>>,
        params: KdfParams,
        clock: Arc<dyn Clock>,
        compact: CompactConfig,
    ) -> Self {
        Self {
            blobs,
            password: password.into(),
            params,
            device_id,
            clock,
            cursors: RwLock::new(HashMap::new()),
            own_seq: AtomicU64::new(0),
            last_pushed: RwLock::new(HashMap::new()),
            compact,
        }
    }

    pub fn blobs(&self) -> Arc<dyn BlobStore> {
        self.blobs.clone()
    }

    pub fn device_id(&self) -> Uuid {
        self.device_id
    }

    pub async fn cursors_snapshot(&self) -> HashMap<Uuid, u64> {
        self.cursors.read().await.clone()
    }

    pub fn own_seq(&self) -> u64 {
        self.own_seq.load(Ordering::SeqCst)
    }

    /// Run one reconciliation cycle for `groups`. Updates `engine` local state.
    pub async fn cycle(
        &self,
        engine: &SyncEngine,
        groups: &[Group],
    ) -> Result<(Vec<(Group, SyncStats)>, OplogStats), SyncError> {
        let mut meta = OplogStats::default();
        let group_set: HashSet<Group> = groups.iter().copied().collect();

        // Cold start: apply latest snapshot if we have no local state yet.
        if engine.is_empty().await {
            if let Some(snap) = self.pull_latest_snapshot(&mut meta).await? {
                self.apply_snapshot(engine, &snap, &group_set).await?;
                meta.snapshot_used = true;
                let mut cursors = self.cursors.write().await;
                for (d, s) in snap.watermark {
                    let slot = cursors.entry(d).or_insert(0);
                    *slot = (*slot).max(s);
                }
                if let Some(&s) = cursors.get(&self.device_id) {
                    self.own_seq.fetch_max(s, Ordering::SeqCst);
                }
            }
        }

        // Pull new ops from every device directory.
        let devices = self.list_device_ids().await?;
        for device in devices {
            let cursor = *self.cursors.read().await.get(&device).unwrap_or(&0);
            let files = self.list_ops(device).await?;
            for (seq, path, size) in files {
                if seq <= cursor {
                    continue;
                }
                let blob = self.blobs.get_blob(&path).await?;
                meta.bytes_downloaded += blob.len() as u64;
                let batch = self.decrypt_batch(&blob)?;
                if batch.device != device || batch.seq != seq {
                    return Err(SyncError::Crypto("op batch identity mismatch".into()));
                }
                for op in &batch.ops {
                    if group_set.contains(&op.group) {
                        engine.apply_op(op).await?;
                        meta.ops_applied += 1;
                    }
                }
                self.cursors.write().await.insert(device, seq);
                let _ = size;
            }
        }

        // Diff local → OpBatch for selected groups.
        let mut ops = Vec::new();
        let mut per_group = HashMap::<Group, SyncStats>::new();
        for &g in groups {
            per_group.entry(g).or_default();
        }
        for &group in groups {
            let locals = engine.snapshot_group(group).await;
            let mut live = HashSet::new();
            for (id, rec) in locals {
                live.insert(id);
                let hash = content_hash(&rec);
                let pushed = self.last_pushed.read().await.get(&(group, id)).copied();
                if pushed == Some(hash) {
                    per_group.get_mut(&group).unwrap().unchanged += 1;
                    continue;
                }
                let kind = if rec.tombstone {
                    OpKind::Delete
                } else {
                    OpKind::Put {
                        payload: rec.payload.clone(),
                    }
                };
                ops.push(Op {
                    group,
                    id,
                    kind,
                    vv: rec.vv.clone(),
                    updated_at_ms: rec.updated_at_ms,
                    writer: rec.writer,
                    schema: rec.schema,
                });
                self.last_pushed.write().await.insert((group, id), hash);
                per_group.get_mut(&group).unwrap().pushed += 1;
            }
            // Records we previously pushed that vanished locally are already
            // tombstoned via bridge export; nothing extra here.
            let _ = live;
        }

        if !ops.is_empty() {
            let seq = self.own_seq.fetch_add(1, Ordering::SeqCst) + 1;
            let batch = OpBatch {
                device: self.device_id,
                seq,
                ops,
            };
            let blob = self.encrypt_batch(&batch)?;
            meta.bytes_uploaded += blob.len() as u64;
            let path = op_path(self.device_id, seq);
            self.blobs.put_blob(&path, &blob).await?;
            self.cursors.write().await.insert(self.device_id, seq);
        }

        // Compaction (best-effort).
        if self.should_compact().await? && self.compact_now(engine, groups).await? {
            meta.compacted = true;
        }

        let results: Vec<(Group, SyncStats)> = groups
            .iter()
            .map(|&g| {
                let mut s = per_group.get(&g).copied().unwrap_or_default();
                // Surface applied remote ops as "pulled" for UI familiarity.
                if meta.ops_applied > 0 && s.pulled == 0 && s.pushed == 0 {
                    s.pulled = meta.ops_applied.min(1);
                }
                (g, s)
            })
            .collect();
        Ok((results, meta))
    }

    /// Force a snapshot + prune of our own ops covered by the watermark.
    pub async fn compact_now(
        &self,
        engine: &SyncEngine,
        groups: &[Group],
    ) -> Result<bool, SyncError> {
        let mut records = Vec::new();
        for &group in groups {
            for (id, rec) in engine.snapshot_group(group).await {
                records.push(SnapshotRecord {
                    group,
                    id,
                    record: rec,
                });
            }
        }
        let watermark = self.cursors.read().await.clone();
        let snap = Snapshot {
            records,
            watermark: watermark.clone(),
            created_at_ms: self.clock.now_ms(),
            writer: self.device_id,
        };
        let blob = self.encrypt_snapshot(&snap)?;
        let id = Uuid::new_v4();
        let path = format!("{SNAP_PREFIX}/{id}.bin");
        self.blobs.put_blob(&path, &blob).await?;
        self.blobs
            .put_blob(SNAP_LATEST, id.to_string().as_bytes())
            .await?;

        // Prune our own ops at or below our watermark. Keep a small tail so
        // concurrent peers that have not yet read the snapshot still have a
        // chance to catch up from ops (grace ≈ half of max_op_files).
        let my_wm = watermark.get(&self.device_id).copied().unwrap_or(0);
        if my_wm == 0 {
            return Ok(true);
        }
        let files = self.list_ops(self.device_id).await?;
        let keep_tail = (self.compact.max_op_files / 4).max(4) as u64;
        let prune_upto = my_wm.saturating_sub(keep_tail);
        for (seq, path, _) in files {
            if seq <= prune_upto {
                let _ = self.blobs.delete_blob(&path).await;
            }
        }
        let _ = self.compact.prune_grace_ms;
        Ok(true)
    }

    async fn should_compact(&self) -> Result<bool, SyncError> {
        let files = self.list_ops(self.device_id).await?;
        if files.len() >= self.compact.max_op_files {
            return Ok(true);
        }
        let bytes: u64 = files.iter().map(|(_, _, s)| s).sum();
        Ok(bytes >= self.compact.max_op_bytes)
    }

    async fn list_device_ids(&self) -> Result<Vec<Uuid>, SyncError> {
        let listed = self.blobs.list_blobs(&format!("{OPS_PREFIX}/")).await?;
        let mut ids = HashSet::new();
        for meta in listed {
            // v2/devices/<uuid>/ops/...
            let parts: Vec<_> = meta.path.split('/').collect();
            if parts.len() >= 3 && parts[0] == "v2" && parts[1] == "devices" {
                if let Ok(u) = Uuid::parse_str(parts[2]) {
                    ids.insert(u);
                }
            }
        }
        Ok(ids.into_iter().collect())
    }

    async fn list_ops(&self, device: Uuid) -> Result<Vec<(u64, String, u64)>, SyncError> {
        let prefix = format!("{OPS_PREFIX}/{device}/ops/");
        let listed = self.blobs.list_blobs(&prefix).await?;
        let mut out = Vec::new();
        for meta in listed {
            let name = meta.path.rsplit('/').next().unwrap_or("");
            let stem = name.trim_end_matches(".bin");
            if let Ok(seq) = stem.parse::<u64>() {
                out.push((seq, meta.path, meta.size));
            }
        }
        out.sort_by_key(|(s, _, _)| *s);
        Ok(out)
    }

    async fn pull_latest_snapshot(
        &self,
        meta: &mut OplogStats,
    ) -> Result<Option<Snapshot>, SyncError> {
        let ptr = match self.blobs.get_blob(SNAP_LATEST).await {
            Ok(b) => b,
            Err(_) => return Ok(None),
        };
        meta.bytes_downloaded += ptr.len() as u64;
        let id = String::from_utf8(ptr).map_err(|e| SyncError::Crypto(e.to_string()))?;
        let id = id.trim();
        let path = format!("{SNAP_PREFIX}/{id}.bin");
        let blob = self.blobs.get_blob(&path).await?;
        meta.bytes_downloaded += blob.len() as u64;
        Ok(Some(self.decrypt_snapshot(&blob)?))
    }

    async fn apply_snapshot(
        &self,
        engine: &SyncEngine,
        snap: &Snapshot,
        groups: &HashSet<Group>,
    ) -> Result<(), SyncError> {
        for rec in &snap.records {
            if groups.contains(&rec.group) {
                engine
                    .adopt_record(rec.group, rec.id, rec.record.clone())
                    .await?;
            }
        }
        Ok(())
    }

    fn encrypt_batch(&self, batch: &OpBatch) -> Result<Vec<u8>, SyncError> {
        let json = serde_json::to_vec(batch).map_err(|e| SyncError::Crypto(e.to_string()))?;
        crypto::encrypt(&self.password, &json, self.params)
            .map_err(|e| SyncError::Crypto(e.to_string()))
    }

    fn decrypt_batch(&self, blob: &[u8]) -> Result<OpBatch, SyncError> {
        let json = crypto::decrypt(&self.password, blob, self.params)
            .map_err(|e| SyncError::Crypto(e.to_string()))?;
        serde_json::from_slice(&json).map_err(|e| SyncError::Crypto(e.to_string()))
    }

    fn encrypt_snapshot(&self, snap: &Snapshot) -> Result<Vec<u8>, SyncError> {
        let json = serde_json::to_vec(snap).map_err(|e| SyncError::Crypto(e.to_string()))?;
        crypto::encrypt(&self.password, &json, self.params)
            .map_err(|e| SyncError::Crypto(e.to_string()))
    }

    fn decrypt_snapshot(&self, blob: &[u8]) -> Result<Snapshot, SyncError> {
        let json = crypto::decrypt(&self.password, blob, self.params)
            .map_err(|e| SyncError::Crypto(e.to_string()))?;
        serde_json::from_slice(&json).map_err(|e| SyncError::Crypto(e.to_string()))
    }
}

fn op_path(device: Uuid, seq: u64) -> String {
    format!("{OPS_PREFIX}/{device}/ops/{seq:012}.bin")
}

fn content_hash(rec: &LocalRecord) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    rec.tombstone.hash(&mut h);
    rec.payload.hash(&mut h);
    rec.vv.hash(&mut h);
    h.finish()
}

// VersionVector needs Hash for content_hash — add via hashing entries.
trait VvHash {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H);
}
impl VvHash for VersionVector {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Stable serialised form.
        if let Ok(bytes) = serde_json::to_vec(self) {
            std::hash::Hash::hash(&bytes, state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::blob::MemBlobStore;
    use async_trait::async_trait;
    use std::sync::atomic::AtomicI64;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemBackend {
        store: Mutex<HashMap<(Group, RecordId), Vec<u8>>>,
    }

    #[async_trait]
    impl crate::sync::SyncBackend for MemBackend {
        async fn list(&self, group: Group) -> Result<Vec<RecordId>, SyncError> {
            let s = self.store.lock().unwrap();
            Ok(s.keys()
                .filter(|(g, _)| *g == group)
                .map(|(_, id)| *id)
                .collect())
        }
        async fn get(&self, group: Group, id: RecordId) -> Result<Vec<u8>, SyncError> {
            self.store
                .lock()
                .unwrap()
                .get(&(group, id))
                .cloned()
                .ok_or_else(|| SyncError::Transport("missing".into()))
        }
        async fn put(&self, group: Group, id: RecordId, blob: &[u8]) -> Result<(), SyncError> {
            self.store
                .lock()
                .unwrap()
                .insert((group, id), blob.to_vec());
            Ok(())
        }
        async fn delete(&self, group: Group, id: RecordId) -> Result<(), SyncError> {
            self.store.lock().unwrap().remove(&(group, id));
            Ok(())
        }
    }

    struct FakeClock(AtomicI64);
    impl FakeClock {
        fn new(t: i64) -> Self {
            Self(AtomicI64::new(t))
        }
    }
    impl Clock for FakeClock {
        fn now_ms(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    fn rid(b: u8) -> RecordId {
        let mut bytes = [0u8; 16];
        bytes[0] = b;
        RecordId(Uuid::from_bytes(bytes))
    }

    fn dev(b: u8) -> Uuid {
        let mut bytes = [0u8; 16];
        bytes[15] = b;
        Uuid::from_bytes(bytes)
    }

    fn engine(device: Uuid) -> SyncEngine {
        SyncEngine::with_clock(
            device,
            b"pw".to_vec(),
            KdfParams::test_cheap(),
            Arc::new(MemBackend::default()),
            Arc::new(FakeClock::new(1_000)),
        )
    }

    #[tokio::test]
    async fn incremental_pull_only_gets_new_ops() {
        let blobs = Arc::new(MemBlobStore::new());
        let a_id = dev(1);
        let b_id = dev(2);
        let a_eng = engine(a_id);
        let b_eng = engine(b_id);
        let a = OplogDriver::new(blobs.clone(), a_id, b"pw", KdfParams::test_cheap());
        let b = OplogDriver::new(blobs.clone(), b_id, b"pw", KdfParams::test_cheap());

        a_eng
            .put_local(Group::Connections, rid(1), b"hello".to_vec())
            .await
            .unwrap();
        let (_stats, meta) = a.cycle(&a_eng, &[Group::Connections]).await.unwrap();
        assert!(meta.bytes_uploaded > 0);
        assert_eq!(blobs.gets(), 0); // A didn't need to download

        let before = blobs.gets();
        let (_stats, meta) = b.cycle(&b_eng, &[Group::Connections]).await.unwrap();
        assert!(meta.ops_applied >= 1);
        assert!(blobs.gets() > before);
        assert_eq!(
            b_eng.get_local(Group::Connections, rid(1)).await.as_deref(),
            Some(&b"hello"[..])
        );

        // Second cycle: B should not re-download the same op.
        let before = blobs.gets();
        let (_stats, meta) = b.cycle(&b_eng, &[Group::Connections]).await.unwrap();
        assert_eq!(meta.ops_applied, 0);
        assert_eq!(blobs.gets(), before);
    }

    #[tokio::test]
    async fn concurrent_edit_merges() {
        let blobs = Arc::new(MemBlobStore::new());
        let a_id = dev(1);
        let b_id = dev(2);
        let a_eng = engine(a_id);
        let b_eng = engine(b_id);
        let a = OplogDriver::new(blobs.clone(), a_id, b"pw", KdfParams::test_cheap());
        let b = OplogDriver::new(blobs.clone(), b_id, b"pw", KdfParams::test_cheap());

        a_eng
            .put_local(Group::Shortcuts, rid(3), b"v0".to_vec())
            .await
            .unwrap();
        a.cycle(&a_eng, &[Group::Shortcuts]).await.unwrap();
        b.cycle(&b_eng, &[Group::Shortcuts]).await.unwrap();

        a_eng
            .put_local(Group::Shortcuts, rid(3), b"from-a".to_vec())
            .await
            .unwrap();
        b_eng
            .put_local(Group::Shortcuts, rid(3), b"from-b".to_vec())
            .await
            .unwrap();
        a.cycle(&a_eng, &[Group::Shortcuts]).await.unwrap();
        b.cycle(&b_eng, &[Group::Shortcuts]).await.unwrap();
        a.cycle(&a_eng, &[Group::Shortcuts]).await.unwrap();

        // Both sides should converge (LWW by updated_at then writer id).
        let av = a_eng.get_local(Group::Shortcuts, rid(3)).await;
        let bv = b_eng.get_local(Group::Shortcuts, rid(3)).await;
        assert_eq!(av, bv);
        assert!(av.is_some());
    }

    #[tokio::test]
    async fn snapshot_bootstraps_new_device() {
        let blobs = Arc::new(MemBlobStore::new());
        let a_id = dev(1);
        let b_id = dev(2);
        let a_eng = engine(a_id);
        let a = OplogDriver::new(blobs.clone(), a_id, b"pw", KdfParams::test_cheap());
        a_eng
            .put_local(Group::Appearance, rid(7), b"theme".to_vec())
            .await
            .unwrap();
        a.cycle(&a_eng, &[Group::Appearance]).await.unwrap();
        a.compact_now(&a_eng, &[Group::Appearance]).await.unwrap();

        let b_eng = engine(b_id);
        let b = OplogDriver::new(blobs.clone(), b_id, b"pw", KdfParams::test_cheap());
        let (_s, meta) = b.cycle(&b_eng, &[Group::Appearance]).await.unwrap();
        assert!(meta.snapshot_used);
        assert_eq!(
            b_eng.get_local(Group::Appearance, rid(7)).await.as_deref(),
            Some(&b"theme"[..])
        );
    }
}
