//! Sync engine: ties [`SyncBackend`], [`crypto`](super::crypto) and
//! [`VersionVector`] into one reconciliation loop.
//!
//! Storage model
//! -------------
//!
//! Every record is stored on the backend as a single AEAD envelope whose
//! plaintext is the JSON-serialised [`WireRecord`]. The envelope carries
//! metadata (version vector, tombstone flag, schema, writer clock) inside
//! the encrypted payload — the backend therefore never sees anything but
//! opaque ciphertext.
//!
//! Conflict resolution
//! -------------------
//!
//! Per [`docs/sync-protocol.md`]:
//!
//! - **Equal**: no-op.
//! - **Local dominates**: push local to remote.
//! - **Remote dominates**: adopt remote locally.
//! - **Concurrent**: merge version vectors (elementwise max), bump local
//!   device counter, then choose the payload deterministically by:
//!     1. tombstone wins over a live value (deletions propagate),
//!     2. otherwise the higher `updated_at_ms`,
//!     3. otherwise the writer with the lexicographically larger device id.
//!
//! Tombstones
//! ----------
//!
//! Deletes are soft: the record is kept with `tombstone = true` and an
//! empty payload. Real garbage collection is out of scope for v2.0.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use super::crypto::{self, KdfParams};
use super::persistence::SyncStore;
use super::version_vector::{Ordering as VvOrdering, VersionVector};
use super::{Group, Record, RecordId, SyncBackend, SyncError};

/// Plaintext envelope written into the AEAD payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct WireRecord {
    vv: VersionVector,
    schema: u32,
    updated_at_ms: i64,
    tombstone: bool,
    /// User payload bytes (caller-defined encoding; the engine is agnostic).
    payload: Vec<u8>,
    /// Device that produced this version of the record. Used as a stable
    /// tie-breaker on otherwise-undecidable concurrent merges.
    writer: Uuid,
}

/// Authoritative local view of a record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalRecord {
    pub vv: VersionVector,
    pub schema: u32,
    pub updated_at_ms: i64,
    pub tombstone: bool,
    pub payload: Vec<u8>,
    pub writer: Uuid,
}

impl LocalRecord {
    fn from_wire(w: WireRecord) -> Self {
        Self {
            vv: w.vv,
            schema: w.schema,
            updated_at_ms: w.updated_at_ms,
            tombstone: w.tombstone,
            payload: w.payload,
            writer: w.writer,
        }
    }

    fn to_wire(&self) -> WireRecord {
        WireRecord {
            vv: self.vv.clone(),
            schema: self.schema,
            updated_at_ms: self.updated_at_ms,
            tombstone: self.tombstone,
            payload: self.payload.clone(),
            writer: self.writer,
        }
    }
}

/// Per-group reconciliation counters.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SyncStats {
    pub pushed: usize,
    pub pulled: usize,
    pub merged: usize,
    pub unchanged: usize,
}

impl SyncStats {
    pub fn total(&self) -> usize {
        self.pushed + self.pulled + self.merged + self.unchanged
    }
}

/// Provider for the wall clock — abstracted so tests can pin time.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_millis() as i64,
            Err(_) => 0,
        }
    }
}

/// Reconciliation engine.
///
/// `SyncEngine` is `Send + Sync` and cheap to clone (everything large is
/// behind an `Arc`); spawn it on a tokio task and call [`SyncEngine::tick`]
/// on a timer.
pub struct SyncEngine {
    device_id: Uuid,
    password: Vec<u8>,
    params: KdfParams,
    backend: Arc<dyn SyncBackend>,
    clock: Arc<dyn Clock>,
    store: Option<Arc<dyn SyncStore>>,
    local: RwLock<HashMap<(Group, RecordId), LocalRecord>>,
}

impl SyncEngine {
    pub fn new(
        device_id: Uuid,
        password: impl Into<Vec<u8>>,
        params: KdfParams,
        backend: Arc<dyn SyncBackend>,
    ) -> Self {
        Self::with_clock(device_id, password, params, backend, Arc::new(SystemClock))
    }

