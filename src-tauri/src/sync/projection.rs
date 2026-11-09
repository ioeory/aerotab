//! Durable maintenance projection: replay before any subsequent local export.
use super::*;

async fn fault(st: &AppState, step: &str) -> Result<(), RpcError> {
    #[cfg(test)]
    if let Ok(settings) = require_settings(st).await {
        if settings
            .local_get("sync.testProjectionFault")
            .ok()
            .flatten()
            .and_then(|v| v.as_str().map(str::to_owned))
            .as_deref()
            == Some(step)
        {
            return Err(internal(format!("injected projection crash: {step}")));
        }
    }
    let _ = (st, step);
    Ok(())
}

pub(super) async fn export_candidate(
    st: &AppState,
    engine: &SyncEngine,
    groups: &[Group],
) -> Result<(), RpcError> {
    if groups.contains(&Group::Notes) {
        if let Some(notes) = st.notes.lock().await.clone() {
            if notes.sync_paused() {
                return Err(invalid_params("save Notes drafts before maintenance"));
            }
            crate::sync::bridge::preview_notes(&notes, engine)
                .await
                .map_err(|e| internal(e.to_string()))?;
        }
    }
    if let (Ok(profiles), Ok(settings)) = (require_profiles(st).await, require_settings(st).await) {
        crate::sync::bridge::export_locals(
            &profiles,
            &settings,
            st.vault.lock().await.as_ref(),
            &st.wasm_host.list().await,
            engine,
            groups,
        )
        .await
        .map_err(|e| internal(e.to_string()))?;
    }
    Ok(())
}

async fn invalidate_maintenance_approval(st: &AppState) -> Result<(), RpcError> {
    if let Some(store) = st.sync_approval_store.lock().await.as_ref() {
        store
            .local_set("sync.approval", &Value::Null)
            .map_err(|e| internal(e.to_string()))?;
    }
    let mut approval = st.sync_approval.lock().await;
    approval.approved = None;
    approval.pending = None;
    approval.generation = Uuid::new_v4();
    Ok(())
}

pub(super) async fn local_baseline(st: &AppState, groups: &[Group]) -> Result<Value, RpcError> {
    let mut result = json!({});
    if let Ok(settings) = require_settings(st).await {
        if groups
            .iter()
            .any(|g| matches!(g, Group::Appearance | Group::Shortcuts | Group::PluginCfg))
        {
            let mut all = settings.all().map_err(|e| internal(e.to_string()))?;
            all.retain(|e| e.key != "sync");
            all.sort_by(|a, b| a.key.cmp(&b.key));
            result["settings"] = json!(all);
        }
    }
    if groups.contains(&Group::Connections) {
        if let Ok(profiles) = require_profiles(st).await {
            let mut list = profiles.list().await.map_err(|e| internal(e.to_string()))?;
            list.sort_by_key(|p| p.id);
            let mut deleted = profiles
                .explicit_deletions()
                .await
                .map_err(|e| internal(e.to_string()))?;
            deleted.sort();
            result["profiles"] = json!([json!(list), json!(deleted)]);
        }
    }
    if groups.contains(&Group::Notes) {
        if let Some(notes) = st.notes.lock().await.clone() {
            let (mut entities, mut deleted, revision) = notes.sync_snapshot().map_err(internal)?;
            entities.sort_by(|a, b| a.id().cmp(b.id()));
            deleted.sort();
            result["notes"] = json!([json!(entities), json!(deleted), json!(revision)]);
        }
    }
    if groups.contains(&Group::Credentials) {
        let vault = st.vault.lock().await;
        if let Some(vault) = vault.as_ref() {
            if vault
                .is_initialized()
                .map_err(|e| internal(e.to_string()))?
            {
                if !vault.is_unlocked().await {
                    return Err(invalid_params(
                        "unlock Vault before maintenance baseline or resolution",
                    ));
                }
                let mut entries = Vec::new();
                for meta in vault.list().await.map_err(|e| internal(e.to_string()))? {
                    entries.push(
                        vault
                            .get(&meta.id)
                            .await
                            .map_err(|e| internal(e.to_string()))?,
                    );
                }
                entries.sort_by(|a, b| a.id.cmp(&b.id));
                let mut deleted = vault
                    .explicit_deletions()
                    .map_err(|e| internal(e.to_string()))?;
                deleted.sort();
                let bytes =
                    serde_json::to_vec(&(entries, deleted)).map_err(|e| internal(e.to_string()))?;
                use sha2::Digest;
                result["credentialsHash"] = json!(format!("{:x}", sha2::Sha256::digest(bytes)));
            } else {
                result["credentialsHash"] = json!("uninitialized");
            }
        } else {
            result["credentialsHash"] = json!("absent");
        }
    }
    Ok(result)
}

