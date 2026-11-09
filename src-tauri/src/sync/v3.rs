//! Isolated protocol-3 immutable publications; no shared latest pointer.
use super::*;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Publication {
    protocol: u32,
    snapshot: Snapshot,
    coverage: HashMap<Group, HashMap<Uuid, u64>>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PeerEvent {
    protocol: u32,
    writer: Uuid,
    at_ms: i64,
    cursors: HashMap<Group, HashMap<Uuid, u64>>,
    retired: Option<Uuid>,
}

impl OplogDriver {
    pub fn empty_engine(&self) -> SyncEngine {
        SyncEngine::with_clock(
            self.device_id,
            self.password.clone(),
            self.params,
            Arc::new(crate::sync::backends::cloud::CloudRecords(
                self.blobs.clone(),
            )),
            self.clock.clone(),
        )
    }
    pub async fn readonly_v3_engine(&self, engine: &SyncEngine) -> Result<SyncEngine, SyncError> {
        Ok(self.maintenance_candidate(engine, &Group::ALL).await?.0)
    }
    pub async fn maintenance_candidate(
        &self,
        engine: &SyncEngine,
        groups: &[Group],
    ) -> Result<(SyncEngine, HashMap<Group, HashMap<Uuid, u64>>), SyncError> {
        struct Overlay(Arc<dyn BlobStore>);
        #[async_trait::async_trait]
        impl BlobStore for Overlay {
            async fn list_blobs(
                &self,
                p: &str,
            ) -> Result<Vec<super::super::blob::BlobMeta>, SyncError> {
                self.0.list_blobs(p).await
            }
            async fn get_blob(&self, p: &str) -> Result<Vec<u8>, SyncError> {
                self.0.get_blob(p).await
            }
            async fn put_blob(&self, _: &str, _: &[u8]) -> Result<(), SyncError> {
                Ok(())
            }
            async fn delete_blob(&self, _: &str) -> Result<(), SyncError> {
                Err(SyncError::Transport("read-only recovery".into()))
            }
        }
        let copy = engine.preview_copy().await;
        let mut reader = Self::new(
            Arc::new(Overlay(self.blobs.clone())),
            Uuid::new_v4(),
            self.password.clone(),
            self.params,
        )
        .with_protocol(3)?;
        reader.read_only = true;
        reader.legacy_import_approved = true;
        reader.cycle(&copy, groups).await?;
        let mut cursors = reader.cursors.read().await.clone();
        // Reader's synthetic append exists only in the overlay, never claim it.
        for progress in cursors.values_mut() {
            progress.remove(&reader.device_id);
        }
        Ok((copy, cursors))
    }

    pub async fn accept_maintenance_coverage(
        &self,
        cursors: HashMap<Group, HashMap<Uuid, u64>>,
    ) -> Result<(), SyncError> {
        self.coverage.write().await.extend(cursors.keys().copied());
        for progress in cursors.values() {
            if let Some(seq) = progress.get(&self.device_id) {
                self.own_seq.fetch_max(*seq, Ordering::SeqCst);
            }
        }
        *self.cursors.write().await = cursors;
        self.persist_state().await
    }

    fn maintenance_target(&self) -> String {
        Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            self.target
                .trim_end_matches(":protocol:2")
                .trim_end_matches(":protocol:3")
                .as_bytes(),
        )
        .to_string()
    }
    pub fn save_recovery_bundle(&self, value: &serde_json::Value) -> Result<(), SyncError> {
        let sealed=self.seal(&serde_json::json!({"schema":1,"target":self.maintenance_target(),"device":self.device_id,"protocol":self.protocol,"payload":value}))?;
        if let Some(path) = &self.state_file {
            crate::sync::persistence::atomic_local_write(
                &path.with_file_name(format!("sync-recovery-{}.bin", self.maintenance_target())),
                &sealed,
            )?;
        }
        if let Some(s) = &self.state_settings {
            s.local_set(
                &format!("sync.recovery.{}", self.maintenance_target()),
                &serde_json::json!(sealed),
            )
            .map_err(|e| SyncError::Transport(e.to_string()))?;
        }
        Ok(())
    }
    pub fn recovery_bundle(&self) -> Result<Option<(Vec<u8>, serde_json::Value)>, SyncError> {
        let bytes = if let Some(path) = &self.state_file {
            match std::fs::read(
                path.with_file_name(format!("sync-recovery-{}.bin", self.maintenance_target())),
            ) {
                Ok(b) => Some(b),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(SyncError::Transport(e.to_string())),
            }
        } else if let Some(s) = &self.state_settings {
            s.local_get(&format!("sync.recovery.{}", self.maintenance_target()))
                .map_err(|e| SyncError::Transport(e.to_string()))?
                .map(serde_json::from_value)
                .transpose()
                .map_err(|e| SyncError::Crypto(e.to_string()))?
        } else {
            None
        };
        bytes
            .map(|b| {
                self.unseal(&b)
                    .and_then(|v| self.validate_journal(v))
                    .map(|v| (b, v))
            })
            .transpose()
    }
    pub fn approval_binding(&self) -> String {
        format!(
            "{}:{}:{}",
            self.maintenance_target(),
            self.device_id,
            self.protocol
        )
    }
    fn journal_path(&self, path: &std::path::Path) -> std::path::PathBuf {
        path.with_file_name(format!(
            "sync-maintenance-{}.bin",
            self.maintenance_target()
        ))
    }
    fn validate_journal(&self, value: serde_json::Value) -> Result<serde_json::Value, SyncError> {
        if value.is_null() {
            return Ok(value);
        }
        let migration = value["payload"]["migration"] == true;
        if value["schema"] != 1
            || value["target"] != self.maintenance_target()
            || value["device"] != self.device_id.to_string()
            || !(value["protocol"] == self.protocol
                || (migration && value["protocol"] == 2 && self.protocol == 3))
        {
            return Err(SyncError::Transport(
                "maintenance journal identity/target/protocol mismatch".into(),
            ));
        }
        Ok(value["payload"].clone())
    }
    pub fn maintenance_journal(&self) -> Result<Option<serde_json::Value>, SyncError> {
        if let Some(path) = &self.state_file {
            if path.with_file_name("sync-maintenance.json").exists() {
                return Err(SyncError::Transport("unbound legacy maintenance journal requires explicit recovery; refusing target mutation".into()));
            }
            let file = self.journal_path(path);
            return match std::fs::read(file) {
                Ok(bytes) => self
                    .unseal(&bytes)
                    .and_then(|v| self.validate_journal(v))
                    .map(Some),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(SyncError::Transport(e.to_string())),
            };
        }
        if let Some(settings) = &self.state_settings {
            let value = settings
                .local_get(&format!(
                    "sync.maintenanceJournal.{}",
                    self.target
                        .trim_end_matches(":protocol:2")
                        .trim_end_matches(":protocol:3")
                ))
                .map_err(|e| SyncError::Transport(e.to_string()))?;
            return value
                .filter(|v| !v.is_null())
                .map(|v| {
                    serde_json::from_value::<Vec<u8>>(v)
                        .map_err(|e| SyncError::Crypto(e.to_string()))
                        .and_then(|bytes| self.unseal(&bytes))
                        .and_then(|v| self.validate_journal(v))
                })
                .transpose();
        }
        Ok(None)
    }
    pub fn write_maintenance_journal(&self, value: &serde_json::Value) -> Result<(), SyncError> {
        let envelope = if value.is_null() {
            serde_json::Value::Null
        } else {
            serde_json::json!({"schema":1,"target":self.maintenance_target(),"device":self.device_id,"protocol":self.protocol,"payload":value})
        };
        let value = &envelope;
        if let Some(path) = &self.state_file {
            let file = self.journal_path(path);
            crate::sync::persistence::atomic_local_write(&file, &self.seal(value)?)?;
        }
        if let Some(settings) = &self.state_settings {
            settings
                .local_set(
                    &format!(
                        "sync.maintenanceJournal.{}",
                        self.target
                            .trim_end_matches(":protocol:2")
                            .trim_end_matches(":protocol:3")
                    ),
                    &if value.is_null() {
                        serde_json::Value::Null
                    } else {
                        serde_json::json!(self.seal(value)?)
                    },
                )
                .map_err(|e| SyncError::Transport(e.to_string()))?;
        }
        Ok(())
    }
    pub fn persist_identity(&self, id: Uuid) -> Result<(), SyncError> {
        if let Some(path) = &self.state_file {
            if let Some(dir) = path.parent() {
                std::fs::write(dir.join("device-id"), id.to_string())
                    .map_err(|e| SyncError::Transport(e.to_string()))?;
            }
        }
        if let Some(settings) = &self.state_settings {
            settings
                .local_set("sync.deviceId", &serde_json::json!(id))
                .map_err(|e| SyncError::Transport(e.to_string()))?;
        }
        Ok(())
    }
    pub async fn rejoin_successor(&self, id: Uuid) -> Result<Self, SyncError> {
        let mut next = Self::with_clock(
            self.blobs.clone(),
            id,
            self.password.clone(),
            self.params,
            self.clock.clone(),
            self.compact,
        )
        .with_protocol(3)?;
        next.target = self.target.clone();
        next.legacy_import_approved = self.legacy_import_approved;
        next.defer_confirmation = self.defer_confirmation;
        // New identity starts fresh; publish sequence 1, never reuse retired progress.
        next.state_file = self.state_file.clone();
        next.state_settings = self.state_settings.clone();
        let state = DriverState {
            device_id: id,
            target: next.target.clone(),
            legacy_import_approved: next.legacy_import_approved,
            own_seq: 0,
            cursors: HashMap::new(),
            last_pushed: Vec::new(),
        };
        let value = serde_json::to_value(&state).map_err(|e| SyncError::Crypto(e.to_string()))?;
        if let Some(path) = &next.state_file {
            if let Some(dir) = path.parent() {
                crate::sync::persistence::atomic_local_write(
                    &dir.join("rejoin-pending.json"),
                    &serde_json::to_vec(&serde_json::json!({"deviceId":id,"state":value}))
                        .map_err(|e| SyncError::Crypto(e.to_string()))?,
                )?;
                crate::sync::persistence::recover_rejoin(dir)?;
            }
        }
        if let Some(settings) = &next.state_settings {
            settings
                .local_set_batch(&[
                    ("sync.deviceId", serde_json::json!(id)),
                    ("sync.approval", serde_json::Value::Null),
                    (&format!("sync.oplogState.{}", next.target), value),
                ])
                .map_err(|e| SyncError::Transport(e.to_string()))?;
        }
        Ok(next)
    }
    pub async fn v3_successor(&self) -> Result<Self, SyncError> {
        let mut next = Self::with_clock(
            self.blobs.clone(),
            self.device_id,
            self.password.clone(),
            self.params,
            self.clock.clone(),
            self.compact,
        )
        .with_protocol(3)?;
        next.target = if self.target.ends_with(":protocol:2") {
            format!("{}:protocol:3", self.target.trim_end_matches(":protocol:2"))
        } else {
            self.target.clone()
        };
        next.defer_confirmation = self.defer_confirmation;
        if let Some(path) = &self.state_file {
            next = next.with_state_file(path.clone()).await?;
        }
        if let Some(settings) = &self.state_settings {
            next = next.with_settings(settings.clone())?;
        }
        next.legacy_import_approved = true;
        next.persist_state().await?;
        Ok(next)
    }

    pub fn protocol(&self) -> u32 {
        self.protocol
    }
    pub fn with_protocol(mut self, protocol: u32) -> Result<Self, SyncError> {
        if !matches!(protocol, 2 | 3) {
            return Err(SyncError::Transport("unsupported sync protocol".into()));
        }
        self.protocol = protocol;
        // Never load v2 transport cursors/hashes into the v3 namespace.
        self.target = format!("{}:protocol:{protocol}", self.target);
        Ok(self)
    }
    fn seal<T: Serialize>(&self, value: &T) -> Result<Vec<u8>, SyncError> {
        crypto::encrypt(
            &self.password,
            &serde_json::to_vec(value).map_err(|e| SyncError::Crypto(e.to_string()))?,
            self.params,
        )
        .map_err(|e| SyncError::Crypto(e.to_string()))
    }
    fn unseal<T: serde::de::DeserializeOwned>(&self, bytes: &[u8]) -> Result<T, SyncError> {
        let json = crypto::decrypt(&self.password, bytes, self.params)
            .map_err(|e| SyncError::Crypto(e.to_string()))?;
        serde_json::from_slice(&json).map_err(|e| SyncError::Crypto(e.to_string()))
    }
    pub(super) fn ops_prefix(&self) -> &'static str {
        if self.protocol == 3 {
            "v3/devices"
        } else {
            OPS_PREFIX
        }
    }

    async fn peer_events(&self) -> Result<Vec<(String, PeerEvent)>, SyncError> {
        let mut out = Vec::new();
        for blob in self.blobs.list_blobs("v3/peers/").await? {
            let event: PeerEvent = self.unseal(&self.blobs.get_blob(&blob.path).await?)?;
            if event.protocol != 3
                || !blob
                    .path
                    .starts_with(&format!("v3/peers/{}/", event.writer))
            {
                return Err(SyncError::Crypto(
                    "invalid peer event identity/capability".into(),
                ));
            }
            out.push((blob.path, event));
        }
        Ok(out)
    }
    pub(super) async fn ensure_active(&self) -> Result<(), SyncError> {
        if self.protocol == 3
            && self
                .peer_events()
                .await?
                .iter()
                .any(|(_, e)| e.retired == Some(self.device_id))
        {
            return Err(SyncError::Transport(
                "device retired; explicitly rejoin using a new device identity".into(),
            ));
        }
        Ok(())
    }
    pub(super) async fn write_peer_event(&self, retired: Option<Uuid>) -> Result<(), SyncError> {
        let event = PeerEvent {
            protocol: 3,
            writer: self.device_id,
            at_ms: self.clock.now_ms(),
            cursors: self.cursors.read().await.clone(),
            retired,
        };
        let path = format!("v3/peers/{}/{}.bin", self.device_id, Uuid::new_v4());
        self.blobs.put_blob(&path, &self.seal(&event)?).await?;
        // Keep retirement events permanently. ACKs are monotone, so retain
        // the newest successful ACK plus its predecessor; never delete a peer's.
        if retired.is_none() {
            let mut own: Vec<_> = self
                .peer_events()
                .await?
                .into_iter()
                .filter(|(_, e)| e.writer == self.device_id && e.retired.is_none())
                .collect();
            own.sort_by(|a, b| (b.1.at_ms, &b.0).cmp(&(a.1.at_ms, &a.0)));
            for (old, _) in own.into_iter().skip(2) {
                if old != path {
                    self.blobs.delete_blob(&old).await?;
                }
            }
        }
        Ok(())
    }
    pub async fn retire_device(&self, device: Uuid) -> Result<(), SyncError> {
        if self.protocol != 3 {
            return Err(SyncError::Transport("device retirement requires v3".into()));
        }
        self.ensure_active().await?;
        self.write_peer_event(Some(device)).await
    }
    pub async fn peer_status(&self) -> Result<serde_json::Value, SyncError> {
        let events = self.peer_events().await?;
        let retired: HashSet<_> = events.iter().filter_map(|(_, e)| e.retired).collect();
        let mut devices: HashSet<_> = events.iter().map(|(_, e)| e.writer).collect();
        devices.extend(retired.iter().copied());
        Ok(
            serde_json::json!({"devices":devices.into_iter().map(|id| serde_json::json!({"id":id,"retired":retired.contains(&id),"self":id==self.device_id})).collect::<Vec<_>>(),"graceMs":self.compact.prune_grace_ms,"retainGenerations":2}),
        )
    }
    async fn publications(&self) -> Result<Vec<(String, Publication)>, SyncError> {
        let mut out = Vec::new();
        for blob in self.blobs.list_blobs("v3/snapshots/").await? {
            let publication: Publication = self.unseal(&self.blobs.get_blob(&blob.path).await?)?;
            if publication.protocol != 3
                || !blob
                    .path
                    .starts_with(&format!("v3/snapshots/{}/", publication.snapshot.writer))
                || publication
                    .snapshot
                    .records
                    .iter()
                    .any(|r| !publication.coverage.contains_key(&r.group))
            {
                return Err(SyncError::Crypto(
                    "invalid v3 snapshot coverage/identity".into(),
                ));
            }
            out.push((blob.path, publication));
        }
        Ok(out)
    }
    pub(super) async fn pull_v3_snapshots(
        &self,
        engine: &SyncEngine,
        groups: &HashSet<Group>,
        meta: &mut OplogStats,
    ) -> Result<(), SyncError> {
        for blob in self.blobs.list_blobs("v3/snapshots/").await? {
            let applied = self
                .v3_applied
                .read()
                .await
                .get(&blob.path)
                .cloned()
                .unwrap_or_default();
            if groups.is_subset(&applied) {
                continue;
            }
            let bytes = self.blobs.get_blob(&blob.path).await?;
            meta.bytes_downloaded += bytes.len() as u64;
            let publication: Publication = self.unseal(&bytes)?;
            if publication.protocol != 3
                || !blob
                    .path
                    .starts_with(&format!("v3/snapshots/{}/", publication.snapshot.writer))
                || publication
                    .snapshot
                    .records
                    .iter()
                    .any(|r| !publication.coverage.contains_key(&r.group))
            {
                return Err(SyncError::Crypto("invalid v3 publication".into()));
            }
            let selected: HashSet<_> = groups
                .iter()
                .filter(|g| publication.coverage.contains_key(g) && !applied.contains(g))
                .copied()
                .collect();
            meta.conflicts += self
                .apply_snapshot(engine, &publication.snapshot, &selected)
                .await?;
            let mut cursors = self.cursors.write().await;
            for group in selected {
                for (&device, &seq) in &publication.coverage[&group] {
                    let current = cursors.entry(group).or_default().entry(device).or_default();
                    *current = (*current).max(seq);
                    if device == self.device_id {
                        self.own_seq.fetch_max(seq, Ordering::SeqCst);
                    }
                }
            }
            self.v3_applied
                .write()
                .await
                .entry(blob.path)
                .or_default()
                .extend(groups.iter().copied());
            meta.snapshot_used = true;
        }
        Ok(())
    }
    pub(super) async fn compact_v3(
        &self,
        engine: &SyncEngine,
        groups: &[Group],
    ) -> Result<bool, SyncError> {
        self.ensure_active().await?;
        // Command maintenance installs a projected candidate and coverage
        // before this call. Direct driver callers reconcile when not covered.
        if groups
            .iter()
            .any(|g| !self.coverage.try_read().is_ok_and(|c| c.contains(g)))
        {
            self.cycle(engine, groups).await?;
        }
        let covered = self.coverage.read().await;
        if groups.is_empty() || groups.iter().any(|g| !covered.contains(g)) {
            return Err(SyncError::Transport(
                "snapshot coverage was not reconciled".into(),
            ));
        }
        let cursors = self.cursors.read().await.clone();
        let mut records = Vec::new();
        for &group in groups {
            records.extend(
                engine
                    .snapshot_group(group)
                    .await
                    .into_iter()
                    .map(|(id, record)| SnapshotRecord { group, id, record }),
            );
        }
        let publication = Publication {
            protocol: 3,
            coverage: groups
                .iter()
                .map(|g| (*g, cursors.get(g).cloned().unwrap_or_default()))
                .collect(),
            snapshot: Snapshot {
                records,
                watermark: HashMap::new(),
                created_at_ms: self.clock.now_ms(),
                writer: self.device_id,
            },
        };
        let path = format!("v3/snapshots/{}/{}.bin", self.device_id, Uuid::new_v4());
        let bytes = self.seal(&publication)?;
        self.blobs.put_blob(&path, &bytes).await?;
        if self.blobs.get_blob(&path).await? != bytes {
            return Err(SyncError::Transport(
                "snapshot read-after-write verification failed".into(),
            ));
        }
        self.write_peer_event(None).await?;
        self.prune_v3().await?;
        Ok(true)
    }
    pub(super) async fn prune_v3(&self) -> Result<(), SyncError> {
        let events = self.peer_events().await?;
        let retired: HashSet<_> = events.iter().filter_map(|(_, e)| e.retired).collect();
        let mut peers = HashMap::<Uuid, HashMap<Group, HashMap<Uuid, u64>>>::new();
        for (_, event) in &events {
            if retired.contains(&event.writer) {
                continue;
            }
            let ack = peers.entry(event.writer).or_default();
            for (&group, devices) in &event.cursors {
                for (&device, &seq) in devices {
                    let value = ack.entry(group).or_default().entry(device).or_default();
                    *value = (*value).max(seq);
                }
            }
        }
        // A crashed first upload may have ops but no ACK event. It is an
        // active peer and must block pruning until acknowledged or retired.
        for device in self.list_device_ids().await? {
            if !retired.contains(&device) {
                peers.entry(device).or_default();
            }
        }
        let mut own: Vec<_> = self
            .publications()
            .await?
            .into_iter()
            .filter(|(_, p)| p.snapshot.writer == self.device_id)
            .collect();
        own.sort_by(|a, b| {
            (b.1.snapshot.created_at_ms, &b.0).cmp(&(a.1.snapshot.created_at_ms, &a.0))
        });
        // Protect two generations PER GROUP: a new Connections snapshot must
        // never erase the only offline recovery snapshot for Appearance.
        let mut protected = HashSet::new();
        for group in Group::ALL {
            protected.extend(
                own.iter()
                    .filter(|(_, p)| p.coverage.contains_key(&group))
                    .take(2)
                    .map(|(path, _)| path.clone()),
            );
        }
        let now = self.clock.now_ms();
        let grace = self.compact.prune_grace_ms.max(0);
        for (seq, path, _) in self.list_ops(self.device_id).await? {
            let batch = self.decrypt_batch(&self.blobs.get_blob(&path).await?)?;
            if batch.device != self.device_id || batch.seq != seq {
                return Err(SyncError::Crypto("prune batch identity mismatch".into()));
            }
            let batch_groups: HashSet<_> = batch.ops.iter().map(|op| op.group).collect();
            let safe_snapshot = own.iter().any(|(_, p)| {
                now.saturating_sub(p.snapshot.created_at_ms) >= grace
                    && batch_groups.iter().all(|g| {
                        p.coverage
                            .get(g)
                            .and_then(|m| m.get(&self.device_id))
                            .copied()
                            .unwrap_or(0)
                            >= seq
                    })
            });
            let acked = peers.values().all(|ack| {
                batch_groups.iter().all(|g| {
                    ack.get(g)
                        .and_then(|m| m.get(&self.device_id))
                        .copied()
                        .unwrap_or(0)
                        >= seq
                })
            });
            if safe_snapshot && acked {
                self.blobs.delete_blob(&path).await?;
            }
        }
        for (path, p) in &own {
            let recoverable = p.coverage.iter().all(|(group, watermark)| {
                own.iter()
                    .filter(|(candidate, q)| {
                        protected.contains(candidate)
                            && q.coverage.get(group).is_some_and(|next| {
                                watermark.iter().all(|(device, seq)| {
                                    next.get(device).copied().unwrap_or(0) >= *seq
                                })
                            })
                            && p.snapshot
                                .records
                                .iter()
                                .filter(|r| r.group == *group)
                                .all(|old| {
                                    q.snapshot.records.iter().any(|new| {
                                        new.group == *group
                                            && new.id == old.id
                                            && (new.record.vv.dominates(&old.record.vv)
                                                || (new.record.vv == old.record.vv
                                                    && new.record.payload == old.record.payload
                                                    && new.record.tombstone
                                                        == old.record.tombstone))
                                    })
                                })
                    })
                    .count()
                    >= 2
            });
            if !protected.contains(path)
                && recoverable
                && now.saturating_sub(p.snapshot.created_at_ms) >= grace
            {
                self.blobs.delete_blob(path).await?;
            }
        }
        Ok(())
    }

    /// Read legacy data using an overlay that drops every attempted write.
    /// The live engine is only replaced by the caller after explicit approval.
    pub async fn legacy_preview_engine(
        &self,
        engine: &SyncEngine,
        blobs: Arc<dyn BlobStore>,
    ) -> Result<SyncEngine, SyncError> {
        struct ImportOnly(Arc<dyn BlobStore>);
        #[async_trait::async_trait]
        impl BlobStore for ImportOnly {
            async fn list_blobs(
                &self,
                p: &str,
            ) -> Result<Vec<super::super::blob::BlobMeta>, SyncError> {
                self.0.list_blobs(p).await
            }
            async fn get_blob(&self, p: &str) -> Result<Vec<u8>, SyncError> {
                self.0.get_blob(p).await
            }
            async fn put_blob(&self, _: &str, _: &[u8]) -> Result<(), SyncError> {
                Ok(())
            }
            async fn delete_blob(&self, _: &str) -> Result<(), SyncError> {
                Err(SyncError::Transport("legacy import cannot delete".into()))
            }
        }
        let copy = engine.preview_copy().await;
        let mut reader = Self::new(
            Arc::new(ImportOnly(blobs)),
            self.device_id,
            self.password.clone(),
            self.params,
        );
        reader.read_only = true;
        reader.legacy_import_approved = true;
        reader.cycle(&copy, &Group::ALL).await?;
        for group in Group::ALL {
            for blob in reader
                .blobs
                .list_blobs(&format!("{}/", group.as_str()))
                .await?
            {
                let Some(id) = blob
                    .path
                    .strip_prefix(&format!("{}/", group.as_str()))
                    .and_then(|s| s.strip_suffix(".bin"))
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .map(RecordId)
                else {
                    continue;
                };
                let record = engine.decode_legacy(&reader.blobs.get_blob(&blob.path).await?)?;
                copy.apply_op(&Op {
                    group,
                    id,
                    kind: if record.tombstone {
                        OpKind::Delete
                    } else {
                        OpKind::Put {
                            payload: record.payload,
                        }
                    },
                    vv: record.vv,
                    updated_at_ms: record.updated_at_ms,
                    writer: record.writer,
                    schema: record.schema,
                })
                .await?;
            }
        }
        Ok(copy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::blob::MemBlobStore;
    fn engine(id: Uuid) -> SyncEngine {
        super::super::tests::engine(id)
    }
    #[tokio::test]
    pub(super) async fn v3_partial_snapshot_recovers_after_prune_without_touching_v2() {
        let store = Arc::new(MemBlobStore::new());
        store.put_blob("v2/untouched", b"legacy").await.unwrap();
        let id = Uuid::new_v4();
        let e = engine(id);
        let d = OplogDriver::new(store.clone(), id, b"pw", KdfParams::test_cheap())
            .with_protocol(3)
            .unwrap()
            .v3_successor()
            .await
            .unwrap();
        e.put_local(
            Group::Connections,
            RecordId(Uuid::new_v4()),
            b"host".to_vec(),
        )
        .await
        .unwrap();
        d.cycle(&e, &[Group::Connections]).await.unwrap();
        assert!(d.compact_now(&e, &[Group::Connections]).await.unwrap());
        let peer = Uuid::new_v4();
        let pe = engine(peer);
        let pd = OplogDriver::new(store.clone(), peer, b"pw", KdfParams::test_cheap())
            .with_protocol(3)
            .unwrap()
            .v3_successor()
            .await
            .unwrap();
        for blob in store.list_blobs("v3/devices/").await.unwrap() {
            if blob.path.contains("/ops/") {
                store.delete_blob(&blob.path).await.unwrap();
            }
        }
        pd.cycle(&pe, &[Group::Connections]).await.unwrap();
        assert_eq!(pe.snapshot_group(Group::Connections).await.len(), 1);
        assert_eq!(store.get_blob("v2/untouched").await.unwrap(), b"legacy");
        assert!(store
            .list_blobs("v3/snapshots/")
            .await
            .unwrap()
            .iter()
            .all(|m| !m.path.ends_with("latest")));
    }
    #[tokio::test]
    async fn maintenance_journal_cannot_cross_target_same_directory() {
        let root = std::env::temp_dir().join(Uuid::new_v4().to_string());
        let id = Uuid::new_v4();
        let blobs = Arc::new(MemBlobStore::new());
        let a = OplogDriver::new(blobs.clone(), id, b"pw", KdfParams::test_cheap())
            .with_protocol(3)
            .unwrap()
            .with_target("A")
            .with_state_file(root.join("oplog.json"))
            .await
            .unwrap();
        a.write_maintenance_journal(&serde_json::json!({"migration":false,"records":[]}))
            .unwrap();
        let b = OplogDriver::new(blobs, id, b"pw", KdfParams::test_cheap())
            .with_protocol(3)
            .unwrap()
            .with_target("B")
            .with_state_file(root.join("oplog.json"))
            .await
            .unwrap();
        assert!(
            b.maintenance_journal().unwrap().is_none(),
            "B must not replay A journal"
        );
        assert!(a.maintenance_journal().unwrap().is_some());
        // Even copying the sealed bytes to B's isolated path cannot bypass
        // authenticated target binding, despite sharing password and identity.
        std::fs::copy(
            a.journal_path(&root.join("oplog.json")),
            b.journal_path(&root.join("oplog.json")),
        )
        .unwrap();
        assert!(b.maintenance_journal().is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn rejoin_successor_never_leaves_identity_state_mismatch_on_restart() {
        let root = std::env::temp_dir().join(format!("aerotab-rejoin-crash-{}", Uuid::new_v4()));
        let old = crate::sync::persistence::device_identity(&root, None).unwrap();
        let blobs = Arc::new(MemBlobStore::new());
        let driver = OplogDriver::new(blobs.clone(), old, b"pw", KdfParams::test_cheap())
            .with_protocol(3)
            .unwrap()
            .with_state_file(root.join("oplog.json"))
            .await
            .unwrap();
        let new = Uuid::new_v4();
        let _next = driver.rejoin_successor(new).await.unwrap();
        // Crash immediately after successor persistence, before RPC installation.
        let recovered = crate::sync::persistence::device_identity(&root, None).unwrap();
        let loaded = OplogDriver::new(blobs, recovered, b"pw", KdfParams::test_cheap())
            .with_protocol(3)
            .unwrap()
            .with_state_file(root.join("oplog.json"))
            .await;
        assert!(
            loaded.is_ok(),
            "restart must recover identity/state together"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn legacy_import_is_read_only_and_keeps_tombstones() {
        let store = Arc::new(MemBlobStore::new());
        let id = Uuid::new_v4();
        let e = engine(id);
        let record = RecordId(Uuid::new_v4());
        e.put_local(Group::Connections, record, b"deleted".to_vec())
            .await
            .unwrap();
        e.delete_local(Group::Connections, record).await.unwrap();
        let old = OplogDriver::new(store.clone(), id, b"pw", KdfParams::test_cheap());
        old.cycle(&e, &[Group::Connections]).await.unwrap();
        let before = store.store.lock().unwrap().clone();
        let fresh = engine(Uuid::new_v4());
        let imported = old
            .legacy_preview_engine(&fresh, store.clone())
            .await
            .unwrap();
        assert!(
            imported.snapshot_group(Group::Connections).await[0]
                .1
                .tombstone
        );
        assert_eq!(*store.store.lock().unwrap(), before);
    }

    #[tokio::test]
    async fn prune_partial_group_batches_after_grace_and_peer_ack() {
        let store = Arc::new(MemBlobStore::new());
        let id = Uuid::new_v4();
        struct TestClock(std::sync::atomic::AtomicI64);
        impl Clock for TestClock {
            fn now_ms(&self) -> i64 {
                self.0.load(Ordering::SeqCst)
            }
        }
        let clock = Arc::new(TestClock(std::sync::atomic::AtomicI64::new(1000)));
        let e = engine(id);
        let d = OplogDriver::with_clock(
            store.clone(),
            id,
            b"pw",
            KdfParams::test_cheap(),
            clock.clone(),
            CompactConfig {
                prune_grace_ms: 10,
                ..Default::default()
            },
        )
        .with_protocol(3)
        .unwrap();
        e.put_local(
            Group::Connections,
            RecordId(Uuid::new_v4()),
            b"host".to_vec(),
        )
        .await
        .unwrap();
        d.cycle(&e, &[Group::Connections]).await.unwrap();
        d.compact_now(&e, &[Group::Connections]).await.unwrap();
        assert_eq!(d.list_ops(id).await.unwrap().len(), 1);
        clock.0.fetch_add(11, Ordering::SeqCst);
        d.prune_v3().await.unwrap();
        assert!(
            d.list_ops(id).await.unwrap().is_empty(),
            "selected group coverage must suffice for a selected-group batch"
        );
    }

    #[tokio::test]
    async fn offline_peer_blocks_prune_until_retirement_then_restores_from_retained_snapshot() {
        struct TestClock(std::sync::atomic::AtomicI64);
        impl Clock for TestClock {
            fn now_ms(&self) -> i64 {
                self.0.load(Ordering::SeqCst)
            }
        }
        let clock = Arc::new(TestClock(std::sync::atomic::AtomicI64::new(1000)));
        let blobs = Arc::new(MemBlobStore::new());
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let e = engine(a);
        let da = OplogDriver::with_clock(
            blobs.clone(),
            a,
            b"pw",
            KdfParams::test_cheap(),
            clock.clone(),
            CompactConfig {
                prune_grace_ms: 10,
                ..Default::default()
            },
        )
        .with_protocol(3)
        .unwrap();
        let db = OplogDriver::new(blobs.clone(), b, b"pw", KdfParams::test_cheap())
            .with_protocol(3)
            .unwrap();
        db.cycle(&engine(b), &[Group::Connections]).await.unwrap();
        e.put_local(
            Group::Connections,
            RecordId(Uuid::new_v4()),
            b"host".to_vec(),
        )
        .await
        .unwrap();
        da.cycle(&e, &[Group::Connections]).await.unwrap();
        for _ in 0..4 {
            da.compact_now(&e, &[Group::Connections]).await.unwrap();
            clock.0.fetch_add(11, Ordering::SeqCst);
        }
        da.prune_v3().await.unwrap();
        assert_eq!(
            da.list_ops(a).await.unwrap().len(),
            1,
            "offline peer must block log deletion"
        );
        da.retire_device(b).await.unwrap();
        da.prune_v3().await.unwrap();
        assert!(da.list_ops(a).await.unwrap().is_empty());
        assert_eq!(
            blobs
                .list_blobs(&format!("v3/snapshots/{a}/"))
                .await
                .unwrap()
                .len(),
            2
        );
        let newcomer = Uuid::new_v4();
        let recovered = engine(newcomer);
        let next = OplogDriver::new(blobs, newcomer, b"pw", KdfParams::test_cheap())
            .with_protocol(3)
            .unwrap();
        next.cycle(&recovered, &[Group::Connections]).await.unwrap();
        assert_eq!(recovered.snapshot_group(Group::Connections).await.len(), 1);
    }

    #[tokio::test]
    pub(super) async fn retired_device_cannot_publish_and_requires_new_identity() {
        let store = Arc::new(MemBlobStore::new());
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let da = OplogDriver::new(store.clone(), a, b"pw", KdfParams::test_cheap())
            .with_protocol(3)
            .unwrap();
        let db = OplogDriver::new(store, b, b"pw", KdfParams::test_cheap())
            .with_protocol(3)
            .unwrap();
        da.retire_device(b).await.unwrap();
        assert!(db
            .cycle(&engine(b), &[Group::Connections])
            .await
            .unwrap_err()
            .to_string()
            .contains("retired"));
    }
}