    pub fn with_clock(
        device_id: Uuid,
        password: impl Into<Vec<u8>>,
        params: KdfParams,
        backend: Arc<dyn SyncBackend>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            device_id,
            password: password.into(),
            params,
            backend,
            clock,
            store: None,
            local: RwLock::new(HashMap::new()),
        }
    }

    /// Attach a persistence store. Existing on-disk state is loaded
    /// immediately so the engine resumes from where it left off.
    pub async fn with_store(mut self, store: Arc<dyn SyncStore>) -> Result<Self, SyncError> {
        let loaded = store.load_all().await?;
        {
            let map = self.local.get_mut();
            for (key, rec) in loaded {
                map.insert(key, rec);
            }
        }
        self.store = Some(store);
        Ok(self)
    }

    /// Isolated in-memory copy for read-only merge previews. No persistence is attached.
    pub async fn preview_copy(&self) -> Self {
        let mut copy = Self::with_clock(
            self.device_id,
            self.password.clone(),
            self.params,
            self.backend.clone(),
            self.clock.clone(),
        );
        *copy.local.get_mut() = self.local.read().await.clone();
        copy
    }

    /// Explicit user resolution: current payload causally follows all known variants.
    pub async fn resolve_current(
        &self,
        group: Group,
        id: RecordId,
        current: &LocalRecord,
        known: &[LocalRecord],
    ) -> Result<(), SyncError> {
        let mut r = current.clone();
        for other in known {
            r.vv.merge(&other.vv);
        }
        r.vv.bump(self.device_id);
        r.writer = self.device_id;
        r.updated_at_ms = self.clock.now_ms();
        self.local.write().await.insert((group, id), r.clone());
        self.persist(group, id, &r).await
    }

    pub async fn replace_candidate(&self, candidate: &Self) -> Result<Self, SyncError> {
        let mut next = candidate.preview_copy().await;
        if let Some(store) = &self.store {
            let records = next
                .local
                .read()
                .await
                .iter()
                .map(|(k, v)| (*k, v.clone()))
                .collect();
            store.replace_all(records).await?;
        }
        next.store = self.store.clone();
        Ok(next)
    }

    pub async fn persist_candidate(&self, candidate: &Self) -> Result<Self, SyncError> {
        let mut next = candidate.preview_copy().await;
        if let Some(store) = &self.store {
            for ((group, id), record) in next.local.read().await.iter() {
                store.upsert(*group, *id, record).await?;
            }
        }
        next.store = self.store.clone();
        Ok(next)
    }

    pub async fn preview_digest(&self) -> Result<String, SyncError> {
        let map = self.local.read().await;
        let mut records: Vec<_> = map
            .iter()
            .map(|((g, id), r)| (g.as_str(), id.0, r))
            .collect();
        records.sort_by_key(|(g, id, _)| (*g, *id));
        let bytes =
            serde_json::to_vec(&records).map_err(|e| SyncError::Transport(e.to_string()))?;
        Ok(Uuid::new_v5(&Uuid::NAMESPACE_OID, &bytes).to_string())
    }
    pub async fn rejoin_identity(&self, id: Uuid) -> Self {
        let mut next = Self::with_clock(
            id,
            self.password.clone(),
            self.params,
            self.backend.clone(),
            self.clock.clone(),
        );
        *next.local.get_mut() = self.local.read().await.clone();
        next.store = self.store.clone();
        next
    }

    /// Decode a legacy envelope without adopting it or writing remote data.
    pub fn decode_legacy(&self, bytes: &[u8]) -> Result<LocalRecord, SyncError> {
        let plain = crypto::decrypt(&self.password, bytes, self.params)
            .map_err(|e| SyncError::Crypto(e.to_string()))?;
        let wire: WireRecord =
            serde_json::from_slice(&plain).map_err(|e| SyncError::Crypto(e.to_string()))?;
        Ok(LocalRecord::from_wire(wire))
    }

    pub fn device_id(&self) -> Uuid {
        self.device_id
    }

    pub fn clock(&self) -> Arc<dyn Clock> {
        self.clock.clone()
    }

    /// True when the in-memory local map is empty (used for cold-start snapshot).
    pub async fn is_empty(&self) -> bool {
        self.local.read().await.is_empty()
    }

    /// Snapshot every local record in `group` (including tombstones).
    pub async fn snapshot_group(&self, group: Group) -> Vec<(RecordId, LocalRecord)> {
        let map = self.local.read().await;
        map.iter()
            .filter(|((g, _), _)| *g == group)
            .map(|((_, id), rec)| (*id, rec.clone()))
            .collect()
    }

    /// Replace local state for `(group, id)` without bumping the VV (remote adopt).
    pub async fn adopt_record(
        &self,
        group: Group,
        id: RecordId,
        rec: LocalRecord,
    ) -> Result<(), SyncError> {
        self.local.write().await.insert((group, id), rec.clone());
        self.persist(group, id, &rec).await
    }

    /// Apply a remote op with the same VV semantics as [`Self::sync_group`].
    pub async fn apply_op(&self, op: &crate::sync::oplog::Op) -> Result<bool, SyncError> {
        use crate::sync::oplog::OpKind;
        let remote = LocalRecord {
            vv: op.vv.clone(),
            schema: op.schema,
            updated_at_ms: op.updated_at_ms,
            tombstone: matches!(op.kind, OpKind::Delete),
            payload: match &op.kind {
                OpKind::Put { payload } => payload.clone(),
                OpKind::Delete => Vec::new(),
            },
            writer: op.writer,
        };
        let local = {
            let map = self.local.read().await;
            map.get(&(op.group, op.id)).cloned()
        };
        let conflict = local
            .as_ref()
            .is_some_and(|local| local.vv.compare(&remote.vv) == VvOrdering::Concurrent);
        match local {
            None => {
                self.local
                    .write()
                    .await
                    .insert((op.group, op.id), remote.clone());
                self.persist(op.group, op.id, &remote).await?;
            }
            Some(local) => match local.vv.compare(&remote.vv) {
                VvOrdering::Equal | VvOrdering::Dominates => {}
                VvOrdering::DominatedBy => {
                    self.local
                        .write()
                        .await
                        .insert((op.group, op.id), remote.clone());
                    self.persist(op.group, op.id, &remote).await?;
                }
                VvOrdering::Concurrent => {
                    self.preserve_note_conflict(op.group, op.id, &local, &remote)
                        .await?;
                    let merged = self.merge_for_group(op.group, local, remote);
                    self.local
                        .write()
                        .await
                        .insert((op.group, op.id), merged.clone());
                    self.persist(op.group, op.id, &merged).await?;
                }
            },
        }
        Ok(conflict)
    }

    /// Local write. Bumps the device counter and stores locally; the new
    /// state will be pushed on the next [`Self::sync_group`].
    pub async fn put_local(
        &self,
        group: Group,
        id: RecordId,
        payload: Vec<u8>,
    ) -> Result<(), SyncError> {
        self.write_local(group, id, payload, false).await
    }

    /// Local write that only bumps the version vector when the plaintext changed.
    pub async fn put_local_if_changed(
        &self,
        group: Group,
        id: RecordId,
        payload: Vec<u8>,
    ) -> Result<(), SyncError> {
        if self
            .get_local(group, id)
            .await
            .is_some_and(|existing| existing == payload)
        {
            return Ok(());
        }
        self.put_local(group, id, payload).await
    }

    /// Mark a record as deleted (tombstone).
    pub async fn delete_local(&self, group: Group, id: RecordId) -> Result<(), SyncError> {
        self.write_local(group, id, Vec::new(), true).await
    }

    async fn write_local(
        &self,
        group: Group,
        id: RecordId,
        payload: Vec<u8>,
        tombstone: bool,
    ) -> Result<(), SyncError> {
        let snapshot = {
            let mut map = self.local.write().await;
            let entry = map.entry((group, id)).or_insert_with(|| LocalRecord {
                vv: VersionVector::new(),
                schema: Record::CURRENT_SCHEMA,
                updated_at_ms: 0,
                tombstone: false,
                payload: Vec::new(),
                writer: self.device_id,
            });
            entry.vv.bump(self.device_id);
            entry.payload = payload;
            entry.tombstone = tombstone;
            entry.updated_at_ms = self.clock.now_ms();
            entry.writer = self.device_id;
            entry.schema = Record::CURRENT_SCHEMA;
            entry.clone()
        };
        self.persist(group, id, &snapshot).await?;
        Ok(())
    }

    async fn persist(
        &self,
        group: Group,
        id: RecordId,
        rec: &LocalRecord,
    ) -> Result<(), SyncError> {
        if let Some(store) = &self.store {
            store.upsert(group, id, rec).await?;
        }
        Ok(())
    }

    /// Returns the local plaintext for `id` if present and not a tombstone.
    pub async fn get_local(&self, group: Group, id: RecordId) -> Option<Vec<u8>> {
        let map = self.local.read().await;
        map.get(&(group, id))
            .filter(|r| !r.tombstone)
            .map(|r| r.payload.clone())
    }

    /// All live (non-tombstoned) record ids for `group`.
    pub async fn list_local(&self, group: Group) -> Vec<RecordId> {
        let map = self.local.read().await;
        map.iter()
            .filter(|((g, _), r)| *g == group && !r.tombstone)
            .map(|((_, id), _)| *id)
            .collect()
    }

    /// Synchronise a single group.
    pub async fn sync_group(&self, group: Group) -> Result<SyncStats, SyncError> {
        let mut stats = SyncStats::default();

        // 1. Enumerate union of local + remote ids.
        let remote_ids: HashSet<RecordId> = self.backend.list(group).await?.into_iter().collect();
        let local_ids: HashSet<RecordId> = {
            let map = self.local.read().await;
            map.keys()
                .filter(|(g, _)| *g == group)
                .map(|(_, id)| *id)
                .collect()
        };
        let all_ids: HashSet<RecordId> = remote_ids.union(&local_ids).copied().collect();

        for id in all_ids {
            let local = {
                let map = self.local.read().await;
                map.get(&(group, id)).cloned()
            };
            let remote_present = remote_ids.contains(&id);

            match (local, remote_present) {
                (None, true) => {
                    // Pure pull.
                    let blob = self.backend.get(group, id).await?;
                    let wire = self.decrypt_wire(&blob)?;
                    let rec = LocalRecord::from_wire(wire);
                    self.local.write().await.insert((group, id), rec.clone());
                    self.persist(group, id, &rec).await?;
                    stats.pulled += 1;
                }
                (Some(local), false) => {
                    // Pure push.
                    let blob = self.encrypt_wire(&local.to_wire())?;
                    self.backend.put(group, id, &blob).await?;
                    stats.pushed += 1;
                }
                (Some(local), true) => {
                    let blob = self.backend.get(group, id).await?;
                    let remote = LocalRecord::from_wire(self.decrypt_wire(&blob)?);
                    match local.vv.compare(&remote.vv) {
                        VvOrdering::Equal => {
                            stats.unchanged += 1;
                        }
                        VvOrdering::Dominates => {
                            let blob = self.encrypt_wire(&local.to_wire())?;
                            self.backend.put(group, id, &blob).await?;
                            stats.pushed += 1;
                        }
                        VvOrdering::DominatedBy => {
                            self.local.write().await.insert((group, id), remote.clone());
                            self.persist(group, id, &remote).await?;
                            stats.pulled += 1;
                        }
                        VvOrdering::Concurrent => {
                            self.preserve_note_conflict(group, id, &local, &remote)
                                .await?;
                            let merged = self.merge_for_group(group, local, remote);
                            let blob = self.encrypt_wire(&merged.to_wire())?;
                            self.backend.put(group, id, &blob).await?;
                            self.local.write().await.insert((group, id), merged.clone());
                            self.persist(group, id, &merged).await?;
                            stats.merged += 1;
                        }
                    }
                }
                (None, false) => unreachable!("id came from union"),
            }
        }
        Ok(stats)
    }

    /// Synchronise every group.
    pub async fn sync_all(&self) -> Result<Vec<(Group, SyncStats)>, SyncError> {
        self.sync_groups(Group::ALL).await
    }

    /// Synchronise the provided groups in order.
    pub async fn sync_groups<I>(&self, groups: I) -> Result<Vec<(Group, SyncStats)>, SyncError>
    where
        I: IntoIterator<Item = Group>,
    {
        let mut out = Vec::new();
        for g in groups {
            out.push((g, self.sync_group(g).await?));
        }
        Ok(out)
    }

    /// Convenience: one timer-driven tick.
    pub async fn tick(&self) -> Result<Vec<(Group, SyncStats)>, SyncError> {
        self.sync_all().await
    }

    async fn preserve_note_conflict(
        &self,
        group: Group,
        id: RecordId,
        local: &LocalRecord,
        remote: &LocalRecord,
    ) -> Result<(), SyncError> {
        if group != Group::Notes || local.payload == remote.payload {
            return Ok(());
        }
        // Preserve both live variants, including edit-vs-delete. Content-addressed
        // IDs make replay idempotent across devices, independent of wall clocks.
        for record in [local, remote] {
            if record.tombstone {
                continue;
            }
            let mut entity: crate::notes::SyncEntity = serde_json::from_slice(&record.payload)
                .map_err(|_| SyncError::Transport("invalid notes conflict payload".into()))?;
            let copy_id = Uuid::new_v5(&id.0, &record.payload).to_string();
            match &mut entity {
                crate::notes::SyncEntity::Note(n) => {
                    n.id = copy_id;
                    n.title.push_str(" (conflict)");
                }
                crate::notes::SyncEntity::Folder(f) => {
                    f.id = copy_id;
                    f.name.push_str(" (conflict)");
                }
            }
            self.put_local_if_changed(
                group,
                entity.record_id(),
                serde_json::to_vec(&entity).map_err(|e| SyncError::Transport(e.to_string()))?,
            )
            .await?;
        }
        Ok(())
    }
    fn merge_for_group(
        &self,
        group: Group,
        mut local: LocalRecord,
        mut remote: LocalRecord,
    ) -> LocalRecord {
        if group == Group::Notes {
            // Stable content ordering, never timestamp-overwrite Markdown.
            let order = local.payload.cmp(&remote.payload);
            local.updated_at_ms = 0;
            remote.updated_at_ms = 0;
            local.writer = if order.is_gt() {
                Uuid::from_u128(1)
            } else {
                Uuid::nil()
            };
            remote.writer = if order.is_gt() {
                Uuid::nil()
            } else {
                Uuid::from_u128(1)
            };
        }
        self.merge(local, remote)
    }
    fn merge(&self, mut local: LocalRecord, remote: LocalRecord) -> LocalRecord {
        // Deterministic concurrent merge:
        //   1. tombstone wins (deletions propagate);
        //   2. higher updated_at_ms wins;
        //   3. lexicographically larger writer id wins.
        let pick_remote = if local.tombstone != remote.tombstone {
            remote.tombstone
        } else if local.updated_at_ms != remote.updated_at_ms {
            remote.updated_at_ms > local.updated_at_ms
        } else {
            remote.writer.as_bytes() > local.writer.as_bytes()
        };

        let (chosen_payload, chosen_tombstone, chosen_updated, chosen_writer) = if pick_remote {
            (
                remote.payload,
                remote.tombstone,
                remote.updated_at_ms,
                remote.writer,
            )
        } else {
            (
                local.payload.clone(),
                local.tombstone,
                local.updated_at_ms,
                local.writer,
            )
        };

        local.vv.merge(&remote.vv);
        local.vv.bump(self.device_id);
        local.payload = chosen_payload;
        local.tombstone = chosen_tombstone;
        local.updated_at_ms = chosen_updated.max(self.clock.now_ms());
        local.writer = chosen_writer;
        local.schema = Record::CURRENT_SCHEMA;
        local
    }

    fn encrypt_wire(&self, wire: &WireRecord) -> Result<Vec<u8>, SyncError> {
        let json = serde_json::to_vec(wire).map_err(|e| SyncError::Crypto(e.to_string()))?;
        crypto::encrypt(&self.password, &json, self.params)
            .map_err(|e| SyncError::Crypto(e.to_string()))
    }

    fn decrypt_wire(&self, blob: &[u8]) -> Result<WireRecord, SyncError> {
        let json = crypto::decrypt(&self.password, blob, self.params)
            .map_err(|e| SyncError::Crypto(e.to_string()))?;
        serde_json::from_slice(&json).map_err(|e| SyncError::Crypto(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicI64, Ordering as AtomicOrdering};
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemBackend {
        store: Mutex<HashMap<(Group, RecordId), Vec<u8>>>,
    }

    #[async_trait]
    impl SyncBackend for MemBackend {
        async fn list(&self, group: Group) -> Result<Vec<RecordId>, SyncError> {
            let s = self.store.lock().unwrap();
            Ok(s.keys()
                .filter(|(g, _)| *g == group)
                .map(|(_, id)| *id)
                .collect())
        }
        async fn get(&self, group: Group, id: RecordId) -> Result<Vec<u8>, SyncError> {
            let s = self.store.lock().unwrap();
            s.get(&(group, id))
                .cloned()
                .ok_or_else(|| SyncError::Transport("not found".into()))
        }
        async fn put(&self, group: Group, id: RecordId, blob: &[u8]) -> Result<(), SyncError> {
            let mut s = self.store.lock().unwrap();
            s.insert((group, id), blob.to_vec());
            Ok(())
        }
        async fn delete(&self, group: Group, id: RecordId) -> Result<(), SyncError> {
            let mut s = self.store.lock().unwrap();
            s.remove(&(group, id));
            Ok(())
        }
    }

    struct FakeClock(AtomicI64);
    impl FakeClock {
        fn new(t: i64) -> Self {
            Self(AtomicI64::new(t))
        }
        fn advance(&self, ms: i64) {
            self.0.fetch_add(ms, AtomicOrdering::SeqCst);
        }
    }
    impl Clock for FakeClock {
        fn now_ms(&self) -> i64 {
            self.0.load(AtomicOrdering::SeqCst)
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

    fn engine(device: Uuid, backend: Arc<dyn SyncBackend>, clock: Arc<FakeClock>) -> SyncEngine {
        SyncEngine::with_clock(
            device,
            b"pw".to_vec(),
            KdfParams::test_cheap(),
            backend,
            clock,
        )
    }

    #[tokio::test]
    async fn two_devices_notes_keep_concurrent_markdown_and_edit_delete() {
        let backend = Arc::new(MemBackend::default());
        let a = engine(dev(1), backend.clone(), Arc::new(FakeClock::new(99999)));
        let b = engine(dev(2), backend, Arc::new(FakeClock::new(1)));
        let note = |body: &str| {
            crate::notes::SyncEntity::Note(crate::notes::Note {
                id: "stable".into(),
                parent_id: None,
                title: "N".into(),
                markdown: body.into(),
                tags: vec!["tag".into()],
                profile_ids: vec!["missing-profile".into()],
                version: 0,
                updated_at: 0,
            })
        };
        let id = note("base").record_id();
        a.put_local(Group::Notes, id, serde_json::to_vec(&note("base")).unwrap())
            .await
            .unwrap();
        a.sync_group(Group::Notes).await.unwrap();
        b.sync_group(Group::Notes).await.unwrap();
        a.put_local(Group::Notes, id, serde_json::to_vec(&note("left")).unwrap())
            .await
            .unwrap();
        b.put_local(
            Group::Notes,
            id,
            serde_json::to_vec(&note("right")).unwrap(),
        )
        .await
        .unwrap();
        a.sync_group(Group::Notes).await.unwrap();
        b.sync_group(Group::Notes).await.unwrap();
        b.sync_group(Group::Notes).await.unwrap();
        a.sync_group(Group::Notes).await.unwrap();
        for e in [&a, &b] {
            let bodies: Vec<_> = e
                .snapshot_group(Group::Notes)
                .await
                .into_iter()
                .filter(|(_, r)| !r.tombstone)
                .map(|(_, r)| {
                    let crate::notes::SyncEntity::Note(n) =
                        serde_json::from_slice(&r.payload).unwrap()
                    else {
                        panic!()
                    };
                    assert_eq!(n.profile_ids, vec!["missing-profile"]);
                    n.markdown
                })
                .collect();
            assert!(bodies.contains(&"left".into()) && bodies.contains(&"right".into()));
        }
        a.delete_local(Group::Notes, id).await.unwrap();
        b.put_local(
            Group::Notes,
            id,
            serde_json::to_vec(&note("edit against delete")).unwrap(),
        )
        .await
        .unwrap();
        a.sync_group(Group::Notes).await.unwrap();
        b.sync_group(Group::Notes).await.unwrap();
        b.sync_group(Group::Notes).await.unwrap();
        a.sync_group(Group::Notes).await.unwrap();
        assert!(a.get_local(Group::Notes, id).await.is_none());
        assert!(a
            .snapshot_group(Group::Notes)
            .await
            .iter()
            .any(|(_, r)| !r.tombstone
                && String::from_utf8_lossy(&r.payload).contains("edit against delete")));
    }

    #[tokio::test]
    async fn push_then_pull_round_trips() {
        let backend = Arc::new(MemBackend::default());
        let clock_a = Arc::new(FakeClock::new(1_000));
        let clock_b = Arc::new(FakeClock::new(1_000));
        let a = engine(dev(1), backend.clone(), clock_a.clone());
        let b = engine(dev(2), backend.clone(), clock_b.clone());

        a.put_local(Group::Connections, rid(1), b"hello".to_vec())
            .await
            .unwrap();
        let stats = a.sync_group(Group::Connections).await.unwrap();
        assert_eq!(stats.pushed, 1);

        let stats = b.sync_group(Group::Connections).await.unwrap();
        assert_eq!(stats.pulled, 1);
        assert_eq!(
            b.get_local(Group::Connections, rid(1)).await.as_deref(),
            Some(&b"hello"[..]),
        );
    }

    #[tokio::test]
    async fn put_local_if_changed_does_not_bump_unchanged_payload() {
        let backend = Arc::new(MemBackend::default());
        let clock = Arc::new(FakeClock::new(1_000));
        let a = engine(dev(1), backend.clone(), clock.clone());

        a.put_local_if_changed(Group::Connections, rid(4), b"rarecloud-a".to_vec())
            .await
            .unwrap();
        let stats = a.sync_group(Group::Connections).await.unwrap();
        assert_eq!(stats.pushed, 1);

        clock.advance(1_000);
        a.put_local_if_changed(Group::Connections, rid(4), b"rarecloud-a".to_vec())
            .await
            .unwrap();
        let stats = a.sync_group(Group::Connections).await.unwrap();
        assert_eq!(stats.unchanged, 1);
        assert_eq!(stats.pushed, 0);

        a.put_local_if_changed(Group::Connections, rid(4), b"rarecloud-b".to_vec())
            .await
            .unwrap();
        let stats = a.sync_group(Group::Connections).await.unwrap();
        assert_eq!(stats.pushed, 1);
    }

    #[tokio::test]
    async fn sequential_edit_remote_dominates() {
        let backend = Arc::new(MemBackend::default());
        let clock = Arc::new(FakeClock::new(1_000));
        let a = engine(dev(1), backend.clone(), clock.clone());
        let b = engine(dev(2), backend.clone(), clock.clone());

        a.put_local(Group::Appearance, rid(7), b"v1".to_vec())
            .await
            .unwrap();
        a.sync_group(Group::Appearance).await.unwrap();
        b.sync_group(Group::Appearance).await.unwrap();
        assert_eq!(
            b.get_local(Group::Appearance, rid(7)).await.as_deref(),
            Some(&b"v1"[..]),
        );

        clock.advance(10);
        b.put_local(Group::Appearance, rid(7), b"v2".to_vec())
            .await
            .unwrap();
        let stats = b.sync_group(Group::Appearance).await.unwrap();
        assert_eq!(stats.pushed, 1);

        let stats = a.sync_group(Group::Appearance).await.unwrap();
        assert_eq!(stats.pulled, 1);
        assert_eq!(
            a.get_local(Group::Appearance, rid(7)).await.as_deref(),
            Some(&b"v2"[..]),
        );
    }

    #[tokio::test]
    async fn concurrent_edit_merges_higher_clock_wins() {
        let backend = Arc::new(MemBackend::default());
        let clock_a = Arc::new(FakeClock::new(1_000));
        let clock_b = Arc::new(FakeClock::new(1_000));
        let a = engine(dev(1), backend.clone(), clock_a.clone());
        let b = engine(dev(2), backend.clone(), clock_b.clone());

        // Establish shared baseline.
        a.put_local(Group::Shortcuts, rid(3), b"v0".to_vec())
            .await
            .unwrap();
        a.sync_group(Group::Shortcuts).await.unwrap();
        b.sync_group(Group::Shortcuts).await.unwrap();

        // Concurrent edits without a sync in between.
        clock_a.advance(5);
        clock_b.advance(10); // b's edit is "later" by wall clock
        a.put_local(Group::Shortcuts, rid(3), b"from-a".to_vec())
            .await
            .unwrap();
        b.put_local(Group::Shortcuts, rid(3), b"from-b".to_vec())
            .await
            .unwrap();

        // a pushes first, b syncs second and detects concurrent.
        a.sync_group(Group::Shortcuts).await.unwrap();
        let stats = b.sync_group(Group::Shortcuts).await.unwrap();
        assert_eq!(stats.merged, 1);
        // b's edit had the higher updated_at_ms, so b wins on tie-break.
        assert_eq!(
            b.get_local(Group::Shortcuts, rid(3)).await.as_deref(),
            Some(&b"from-b"[..]),
        );

        // a pulls the merged result.
        let stats = a.sync_group(Group::Shortcuts).await.unwrap();
        assert_eq!(stats.pulled, 1);
        assert_eq!(
            a.get_local(Group::Shortcuts, rid(3)).await.as_deref(),
            Some(&b"from-b"[..]),
        );
    }

    #[tokio::test]
    async fn tombstone_propagates() {
        let backend = Arc::new(MemBackend::default());
        let clock = Arc::new(FakeClock::new(1_000));
        let a = engine(dev(1), backend.clone(), clock.clone());
        let b = engine(dev(2), backend.clone(), clock.clone());

        a.put_local(Group::PluginCfg, rid(9), b"data".to_vec())
            .await
            .unwrap();
        a.sync_group(Group::PluginCfg).await.unwrap();
        b.sync_group(Group::PluginCfg).await.unwrap();

        clock.advance(1);
        a.delete_local(Group::PluginCfg, rid(9)).await.unwrap();
        a.sync_group(Group::PluginCfg).await.unwrap();
        b.sync_group(Group::PluginCfg).await.unwrap();
        assert!(b.get_local(Group::PluginCfg, rid(9)).await.is_none());
    }

    #[tokio::test]
    async fn idempotent_when_unchanged() {
        let backend = Arc::new(MemBackend::default());
        let clock = Arc::new(FakeClock::new(1_000));
        let a = engine(dev(1), backend.clone(), clock.clone());

        a.put_local(Group::Credentials, rid(2), b"x".to_vec())
            .await
            .unwrap();
        a.sync_group(Group::Credentials).await.unwrap();
        let stats = a.sync_group(Group::Credentials).await.unwrap();
        assert_eq!(stats.unchanged, 1);
        assert_eq!(stats.pushed, 0);
        assert_eq!(stats.pulled, 0);
    }

    #[tokio::test]
    async fn sync_all_covers_every_group() {
        let backend = Arc::new(MemBackend::default());
        let clock = Arc::new(FakeClock::new(1_000));
        let a = engine(dev(1), backend.clone(), clock.clone());

        a.put_local(Group::Connections, rid(1), b"c".to_vec())
            .await
            .unwrap();
        a.put_local(Group::Appearance, rid(2), b"a".to_vec())
            .await
            .unwrap();
        let results = a.sync_all().await.unwrap();
        assert_eq!(results.len(), Group::ALL.len());
        let total: usize = results.iter().map(|(_, s)| s.total()).sum();
        assert!(total >= 2);
    }

    #[tokio::test]
    async fn sync_groups_only_covers_selected_groups() {
        let backend = Arc::new(MemBackend::default());
        let clock = Arc::new(FakeClock::new(1_000));
        let a = engine(dev(1), backend.clone(), clock.clone());

        a.put_local(Group::Connections, rid(1), b"c".to_vec())
            .await
            .unwrap();
        a.put_local(Group::Appearance, rid(2), b"a".to_vec())
            .await
            .unwrap();

        let results = a.sync_groups([Group::Appearance]).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, Group::Appearance);
        assert_eq!(backend.list(Group::Appearance).await.unwrap().len(), 1);
        assert!(backend.list(Group::Connections).await.unwrap().is_empty());
    }
}