pub(super) async fn commit_projection(
    st: &AppState,
    live: &SyncEngine,
    candidate: &SyncEngine,
    groups: &[Group],
    migration: bool,
) -> Result<(), RpcError> {
    if groups.contains(&Group::Credentials)
        && !candidate
            .snapshot_group(Group::Credentials)
            .await
            .is_empty()
    {
        let vault = st.vault.lock().await;
        let Some(vault) = vault.as_ref() else {
            return Err(invalid_params("unlock Vault before maintenance projection"));
        };
        if !vault.is_unlocked().await {
            return Err(invalid_params("unlock Vault before maintenance projection"));
        }
    }
    // Reject malformed remote records before committing a replay journal or
    // touching any application store. I/O failures after this point are replayable.
    for group in groups {
        for (id, r) in candidate.snapshot_group(*group).await {
            if r.tombstone {
                continue;
            }
            match group {
                Group::Connections => {
                    let profile: crate::profile::Profile =
                        serde_json::from_slice(&r.payload).map_err(|e| internal(e.to_string()))?;
                    if profile.id != id.0 {
                        return Err(invalid_params("profile record id mismatch"));
                    }
                }
                Group::Notes => {
                    let entity: crate::notes::SyncEntity =
                        serde_json::from_slice(&r.payload).map_err(|e| internal(e.to_string()))?;
                    if entity.record_id() != id {
                        return Err(invalid_params("notes record id mismatch"));
                    }
                }
                _ => {}
            }
        }
    }
    let driver = st
        .oplog
        .lock()
        .await
        .clone()
        .ok_or_else(|| invalid_params("sync not configured"))?;
    let mut records = Vec::new();
    for group in Group::ALL {
        for (id, record) in candidate.snapshot_group(group).await {
            records.push((group, id, record));
        }
    }
    let mut rollback = Vec::new();
    for group in Group::ALL {
        for (id, r) in live.snapshot_group(group).await {
            rollback.push((group, id, r));
        }
    }
    let baseline = local_baseline(st, groups).await?;
    driver
        .write_maintenance_journal(
            &json!({"records":records,"groups":groups,"migration":migration,"baseline":baseline,"rollback":rollback}),
        )
        .map_err(|e| internal(e.to_string()))?;
    if migration {
        invalidate_maintenance_approval(st).await?;
    }
    fault(st, "journal").await?;
    project(st, candidate, groups).await?;
    // Checkpoint the exact projected store state before any later crash. A
    // partial import without this checkpoint conservatively blocks replay.
    let projected = local_baseline(st, groups).await?;
    driver
        .write_maintenance_journal(
            &json!({"records":records,"groups":groups,"migration":migration,"baseline":projected,"rollback":rollback}),
        )
        .map_err(|e| internal(e.to_string()))?;
    fault(st, "projection").await?;
    let next = live
        .persist_candidate(candidate)
        .await
        .map_err(|e| internal(e.to_string()))?;
    fault(st, "persistence").await?;
    *st.sync.lock().await = Some(Arc::new(next));
    fault(st, "install").await?;
    // Migration journal remains until protocol installation completes, so a
    // crash after projection never exports stale A over staged B.
    if !migration {
        driver
            .write_maintenance_journal(&Value::Null)
            .map_err(|e| internal(e.to_string()))?;
    }
    Ok(())
}
async fn project(st: &AppState, candidate: &SyncEngine, groups: &[Group]) -> Result<(), RpcError> {
    if groups.contains(&Group::Credentials)
        && !candidate
            .snapshot_group(Group::Credentials)
            .await
            .is_empty()
    {
        let vault = st.vault.lock().await;
        if let Some(vault) = vault.as_ref() {
            if !vault.is_unlocked().await {
                return Err(invalid_params("unlock Vault before maintenance recovery"));
            }
        } else {
            return Err(invalid_params("unlock Vault before maintenance recovery"));
        }
    }
    if groups.contains(&Group::Notes) {
        if let Some(notes) = st.notes.lock().await.clone() {
            if notes.sync_paused() {
                return Err(invalid_params("save Notes drafts before recovery"));
            }
            let revision = notes.sync_snapshot().map_err(internal)?.2;
            crate::sync::bridge::import_notes(&notes, candidate, revision)
                .await
                .map_err(|e| internal(e.to_string()))?;
        }
    }
    if let (Ok(profiles), Ok(settings)) = (require_profiles(st).await, require_settings(st).await) {
        crate::sync::bridge::import_locals(
            &profiles,
            &settings,
            st.vault.lock().await.as_ref(),
            candidate,
            groups,
        )
        .await
        .map_err(|e| internal(e.to_string()))?;
    }
    Ok(())
}
pub(super) async fn maintenance_status(st: &AppState) -> Result<Value, RpcError> {
    let driver = st
        .oplog
        .lock()
        .await
        .clone()
        .ok_or_else(|| invalid_params("sync not configured"))?;
    let Some(j) = driver
        .maintenance_journal()
        .map_err(|e| internal(e.to_string()))?
        .filter(|v| !v.is_null())
    else {
        return Ok(json!({"pending":false}));
    };
    let groups: Vec<Group> =
        serde_json::from_value(j["groups"].clone()).map_err(|e| internal(e.to_string()))?;
    let current = local_baseline(st, &groups).await;
    let changed = current
        .as_ref()
        .map(|v| v != &j["baseline"])
        .unwrap_or(true);
    let mut counts = serde_json::Map::new();
    for group in &groups {
        let rows = j["records"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| r[0] == json!(group))
            .collect::<Vec<_>>();
        let previous = j["rollback"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| r[0] == json!(group))
            .collect::<Vec<_>>();
        let different = rows
            .iter()
            .filter(|r| {
                !previous.iter().any(|p| {
                    p[1] == r[1]
                        && p[2]["payload"] == r[2]["payload"]
                        && p[2]["tombstone"] == r[2]["tombstone"]
                })
            })
            .count();
        counts.insert(group.as_str().into(),json!({"candidate":rows.len(),"checkpoint":previous.len(),"different":different,"tombstones":rows.iter().filter(|r|r[2]["tombstone"]==true).count()}));
    }
    Ok(
        json!({"pending":true,"migration":j["migration"],"groups":groups,"counts":counts,"checkpointChanged":changed,"blocked":current.is_err(),"canCancel":j["rollback"].is_array(),"requiresPreview":true}),
    )
}

