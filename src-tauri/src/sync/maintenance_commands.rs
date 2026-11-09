//! Explicit protocol migration and device lifecycle RPCs.
use super::projection::{commit_projection, export_candidate, recover_projection};
use super::*;

pub(super) fn register(dispatcher: &Dispatcher, state: Arc<AppState>) {
    for method in [
        "sync.recoveryInspect",
        "sync.recoveryExport",
        "sync.recoveryRestore",
    ] {
        let st = state.clone();
        dispatcher.register(method,move|params|{let st=st.clone();async move{let _operation=st.sync_operation.lock().await;let driver=st.oplog.lock().await.clone().ok_or_else(||invalid_params("sync not configured"))?;let Some((sealed,bundle))=driver.recovery_bundle().map_err(|e|internal(e.to_string()))? else{return Ok(json!({"available":false}));};if method=="sync.recoveryExport"{return Ok(json!({"available":true,"encrypted":true,"bundle":BASE64.encode(sealed)}));}
if method=="sync.recoveryRestore"{if params["confirm"]!="RESTORE LOCAL RECOVERY"{return Err(invalid_params("explicit RESTORE LOCAL RECOVERY confirmation required"));}
if driver.maintenance_journal().map_err(|e|internal(e.to_string()))?.is_some_and(|v|!v.is_null()){return Err(invalid_params("resolve pending maintenance first"));}let mut bundle=bundle;let groups:Vec<Group>=serde_json::from_value(bundle["groups"].clone()).map_err(|e|internal(e.to_string()))?;bundle["baseline"]=super::projection::local_baseline(&st,&groups).await?;driver.write_maintenance_journal(&bundle).map_err(|e|internal(e.to_string()))?;recover_projection(&st).await?;let target=st.sync_approval.lock().await.target.clone();invalidate_sync_approval(&st).await;st.sync_approval.lock().await.target=target;return Ok(json!({"restored":true,"requiresPreview":true}));}Ok(json!({"available":true,"encrypted":true,"groups":bundle["groups"],"records":bundle["records"].as_array().map_or(0,Vec::len)}))}});
    }
    for method in ["sync.maintenanceStatus", "sync.maintenanceCancel"] {
        let st = state.clone();
        dispatcher.register(method, move |params| {
            let st = st.clone();
            async move {
                let _operation = st.sync_operation.lock().await;
                if method == "sync.maintenanceStatus" {
                    super::projection::maintenance_status(&st).await
                } else {
                    super::projection::cancel_projection(
                        &st,
                        params["confirm"].as_str().unwrap_or(""),
                    )
                    .await
                }
            }
        });
    }
    for method in ["sync.migrationPreview", "sync.migrateV3"] {
        let st = state.clone();
        dispatcher.register(method, move |params| {
            let st = st.clone();
            async move {
                let _operation = st.sync_operation.lock().await;
                let driver = st.oplog.lock().await.clone().ok_or_else(||invalid_params("sync not configured"))?;
                recover_projection(&st).await?;
                let engine = require_engine(&st).await?;
                let blobs = if let Some(git) = st.git_backend.lock().await.clone() { git.preview_blobs().await.map_err(|e|internal(e.to_string()))? } else { driver.blobs() };
                let base=engine.preview_copy().await;
                export_candidate(&st,&base,&Group::ALL).await?;
                let imported = driver.legacy_preview_engine(&base, blobs.clone()).await.map_err(|e|internal(e.to_string()))?;
                let mut legacy = blobs.list_blobs("v2/").await.map_err(|e|internal(e.to_string()))?;
                legacy.sort_by(|a,b|a.path.cmp(&b.path));
                let mut semantic=Vec::new();for group in Group::ALL {for (id,r) in imported.snapshot_group(group).await {semantic.push((group.as_str(),id,r.payload,r.tombstone));}}semantic.sort_by_key(|(g,id,_,_)|(*g,id.0));
                let signature = format!("{}:{}:{}", engine.preview_digest().await.map_err(|e|internal(e.to_string()))?, serde_json::to_string(&semantic).map_err(|e|internal(e.to_string()))?, serde_json::to_string(&legacy).map_err(|e|internal(e.to_string()))?);
                if method == "sync.migrationPreview" {
                    let token = Uuid::new_v4();
                    *st.sync_migration.lock().await = Some((token,signature));
                    let mut groups=serde_json::Map::new();
                    for group in Group::ALL { groups.insert(group.as_str().into(),json!(imported.snapshot_group(group).await.len())); }
                    return Ok(json!({"previewToken":token,"groups":groups,"legacyObjects":legacy.len(),"readOnly":true,"preservesV2":true}));
                }
                let token = params["previewToken"].as_str().and_then(|s|Uuid::parse_str(s).ok()).ok_or_else(||invalid_params("migration preview token required"))?;
                if params["confirm"] != "MIGRATE TO V3" { return Err(invalid_params("explicit MIGRATE TO V3 confirmation required")); }
                if st.sync_migration.lock().await.take() != Some((token,signature)) { return Err(invalid_params("migration preview stale; preview again")); }
                if st.notes.lock().await.as_ref().is_some_and(|n|n.sync_paused()) { return Err(invalid_params("save Notes drafts before migration")); }
                commit_projection(&st,&engine,&imported,&Group::ALL,true).await?;
                // Reconfigure a v3 driver with protocol-isolated persistence.
                let next = driver.v3_successor().await.map_err(|e|internal(e.to_string()))?;
                *st.oplog.lock().await = Some(Arc::new(next));
                stop_auto(&st).await;
                let target=st.sync_approval.lock().await.target.clone();
                invalidate_sync_approval(&st).await;
                st.sync_approval.lock().await.target=target;
                if let Ok(settings)=require_settings(&st).await {
                    if let Some(mut value)=settings.get("sync").map_err(|e|internal(e.to_string()))? { value["protocol"]=json!(3); settings.set("sync",&value).map_err(|e|internal(e.to_string()))?; }
                }
                driver.write_maintenance_journal(&Value::Null).map_err(|e|internal(e.to_string()))?;
                Ok(json!({"protocol":3,"preservedV2":true,"requiresSyncPreview":true}))
            }
        });
    }
    {
        let st = state.clone();
        dispatcher.register("sync.rejoin", move |params| {
            let st = st.clone();
            async move {
                let _operation = st.sync_operation.lock().await;
                if params["confirm"] != "REJOIN AS NEW DEVICE" {
                    return Err(invalid_params("explicit rejoin confirmation required"));
                }
                let driver = st
                    .oplog
                    .lock()
                    .await
                    .clone()
                    .ok_or_else(|| invalid_params("sync not configured"))?;
                if driver.protocol() != 3 {
                    return Err(invalid_params("v3 required"));
                }
                let id = Uuid::new_v4();
                let next = driver
                    .rejoin_successor(id)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                let engine = require_engine(&st).await?.rejoin_identity(id).await;
                *st.sync.lock().await = Some(Arc::new(engine));
                *st.oplog.lock().await = Some(Arc::new(next));
                if let Ok(settings) = require_settings(&st).await {
                    settings
                        .local_set("sync.deviceId", &json!(id))
                        .map_err(|e| internal(e.to_string()))?;
                }
                stop_auto(&st).await;
                let target = st.sync_approval.lock().await.target.clone();
                invalidate_sync_approval(&st).await;
                st.sync_approval.lock().await.target = target;
                Ok(json!({"deviceId":id,"requiresSyncPreview":true}))
            }
        });
    }
    for method in ["sync.devices", "sync.retireDevice"] {
        let st = state.clone();
        dispatcher.register(method, move |params| {
            let st = st.clone();
            async move {
                let _operation = st.sync_operation.lock().await;
                let driver = st
                    .oplog
                    .lock()
                    .await
                    .clone()
                    .ok_or_else(|| invalid_params("sync not configured"))?;
                if method == "sync.retireDevice" {
                    let device = params["deviceId"]
                        .as_str()
                        .and_then(|s| Uuid::parse_str(s).ok())
                        .ok_or_else(|| invalid_params("deviceId required"))?;
                    if params["confirm"] != device.to_string() {
                        return Err(invalid_params("confirm must equal retired device ID"));
                    }
                    driver
                        .retire_device(device)
                        .await
                        .map_err(|e| internal(e.to_string()))?;
                    if let Some(git) = st.git_backend.lock().await.clone() {
                        git.commit_cycle("sync: explicit device retirement")
                            .await
                            .map_err(|e| internal(e.to_string()))?;
                        git.push_remote()
                            .await
                            .map_err(|e| internal(e.to_string()))?;
                    }
                }
                driver
                    .peer_status()
                    .await
                    .map_err(|e| internal(e.to_string()))
            }
        });
    }
}
