//! Cloud-provider RPC wiring; kept separate from the existing Git/WebDAV paths.
use super::*;
use crate::sync::{
    backends::{
        cloud::CloudRecords,
        drive::DriveBackend,
        s3::{S3Backend, S3Config, S3Credentials},
    },
    blob::BlobStore,
    crypto::KdfParams,
    engine::SyncEngine,
    oplog::OplogDriver,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Deserialize)]
struct Configure {
    #[serde(default = "default_sync_protocol")]
    protocol: u32,
    master_password: String,
    #[serde(default)]
    state_dir: Option<String>,
    #[serde(default)]
    prefix: String,
    #[serde(default)]
    client_id: String,
}
async fn factory(kind: &str, params: &Value) -> Result<(Arc<dyn BlobStore>, String), RpcError> {
    if kind == "s3" {
        let config: S3Config =
            serde_json::from_value(params.clone()).map_err(|e| invalid_params(e.to_string()))?;
        let raw = crate::secret::get_secret("sync.s3.credentials")
            .map_err(|_| invalid_params("S3 credentials missing from OS keyring"))?;
        let credentials: S3Credentials =
            serde_json::from_str(&raw).map_err(|_| invalid_params("invalid S3 credentials"))?;
        let target = format!(
            "s3:{}:{}:{}:{}:{}",
            config.endpoint,
            config.region,
            config.bucket,
            config.prefix,
            Uuid::new_v5(&Uuid::NAMESPACE_OID, raw.as_bytes())
        );
        Ok((
            Arc::new(
                S3Backend::new(config, credentials).map_err(|e| invalid_params(e.to_string()))?,
            ),
            target,
        ))
    } else {
        let config: Configure =
            serde_json::from_value(params.clone()).map_err(|e| invalid_params(e.to_string()))?;
        let generation = crate::secret::get_secret("sync.drive.authGeneration").map_err(|_| {
            invalid_params("Drive re-authorization required for isolated account identity")
        })?;
        let target = format!(
            "drive:{}:{}:{}",
            config.client_id, config.prefix, generation
        );
        Ok((
            Arc::new(
                DriveBackend::new(config.client_id, config.prefix)
                    .await
                    .map_err(|e| invalid_params(e.to_string()))?,
            ),
            target,
        ))
    }
}
pub(crate) fn register(dispatcher: &Dispatcher, state: Arc<AppState>) {
    dispatcher.register("sync.testProvider", move |params| async move {
        if params["backend"] == "git" {
            let path = params["gitRepoPath"].as_str().ok_or_else(||invalid_params("Git path required"))?;
            let mut backend = GitBackend::open_or_init(path).map_err(|e|internal(e.to_string()))?;
            if let Some(url) = params["gitRemoteUrl"].as_str().filter(|s| !s.is_empty()) {
                backend = backend.with_remote(params["gitRemoteName"].as_str().unwrap_or("origin"),url,params["gitRemoteBranch"].as_str().unwrap_or("master")).map_err(|e|internal(e.to_string()))?;
                if params["gitAuthMode"] == "https" { let token=crate::secret::get_secret("sync.git.https").map_err(|_|invalid_params("Git token missing"))?; backend=backend.with_https_auth(params["gitRemoteUser"].as_str().unwrap_or("oauth2").to_string(),token); }
                if params["gitAuthMode"] == "ssh" { backend=backend.with_ssh_key(params["gitSshKeyPath"].as_str().unwrap_or_default().to_string(),crate::secret::get_secret("sync.git.ssh").ok()); }
                if let Some(provider)=params["gitAuthMode"].as_str().and_then(|s|s.strip_prefix("oauth_")) { let prov=parse_oauth_provider(provider)?; let token=oauth::load_token(prov).map_err(|e|internal(e.to_string()))?.ok_or_else(||invalid_params("OAuth token missing"))?; let (user,pass)=oauth_https_credentials(prov,token);backend=backend.with_https_auth(user,pass); }
            }
            let blobs=backend.preview_blobs().await.map_err(|e|internal(e.to_string()))?;
            Ok(json!({"readOnly":true,"objects":blobs.list_blobs("").await.map_err(|e|internal(e.to_string()))?.len()}))
        } else if params["backend"] == "webdav" {
            let url=params["webdavUrl"].as_str().filter(|s|s.starts_with("https://")).ok_or_else(||invalid_params("HTTPS WebDAV URL required"))?;
            let backend=WebDavBackend::with_auth(url,params["webdavUser"].as_str().map(str::to_owned),crate::secret::get_secret("sync.webdav.password").ok());
            let blobs=backend.list_blobs("").await.map_err(|e|internal(e.to_string()))?;Ok(json!({"readOnly":true,"objects":blobs.len()}))
        } else { Err(invalid_params("Git or WebDAV required")) }
    });
    {
        let st = state.clone();
        dispatcher.register("sync.preview", move |params| {
            let st = st.clone();
            async move {
                let _operation = st.sync_operation.lock().await;
                super::projection::recover_projection(&st).await?;
                let p: SyncNowParams =
                    serde_json::from_value(if params.is_null() { json!({}) } else { params })
                        .map_err(|e| invalid_params(e.to_string()))?;
                let groups = selected_sync_groups(p.groups);
                let git = st.git_backend.lock().await.clone();
                let current = require_engine(&st).await?;
                let copy = current.preview_copy().await;
                if groups.contains(&Group::Notes) {
                    let store = st
                        .notes
                        .lock()
                        .await
                        .clone()
                        .ok_or_else(|| internal("notes store unavailable"))?;
                    if store.sync_paused() {
                        return Err(invalid_params(
                            "Notes sync paused: save unsaved drafts before preview",
                        ));
                    }
                    crate::sync::bridge::preview_notes(&store, &copy)
                        .await
                        .map_err(|e| internal(e.to_string()))?;
                }
                if let (Ok(profiles), Ok(settings)) =
                    (require_profiles(&st).await, require_settings(&st).await)
                {
                    crate::sync::bridge::export_locals(
                        &profiles,
                        &settings,
                        st.vault.lock().await.as_ref(),
                        &st.wasm_host.list().await,
                        &copy,
                        &groups,
                    )
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                }
                let driver = st
                    .oplog
                    .lock()
                    .await
                    .clone()
                    .ok_or_else(|| invalid_params("protocol 2 required"))?;
                let mut result = if let Some(git) = git {
                    let blobs = git
                        .preview_blobs()
                        .await
                        .map_err(|e| internal(e.to_string()))?;
                    driver
                        .preview_with_blobs(&copy, &groups, blobs)
                        .await
                        .map_err(|e| internal(e.to_string()))
                } else {
                    driver
                        .preview(&copy, &groups)
                        .await
                        .map_err(|e| internal(e.to_string()))
                }?;
                let token = Uuid::new_v4();
                let binding = Some(driver.approval_binding());
                let mut approval = st.sync_approval.lock().await;
                if approval.binding != binding {
                    approval.approved = None;
                    approval.pending = None;
                    approval.binding = binding;
                    approval.generation = Uuid::new_v4();
                }
                if groups.contains(&Group::Notes) {
                    let store = st
                        .notes
                        .lock()
                        .await
                        .clone()
                        .ok_or_else(|| internal("notes store unavailable"))?;
                    result["notesRevision"] = json!(store.sync_snapshot().map_err(internal)?.2);
                }
                result["engineDigest"] = json!(current
                    .preview_digest()
                    .await
                    .map_err(|e| internal(e.to_string()))?);
                result["previewToken"] = json!(token);
                result["generation"] = json!(approval.generation);
                approval.pending = Some((token, groups, result.clone()));
                Ok(result)
            }
        });
    }
    let pending = Arc::new(tokio::sync::Mutex::new(
        None::<crate::sync::oauth::drive::Pending>,
    ));
    {
        let pending = pending.clone();
        dispatcher.register("sync.driveAuthorize", move |params| {
            let pending = pending.clone();
            async move {
                let client_id = params["client_id"]
                    .as_str()
                    .ok_or_else(|| invalid_params("client_id required"))?
                    .to_string();
                let flow = crate::sync::oauth::drive::Pending::start(client_id)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                let url = flow.url.clone();
                #[cfg(target_os = "windows")]
                let opened = std::process::Command::new("rundll32.exe")
                    .args(["url.dll,FileProtocolHandler", &url])
                    .spawn();
                #[cfg(target_os = "macos")]
                let opened = std::process::Command::new("open").arg(&url).spawn();
                #[cfg(not(any(target_os = "windows", target_os = "macos")))]
                let opened = std::process::Command::new("xdg-open").arg(&url).spawn();
                opened.map_err(|_| internal("could not launch system browser"))?;
                *pending.lock().await = Some(flow);
                Ok(json!({"authorizationUrl":url}))
            }
        });
    }
    let drive_state = state.clone();
    dispatcher.register("sync.driveFinishAuthorization", move |_| {
        let pending = pending.clone();
        let st = drive_state.clone();
        async move {
            let _operation = st.sync_operation.lock().await;
            invalidate_sync_approval(&st).await;
            stop_auto(&st).await;
            *st.sync.lock().await = None;
            *st.oplog.lock().await = None;
            let flow = pending
                .lock()
                .await
                .take()
                .ok_or_else(|| invalid_params("no pending Drive authorization"))?;
            flow.finish().await.map_err(|e| internal(e.to_string()))?;
            Ok(json!({"connected":true}))
        }
    });
    for (method, kind) in [("sync.configureS3", "s3"), ("sync.configureDrive", "drive")] {
        let st = state.clone();
        dispatcher.register(method, move |params| {
            let st = st.clone();
            async move {
                let _operation = st.sync_operation.lock().await;
                let p: Configure = serde_json::from_value(params.clone())
                    .map_err(|e| invalid_params(e.to_string()))?;
                if p.master_password.is_empty() {
                    return Err(invalid_params("master password required"));
                }
                let (blobs, target) = factory(kind, &params).await?;
                let device_id = if let Some(dir) = p.state_dir.as_ref() {
                    crate::sync::persistence::device_identity(std::path::Path::new(dir), None)
                        .map_err(|e| internal(e.to_string()))?
                } else if let Some(settings) = st.settings.lock().await.as_ref() {
                    if let Some(value) = settings
                        .local_get("sync.deviceId")
                        .map_err(|e| internal(e.to_string()))?
                    {
                        serde_json::from_value(value).map_err(|e| internal(e.to_string()))?
                    } else {
                        let id = Uuid::new_v4();
                        settings
                            .local_set("sync.deviceId", &json!(id))
                            .map_err(|e| internal(e.to_string()))?;
                        id
                    }
                } else {
                    Uuid::new_v4()
                };
                let password = p.master_password.into_bytes();
                let mut engine = SyncEngine::new(
                    device_id,
                    password.clone(),
                    KdfParams::DEFAULT,
                    Arc::new(CloudRecords(blobs.clone())),
                );
                let mut driver = OplogDriver::new(blobs, device_id, password, KdfParams::DEFAULT)
                    .with_protocol(p.protocol)
                    .map_err(|e| invalid_params(e.to_string()))?
                    .with_target(&target);
                if let Some(dir) = p.state_dir.as_ref() {
                    engine = engine
                        .with_store(Arc::new(
                            crate::sync::SledStore::open(dir)
                                .map_err(|e| internal(e.to_string()))?,
                        ))
                        .await
                        .map_err(|e| internal(e.to_string()))?;
                    driver = driver
                        .with_state_file(std::path::Path::new(&dir).join("oplog.json"))
                        .await
                        .map_err(|e| internal(e.to_string()))?;
                } else if let Some(settings) = st.settings.lock().await.clone() {
                    engine = engine
                        .with_store(Arc::new(crate::sync::persistence::SettingsSyncStore(
                            settings.clone(),
                        )))
                        .await
                        .map_err(|e| internal(e.to_string()))?;
                    driver = driver
                        .with_settings(settings)
                        .map_err(|e| internal(e.to_string()))?;
                }
                stop_auto(&st).await;
                configure_sync_approval(&st, &target, p.state_dir.as_deref(), &driver).await?;
                *st.sync.lock().await = Some(Arc::new(engine));
                *st.oplog.lock().await = Some(Arc::new(driver));
                *st.sync_kind.lock().await = Some(kind);
                *st.git_backend.lock().await = None;
                *st.oplog_last.lock().await = None;
                Ok(json!({"device_id":device_id,"protocol":p.protocol}))
            }
        });
    }
    dispatcher.register("sync.testCloud",move|params|async move{
        let kind=params["provider"].as_str().filter(|s|matches!(*s,"s3"|"drive")).ok_or_else(||invalid_params("provider must be s3 or drive"))?;
        let (store,_)=factory(kind,&params).await?;
        let blobs=store.list_blobs("").await.map_err(|e|internal(e.to_string()))?;
        Ok(json!({"objects":blobs.len(),"bytes":blobs.iter().map(|b|b.size).sum::<u64>(),"readOnly":true}))
    });
}