pub(super) async fn cancel_projection(st: &AppState, confirm: &str) -> Result<Value, RpcError> {
    if confirm != "CANCEL MAINTENANCE KEEP LOCAL" {
        return Err(invalid_params(
            "explicit CANCEL MAINTENANCE KEEP LOCAL confirmation required",
        ));
    }
    let driver = st
        .oplog
        .lock()
        .await
        .clone()
        .ok_or_else(|| invalid_params("sync not configured"))?;
    let Some(mut journal) = driver
        .maintenance_journal()
        .map_err(|e| internal(e.to_string()))?
        .filter(|v| !v.is_null())
    else {
        return Err(invalid_params("no pending maintenance"));
    };
    let groups: Vec<Group> =
        serde_json::from_value(journal["groups"].clone()).map_err(|e| internal(e.to_string()))?;
    local_baseline(st, &groups).await?; // Locked credentials must fail closed.
    if journal["rollback"].as_array().is_none() {
        return Err(invalid_params("candidate has no rollback checkpoint"));
    }
    if journal["migration"] == true && driver.protocol() != 2 {
        return Err(invalid_params(
            "migration already installed; cancel refused",
        ));
    }
    // Durable cancel decision replays only engine rollback, never app imports.
    journal["cancel"] = json!(true);
    driver
        .write_maintenance_journal(&journal)
        .map_err(|e| internal(e.to_string()))?;
    finish_cancel(st, &driver, &journal).await?;
    Ok(json!({"cancelled":true,"keptLocal":true,"requiresPreview":true,"preservedRemote":true}))
}
async fn finish_cancel(
    st: &AppState,
    driver: &OplogDriver,
    journal: &Value,
) -> Result<(), RpcError> {
    let selected: Vec<Group> =
        serde_json::from_value(journal["groups"].clone()).map_err(|e| internal(e.to_string()))?;
    local_baseline(st, &selected).await?;
    let rollback = driver.empty_engine();
    let records: Vec<(
        Group,
        crate::sync::RecordId,
        crate::sync::engine::LocalRecord,
    )> =
        serde_json::from_value(journal["rollback"].clone()).map_err(|e| internal(e.to_string()))?;
    for (group, id, r) in records {
        rollback
            .apply_op(&crate::sync::oplog::Op {
                group,
                id,
                kind: if r.tombstone {
                    crate::sync::oplog::OpKind::Delete
                } else {
                    crate::sync::oplog::OpKind::Put { payload: r.payload }
                },
                vv: r.vv,
                updated_at_ms: r.updated_at_ms,
                writer: r.writer,
                schema: r.schema,
            })
            .await
            .map_err(|e| internal(e.to_string()))?;
    }
    let current = driver.empty_engine();
    export_candidate(st, &current, &selected).await?;
    let known: Vec<(
        Group,
        crate::sync::RecordId,
        crate::sync::engine::LocalRecord,
    )> = serde_json::from_value(journal["records"].clone()).map_err(|e| internal(e.to_string()))?;
    for group in &selected {
        let original = rollback.snapshot_group(*group).await;
        let mut ids = original.iter().map(|(id, _)| *id).collect::<Vec<_>>();
        ids.extend(
            known
                .iter()
                .filter(|(g, _, _)| g == group)
                .map(|(_, id, _)| *id),
        );
        ids.sort_by_key(|id| id.0);
        ids.dedup();
        for id in &ids {
            if !current
                .snapshot_group(*group)
                .await
                .iter()
                .any(|(i, _)| i == id)
            {
                current
                    .delete_local(*group, *id)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
            }
        }
        for (id, r) in current.snapshot_group(*group).await {
            let mut variants = known
                .iter()
                .filter(|(g, i, _)| g == group && *i == id)
                .map(|(_, _, r)| r.clone())
                .collect::<Vec<_>>();
            variants.extend(
                original
                    .iter()
                    .filter(|(i, _)| *i == id)
                    .map(|(_, r)| r.clone()),
            );
            rollback
                .resolve_current(*group, id, &r, &variants)
                .await
                .map_err(|e| internal(e.to_string()))?;
        }
    }
    for (group, id, r) in &known {
        if *group == Group::Notes && !r.tombstone {
            let mut entity: crate::notes::SyncEntity =
                serde_json::from_slice(&r.payload).map_err(|e| internal(e.to_string()))?;
            let copy_id = Uuid::new_v5(&id.0, &r.payload).to_string();
            match &mut entity {
                crate::notes::SyncEntity::Note(n) => {
                    n.id = copy_id;
                    n.title.push_str(" (candidate recovery)");
                }
                crate::notes::SyncEntity::Folder(f) => {
                    f.id = copy_id;
                    f.name.push_str(" (candidate recovery)");
                }
            }
            rollback
                .put_local(
                    *group,
                    entity.record_id(),
                    serde_json::to_vec(&entity).map_err(|e| internal(e.to_string()))?,
                )
                .await
                .map_err(|e| internal(e.to_string()))?;
        }
    }
    // Preserve current Notes as deterministic engine-only conflict variants.
    // The next approved cycle imports these alongside remote B; app stores
    // remain untouched by cancel itself.
    let groups: Vec<Group> =
        serde_json::from_value(journal["groups"].clone()).map_err(|e| internal(e.to_string()))?;
    if groups.contains(&Group::Notes) {
        if let Some(notes) = st.notes.lock().await.clone() {
            for mut entity in notes.sync_snapshot().map_err(internal)?.0 {
                let original = entity.record_id();
                let remote = journal["records"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|r| r[0] == json!(Group::Notes) && r[1] == json!(original));
                if remote.is_none() {
                    continue;
                }
                let bytes = serde_json::to_vec(&entity).map_err(|e| internal(e.to_string()))?;
                let copy_id = Uuid::new_v5(&original.0, &bytes).to_string();
                match &mut entity {
                    crate::notes::SyncEntity::Note(n) => {
                        n.id = copy_id;
                        n.title.push_str(" (local recovery)");
                        n.version = 0;
                        n.updated_at = 0;
                    }
                    crate::notes::SyncEntity::Folder(f) => {
                        f.id = copy_id;
                        f.name.push_str(" (local recovery)");
                        f.version = 0;
                        f.updated_at = 0;
                    }
                }
                rollback
                    .put_local(
                        Group::Notes,
                        entity.record_id(),
                        serde_json::to_vec(&entity).map_err(|e| internal(e.to_string()))?,
                    )
                    .await
                    .map_err(|e| internal(e.to_string()))?;
            }
        }
    }
    let mut resolved = Vec::new();
    for group in Group::ALL {
        for (id, r) in rollback.snapshot_group(group).await {
            resolved.push((group, id, r));
        }
    }
    driver.save_recovery_bundle(&json!({"groups":selected,"records":resolved,"candidate":journal["records"],"migration":false,"baseline":local_baseline(st,&selected).await?})).map_err(|e|internal(e.to_string()))?;
    invalidate_maintenance_approval(st).await?;
    let live = require_engine(st).await?;
    let restored = live
        .replace_candidate(&rollback)
        .await
        .map_err(|e| internal(e.to_string()))?;
    *st.sync.lock().await = Some(Arc::new(restored));
    // Re-read immutable remote publications next cycle; no pulled candidate
    // watermark can suppress remote B after cancelling this projection.
    driver
        .reconcile_transport_change()
        .await
        .map_err(|e| internal(e.to_string()))?;
    driver
        .write_maintenance_journal(&Value::Null)
        .map_err(|e| internal(e.to_string()))?;
    Ok(())
}

