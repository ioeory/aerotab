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

#[path = "v3.rs"]
mod v3;

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
    pub conflicts: usize,
    pub snapshot_used: bool,
    pub compacted: bool,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DriverState {
    device_id: Uuid,
    #[serde(default)]
    target: String,
    #[serde(default)]
    legacy_import_approved: bool,
    own_seq: u64,
    cursors: HashMap<Group, HashMap<Uuid, u64>>,
    last_pushed: Vec<((Group, RecordId), u64)>,
}

/// Per-device cursors + own sequence counter, backed by a BlobStore.
pub struct OplogDriver {
    blobs: Arc<dyn BlobStore>,
    password: Vec<u8>,
    params: KdfParams,
    device_id: Uuid,
    clock: Arc<dyn Clock>,
    cursors: RwLock<HashMap<Group, HashMap<Uuid, u64>>>,
    own_seq: AtomicU64,
    /// Content hash of the last payload we pushed per record (skip unchanged).
    last_pushed: RwLock<HashMap<(Group, RecordId), u64>>,
    compact: CompactConfig,
    state_file: Option<std::path::PathBuf>,
    state_settings: Option<crate::settings::SettingsStore>,
    defer_confirmation: bool,
    pending_hashes: RwLock<HashMap<(Group, RecordId), u64>>,
    target: String,
    snapshot_cache: RwLock<Option<(String, Snapshot, HashSet<Group>)>>,
    protocol: u32,
    read_only: bool,
    legacy_import_approved: bool,
    v3_applied: RwLock<HashMap<String, HashSet<Group>>>,
    coverage: RwLock<HashSet<Group>>,
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
            state_file: None,
            state_settings: None,
            defer_confirmation: false,
            pending_hashes: RwLock::new(HashMap::new()),
            target: String::new(),
            snapshot_cache: RwLock::new(None),
            protocol: 2,
            read_only: false,
            legacy_import_approved: false,
            v3_applied: RwLock::new(HashMap::new()),
            coverage: RwLock::new(HashSet::new()),
        }
    }

    pub fn with_target(mut self, target: &str) -> Self {
        self.target = format!("{target}:protocol:{}", self.protocol);
        self
    }

    pub async fn with_state_file(mut self, path: std::path::PathBuf) -> Result<Self, SyncError> {
        match std::fs::read(&path) {
            Ok(bytes) => {
                let state: DriverState =
                    serde_json::from_slice(&bytes).map_err(|e| SyncError::Crypto(e.to_string()))?;
                if state.device_id != self.device_id {
                    return Err(SyncError::Transport(
                        "oplog state belongs to another device".into(),
                    ));
                }
                self.own_seq.store(state.own_seq, Ordering::SeqCst);
                if state.target == self.target {
                    self.legacy_import_approved = state.legacy_import_approved;
                    *self.cursors.get_mut() = state.cursors;
                    *self.last_pushed.get_mut() = state.last_pushed.into_iter().collect();
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(SyncError::Transport(e.to_string())),
        }
        self.state_file = Some(path);
        Ok(self)
    }

    pub fn with_settings(
        mut self,
        settings: crate::settings::SettingsStore,
    ) -> Result<Self, SyncError> {
        // Rejoin atomically changes the process-wide identity. Other targets'
        // progress belongs to the old identity and must start fresh, not brick
        // future configuration with belongs-to-other-device errors.
        if let Some(value) = settings
            .local_get(&format!("sync.oplogState.{}", self.target))
            .map_err(|e| SyncError::Transport(e.to_string()))?
        {
            let state: DriverState =
                serde_json::from_value(value).map_err(|e| SyncError::Crypto(e.to_string()))?;
            if state.device_id != self.device_id {
                let current = settings
                    .local_get("sync.deviceId")
                    .map_err(|e| SyncError::Transport(e.to_string()))?;
                if current != Some(serde_json::json!(self.device_id)) {
                    return Err(SyncError::Transport(
                        "oplog state belongs to another device".into(),
                    ));
                }
                self.state_settings = Some(settings);
                return Ok(self);
            }
            self.own_seq.store(state.own_seq, Ordering::SeqCst);
            self.legacy_import_approved = state.legacy_import_approved;
            *self.cursors.get_mut() = state.cursors;
            *self.last_pushed.get_mut() = state.last_pushed.into_iter().collect();
        }
        self.state_settings = Some(settings);
        Ok(self)
    }

    async fn persist_state(&self) -> Result<(), SyncError> {
        let state = DriverState {
            device_id: self.device_id,
            target: self.target.clone(),
            legacy_import_approved: self.legacy_import_approved,
            own_seq: self.own_seq(),
            cursors: self.cursors.read().await.clone(),
            last_pushed: self
                .last_pushed
                .read()
                .await
                .iter()
                .map(|(k, v)| (*k, *v))
                .collect(),
        };
        if let Some(settings) = &self.state_settings {
            let value =
                serde_json::to_value(&state).map_err(|e| SyncError::Crypto(e.to_string()))?;
            settings
                .local_set(&format!("sync.oplogState.{}", self.target), &value)
                .map_err(|e| SyncError::Transport(e.to_string()))?;
        }
        let Some(path) = &self.state_file else {
            return Ok(());
        };
        let bytes = serde_json::to_vec(&state).map_err(|e| SyncError::Crypto(e.to_string()))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| SyncError::Transport(e.to_string()))?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, bytes)
            .and_then(|()| std::fs::rename(&tmp, path))
            .map_err(|e| SyncError::Transport(e.to_string()))
    }

    /// A changed Git tree may have discarded locally-only batches while
    /// aligning diverged history. Re-publish authoritative engine state.
    pub async fn reconcile_transport_change(&self) -> Result<(), SyncError> {
        self.last_pushed.write().await.clear();
        self.pending_hashes.write().await.clear();
        self.cursors.write().await.clear();
        *self.snapshot_cache.write().await = None;
        self.v3_applied.write().await.clear();
        self.coverage.write().await.clear();
        self.persist_state().await
    }

    /// Git working-tree writes are not remote acknowledgement.
    pub fn defer_upload_confirmation(mut self) -> Self {
        self.defer_confirmation = true;
        self
    }

    pub async fn confirm_uploaded(&self) -> Result<(), SyncError> {
        let mut pending = self.pending_hashes.write().await;
        self.last_pushed.write().await.extend(pending.drain());
        drop(pending);
        self.persist_state().await
    }

    pub fn blobs(&self) -> Arc<dyn BlobStore> {
        self.blobs.clone()
    }

    pub fn device_id(&self) -> Uuid {
        self.device_id
    }

    pub async fn cursors_snapshot(&self) -> HashMap<Uuid, u64> {
        // A protocol-2 watermark is only safe when every group was consumed.
        let cursors = self.cursors.read().await;
        let mut minimum = HashMap::new();
        for device in cursors.values().flat_map(|m| m.keys()) {
            let seq = Group::ALL
                .iter()
                .map(|g| {
                    cursors
                        .get(g)
                        .and_then(|m| m.get(device))
                        .copied()
                        .unwrap_or(0)
                })
                .min()
                .unwrap_or(0);
            minimum.insert(*device, seq);
        }
        minimum
    }

    pub fn own_seq(&self) -> u64 {
        self.own_seq.load(Ordering::SeqCst)
    }

    /// Simulate reconciliation using a detached engine and a read-only overlay.
    /// No remote writes, cursor persistence, or live-engine mutations occur.
    pub async fn preview(
        &self,
        engine: &SyncEngine,
        groups: &[Group],
    ) -> Result<serde_json::Value, SyncError> {
        self.preview_with_blobs(engine, groups, self.blobs.clone())
            .await
    }

    pub async fn preview_with_blobs(
        &self,
        engine: &SyncEngine,
        groups: &[Group],
        blobs: Arc<dyn BlobStore>,
    ) -> Result<serde_json::Value, SyncError> {
        struct ReadOnlyOverlay {
            remote: Arc<dyn BlobStore>,
        }
        #[async_trait::async_trait]
        impl BlobStore for ReadOnlyOverlay {
            async fn list_blobs(
                &self,
                prefix: &str,
            ) -> Result<Vec<super::blob::BlobMeta>, SyncError> {
                self.remote.list_blobs(prefix).await
            }
            async fn get_blob(&self, path: &str) -> Result<Vec<u8>, SyncError> {
                self.remote.get_blob(path).await
            }
            async fn put_blob(&self, _: &str, _: &[u8]) -> Result<(), SyncError> {
                Ok(())
            }
            async fn delete_blob(&self, _: &str) -> Result<(), SyncError> {
                Err(SyncError::Transport("preview cannot delete".into()))
            }
        }
        let copy = engine.preview_copy().await;
        let mut driver = Self::new(
            Arc::new(ReadOnlyOverlay { remote: blobs }),
            self.device_id,
            self.password.clone(),
            self.params,
        )
        .with_protocol(self.protocol)?;
        driver.read_only = true;
        driver.legacy_import_approved = self.legacy_import_approved;
        // Clone progress so already published records are not falsely reported
        // as pending; never attach live persistence to the simulation.
        *driver.cursors.write().await = self.cursors.read().await.clone();
        *driver.last_pushed.write().await = self.last_pushed.read().await.clone();
        driver.own_seq.store(self.own_seq(), Ordering::SeqCst);
        let mut before = HashMap::new();
        for &g in groups {
            for (id, record) in copy.snapshot_group(g).await {
                before.insert((g, id), record);
            }
        }
        let (stats, meta) = driver.cycle(&copy, groups).await?;
        let publish: usize = stats.iter().map(|(_, stats)| stats.pushed).sum();
        let conflicts = meta.conflicts;
        let mut added = 0usize;
        let mut changed = 0usize;
        let mut deleted = 0usize;
        for &g in groups {
            for (id, record) in copy.snapshot_group(g).await {
                match before.get(&(g, id)) {
                    None if !record.tombstone => added += 1,
                    Some(old) if !old.tombstone && record.tombstone => deleted += 1,
                    Some(old)
                        if old.payload != record.payload || old.tombstone != record.tombstone =>
                    {
                        changed += 1
                    }
                    _ => {}
                }
            }
        }
        Ok(
            serde_json::json!({"added":added,"changed":changed,"deleted":deleted,"publish":publish,"conflicts":conflicts,"readOnly":true,"strategy":"merge"}),
        )
    }

    /// Run one reconciliation cycle for `groups`. Updates `engine` local state.
    pub async fn cycle(
        &self,
        engine: &SyncEngine,
        groups: &[Group],
    ) -> Result<(Vec<(Group, SyncStats)>, OplogStats), SyncError> {
        self.ensure_active().await?;
        if self.protocol == 3
            && !self.legacy_import_approved
            && !self.blobs.list_blobs("v2/").await?.is_empty()
        {
            return Err(SyncError::Transport(
                "legacy v2 detected: preview and explicitly confirm migration before v3 writes"
                    .into(),
            ));
        }
        let mut meta = OplogStats::default();
        let group_set: HashSet<Group> = groups.iter().copied().collect();
        if self.protocol == 3 {
            self.pull_v3_snapshots(engine, &group_set, &mut meta)
                .await?;
        }

        // Snapshot recovery depends on per-group progress, never local emptiness:
        // bridge export intentionally populates the engine before reconciliation.
        if self.protocol == 2 {
            if let Some(snap) = self.pull_latest_snapshot(&mut meta, &group_set).await? {
                meta.conflicts += self.apply_snapshot(engine, &snap, &group_set).await?;
                meta.snapshot_used = true;
                if let Some((_, _, applied)) = self.snapshot_cache.write().await.as_mut() {
                    applied.extend(group_set.iter().copied());
                }
                // Legacy snapshots do not declare group coverage. Their global
                // watermark cannot safely advance any per-group cursor (a previous
                // client may have compacted only one selected group).
                if let Some(&seq) = snap.watermark.get(&self.device_id) {
                    self.own_seq.fetch_max(seq, Ordering::SeqCst);
                }
            }
        }
        // Pull new ops from every device directory.
        let devices = self.list_device_ids().await?;
        for device in devices {
            let cursor = {
                let cursors = self.cursors.read().await;
                groups
                    .iter()
                    .map(|g| {
                        cursors
                            .get(g)
                            .and_then(|m| m.get(&device))
                            .copied()
                            .unwrap_or(0)
                    })
                    .min()
                    .unwrap_or(0)
            };
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
                        if engine.apply_op(op).await? {
                            meta.conflicts += 1;
                        }
                        meta.ops_applied += 1;
                    }
                }
                let mut cursors = self.cursors.write().await;
                for &group in groups {
                    let progress = cursors.entry(group).or_default().entry(device).or_default();
                    *progress = (*progress).max(seq);
                }
                if device == self.device_id {
                    self.own_seq.fetch_max(seq, Ordering::SeqCst);
                }
                let _ = size;
            }
        }

        // Diff local → OpBatch for selected groups.
        let mut ops = Vec::new();
        let mut uploaded_hashes = Vec::new();
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
                uploaded_hashes.push(((group, id), hash));
                per_group.get_mut(&group).unwrap().pushed += 1;
            }
            // Records we previously pushed that vanished locally are already
            // tombstoned via bridge export; nothing extra here.
            let _ = live;
        }

        if !ops.is_empty() {
            let seq = self.own_seq.fetch_add(1, Ordering::SeqCst) + 1;
            // Reserve the sequence durably before publishing: never overwrite
            // a possibly-uploaded batch after a process crash.
            self.persist_state().await?;
            let batch = OpBatch {
                device: self.device_id,
                seq,
                ops,
            };
            let blob = self.encrypt_batch(&batch)?;
            meta.bytes_uploaded += blob.len() as u64;
            let path = if self.protocol == 3 {
                format!("v3/devices/{}/ops/{seq:012}.bin", self.device_id)
            } else {
                op_path(self.device_id, seq)
            };
            self.blobs.put_blob(&path, &blob).await?;
            let mut cursors = self.cursors.write().await;
            for &group in groups {
                cursors
                    .entry(group)
                    .or_default()
                    .insert(self.device_id, seq);
            }
            let mut pushed = if self.defer_confirmation {
                self.pending_hashes.write().await
            } else {
                self.last_pushed.write().await
            };
            for (key, hash) in uploaded_hashes {
                pushed.insert(key, hash);
            }
        }

        self.persist_state().await?;

        // Protocol-2 has no safe pruning/retirement negotiation. Do not create
        // an unbounded new random snapshot every time an over-threshold log is
        // observed; maintenance remains explicit and non-destructive.
        let _maintenance_recommended = self.should_compact().await?;

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
        self.coverage.write().await.extend(groups.iter().copied());
        if self.protocol == 3 && !self.read_only {
            self.write_peer_event(None).await?;
        }
        Ok((results, meta))
    }

    /// Force a snapshot + prune of our own ops covered by the watermark.
    pub async fn compact_now(
        &self,
        engine: &SyncEngine,
        groups: &[Group],
    ) -> Result<bool, SyncError> {
        if self.protocol == 3 {
            return self.compact_v3(engine, groups).await;
        }
        // Protocol-2 latest is a single pointer without group coverage. Never
        // replace a full recovery snapshot with a partial one.
        if !Group::ALL.iter().all(|g| groups.contains(g)) {
            return Ok(false);
        }
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
        // Legacy v2 has one global watermark and no group coverage field.
        // A partial snapshot must not claim progress for absent groups.
        let watermark = if Group::ALL.iter().all(|g| groups.contains(g)) {
            self.cursors_snapshot().await
        } else {
            HashMap::new()
        };
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

        // Do not prune protocol-2 logs: old clients do not recover snapshots
        // once nonempty, and no acknowledged device retirement/grace exists.
        // Retention is intentionally conservative until a negotiated protocol
        // provides per-group coverage and peer acknowledgements.
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
        let listed = self
            .blobs
            .list_blobs(&format!("{}/", self.ops_prefix()))
            .await?;
        let mut ids = HashSet::new();
        for meta in listed {
            // v2/devices/<uuid>/ops/...
            let parts: Vec<_> = meta.path.split('/').collect();
            if parts.len() >= 3
                && parts[0] == if self.protocol == 3 { "v3" } else { "v2" }
                && parts[1] == "devices"
            {
                if let Ok(u) = Uuid::parse_str(parts[2]) {
                    ids.insert(u);
                }
            }
        }
        Ok(ids.into_iter().collect())
    }

    async fn list_ops(&self, device: Uuid) -> Result<Vec<(u64, String, u64)>, SyncError> {
        let prefix = format!("{}/{device}/ops/", self.ops_prefix());
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
        groups: &HashSet<Group>,
    ) -> Result<Option<Snapshot>, SyncError> {
        if !self
            .blobs
            .list_blobs(SNAP_LATEST)
            .await?
            .iter()
            .any(|m| m.path == SNAP_LATEST)
        {
            return Ok(None);
        }
        let ptr = match self.blobs.get_blob(SNAP_LATEST).await {
            Ok(b) => b,
            Err(error) => return Err(error),
        };
        meta.bytes_downloaded += ptr.len() as u64;
        let id = String::from_utf8(ptr).map_err(|e| SyncError::Crypto(e.to_string()))?;
        let id = id.trim();
        Uuid::parse_str(id).map_err(|e| SyncError::Crypto(e.to_string()))?;
        if let Some((cached_id, snapshot, applied)) = self.snapshot_cache.read().await.as_ref() {
            if cached_id == id {
                return if groups.is_subset(applied) {
                    Ok(None)
                } else {
                    Ok(Some(snapshot.clone()))
                };
            }
        }
        let path = format!("{SNAP_PREFIX}/{id}.bin");
        let blob = self.blobs.get_blob(&path).await?;
        meta.bytes_downloaded += blob.len() as u64;
        let snapshot = self.decrypt_snapshot(&blob)?;
        *self.snapshot_cache.write().await =
            Some((id.to_string(), snapshot.clone(), HashSet::new()));
        Ok(Some(snapshot))
    }

    async fn apply_snapshot(
        &self,
        engine: &SyncEngine,
        snap: &Snapshot,
        groups: &HashSet<Group>,
    ) -> Result<usize, SyncError> {
        let mut conflicts = 0;
        for rec in &snap.records {
            if groups.contains(&rec.group)
                && engine
                    .apply_op(&Op {
                        group: rec.group,
                        id: rec.id,
                        kind: if rec.record.tombstone {
                            OpKind::Delete
                        } else {
                            OpKind::Put {
                                payload: rec.record.payload.clone(),
                            }
                        },
                        vv: rec.record.vv.clone(),
                        updated_at_ms: rec.record.updated_at_ms,
                        writer: rec.record.writer,
                        schema: rec.record.schema,
                    })
                    .await?
            {
                conflicts += 1;
            }
        }
        Ok(conflicts)
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

    pub(super) fn engine(device: Uuid) -> SyncEngine {
        SyncEngine::with_clock(
            device,
            b"pw".to_vec(),
            KdfParams::test_cheap(),
            Arc::new(MemBackend::default()),
            Arc::new(FakeClock::new(1_000)),
        )
    }

    #[tokio::test]
    async fn preview_counts_one_hundred_local_publications_without_remote_writes() {
        let blobs = Arc::new(MemBlobStore::new());
        let engine = engine(dev(1));
        for i in 0..100 {
            engine
                .put_local(Group::Connections, rid(i), b"local".to_vec())
                .await
                .unwrap();
        }
        let driver = OplogDriver::new(
            blobs.clone(),
            dev(1),
            b"pw".to_vec(),
            KdfParams::test_cheap(),
        );
        let preview = driver
            .preview(&engine, &[Group::Connections])
            .await
            .unwrap();
        assert_eq!(preview["publish"], 100);
        assert_eq!(preview["conflicts"], 0);
        assert!(blobs.list_blobs("").await.unwrap().is_empty());
        assert_eq!(driver.own_seq(), 0);
    }

    struct FailOnceStore {
        inner: MemBlobStore,
        fail: std::sync::atomic::AtomicBool,
    }

    #[async_trait]
    impl BlobStore for FailOnceStore {
        async fn list_blobs(&self, prefix: &str) -> Result<Vec<super::super::BlobMeta>, SyncError> {
            self.inner.list_blobs(prefix).await
        }
        async fn get_blob(&self, path: &str) -> Result<Vec<u8>, SyncError> {
            self.inner.get_blob(path).await
        }
        async fn put_blob(&self, path: &str, bytes: &[u8]) -> Result<(), SyncError> {
            if self.fail.swap(false, Ordering::SeqCst) {
                return Err(SyncError::Transport("injected upload failure".into()));
            }
            self.inner.put_blob(path, bytes).await
        }
        async fn delete_blob(&self, path: &str) -> Result<(), SyncError> {
            self.inner.delete_blob(path).await
        }
    }

    #[tokio::test]
    async fn merge_preview_never_writes_remote_or_mutates_live_engine_or_cursors() {
        let blobs = Arc::new(MemBlobStore::new());
        let writer = OplogDriver::new(blobs.clone(), dev(1), b"pw", KdfParams::test_cheap());
        let remote = engine(dev(1));
        remote
            .put_local(Group::Connections, rid(1), b"remote".to_vec())
            .await
            .unwrap();
        writer.cycle(&remote, &[Group::Connections]).await.unwrap();
        let reader = OplogDriver::new(blobs.clone(), dev(2), b"pw", KdfParams::test_cheap());
        let local = engine(dev(2));
        let before = blobs.list_blobs("").await.unwrap();
        let result = reader.preview(&local, &[Group::Connections]).await.unwrap();
        assert_eq!(result["added"], 1);
        assert!(local.is_empty().await);
        assert_eq!(reader.own_seq(), 0);
        assert!(reader.cursors_snapshot().await.is_empty());
        assert_eq!(before, blobs.list_blobs("").await.unwrap());
    }

    #[tokio::test]
    async fn failed_upload_is_retried_without_losing_changes() {
        let blobs = Arc::new(FailOnceStore {
            inner: MemBlobStore::new(),
            fail: std::sync::atomic::AtomicBool::new(true),
        });
        let a = OplogDriver::new(blobs.clone(), dev(1), b"pw", KdfParams::test_cheap());
        let ae = engine(dev(1));
        ae.put_local(Group::Connections, rid(1), b"pending".to_vec())
            .await
            .unwrap();
        assert!(a.cycle(&ae, &[Group::Connections]).await.is_err());
        a.cycle(&ae, &[Group::Connections]).await.unwrap();
        assert_eq!(blobs.list_blobs(OPS_PREFIX).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn selection_does_not_skip_previously_unselected_group() {
        let blobs = Arc::new(MemBlobStore::new());
        let a = OplogDriver::new(blobs.clone(), dev(1), b"pw", KdfParams::test_cheap());
        let b = OplogDriver::new(blobs, dev(2), b"pw", KdfParams::test_cheap());
        let ae = engine(dev(1));
        let be = engine(dev(2));
        ae.put_local(Group::Connections, rid(1), b"host".to_vec())
            .await
            .unwrap();
        ae.put_local(Group::Shortcuts, rid(2), b"keys".to_vec())
            .await
            .unwrap();
        a.cycle(&ae, &[Group::Connections, Group::Shortcuts])
            .await
            .unwrap();
        b.cycle(&be, &[Group::Connections]).await.unwrap();
        b.cycle(&be, &[Group::Shortcuts]).await.unwrap();
        assert_eq!(
            be.get_local(Group::Shortcuts, rid(2)).await,
            Some(b"keys".to_vec())
        );
    }

    #[tokio::test]
    async fn nonempty_local_state_still_recovers_compacted_snapshot() {
        let blobs = Arc::new(MemBlobStore::new());
        let a = OplogDriver::new(blobs.clone(), dev(1), b"pw", KdfParams::test_cheap());
        let ae = engine(dev(1));
        ae.put_local(Group::Connections, rid(1), b"remote".to_vec())
            .await
            .unwrap();
        a.cycle(&ae, &[Group::Connections]).await.unwrap();
        a.compact_now(&ae, &Group::ALL).await.unwrap();
        // Simulate an offline peer returning after covered logs were removed.
        for f in blobs.list_blobs(OPS_PREFIX).await.unwrap() {
            blobs.delete_blob(&f.path).await.unwrap();
        }
        let b = OplogDriver::new(blobs, dev(2), b"pw", KdfParams::test_cheap());
        let be = engine(dev(2));
        be.put_local(Group::Connections, rid(2), b"exported".to_vec())
            .await
            .unwrap();
        b.cycle(&be, &[Group::Connections]).await.unwrap();
        assert_eq!(
            be.get_local(Group::Connections, rid(1)).await,
            Some(b"remote".to_vec())
        );
        assert_eq!(
            be.get_local(Group::Connections, rid(2)).await,
            Some(b"exported".to_vec())
        );
    }

    #[tokio::test]
    async fn partial_snapshot_cannot_hide_other_group_logs() {
        let blobs = Arc::new(MemBlobStore::new());
        let a = OplogDriver::new(blobs.clone(), dev(1), b"pw", KdfParams::test_cheap());
        let ae = engine(dev(1));
        ae.put_local(Group::Connections, rid(1), b"host".to_vec())
            .await
            .unwrap();
        ae.put_local(Group::Shortcuts, rid(2), b"keys".to_vec())
            .await
            .unwrap();
        a.cycle(&ae, &[Group::Connections, Group::Shortcuts])
            .await
            .unwrap();
        a.compact_now(&ae, &[Group::Connections]).await.unwrap();
        let b = OplogDriver::new(blobs, dev(2), b"pw", KdfParams::test_cheap());
        let be = engine(dev(2));
        b.cycle(&be, &[Group::Shortcuts]).await.unwrap();
        assert_eq!(
            be.get_local(Group::Shortcuts, rid(2)).await,
            Some(b"keys".to_vec())
        );
    }

    #[tokio::test]
    async fn driver_restart_retains_sequence_cursors_and_pushed_hashes() {
        let dir = std::env::temp_dir().join(format!("aerotab-oplog-state-{}", Uuid::new_v4()));
        let blobs = Arc::new(MemBlobStore::new());
        let ae = engine(dev(1));
        ae.put_local(Group::Connections, rid(1), b"host".to_vec())
            .await
            .unwrap();
        {
            let a = OplogDriver::new(blobs.clone(), dev(1), b"pw", KdfParams::test_cheap())
                .with_state_file(dir.join("oplog.json"))
                .await
                .unwrap();
            a.cycle(&ae, &[Group::Connections]).await.unwrap();
            assert_eq!(a.own_seq(), 1);
        }
        let a = OplogDriver::new(blobs.clone(), dev(1), b"pw", KdfParams::test_cheap())
            .with_state_file(dir.join("oplog.json"))
            .await
            .unwrap();
        assert_eq!(a.own_seq(), 1);
        let (_, meta) = a.cycle(&ae, &[Group::Connections]).await.unwrap();
        assert_eq!(meta.bytes_uploaded, 0);
        assert_eq!(blobs.list_blobs(OPS_PREFIX).await.unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn unchanged_snapshot_download_is_cached_per_group() {
        let blobs = Arc::new(MemBlobStore::new());
        let ae = engine(dev(1));
        let a = OplogDriver::new(blobs.clone(), dev(1), b"pw", KdfParams::test_cheap());
        ae.put_local(Group::Connections, rid(1), b"host".to_vec())
            .await
            .unwrap();
        a.cycle(&ae, &Group::ALL).await.unwrap();
        a.compact_now(&ae, &Group::ALL).await.unwrap();
        let b = OplogDriver::new(blobs.clone(), dev(2), b"pw", KdfParams::test_cheap());
        let be = engine(dev(2));
        b.cycle(&be, &[Group::Connections]).await.unwrap();
        let (_, stats) = b.cycle(&be, &[Group::Connections]).await.unwrap();
        assert!(!stats.snapshot_used);
        assert_eq!(
            stats.bytes_downloaded, 36,
            "only UUID pointer is downloaded"
        );
    }

    #[tokio::test]
    async fn target_change_does_not_reuse_acknowledged_hashes() {
        let dir = std::env::temp_dir().join(format!("aerotab-target-{}", Uuid::new_v4()));
        let settings = crate::settings::SettingsStore::open(&dir).unwrap();
        let ae = engine(dev(1));
        ae.put_local(Group::Connections, rid(1), b"host".to_vec())
            .await
            .unwrap();
        let a = OplogDriver::new(
            Arc::new(MemBlobStore::new()),
            dev(1),
            b"pw",
            KdfParams::test_cheap(),
        )
        .with_target("A")
        .with_settings(settings.clone())
        .unwrap();
        a.cycle(&ae, &[Group::Connections]).await.unwrap();
        let remote = Arc::new(MemBlobStore::new());
        let b = OplogDriver::new(remote.clone(), dev(1), b"pw", KdfParams::test_cheap())
            .with_target("B")
            .with_settings(settings)
            .unwrap();
        b.cycle(&ae, &[Group::Connections]).await.unwrap();
        assert_eq!(remote.list_blobs(OPS_PREFIX).await.unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
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
        a.compact_now(&a_eng, &Group::ALL).await.unwrap();

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
