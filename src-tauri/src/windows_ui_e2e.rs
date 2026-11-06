//! Debug-only Windows acceptance command; never compiled into release builds.
use super::*;
use std::sync::Mutex;
static CHANNELS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static CALLS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
// osVisible is sampled using IsWindowVisible, not only desired UI state.
static EVENTS: Mutex<Vec<serde_json::Value>> = Mutex::new(Vec::new());
static FAULT: Mutex<Option<(String, String)>> = Mutex::new(None);
pub(super) fn boundary(phase: &str, id: uuid::Uuid) {
    if !enabled() {
        return;
    }
    let host = NativeHost::state(id).ok();
    let mut events = EVENTS.lock().unwrap();
    let order = events.len() + 1;
    // before-hide / after-hide / before-detach are recorded on the HWND thread.
    events.push(serde_json::json!({"order":order,"phase":phase,"sessionId":id,"host":host}));
}
pub(super) async fn snapshot_fault(
    window: &tauri::WebviewWindow,
    frame: &serde_json::Value,
) -> bool {
    if !enabled() || frame["method"] != "terminal.snapshotEnvelope" {
        return false;
    }
    let fault = FAULT.lock().unwrap().clone();
    let Some((session, generation)) = fault else {
        return false;
    };
    if frame["params"]["id"] != session {
        return false;
    }
    let Ok(id) = uuid::Uuid::parse_str(&session) else {
        return false;
    };
    let matches = native_main(window.app_handle().clone(), move || NativeHost::state(id))
        .await
        .is_ok_and(|s| s.generation.to_string() == generation && s.visible);
    if matches {
        FAULT.lock().unwrap().take();
    }
    matches
}
fn enabled() -> bool {
    std::env::var("AEROTAB_WINDOWS_NATIVE_UI_E2E").as_deref() == Ok("1")
}
pub(super) fn observe_channel(id: &str) {
    if enabled() {
        CHANNELS.lock().unwrap().push(id.into());
    }
}
pub(super) fn observe_rpc(frame: &serde_json::Value) {
    if let (Some(id), Some(method)) = (frame["params"]["id"].as_str(), frame["method"].as_str()) {
        if matches!(
            method,
            "session.write" | "terminal.attach" | "terminal.detach" | "session.close"
        ) {
            CALLS.lock().unwrap().push((id.into(), method.into()));
        }
    }
}
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(super) async fn windows_native_e2e(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppRpc>,
    action: String,
    session_id: Option<String>,
    generation: Option<String>,
    text: Option<String>,
    request_id: Option<u32>,
    report: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    if window.label() != "main" || !enabled() {
        return Err("Windows debug E2E disabled".into());
    }
    if action == "report" {
        let report = report.ok_or("report missing")?;
        let path =
            std::env::var_os("AEROTAB_WINDOWS_NATIVE_UI_REPORT").ok_or("report path missing")?;
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        return Ok(serde_json::json!({"saved":true}));
    }
    if action == "receipt" {
        let mut events = EVENTS.lock().unwrap();
        let order = events.len() + 1;
        events.push(serde_json::json!({"order":order,"phase":"target-receipt","receipt":report}));
        return Ok(serde_json::json!({}));
    }
    if action == "events" {
        return Ok(
            serde_json::json!({"events":EVENTS.lock().unwrap().clone(),"faultPending":FAULT.lock().unwrap().is_some()}),
        );
    }
    if action == "clear-events" {
        EVENTS.lock().unwrap().clear();
        return Ok(serde_json::json!({}));
    }
    let session = session_id.ok_or("session missing")?;
    let id = uuid::Uuid::parse_str(&session).map_err(|e| e.to_string())?;
    let engine = state.state.terminal_engines.get(id).ok_or("core missing")?;
    if action == "state" {
        let calls = CALLS.lock().unwrap();
        return Ok(
            serde_json::json!({"coreIdentity":Arc::as_ptr(&engine) as usize,"channelRegistrations":CHANNELS.lock().unwrap().iter().filter(|s|*s==&session).count(),"attachCount":calls.iter().filter(|(s,m)|s==&session&&m=="terminal.attach").count(),"detachOrCloseCount":calls.iter().filter(|(s,m)|s==&session&&(m=="terminal.detach"||m=="session.close")).count(),"writeCount":calls.iter().filter(|(s,m)|s==&session&&m=="session.write").count(),"markers":engine.buffer_text().matches("WINDOWS_INPUT_ACK_OK").count(),"pasteMarkers":engine.buffer_text().lines().filter(|s|s.trim()=="PASTE_WINDOWS_OK").count()}),
        );
    }
    let generation = uuid::Uuid::parse_str(&generation.ok_or("generation missing")?)
        .map_err(|e| e.to_string())?;
    if action == "fault" {
        *FAULT.lock().unwrap() = Some((session, generation.to_string()));
        return Ok(serde_json::json!({"armed":true}));
    }
    if action == "os-state" {
        return native_main(app, move || NativeHost::debug_windows_state(id))
            .await
            .map_err(|e| format!("{e:?}"));
    }
    if action == "paste" {
        use tauri::Emitter;
        window.emit_to(tauri::EventTarget::webview_window("main"),"aerotab:native-paste-request",serde_json::json!({"sessionId":id,"generation":generation,"requestId":request_id.ok_or("request id missing")?,"text":text.ok_or("paste text missing")?})).map_err(|e|e.to_string())?;
        return Ok(serde_json::json!({"emitted":true,"synthetic":true}));
    }
    if action != "input" {
        return Err("unknown action".into());
    }
    let text = text.ok_or("text missing")?;
    if text.len() > 4096 {
        return Err("test input too large".into());
    }
    let result = native_main(app, move || {
        NativeHost::debug_windows_messages(id, generation, &text)
    })
    .await
    .map_err(|e| format!("{e:?}"))?;
    serde_json::to_value(result).map_err(|e| e.to_string())
}
pub(super) fn start(app: &tauri::AppHandle) {
    if !enabled() {
        return;
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.eval(include_str!("windows_ui_e2e.js"));
    }
}