pub(super) async fn recover_projection(st: &AppState) -> Result<(), RpcError> {
    let Some(driver) = st.oplog.lock().await.clone() else {
        return Ok(());
    };
    let Some(journal) = driver
        .maintenance_journal()
        .map_err(|e| internal(e.to_string()))?
        .filter(|v| !v.is_null())
    else {
        return Ok(());
    };
    if journal["cancel"] == true {
        return finish_cancel(st, &driver, &journal).await;
    }
    let live = require_engine(st).await?;
    let candidate = driver.empty_engine();
    let records: Vec<(
        Group,
        crate::sync::RecordId,
        crate::sync::engine::LocalRecord,
    )> = serde_json::from_value(journal["records"].clone()).map_err(|e| internal(e.to_string()))?;
    let groups: Vec<Group> =
        serde_json::from_value(journal["groups"].clone()).map_err(|e| internal(e.to_string()))?;
    if journal["baseline"] != local_baseline(st, &groups).await? {
        return Err(invalid_params("maintenance recovery stopped: local edits/deletions changed after journal checkpoint; candidate and local stores preserved; restore checkpoint or resolve pending maintenance before retry"));
    }
    if journal["migration"] == true {
        invalidate_maintenance_approval(st).await?;
    }
    for (group, id, record) in records {
        candidate
            .apply_op(&crate::sync::oplog::Op {
                group,
                id,
                kind: if record.tombstone {
                    crate::sync::oplog::OpKind::Delete
                } else {
                    crate::sync::oplog::OpKind::Put {
                        payload: record.payload,
                    }
                },
                vv: record.vv,
                updated_at_ms: record.updated_at_ms,
                writer: record.writer,
                schema: record.schema,
            })
            .await
            .map_err(|e| internal(e.to_string()))?;
    }
    project(st, &candidate, &groups).await?;
    let next = live
        .persist_candidate(&candidate)
        .await
        .map_err(|e| internal(e.to_string()))?;
    *st.sync.lock().await = Some(Arc::new(next));
    if journal["migration"] == true {
        let next = driver
            .v3_successor()
            .await
            .map_err(|e| internal(e.to_string()))?;
        if let Ok(settings) = require_settings(st).await {
            if let Some(mut value) = settings.get("sync").map_err(|e| internal(e.to_string()))? {
                value["protocol"] = json!(3);
                settings
                    .set("sync", &value)
                    .map_err(|e| internal(e.to_string()))?;
            }
        }
        *st.oplog.lock().await = Some(Arc::new(next));
    }
    driver
        .write_maintenance_journal(&Value::Null)
        .map_err(|e| internal(e.to_string()))?;
    Ok(())
}
