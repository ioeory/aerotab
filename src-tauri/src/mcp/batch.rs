//! Device-local batch approvals. Approval is intentionally NOT an MCP tool/RPC allowlist entry.
use super::tools;
use crate::{
    commands::AppState,
    ipc::{Dispatcher, ErrorCode, RpcError},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

pub const TOOLS:&[(&str,&str,&str)]=&[
    ("batch_plan","batchOperations","Plan a saved script across fixed profile IDs. Requires visible local UI approval before execute."),
    ("batch_execute","batchOperations","Execute a locally approved unchanged batch with bounded concurrency."),
    ("batch_status","batchOperations","Read batch progress without command output."),
    ("batch_results","batchOperations","Read sanitized bounded batch task results."),
    ("batch_cancel","batchOperations","Cancel queued targets and active script tasks."),
];
#[derive(Clone)]
pub struct ExecutionGrant {
    pub config: super::McpConfig,
    pub generation: CancellationToken,
    pub shutdown: CancellationToken,
}
impl ExecutionGrant {
    async fn valid(&self, state: &AppState) -> bool {
        if self.generation.is_cancelled() || self.shutdown.is_cancelled() {
            return false;
        }
        let settings = state.settings.lock().await.clone();
        let config = match settings {
            Some(store) => match store.get("mcp") {
                Ok(Some(value)) => match serde_json::from_value::<super::McpConfig>(value) {
                    Ok(c) if c.enabled => c,
                    _ => return false,
                },
                _ => return false,
            },
            None => self.config.clone(),
        };
        super::permissions::authorize(&config, "batch_execute", &json!({})).is_ok()
    }
}
#[derive(Default)]
pub struct BatchRuntime {
    plans: Mutex<HashMap<String, Plan>>,
}
struct Plan {
    public: Value,
    fingerprint: String,
    profile_ids: Vec<String>,
    created: Instant,
    approved: bool,
    expires: Instant,
    cancel: CancellationToken,
    status: String,
    results: Vec<Value>,
    active: Vec<String>,
    concurrency: usize,
}
fn digest(value: &Value) -> String {
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
}
async fn fingerprint(d: &Dispatcher, script_id: &str) -> Result<String, String> {
    let script = tools::dispatch(d, "scripts.get", json!({"id":script_id})).await?;
    let profiles = tools::dispatch(d, "profile.list", json!({})).await?;
    Ok(digest(&json!({"script":script,"profiles":profiles})))
}
fn error() -> RpcError {
    RpcError::new(
        ErrorCode::InvalidParams,
        "Batch approval unavailable or stale",
    )
}
/// Only local desktop RPC dispatch reaches this registration; MCP call_rpc stays exact-allowlisted.
pub fn register_rpc(d: &Dispatcher, state: Arc<AppState>) {
    let st = state.clone();
    d.register("mcp.batchPending", move |_| {
        let st = st.clone();
        async move {
            let plans = st.mcp_batches.plans.lock().await;
            let mut pending: Vec<_> = plans
                .values()
                .filter(|p| p.status == "pending" && p.expires > Instant::now())
                .collect();
            pending.sort_by(|a, b| {
                a.created
                    .cmp(&b.created)
                    .then_with(|| a.public["id"].as_str().cmp(&b.public["id"].as_str()))
            });
            Ok(json!(pending
                .into_iter()
                .map(|p| p.public.clone())
                .collect::<Vec<_>>()))
        }
    });
    d.register("mcp.batchApprove", move |args| {
        let st = state.clone();
        async move {
            let id = args["id"].as_str().ok_or_else(error)?;
            let accepted = args["approved"].as_bool().ok_or_else(error)?;
            let mut plans = st.mcp_batches.plans.lock().await;
            let p = plans.get_mut(id).ok_or_else(error)?;
            if p.status != "pending"
                || p.expires <= Instant::now()
                || args["digest"] != p.public["digest"]
            {
                return Err(error());
            }
            if accepted {
                p.approved = true;
                p.status = "approved".into();
            } else {
                p.cancel.cancel();
                p.status = "cancelled".into();
            }
            audit(id, &p.status);
            Ok(json!({"status":p.status}))
        }
    });
}
fn audit(id: &str, event: &str) {
    tracing::info!(batch_id = id, event = event, "MCP batch lifecycle");
}
pub fn public_result(value: Value) -> Value {
    match tools::redact(value) {
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(k, v)| {
                    let v = if matches!(k.as_str(), "stdout" | "stderr") {
                        json!(sanitize(v.as_str().unwrap_or_default()))
                    } else {
                        public_result(v)
                    };
                    (k, v)
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.into_iter().map(public_result).collect()),
        v => v,
    }
}
fn sanitize(text: &str) -> String {
    let mut private_block = false;
    text.lines()
        .take(2000)
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if lower.contains("-----begin") {
                private_block = true;
            }
            if private_block {
                if lower.contains("-----end") {
                    private_block = false;
                }
                return "[redacted]".to_owned();
            }
            if [
                "password",
                "passwd",
                "passphrase",
                "token",
                "secret",
                "private",
                "authorization",
                "bearer",
                "api_key",
                "apikey",
                "credential",
                "-----begin",
            ]
            .iter()
            .any(|s| lower.contains(s))
            {
                "[redacted]".to_owned()
            } else {
                line.chars()
                    .filter(|c| !c.is_control() || *c == '\t')
                    .take(4096)
                    .collect()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
pub async fn call(
    name: &str,
    args: Value,
    d: Arc<Dispatcher>,
    state: Arc<AppState>,
    grant: ExecutionGrant,
) -> Result<Value, String> {
    tools::validate_args(name, &args)?;
    if name == "batch_plan" {
        let script_id = args["script_id"].as_str().ok_or("script_id required")?;
        let ids = args["profile_ids"]
            .as_array()
            .ok_or("profile_ids required")?;
        let before = fingerprint(&d, script_id).await?;
        let script = tools::dispatch(&d, "scripts.get", json!({"id":script_id})).await?;
        let mut profile_ids = Vec::new();
        let mut targets = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for id in ids {
            let id = id.as_str().ok_or("invalid profile id")?;
            if !seen.insert(id) {
                return Err("duplicate target".into());
            }
            let p = tools::dispatch(
                &d,
                "scripts.preview",
                json!({"id":script_id,"profile_id":id}),
            )
            .await?;
            profile_ids.push(id.to_owned());
            targets.push(json!({"profile_id":id,"target":p["target"]}));
        }
        if fingerprint(&d, script_id).await? != before {
            return Err("script or targets changed; plan again".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let command = &script["command"];
        let public = json!({"id":id,"script_id":script_id,"script_name":script["name"],"command":command,"scriptDigest":format!("{:x}",Sha256::digest(command.as_str().unwrap_or_default().as_bytes())),"targets":targets,"digest":before,"concurrency":args["concurrency"].as_u64().unwrap_or(2),"expiresInSeconds":60});
        let mut plans = state.mcp_batches.plans.lock().await;
        plans.retain(|_, p| p.expires > Instant::now() || p.status == "running");
        if plans.len() >= 32 {
            return Err("too many batches".into());
        }
        plans.insert(
            id.clone(),
            Plan {
                public: public.clone(),
                fingerprint: before,
                profile_ids,
                created: Instant::now(),
                approved: false,
                expires: Instant::now() + Duration::from_secs(60),
                cancel: CancellationToken::new(),
                status: "pending".into(),
                results: Vec::new(),
                active: Vec::new(),
                concurrency: args["concurrency"].as_u64().unwrap_or(2) as usize,
            },
        );
        audit(&id, "planned");
        // The command is shown only in the local UI; clients receive a digest and target snapshot.
        let mut result = public;
        result.as_object_mut().unwrap().remove("command");
        return Ok(result);
    }
    let id = args["id"].as_str().ok_or("id required")?.to_owned();
    if name == "batch_execute" {
        let mut plans = state.mcp_batches.plans.lock().await;
        let p = plans.get_mut(&id).ok_or("batch not found")?;
        if !p.approved
            || p.status != "approved"
            || p.expires <= Instant::now()
            || p.cancel.is_cancelled()
        {
            return Err("visible local approval required or expired".into());
        }
        let script_id = p.public["script_id"].as_str().ok_or("script missing")?;
        if fingerprint(&d, script_id).await? != p.fingerprint {
            p.approved = false;
            p.status = "invalidated".into();
            return Err("script or targets changed; plan and approve again".into());
        }
        p.approved = false;
        p.status = "running".into();
        let profile_ids = p.profile_ids.clone();
        let script_id = script_id.to_owned();
        let expected = p.fingerprint.clone();
        let cancel = p.cancel.clone();
        let concurrency = p.concurrency;
        drop(plans);
        let st = state.clone();
        let batch_id = id.clone();
        tokio::spawn(async move {
            execute(
                st,
                d,
                batch_id,
                ApprovedExecution {
                    profile_ids,
                    cancel,
                    concurrency,
                    script_id,
                    expected,
                    grant,
                },
            )
            .await;
        });
        audit(&id, "started");
        return Ok(json!({"id":id,"status":"running"}));
    }
    let mut plans = state.mcp_batches.plans.lock().await;
    let p = plans.get_mut(&id).ok_or("batch not found")?;
    if name == "batch_cancel" {
        p.cancel.cancel();
        p.approved = false;
        if p.status != "running" {
            p.status = "cancelled".into();
        }
        audit(&id, "cancel_requested");
    }
    Ok(
        json!({"id":id,"status":p.status,"completed":p.results.len(),"total":p.public["targets"].as_array().map_or(0,Vec::len),"results":if name=="batch_results"{json!(p.results)}else{Value::Null}}),
    )
}
struct ApprovedExecution {
    profile_ids: Vec<String>,
    cancel: CancellationToken,
    concurrency: usize,
    script_id: String,
    expected: String,
    grant: ExecutionGrant,
}
async fn execute(
    state: Arc<AppState>,
    d: Arc<Dispatcher>,
    id: String,
    execution: ApprovedExecution,
) {
    let ApprovedExecution {
        profile_ids,
        cancel,
        concurrency,
        script_id,
        expected,
        grant,
    } = execution;
    let watch_cancel = cancel.clone();
    let watch_state = state.clone();
    let watch_grant = grant.clone();
    let watcher = tokio::spawn(async move {
        let grant = watch_grant;
        loop {
            if !grant.valid(&watch_state).await {
                watch_cancel.cancel();
                break;
            }
            tokio::select! { biased;
                _ = watch_cancel.cancelled() => break,
                _ = grant.generation.cancelled() => { watch_cancel.cancel(); break; },
                _ = grant.shutdown.cancelled() => { watch_cancel.cancel(); break; },
                _ = tokio::time::sleep(Duration::from_millis(50)) => {}
            }
        }
    });
    let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let mut jobs = tokio::task::JoinSet::new();
    for profile_id in profile_ids {
        let script_id = script_id.clone();
        let expected = expected.clone();
        let grant = grant.clone();
        let st = state.clone();
        let d = d.clone();
        let id = id.clone();
        let cancel = cancel.clone();
        let semaphore = semaphore.clone();
        jobs.spawn(async move {
            let permit=tokio::select!{biased; _=cancel.cancelled()=>return, p=semaphore.acquire_owned()=>p};
            let Ok(_permit)=permit else{return;};
            if cancel.is_cancelled() || !grant.valid(&st).await {cancel.cancel();return;}
            let result=async {
                if fingerprint(&d,&script_id).await? != expected { cancel.cancel(); return Err("Batch snapshot changed".into()); }
                if cancel.is_cancelled() || !grant.valid(&st).await { cancel.cancel(); return Err("Batch authorization revoked".into()); }
                let prepared=tools::dispatch(&d,"scripts.prepare",json!({"id":script_id,"profile_id":profile_id})).await?;
                if fingerprint(&d,&script_id).await? != expected { cancel.cancel(); return Err("Batch snapshot changed".into()); }
                if cancel.is_cancelled() || !grant.valid(&st).await { cancel.cancel(); return Err("Batch authorization revoked".into()); }
                let task=tools::dispatch(&d,"scripts.run",json!({"token":prepared["token"]})).await?;
                let task_id=task["id"].as_str().ok_or("task missing")?.to_owned();
                if let Some(p)=st.mcp_batches.plans.lock().await.get_mut(&id){p.active.push(task_id.clone());}
                loop {
                    if cancel.is_cancelled() {return tools::dispatch(&d,"scripts.cancel",json!({"id":task_id})).await;}
                    let status=tokio::select! { biased;
                        _=cancel.cancelled()=>return tools::dispatch(&d,"scripts.cancel",json!({"id":task_id})).await,
                        status=tools::dispatch(&d,"scripts.status",json!({"id":task_id}))=>status?,
                    };
                    if status["status"]!="running" {return Ok(status);}
                    tokio::select!{_=cancel.cancelled()=>{},_=tokio::time::sleep(Duration::from_millis(150))=>{}}
                }
            }.await;
            let value=public_result(result.unwrap_or_else(|_|json!({"status":"failed","error":"Script task failed; check local diagnostics"})));
            if let Some(p)=st.mcp_batches.plans.lock().await.get_mut(&id){p.results.push(value);}
        });
    }
    while jobs.join_next().await.is_some() {}
    watcher.abort();
    if let Some(p) = state.mcp_batches.plans.lock().await.get_mut(&id) {
        p.status = if cancel.is_cancelled() {
            "cancelled"
        } else {
            "completed"
        }
        .into();
        audit(&id, &p.status);
    }
}
