//! Tauri 2 desktop shell.
//!
//! This binary embeds the same JSON-RPC [`Dispatcher`] used by the stdio
//! host (`aerotab`), and exposes it to the webview through a single
//! `invoke('rpc', { frame })` command. That keeps every call site, stdio
//! tests, bench harness, and the live UI, going through the same code path.

// Release Windows: GUI subsystem only (no extra console window with stderr logs).
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::{
    io::SeekFrom,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use tauri::Emitter;

static FILE_DIALOG_GUARD: Mutex<()> = Mutex::new(());

fn lock_file_dialog() -> Result<std::sync::MutexGuard<'static, ()>, String> {
    FILE_DIALOG_GUARD
        .lock()
        .map_err(|_| "file dialog is busy".to_string())
}

#[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
use aerotab_core::commands::set_parent_hwnd;
use aerotab_core::commands::{register_all, set_app_handle, AppState};
use aerotab_core::desktop_session::{self, DesktopSessionInfo};
use aerotab_core::ipc::{Dispatcher, ErrorCode, Request, Response, RpcError};
use aerotab_core::settings::SettingsStore;
use aerotab_core::CORE_VERSION;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_updater::UpdaterExt;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tracing_subscriber::EnvFilter;

#[cfg(all(debug_assertions, feature = "native-view", target_os = "windows"))]
#[path = "../windows_ui_e2e.rs"]
mod windows_ui_e2e;
#[cfg(all(debug_assertions, feature = "native-view", target_os = "windows"))]
use windows_ui_e2e::windows_native_e2e;
const MAIN_WINDOW_LABEL: &str = "main";
const TRAY_ID: &str = "main";
const TRAY_SHOW_ID: &str = "tray-show";
const TRAY_HIDE_ID: &str = "tray-hide";
const TRAY_QUIT_ID: &str = "tray-quit";

struct AppRpc {
    dispatcher: Arc<Dispatcher>,
    state: Arc<AppState>,
    // Retain the lifecycle owner for the lifetime of the desktop application.
    _mcp: Arc<aerotab_core::mcp::McpRuntime>,
}

struct DesktopWindowState {
    settings: Option<SettingsStore>,
    tray_available: bool,
    close_authorized: std::sync::atomic::AtomicBool,
}

#[derive(Debug, Clone, Copy, Default)]
struct WindowBehaviorSettings {
    tray_enabled: bool,
    tray_minimize_to_tray: bool,
}

impl DesktopWindowState {
    fn window_behavior(&self) -> WindowBehaviorSettings {
        self.settings
            .as_ref()
            .and_then(|settings| settings.get("window").ok().flatten())
            .map(parse_window_behavior)
            .unwrap_or_default()
    }
}

/// Disable WebView2's built-in context menu (Back / Refresh / Print, etc.).
#[cfg(target_os = "windows")]
fn disable_native_webview_context_menus(window: &tauri::WebviewWindow) {
    let _ = window.with_webview(|platform| {
        let controller = platform.controller();
        if let Ok(core) = unsafe { controller.CoreWebView2() } {
            if let Ok(settings) = unsafe { core.Settings() } {
                let _ = unsafe { settings.SetAreDefaultContextMenusEnabled(false) };
            }
        }
    });
}

#[cfg(not(target_os = "windows"))]
fn disable_native_webview_context_menus(_window: &tauri::WebviewWindow) {}

fn parse_window_behavior(value: serde_json::Value) -> WindowBehaviorSettings {
    let mut out = WindowBehaviorSettings::default();
    let Some(obj) = value.as_object() else {
        return out;
    };
    out.tray_enabled = obj
        .get("trayEnabled")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    out.tray_minimize_to_tray = obj
        .get("trayMinimizeToTray")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    out
}

/// Reveal/focus the main window (also called from the frontend after first paint).
#[tauri::command]
fn show_main_window(app: tauri::AppHandle) -> Result<(), String> {
    reveal_main_window(&app)
}

/// Convert a DOM rect (CSS pixels) to physical screen coordinates for native
/// terminal embedding. The frontend sends `{ x, y, width, height,
/// devicePixelRatio }` from `getBoundingClientRect()`, and we return the
/// adjusted screen position accounting for the window's own top-left and DPI.
#[tauri::command]
fn get_window_screen_rect(
    app: tauri::AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    device_pixel_ratio: f64,
) -> Result<serde_json::Value, String> {
    let window = app
        .get_webview_window(MAIN_WINDOW_LABEL)
        .ok_or_else(|| "main window not found".to_string())?;
    let outer = window.outer_position().map_err(|e| e.to_string())?;
    let scale = window.scale_factor().unwrap_or(device_pixel_ratio);

    Ok(serde_json::json!({
        "x": outer.x as f64 + x * scale,
        "y": outer.y as f64 + y * scale,
        "width": width * scale,
        "height": height * scale,
        "scale": scale,
    }))
}

/// Returns the raw native window handle of the main Tauri window, cast to
/// `usize`. On Windows this is the `HWND` (isize → usize), on X11 the
/// `x11_window::Window` ID, and 0 on Wayland/macOS.
#[tauri::command]
fn get_main_window_hwnd(app: tauri::AppHandle) -> Result<usize, String> {
    #[cfg(windows)]
    {
        let window = app
            .get_webview_window(MAIN_WINDOW_LABEL)
            .ok_or_else(|| "main window not found".to_string())?;
        let hwnd = window.hwnd().map_err(|e| e.to_string())?;
        Ok(hwnd.0 as usize)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // On X11, return the X11 window ID if available
        let window = app
            .get_webview_window(MAIN_WINDOW_LABEL)
            .ok_or_else(|| "main window not found".to_string())?;
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(wh) = window.window_handle() {
            if let RawWindowHandle::Xlib(handle) = wh.as_raw() {
                return Ok(handle.window as usize);
            }
        }
        Ok(0)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        Ok(0)
    }
}

fn reveal_main_window(app: &tauri::AppHandle) -> Result<(), String> {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return Ok(());
    };
    if !window.is_visible().unwrap_or(true) {
        window.show().map_err(|e| e.to_string())?;
    }
    let _ = window.set_focus();
    Ok(())
}

fn query_safe(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        .collect()
}

#[tauri::command]
fn open_file_transfer_window(
    app: tauri::AppHandle,
    profile_id: Option<String>,
) -> Result<(), String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let label = format!("file-transfer-{stamp}");
    let mut path = "index.html?view=file-transfer".to_string();
    if let Some(profile_id) = profile_id.as_deref() {
        let safe = query_safe(profile_id);
        if !safe.is_empty() {
            path.push_str("&profileId=");
            path.push_str(&safe);
        }
    }
    let window = WebviewWindowBuilder::new(&app, label, WebviewUrl::App(path.into()))
        .title("AeroTab File Transfer")
        .inner_size(1280.0, 820.0)
        .min_inner_size(900.0, 560.0)
        .resizable(true)
        .disable_drag_drop_handler()
        .build()
        .map_err(|e| e.to_string())?;
    disable_native_webview_context_menus(&window);
    let _ = window.set_focus();
    Ok(())
}

#[tauri::command]
fn authorize_main_window_close(window: tauri::WebviewWindow) -> Result<(), String> {
    window
        .state::<DesktopWindowState>()
        .close_authorized
        .store(true, std::sync::atomic::Ordering::Release);
    window.close().map_err(|e| e.to_string())
}

#[tauri::command]
fn close_current_window(window: tauri::WebviewWindow) -> Result<(), String> {
    window.close().map_err(|e| e.to_string())
}

#[tauri::command]
async fn rpc(
    _window: tauri::WebviewWindow,
    state: tauri::State<'_, AppRpc>,
    frame: serde_json::Value,
) -> Result<serde_json::Value, String> {
    #[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
    if std::env::var("AEROTAB_NATIVE_UI_E2E").as_deref() == Ok("1")
        && frame["method"] == "session.write"
    {
        E2E_WRITE_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    #[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
    if std::env::var("AEROTAB_NATIVE_UI_E2E").as_deref() == Ok("1")
        && matches!(
            frame["method"].as_str(),
            Some("terminal.attach" | "terminal.detach" | "session.close")
        )
    {
        E2E_CORE_CALLS.lock().unwrap().push((
            frame["params"]["id"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            frame["method"].as_str().unwrap_or_default().to_owned(),
        ));
    }
    #[cfg(all(debug_assertions, feature = "native-view", target_os = "windows"))]
    if std::env::var("AEROTAB_WINDOWS_NATIVE_UI_E2E").as_deref() == Ok("1") {
        windows_ui_e2e::observe_rpc(&frame);
    }
    #[cfg(all(debug_assertions, feature = "native-view", target_os = "windows"))]
    if windows_ui_e2e::snapshot_fault(&_window, &frame).await {
        return Ok(
            serde_json::json!({"jsonrpc":"2.0","id":frame["id"],"error":{"code":-32603,"message":"debug Windows reverse target snapshot failure"}}),
        );
    }
    let req: Request = match serde_json::from_value(frame) {
        Ok(r) => r,
        Err(e) => {
            let resp = Response {
                jsonrpc: "2.0".into(),
                id: None,
                result: None,
                error: Some(RpcError::new(ErrorCode::ParseError, e.to_string())),
            };
            return serde_json::to_value(resp).map_err(|e| e.to_string());
        }
    };
    #[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
    let observe_envelope = std::env::var("AEROTAB_NATIVE_UI_E2E").as_deref() == Ok("1")
        && req.method == "terminal.snapshotEnvelope";
    #[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
    let snapshot_identity = (
        req.params["id"].as_str().unwrap_or_default().to_owned(),
        req.id
            .as_ref()
            .is_some_and(|id| *id != serde_json::json!(90001)),
    );
    #[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
    if req.method == "session.write" && std::env::var("AEROTAB_NATIVE_UI_E2E").as_deref() == Ok("1")
    {
        let target = E2E_ACK_BARRIER
            .lock()
            .unwrap()
            .as_ref()
            .map(|b| (b.0, b.1, b.2));
        if let Some(target) = target {
            let session = req.params["id"]
                .as_str()
                .and_then(|s| uuid::Uuid::parse_str(s).ok());
            if session == Some(target.0) {
                let flow = native_main(_window.app_handle().clone(), move || {
                    NativeHost::debug_e2e_flow(target.0)
                })
                .await
                .ok();
                let generation = flow
                    .as_ref()
                    .and_then(|f| f["generation"].as_str())
                    .and_then(|s| uuid::Uuid::parse_str(s).ok());
                if ack_barrier_matches(true, target, target.0, generation, target.2 + 1) {
                    let receiver = E2E_ACK_BARRIER.lock().unwrap().take().map(|b| b.3);
                    if let Some(receiver) = receiver {
                        E2E_REVERSE_EVENTS.lock().unwrap().push(serde_json::json!({"event":"real writer blocked before PTY dispatch","sessionId":target.0,"generation":target.1}));
                        if wait_writer_barrier(Some(receiver), std::time::Duration::from_secs(4))
                            .await
                            .is_err()
                        {
                            E2E_ACK_RELEASE.lock().unwrap().take();
                            return Err("test writer barrier timeout".into());
                        }
                    }
                }
            }
        }
    }
    let resp = state.dispatcher.dispatch(req).await;
    let value = serde_json::to_value(resp).map_err(|e| e.to_string())?;
    #[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
    if observe_envelope {
        let result = &value["result"];
        E2E_WEBVIEW_CAPTURES.lock().unwrap().push(serde_json::json!({
            "version":result["version"],"frameVersion":result["frameVersion"],
            "outputSeq":result["outputSeq"],"error":value["error"],
            "observedAtMs":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis(),
            "scope":"RPC capture only; renderer ticket is not exposed"
        }));
    }
    #[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
    if observe_envelope {
        let session = snapshot_identity.0;
        let generation = if let Ok(id) = uuid::Uuid::parse_str(&session) {
            native_main(_window.app_handle().clone(), move || {
                NativeHost::debug_e2e_flow(id)
            })
            .await
            .ok()
            .and_then(|v| v["generation"].as_str().map(str::to_owned))
            .unwrap_or_default()
        } else {
            String::new()
        };
        if E2E_SNAPSHOT_FAULT.lock().unwrap().consume(
            true,
            &session,
            &generation,
            snapshot_identity.1,
            &value,
        ) {
            return Ok(
                serde_json::json!({"jsonrpc":"2.0","id":value["id"],"error":{"code":-32603,"message":"debug retained reverse snapshot failure after capture"}}),
            );
        }
    }
    Ok(value)
}

/// Connect a native terminal engine (created via RPC `terminal.attach`) to a
/// Tauri channel. Every encoded frame is pushed as raw bytes (`ArrayBuffer`
/// on the JS side). Passing a new channel replaces the previous sink.
#[tauri::command]
async fn terminal_frames(
    state: tauri::State<'_, AppRpc>,
    session_id: String,
    on_frame: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
) -> Result<(), String> {
    #[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
    if std::env::var("AEROTAB_NATIVE_UI_E2E").as_deref() == Ok("1") {
        E2E_CHANNELS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        E2E_SESSION_CHANNELS
            .lock()
            .unwrap()
            .push(session_id.clone());
    }
    #[cfg(all(debug_assertions, feature = "native-view", target_os = "windows"))]
    windows_ui_e2e::observe_channel(&session_id);
    let id = uuid::Uuid::parse_str(&session_id).map_err(|e| e.to_string())?;
    let engine = state
        .state
        .terminal_engines
        .get(id)
        .ok_or_else(|| format!("no terminal engine for {id}"))?;
    engine.set_sink(Some(Arc::new(move |bytes: Vec<u8>| {
        let _ = on_frame.send(tauri::ipc::InvokeResponseBody::Raw(bytes));
    })));
    Ok(())
}

#[cfg(feature = "native-view")]
use aerotab_core::terminal::native_host::{Geometry, HostError, NativeHost, ViewConfig, ViewState};
#[cfg(feature = "native-view")]
async fn native_main<T: Send + 'static>(
    app: tauri::AppHandle,
    work: impl FnOnce() -> Result<T, HostError> + Send + 'static,
) -> Result<T, HostError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(work());
    })
    .map_err(|e| HostError::new("main_thread", e.to_string()))?;
    rx.await
        .map_err(|e| HostError::new("main_thread", e.to_string()))?
}
#[tauri::command]
fn native_view_capabilities(
    window: tauri::WebviewWindow,
) -> Result<aerotab_core::terminal::native_view_capabilities::Capabilities, String> {
    if window.label() != "main" {
        return Err("only main window may inspect native views".into());
    }
    Ok(aerotab_core::terminal::native_view_capabilities::policy(
        cfg!(feature = "native-view"),
        std::env::consts::OS,
    ))
}
#[cfg(feature = "native-view")]
fn native_caller(window: &tauri::WebviewWindow) -> Result<(), HostError> {
    if window.label() != "main" {
        return Err(HostError::new(
            "permission_denied",
            "only main window may manage native views",
        ));
    }
    Ok(())
}
#[cfg(feature = "native-view")]
#[tauri::command]
async fn native_view_attach(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppRpc>,
    session_id: String,
    geometry: Geometry,
    cfg: ViewConfig,
) -> Result<ViewState, HostError> {
    native_caller(&window)?;
    geometry.validate()?;
    cfg.validate()?;
    let id = uuid::Uuid::parse_str(&session_id)
        .map_err(|e| HostError::new("invalid_session", e.to_string()))?;
    let engine = state.state.terminal_engines.get(id).ok_or_else(|| {
        HostError::new(
            "engine_missing",
            "call terminal.attach for existing session first",
        )
    })?;
    let dispatcher = state.dispatcher.clone();
    let dispatch = Arc::new(move |method: &'static str, params: serde_json::Value| {
        let dispatcher = dispatcher.clone();
        tauri::async_runtime::spawn(async move {
            let response = dispatcher
                .dispatch(Request {
                    jsonrpc: "2.0".into(),
                    id: Some(serde_json::json!("native-view")),
                    method: method.into(),
                    params,
                })
                .await;
            if let Some(error) = response.error {
                tracing::warn!(?error, "native input dispatch failed");
            }
        });
    });
    #[cfg(all(debug_assertions, target_os = "linux"))]
    if std::env::var("AEROTAB_NATIVE_UI_E2E").as_deref() == Ok("1") {
        window.eval("window.__stagingEvidence??=[]; window.__stagingEvidence.push({phase:'before actual host attach',at:Date.now(),canvas:!!window.__stagingSourceCanvas?.isConnected,sourceVisible:!!window.__stagingSourceCanvas&&!window.__stagingSourceCanvas.hidden,backend:document.querySelector('[data-terminal-view-backend]')?.dataset.terminalViewBackend});").map_err(|e|HostError::new("harness_eval",e.to_string()))?;
        if E2E_FAIL_ATTACH.swap(false, std::sync::atomic::Ordering::SeqCst) {
            return Err(HostError::new(
                "harness_target_failure",
                "debug-only injected target attach failure",
            ));
        }
    }
    native_main(app, move || {
        NativeHost::attach(&window, id, geometry, cfg, engine, dispatch)
    })
    .await
}
#[cfg(feature = "native-view")]
#[tauri::command]
async fn native_view_update(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    session_id: String,
    geometry: Geometry,
    visible: bool,
    cfg: Option<ViewConfig>,
) -> Result<ViewState, HostError> {
    native_caller(&window)?;
    let id = uuid::Uuid::parse_str(&session_id)
        .map_err(|e| HostError::new("invalid_session", e.to_string()))?;
    #[cfg(all(debug_assertions, target_os = "linux"))]
    let handshake = cfg
        .as_ref()
        .map(|c| (c.input_generation, c.input_ready, c.input_ack));
    #[cfg(all(debug_assertions, target_os = "linux"))]
    let observed_cfg = cfg.clone();
    #[cfg(all(debug_assertions, target_os = "linux"))]
    let mut observed_transaction = serde_json::Value::Null;
    #[cfg(all(debug_assertions, target_os = "linux"))]
    if !visible && std::env::var("AEROTAB_NATIVE_UI_E2E").as_deref() == Ok("1") {
        let (tx, rx) = tokio::sync::oneshot::channel();
        *E2E_HIDE_REPLY.lock().unwrap() = Some(tx);
        window.eval("(()=>{const p=window.__reverseBoundaryProbe;const i=document.querySelector('[data-terminal-view-backend] textarea');const armed=!!p?.command&&!p.inputSent;if(armed){p.inputSent=true;if(!i||i.disabled)throw Error('target handoff input disabled');i.value=p.command;i.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertText',data:p.command}));}window.__TAURI_INTERNALS__.invoke('native_ui_e2e_report',{report:{boundaryReply:true,armed,label:p?.label,transaction:p?{sessionId:p.sessionId,sourceGeneration:p.generation,epoch:p.epoch,phase:p.label}:null,receipts:p?.receipts||[]}})})()").map_err(|e|HostError::new("harness_eval",e.to_string()))?;
        let reply = tokio::time::timeout(std::time::Duration::from_secs(3), rx)
            .await
            .map_err(|_| HostError::new("harness_timeout", "hide input receipt missing"))?
            .map_err(|_| HostError::new("harness_closed", "hide reply closed"))?;
        observed_transaction = reply["transaction"].clone();
        *E2E_REVERSE_TRANSACTION.lock().unwrap() = observed_transaction.clone();
        if reply["armed"] == true {
            E2E_REVERSE_EVENTS.lock().unwrap().push(serde_json::json!({"event":"target input dispatched before real GTK hide","reply":reply}));
        }
    }
    #[cfg(all(debug_assertions, target_os = "linux"))]
    if std::env::var("AEROTAB_NATIVE_UI_E2E").as_deref() == Ok("1") {
        if let Some(cfg) = &cfg {
            let mut fault = E2E_ACK_FAULT.lock().unwrap();
            if let Some((session, generation, baseline, rejects)) = fault.as_mut() {
                if *session == id
                    && cfg.input_generation == Some(*generation)
                    && cfg.input_ack > *baseline
                {
                    *rejects += 1;
                    return Err(HostError::new(
                        "e2e_ack_rejected",
                        "controlled ACK-only rejection before host apply",
                    ));
                }
            }
        }
    }
    let result = native_main(app, move || {
        #[cfg(all(debug_assertions, target_os = "linux"))]
        let observe = std::env::var("AEROTAB_NATIVE_UI_E2E").as_deref() == Ok("1");
        #[cfg(all(debug_assertions, target_os = "linux"))]
        let before = observe.then(|| NativeHost::state(id).ok());
        #[cfg(all(debug_assertions, target_os = "linux"))]
        let flow_before = observe.then(|| NativeHost::debug_e2e_flow(id).ok());
        #[cfg(all(debug_assertions, target_os="windows"))]
        if !visible {windows_ui_e2e::boundary("before-hide",id);}
        let result = NativeHost::update(id, geometry, visible, cfg);
        #[cfg(all(debug_assertions, target_os="windows"))]
        { if !visible {windows_ui_e2e::boundary("after-hide",id);} else {windows_ui_e2e::boundary("after-update",id);} }
        #[cfg(all(debug_assertions, target_os = "linux"))]
        if observe && !visible {
            let after=NativeHost::state(id).ok();
            let flow_after=NativeHost::debug_e2e_flow(id).ok();
            NativeHost::debug_e2e_late(id)?;
            if let Some(tx)=E2E_ACK_RELEASE.lock().unwrap().take() {
                E2E_REVERSE_EVENTS.lock().unwrap().push(serde_json::json!({"event":"writer barrier released after actual hide","flow":flow_after}));
                let _=tx.send(());
            }
            E2E_REVERSE_EVENTS.lock().unwrap().push(serde_json::json!({"event":"actual GTK hide completed","transaction":observed_transaction,"before":before,"after":after,"flowBefore":flow_before,"flowAfter":flow_after,"afterAdversarialOldWidget":NativeHost::debug_e2e_flow(id).ok()}));
        }
        #[cfg(all(debug_assertions, target_os = "linux"))]
        if observe
            && result
                .as_ref()
                .err()
                .is_some_and(|e| e.code == "config_revision")
        {
            E2E_STALE_OBSERVATIONS.lock().unwrap().push(serde_json::json!({
                "before":before.flatten(),"after":NativeHost::state(id).ok(),
                "flowBefore":flow_before.flatten(),"flowAfter":NativeHost::debug_e2e_flow(id).ok(),
                "error":result.as_ref().err(),"atomicGtkCallback":true
            }));
        }
        result
    })
    .await;
    #[cfg(all(debug_assertions, target_os = "linux"))]
    if std::env::var("AEROTAB_NATIVE_UI_E2E").as_deref() == Ok("1")
        && *E2E_OBSERVE_SESSION.lock().unwrap() == Some(id)
    {
        E2E_UPDATES.lock().unwrap().push(serde_json::json!({"cmd":"native_view_update","args":{"sessionId":id,"geometry":geometry,"visible":visible,"cfg":observed_cfg},"result":result.as_ref().ok()}));
    }
    #[cfg(all(debug_assertions, target_os = "linux"))]
    if std::env::var_os("AEROTAB_NATIVE_UI_E2E").is_some() {
        if let Some((Some(generation), ready, ack)) = handshake {
            eprintln!("NATIVE_UI_E2E input-handshake session={id} generation={generation} ready={ready} ack={ack} accepted={}",result.is_ok());
        }
    }
    result
}
#[cfg(feature = "native-view")]
#[tauri::command]
async fn native_view_detach(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<(), HostError> {
    native_caller(&window)?;
    let id = uuid::Uuid::parse_str(&session_id)
        .map_err(|e| HostError::new("invalid_session", e.to_string()))?;
    native_main(app, move || {
        #[cfg(all(debug_assertions, target_os = "linux"))]
        if std::env::var("AEROTAB_NATIVE_UI_E2E").as_deref()==Ok("1") {
            E2E_REVERSE_EVENTS.lock().unwrap().push(serde_json::json!({"event":"before actual source detach","transaction":E2E_REVERSE_TRANSACTION.lock().unwrap().clone(),"host":NativeHost::state(id).ok(),"flow":NativeHost::debug_e2e_flow(id).ok()}));
        }
        #[cfg(all(debug_assertions, target_os="windows"))]
        windows_ui_e2e::boundary("before-detach",id);
        NativeHost::detach(id)
    }).await
}
#[cfg(feature = "native-view")]
#[tauri::command]
async fn native_view_focus(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<(), HostError> {
    native_caller(&window)?;
    let id = uuid::Uuid::parse_str(&session_id)
        .map_err(|e| HostError::new("invalid_session", e.to_string()))?;
    native_main(app, move || NativeHost::focus(id)).await
}
#[cfg(feature = "native-view")]
#[tauri::command]
async fn native_view_state(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<ViewState, HostError> {
    native_caller(&window)?;
    let id = uuid::Uuid::parse_str(&session_id)
        .map_err(|e| HostError::new("invalid_session", e.to_string()))?;
    native_main(app, move || NativeHost::state(id)).await
}

#[cfg(all(debug_assertions, feature = "native-view", target_os = "windows"))]
fn start_windows_native_host_smoke(app: &tauri::AppHandle) {
    let Some(report_path) = std::env::var_os("AEROTAB_WINDOWS_NATIVE_HOST_SMOKE") else {
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result: Result<serde_json::Value, String> = async {
            let rpc = app.state::<AppRpc>();
            let opened = rpc.dispatcher.dispatch(Request { jsonrpc: "2.0".into(), id: Some(serde_json::json!("windows-native-smoke")), method: "session.openLocal".into(), params: serde_json::json!({"shell":"powershell.exe","shell_args":["-NoLogo","-NoProfile"],"cols":80,"rows":24}) }).await;
            if let Some(error) = opened.error { return Err(format!("openLocal: {error:?}")); }
            let id = uuid::Uuid::parse_str(opened.result.as_ref().and_then(|v|v["id"].as_str()).ok_or("session id missing")?).map_err(|e|e.to_string())?;
            let attached = rpc.dispatcher.dispatch(Request {jsonrpc:"2.0".into(),id:Some(serde_json::json!("windows-native-smoke")),method:"terminal.attach".into(),params:serde_json::json!({"id":id,"cols":80,"rows":24,"scrollback":500})}).await;
            if let Some(error)=attached.error { return Err(format!("terminal.attach: {error:?}")); }
            let written = rpc.dispatcher.dispatch(Request {jsonrpc:"2.0".into(),id:Some(serde_json::json!("windows-native-smoke")),method:"session.write".into(),params:serde_json::json!({"id":id,"data":base64::Engine::encode(&base64::engine::general_purpose::STANDARD,b"Write-Output ('PRODUCTION_' + 'WINDOWS_CORE_OK')\r")})}).await;
            if let Some(error)=written.error { return Err(format!("session.write: {error:?}")); }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let engine=rpc.state.terminal_engines.get(id).ok_or("canonical engine missing")?;
            let main=app.get_webview_window("main").ok_or("main missing")?;
            let dispatcher=rpc.dispatcher.clone();
            let dispatch=Arc::new(move |method:&'static str,params:serde_json::Value| {let d=dispatcher.clone();tauri::async_runtime::spawn(async move { let _=d.dispatch(Request {jsonrpc:"2.0".into(),id:Some(serde_json::json!("windows-native-smoke")),method:method.into(),params}).await; });});
            let view_engine=engine.clone();
            let before=native_main(app.clone(),move || NativeHost::attach(&main,id,Geometry{x:260.,y:120.,width:650.,height:380.},ViewConfig::default(),view_engine,dispatch)).await.map_err(|e|format!("attach: {e:?}"))?;
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let drawn=native_main(app.clone(),move ||NativeHost::state(id)).await.map_err(|e|format!("state: {e:?}"))?;
            if drawn.error.is_some() || drawn.drawn_revision!=Some(0) || drawn.drawn_output_seq.as_ref().and_then(|s|s.parse::<u64>().ok()).unwrap_or(0)<before.handoff_output_seq.parse::<u64>().map_err(|e|e.to_string())? { return Err(format!("draw barrier failed: {drawn:?}")); }
            let snapshot=aerotab_core::terminal::native_host::Snapshot::decode(&engine.native_snapshot_frame(),&ViewConfig::default()).map_err(|e|format!("snapshot: {e:?}"))?;
            let text:String=snapshot.cells.iter().map(|c|c.text.as_str()).collect();
            if !text.contains("PRODUCTION_WINDOWS_CORE_OK") {return Err("real PowerShell output marker missing".into());}
            let generation=before.generation;
            let hidden=native_main(app.clone(),move || NativeHost::update(id,Geometry::default(),false,Some(ViewConfig{expected_generation:Some(generation),revision:1,..ViewConfig::default()}))).await.map_err(|e|format!("hide: {e:?}"))?;
            native_main(app.clone(),move ||NativeHost::detach(id)).await.map_err(|e|format!("detach: {e:?}"))?;
            if !engine.is_alive() {return Err("detach killed canonical engine".into());}
            let _=rpc.dispatcher.dispatch(Request{jsonrpc:"2.0".into(),id:Some(serde_json::json!("windows-native-smoke")),method:"session.close".into(),params:serde_json::json!({"id":id})}).await;
            Ok(serde_json::json!({"ok":true,"proof":"production_tauri_same_core_win32_dx12_surface","before":before,"drawn":drawn,"hidden":hidden,"real_powershell_marker":true,"detach_preserves_engine":true,"input_bridge_tested":false,"full_ui_transaction_tested":false}))
        }.await;
        let ok = result.is_ok();
        let report = result.unwrap_or_else(|error| serde_json::json!({"ok":false,"error":error}));
        let _ = std::fs::write(report_path, serde_json::to_vec_pretty(&report).unwrap());
        app.exit(if ok { 0 } else { 5 });
    });
}

#[cfg(all(feature = "native-view", target_os = "linux"))]
fn start_native_host_smoke(app: &tauri::AppHandle) {
    if std::env::var_os("AEROTAB_NATIVE_HOST_SMOKE").is_none() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let rpc = app.state::<AppRpc>();
        let dispatcher = rpc.dispatcher.clone();
        let call = |method: &str, params: serde_json::Value| {
            let d = dispatcher.clone();
            let method = method.to_string();
            async move {
                let r = d
                    .dispatch(Request {
                        jsonrpc: "2.0".into(),
                        id: Some(serde_json::json!("smoke")),
                        method,
                        params,
                    })
                    .await;
                assert!(r.error.is_none(), "smoke RPC: {:?}", r.error);
                r.result.unwrap_or_default()
            }
        };
        let opened = call(
            "session.openLocal",
            serde_json::json!({"shell":"/bin/sh","shell_args":["-i"],"rows":24,"cols":80}),
        )
        .await;
        let id = uuid::Uuid::parse_str(opened["id"].as_str().unwrap()).unwrap();
        call(
            "terminal.attach",
            serde_json::json!({"id":id,"cols":80,"rows":24,"scrollback":500}),
        )
        .await;
        let engine = rpc.state.terminal_engines.get(id).unwrap();
        let d = dispatcher.clone();
        let input: aerotab_core::terminal::native_host::InputDispatch =
            Arc::new(move |method, params| {
                let d = d.clone();
                tauri::async_runtime::spawn(async move {
                    let r = d
                        .dispatch(Request {
                            jsonrpc: "2.0".into(),
                            id: Some(serde_json::json!("host-smoke")),
                            method: method.into(),
                            params,
                        })
                        .await;
                    assert!(r.error.is_none(), "{:?}", r.error);
                });
            });
        call("session.write",serde_json::json!({"id":id,"data":base64::Engine::encode(&base64::engine::general_purpose::STANDARD,b"printf 'PRODUCTION_%s_OK\\n' SHELL\n")})).await;
        // Ensure attach captures real PTY bytes, not the initial zero watermark.
        for _ in 0..100 {
            if engine.native_snapshot_envelope().output_seq > 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(engine.native_snapshot_envelope().output_seq > 0);
        let view_engine = engine.clone();
        let main = app.get_webview_window("main").unwrap();
        let outcome = native_main(app.clone(), move || {
            NativeHost::attach(
                &main,
                id,
                Geometry {
                    x: 280.,
                    y: 100.,
                    width: 600.,
                    height: 350.,
                },
                ViewConfig::default(),
                view_engine,
                input,
            )
        })
        .await;
        if let Err(e) = outcome {
            eprintln!("native host smoke attach failed: {e:?}");
            app.exit(5);
            return;
        }
        let attached = native_main(app.clone(), move || NativeHost::state(id))
            .await
            .unwrap();
        let target = attached.handoff_output_seq.parse::<u64>().unwrap();
        assert!(target > 0, "attach output target must be nonzero");
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let before = native_main(app.clone(), move || NativeHost::state(id))
            .await
            .unwrap();
        assert!(
            before.generation == attached.generation
                && before
                    .drawn_revision
                    .is_some_and(|r| r >= attached.revision)
                && before.handoff_output_seq == attached.handoff_output_seq
                && before
                    .drawn_output_seq
                    .as_ref()
                    .is_some_and(|s| s.parse::<u64>().unwrap() >= target),
            "no matching generation/revision/output GL draw: {:?}",
            before.error
        );
        let snapshot = aerotab_core::terminal::native_host::Snapshot::decode(
            &engine.native_snapshot_frame(),
            &ViewConfig::default(),
        )
        .unwrap();
        let text: String = snapshot.cells.iter().map(|c| c.text.as_str()).collect();
        assert!(
            text.contains("PRODUCTION_SHELL_OK"),
            "real shell marker missing: {text}"
        );
        native_main(app.clone(), move || {
            NativeHost::update(
                id,
                Geometry::default(),
                true,
                Some(ViewConfig {
                    expected_generation: Some(before.generation),
                    revision: before.revision + 1,
                    font_size: 15.,
                    ..ViewConfig::default()
                }),
            )
        })
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        call("session.write",serde_json::json!({"id":id,"data":base64::Engine::encode(&base64::engine::general_purpose::STANDARD,b"printf 'RESIZE_%s_OK ' PTY; stty size\n")})).await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let resized = aerotab_core::terminal::native_host::Snapshot::decode(
            &engine.native_snapshot_frame(),
            &ViewConfig::default(),
        )
        .unwrap();
        let resized_text: String = resized.cells.iter().map(|c| c.text.as_str()).collect();
        let expected = format!("RESIZE_PTY_OK {} {}", resized.rows, resized.cols);
        assert!(
            resized_text.contains(&expected),
            "resize marker missing: {expected}: {resized_text}"
        );
        let drawn = native_main(app.clone(), move || NativeHost::state(id))
            .await
            .unwrap();
        assert!(drawn.error.is_none(), "{:?}", drawn.error);
        assert_eq!(drawn.generation, attached.generation);
        assert_eq!(drawn.handoff_output_seq, attached.handoff_output_seq);
        assert!(drawn.drawn_revision.is_some_and(|r| r > before.revision));
        assert!(drawn
            .drawn_output_seq
            .as_ref()
            .is_some_and(|s| s.parse::<u64>().unwrap() >= target));
        native_main(app.clone(), move || {
            let current = NativeHost::state(id)?;
            NativeHost::update(
                id,
                Geometry::default(),
                false,
                Some(ViewConfig {
                    expected_generation: Some(current.generation),
                    revision: current.revision + 1,
                    ..ViewConfig::default()
                }),
            )?;
            NativeHost::detach(id)
        })
        .await
        .unwrap();
        assert!(engine.is_alive(), "view detach killed engine");
        call("session.close", serde_json::json!({"id":id})).await;
        println!(
            "{}",
            serde_json::json!({"event":"production_native_host_smoke_pass","real_shell_marker":true,"resize_marker":expected,"output_before":attached,"output_after":drawn,"fixed_output_target":target.to_string(),"frame_seq_used":false,"detach_preserves_engine":true,"frontend_wired":false})
        );
        app.exit(0);
    });
}

#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
static E2E_KEY_SESSION: std::sync::Mutex<Option<(uuid::Uuid, String, String)>> =
    std::sync::Mutex::new(None);
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
static E2E_KEY_USED: std::sync::Mutex<Vec<(uuid::Uuid, String, String)>> =
    std::sync::Mutex::new(Vec::new());
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
fn take_key_arm(
    arm: &mut Option<(uuid::Uuid, String, String)>,
    id: uuid::Uuid,
    generation: &str,
    phase: &str,
) -> bool {
    if arm
        .as_ref()
        .is_some_and(|(s, g, p)| *s == id && g == generation && p == phase)
    {
        arm.take();
        true
    } else {
        false
    }
}
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
static E2E_OBSERVE_SESSION: std::sync::Mutex<Option<uuid::Uuid>> = std::sync::Mutex::new(None);
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
static E2E_UPDATES: std::sync::Mutex<Vec<serde_json::Value>> = std::sync::Mutex::new(Vec::new());
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
static E2E_STALE_OBSERVATIONS: std::sync::Mutex<Vec<serde_json::Value>> =
    std::sync::Mutex::new(Vec::new());
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
static E2E_WRITE_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
static E2E_REVERSE_TRANSACTION: std::sync::Mutex<serde_json::Value> =
    std::sync::Mutex::new(serde_json::Value::Null);
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
static E2E_REVERSE_EVENTS: std::sync::Mutex<Vec<serde_json::Value>> =
    std::sync::Mutex::new(Vec::new());
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
type E2eWriterBarrier = (
    uuid::Uuid,
    uuid::Uuid,
    u32,
    tokio::sync::oneshot::Receiver<()>,
);
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
static E2E_ACK_BARRIER: std::sync::Mutex<Option<E2eWriterBarrier>> = std::sync::Mutex::new(None);
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
static E2E_ACK_RELEASE: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>> =
    std::sync::Mutex::new(None);
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
async fn wait_writer_barrier(
    receiver: Option<tokio::sync::oneshot::Receiver<()>>,
    deadline: std::time::Duration,
) -> Result<(), &'static str> {
    match receiver {
        None => Ok(()),
        Some(receiver) => match tokio::time::timeout(deadline, receiver).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err("writer barrier cancelled"),
            Err(_) => Err("writer barrier timeout"),
        },
    }
}
#[cfg(all(test, debug_assertions, feature = "native-view", target_os = "linux"))]
#[tokio::test]
async fn writer_barrier_real_async_lifecycle() {
    use std::time::Duration;
    wait_writer_barrier(None, Duration::from_secs(4))
        .await
        .unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let (entered, entry) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        entered.send(()).unwrap();
        wait_writer_barrier(Some(rx), Duration::from_secs(4)).await
    });
    entry.await.unwrap();
    assert!(!task.is_finished());
    let mut release = Some(tx);
    release.take().unwrap().send(()).unwrap();
    assert!(release.take().is_none());
    task.await.unwrap().unwrap();
    let (_tx, rx) = tokio::sync::oneshot::channel();
    assert_eq!(
        wait_writer_barrier(Some(rx), Duration::ZERO).await,
        Err("writer barrier timeout")
    );
    let (tx, rx) = tokio::sync::oneshot::channel();
    drop(tx);
    assert_eq!(
        wait_writer_barrier(Some(rx), Duration::from_secs(4)).await,
        Err("writer barrier cancelled")
    );
    let (tx, rx) = tokio::sync::oneshot::channel();
    let cleanup =
        tokio::spawn(async move { wait_writer_barrier(Some(rx), Duration::from_secs(4)).await });
    tx.send(()).unwrap();
    cleanup.await.unwrap().unwrap();
}
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
fn ack_barrier_matches(
    enabled: bool,
    target: (uuid::Uuid, uuid::Uuid, u32),
    session: uuid::Uuid,
    generation: Option<uuid::Uuid>,
    ack: u32,
) -> bool {
    enabled && target.0 == session && Some(target.1) == generation && ack > target.2
}
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
static E2E_HIDE_REPLY: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<serde_json::Value>>> =
    std::sync::Mutex::new(None);
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
static E2E_CHANNELS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
static E2E_CORE_CALLS: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
static E2E_SESSION_CHANNELS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
static E2E_FAIL_ATTACH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
static E2E_ACK_FAULT: std::sync::Mutex<Option<(uuid::Uuid, uuid::Uuid, u32, usize)>> =
    std::sync::Mutex::new(None);
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
static E2E_WEBVIEW_CAPTURES: std::sync::Mutex<Vec<serde_json::Value>> =
    std::sync::Mutex::new(Vec::new());
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
fn inject_e2e_gdk(id: uuid::Uuid, generation: &str, phase: &str) -> Result<(), HostError> {
    let flow = NativeHost::debug_e2e_flow(id)?;
    if flow["generation"].as_str() != Some(generation)
        || !take_key_arm(&mut E2E_KEY_SESSION.lock().unwrap(), id, generation, phase)
    {
        return Err(HostError::new(
            "harness_not_armed",
            "current generation/phase single-use arm required",
        ));
    }
    NativeHost::debug_e2e_inject(id, false)
}
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
#[tauri::command]
fn native_ui_e2e_report(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    report: serde_json::Value,
) -> Result<(), String> {
    if window.label() != "main" || std::env::var_os("AEROTAB_NATIVE_UI_E2E").is_none() {
        return Err("debug UI harness is disabled".into());
    }
    if report["boundaryReply"] == true {
        if let Some(tx) = E2E_HIDE_REPLY.lock().unwrap().take() {
            let _ = tx.send(report);
        }
        return Ok(());
    }
    fn redact(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::Object(m) => {
                for key in ["dom", "body", "calls", "sessions", "title", "activeElement"] {
                    m.remove(key);
                }
                if let Some(serde_json::Value::String(text)) = m.get_mut("text") {
                    *text = text
                        .lines()
                        .filter(|s| {
                            s.starts_with("E2E_")
                                || s.starts_with("TICK_")
                                || s.starts_with("PASTE_")
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                }
                for value in m.values_mut() {
                    redact(value);
                }
            }
            serde_json::Value::Array(a) => a.iter_mut().for_each(redact),
            _ => {}
        }
    }
    if report.get("ok").is_some() {
        E2E_ACK_BARRIER.lock().unwrap().take();
        if let Some(tx) = E2E_ACK_RELEASE.lock().unwrap().take() {
            let _ = tx.send(());
        }
    }
    let mut report = report;
    redact(&mut report);
    println!("NATIVE_UI_E2E {}", report);
    if report.get("ok").is_some() {
        // Tauri exit can be intercepted by tray/lifecycle handlers. Persist the
        // authoritative JSON first and use an unambiguous debug-harness status.
        let code = if report["ok"] == true { 0 } else { 6 };
        let path = std::env::temp_dir().join(format!(
            "aerotab-native-smoke-{}/ui-result.json",
            std::process::id()
        ));
        std::fs::write(
            &path,
            serde_json::to_vec(&report).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        eprintln!("NATIVE_UI_E2E result-file={} status={code}", path.display());
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let _ = app;
        std::process::exit(code);
    }
    Ok(())
}

#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
#[tauri::command]
fn native_ui_e2e_paste(
    window: tauri::WebviewWindow,
    session_id: String,
    generation: String,
    request_id: u32,
    scenario: String,
) -> Result<(), String> {
    if window.label() != "main" || std::env::var_os("AEROTAB_NATIVE_UI_E2E").is_none() {
        return Err("debug UI harness is disabled".into());
    }
    let marker = match scenario.as_str() {
        "single" => "SINGLE",
        "duplicate" => "DUPLICATE",
        "old" => "OLDGEN",
        "reject" => "REJECT",
        "allow" => "ALLOW",
        "late" => "LATE",
        _ => return Err("unknown controlled scenario".into()),
    };
    let mut text = format!("printf 'PASTE_%s_OK\\n' {marker}");
    if matches!(scenario.as_str(), "reject" | "allow") {
        text = format!("# {}\n{text}\n", "controlled-test-padding-".repeat(12));
    }
    use tauri::Emitter;
    window.emit_to(tauri::EventTarget::webview_window("main"),"aerotab:native-paste-request",serde_json::json!({"sessionId":session_id,"generation":generation,"requestId":request_id,"text":text})).map_err(|e|e.to_string())
}

#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
#[tauri::command]
async fn native_ui_e2e_buffer(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppRpc>,
    session_id: String,
    arm: Option<bool>,
    mut inject: Option<serde_json::Value>,
    burst: Option<bool>,
    late: Option<bool>,
) -> Result<serde_json::Value, String> {
    let app = window.app_handle().clone();
    if window.label() != "main" || std::env::var_os("AEROTAB_NATIVE_UI_E2E").is_none() {
        return Err("harness disabled".into());
    }
    let id = uuid::Uuid::parse_str(&session_id).map_err(|e| e.to_string())?;
    if inject
        .as_ref()
        .is_some_and(|value| value["closedProbe"] == true)
    {
        let owner_present = native_main(app.clone(), move || NativeHost::state(id))
            .await
            .is_ok();
        return Ok(
            serde_json::json!({"ownerPresent":owner_present,"sessionWriteCount":E2E_WRITE_COUNT.load(std::sync::atomic::Ordering::SeqCst)}),
        );
    }
    let engine = state
        .state
        .terminal_engines
        .get(id)
        .ok_or("missing engine")?;
    if let Some(enabled) = inject
        .as_ref()
        .and_then(|value| value["ackFault"].as_bool())
    {
        let flow = native_main(app.clone(), move || NativeHost::debug_e2e_flow(id))
            .await
            .map_err(|e| e.message)?;
        let mut fault = E2E_ACK_FAULT.lock().unwrap();
        if enabled {
            let generation = flow["generation"]
                .as_str()
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                .ok_or("missing fault generation")?;
            *fault = Some((
                id,
                generation,
                flow["ack"].as_u64().unwrap_or_default() as u32,
                0,
            ));
        } else {
            *fault = None;
        }
        inject = None;
    }
    if arm == Some(false) {
        E2E_FAIL_ATTACH.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    if arm == Some(true) {
        *E2E_OBSERVE_SESSION.lock().unwrap() = Some(id);
        native_main(app.clone(), move || NativeHost::debug_e2e_arm(id))
            .await
            .map_err(|e| e.message)?;
        engine.arm_debug_ticks(id);
    }
    if let Some(inject) = inject {
        let expected = inject["generation"]
            .as_str()
            .ok_or("explicit generation required")?
            .to_owned();
        let phase = inject["phase"]
            .as_str()
            .ok_or("explicit phase required")?
            .to_owned();
        if phase == "paste submit enter" {
            let target_window = window.clone();
            return native_main(app, move || {
                let gtk_window = target_window
                    .gtk_window()
                    .map_err(|_| HostError::new("missing_window", "GTK window unavailable"))?;
                use glib::translate::*;
                use gtk::prelude::*;
                let flow = NativeHost::debug_e2e_flow(id)?;
                if flow["generation"].as_str() != Some(&expected)
                    || flow["focused"] != true
                    || flow["mapped"] != true
                {
                    return Err(HostError::new(
                        "not_focused",
                        "current focused GTK surface required",
                    ));
                }
                let area = gtk_window
                    .focused_widget()
                    .ok_or_else(|| HostError::new("not_focused", "missing focused widget"))?;
                let event = gtk::gdk::Event::new(gtk::gdk::EventType::KeyPress);
                unsafe {
                    let ptr: *mut gtk::gdk::ffi::GdkEvent = event.to_glib_none().0;
                    (*ptr).key.window = area.window().into_glib_ptr();
                    (*ptr).key.keyval = 0xff0d;
                    (*ptr).key.send_event = 1;
                }
                area.emit_by_name::<bool>("key-press-event", &[&event]);
                Ok(serde_json::json!({"enterDispatched":true}))
            })
            .await
            .map_err(|e| e.message);
        }
        let allowed = match phase.as_str() {
            "initial native input" => E2E_SNAPSHOT_FAULT.lock().unwrap().consumed.is_none(),
            "retained source after rollback" => E2E_SNAPSHOT_FAULT
                .lock()
                .unwrap()
                .consumed
                .as_ref()
                .is_some_and(|v| v["sessionId"] == session_id && v["generation"] == expected),
            _ => false,
        };
        if !allowed {
            return Err("invalid GTK injection phase".into());
        }
        native_main(app.clone(), move || {
            let flow = NativeHost::debug_e2e_flow(id)?;
            if flow["generation"].as_str() != Some(expected.as_str()) {
                return Err(HostError::new(
                    "harness_stale_generation",
                    "cannot arm old widget generation",
                ));
            }
            let identity = (id, expected.clone(), phase.clone());
            let mut used = E2E_KEY_USED.lock().unwrap();
            if used.contains(&identity) {
                return Err(HostError::new(
                    "harness_phase_consumed",
                    "phase already injected",
                ));
            }
            used.push(identity.clone());
            drop(used);
            *E2E_KEY_SESSION.lock().unwrap() = Some(identity);
            inject_e2e_gdk(id, &expected, &phase)
        })
        .await
        .map_err(|e| e.message)?;
        let flow = native_main(app.clone(), move || NativeHost::debug_e2e_flow(id))
            .await
            .map_err(|e| e.message)?;
        let generation = flow["generation"]
            .as_str()
            .ok_or("missing native generation")?
            .to_owned();
        let mut fault = E2E_SNAPSHOT_FAULT.lock().unwrap();
        if fault.armed.is_none()
            && fault.consumed.is_none()
            && std::env::var("AEROTAB_ACK_RECOVERY").as_deref() != Ok("1")
            && std::env::var("AEROTAB_ACK_CLOSE").as_deref() != Ok("1")
        {
            fault.armed = Some((session_id.clone(), generation));
        }
    }
    if burst == Some(true) {
        let window = window.clone();
        native_main(app.clone(),move || {
            let flow=NativeHost::debug_e2e_flow(id)?;
            let generation=flow["generation"].as_str().and_then(|s|uuid::Uuid::parse_str(s).ok()).ok_or_else(||HostError::new("harness_generation","missing generation"))?;
            let ack=flow["ack"].as_u64().unwrap_or_default() as u32;
            if ack!=28 || flow["reserved"]!=28 { return Err(HostError::new("harness_baseline","expected drained watermark28 before arm")); }
            let (tx,rx)=tokio::sync::oneshot::channel();
            *E2E_ACK_BARRIER.lock().unwrap()=Some((id,generation,ack,rx));
            *E2E_ACK_RELEASE.lock().unwrap()=Some(tx);
            E2E_REVERSE_EVENTS.lock().unwrap().push(serde_json::json!({"event":"writer barrier armed before burst","sessionId":id,"generation":generation,"ack":ack}));
            NativeHost::debug_e2e_inject(id,true)?;
            // Schedule the real footer click in the same GTK-main-thread burst,
            // without waiting for a single writer ACK.
            window.eval("[...document.querySelectorAll('button')].find(b=>b.textContent.includes('⇄')).click()").map_err(|_|HostError::new("eval_failed","footer dispatch failed"))?;
            Ok(())
        }).await.map_err(|e|e.message)?;
    }
    if late == Some(true) {
        native_main(app.clone(), move || NativeHost::debug_e2e_late(id))
            .await
            .map_err(|e| e.message)?;
    }
    let trace = native_main(app.clone(), move || NativeHost::debug_e2e_trace(id))
        .await
        .map_err(|e| e.message)?;
    let flow = native_main(app, move || NativeHost::debug_e2e_flow(id))
        .await
        .ok();
    let core_calls = E2E_CORE_CALLS.lock().unwrap().clone();
    let registrations = E2E_SESSION_CHANNELS.lock().unwrap().clone();
    let others: std::collections::HashSet<_> =
        registrations.iter().filter(|s| *s != &session_id).collect();
    let other_sessions: Vec<_> = others.iter().map(|s|serde_json::json!({"channelRegistrations":registrations.iter().filter(|v|*v==*s).count(),"attachCount":core_calls.iter().filter(|(v,m)|v==*s&&m=="terminal.attach").count()})).collect();
    let filter = |s: String| {
        s.lines()
            .filter(|s| s.starts_with("TICK_") || s.starts_with("DIAG_"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    Ok(
        serde_json::json!({"ackFault": E2E_ACK_FAULT.lock().unwrap().map(|(session,generation,baseline,rejects)|serde_json::json!({"sessionId":session,"generation":generation,"baseline":baseline,"rejects":rejects})),"otherSessions":other_sessions,"sessionId":session_id,"reverseBoundaryEvents":E2E_REVERSE_EVENTS.lock().unwrap().clone(),"gdkMarkerCount":engine.buffer_text().lines().filter(|line|line.trim()=="GDK_BRIDGE_OK").count(),"snapshotFault":E2E_SNAPSHOT_FAULT.lock().unwrap().consumed.clone(),"buffer":filter(engine.buffer_text()),"viewport":filter(engine.viewport_text()),"rawTicks":engine.debug_tick_records(),"channelRegistrations":E2E_SESSION_CHANNELS.lock().unwrap().iter().filter(|s|*s==&session_id).count(),"globalChannelRegistrations":E2E_CHANNELS.load(std::sync::atomic::Ordering::SeqCst),"coreIdentity":Arc::as_ptr(&engine) as usize,"attachCount":core_calls.iter().filter(|(s,m)|s==&session_id&&m=="terminal.attach").count(),"detachOrCloseCount":core_calls.iter().filter(|(s,m)|s==&session_id&&m!="terminal.attach").count(),"sessionWriteCount":E2E_WRITE_COUNT.load(std::sync::atomic::Ordering::SeqCst),"pasteSingleCount":engine.buffer_text().lines().filter(|line|line.trim()=="PASTE_SINGLE_OK").count(),"rollbackMarkerCount":engine.buffer_text().lines().filter(|line|line.trim()=="ROLLBACK_WRITE_OK").count(),"flow":flow,"trace":trace,"updates":E2E_UPDATES.lock().unwrap().clone(),"webviewCaptures":E2E_WEBVIEW_CAPTURES.lock().unwrap().clone(),"staleObservations":E2E_STALE_OBSERVATIONS.lock().unwrap().clone(),"reverseWindow1Count":engine.buffer_text().lines().filter(|line|line.trim()=="DIAG_REVERSE_WINDOW1_OK").count(),"reverseWindow2Count":engine.buffer_text().lines().filter(|line|line.trim()=="DIAG_REVERSE_WINDOW2_OK").count(),"reverseWindow1AfterRace":engine.buffer_text().find("E2E_RACE_aB3dE5fG7hJ9_END").zip(engine.buffer_text().find("DIAG_REVERSE_WINDOW1_OK")).is_some_and(|(a,b)|b>a),"raceExactCount":engine.buffer_text().lines().filter(|line|line.trim()=="E2E_RACE_aB3dE5fG7hJ9_END").count()}),
    )
}
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
static E2E_SNAPSHOT_FAULT: std::sync::Mutex<SnapshotFault> = std::sync::Mutex::new(SnapshotFault {
    armed: None,
    consumed: None,
});
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
struct SnapshotFault {
    armed: Option<(String, String)>,
    consumed: Option<serde_json::Value>,
}
#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
impl SnapshotFault {
    fn consume(
        &mut self,
        enabled: bool,
        session: &str,
        generation: &str,
        production: bool,
        result: &serde_json::Value,
    ) -> bool {
        if !enabled
            || !production
            || result["result"]["version"] != 1
            || !result["error"].is_null()
            || self
                .armed
                .as_ref()
                .map_or(true, |(s, g)| s != session || g != generation)
        {
            return false;
        }
        self.armed = None;
        self.consumed = Some(
            serde_json::json!({"injected":true,"phase":"first retained reverse snapshot","sessionId":session,"generation":generation,"snapshotCompleted":true,"outputSeq":result["result"]["outputSeq"]}),
        );
        true
    }
}

#[cfg(all(test, debug_assertions, feature = "native-view", target_os = "linux"))]
#[test]
fn snapshot_fault_matching_once_and_transparency() {
    let id = uuid::Uuid::nil();
    let generation = uuid::Uuid::new_v4();
    assert!(ack_barrier_matches(
        true,
        (id, generation, 28),
        id,
        Some(generation),
        29
    ));
    assert!(!ack_barrier_matches(
        false,
        (id, generation, 28),
        id,
        Some(generation),
        29
    ));
    assert!(!ack_barrier_matches(
        true,
        (id, generation, 28),
        id,
        Some(uuid::Uuid::new_v4()),
        29
    ));
    assert!(!ack_barrier_matches(
        true,
        (id, generation, 28),
        uuid::Uuid::new_v4(),
        Some(generation),
        29
    ));
    assert!(!ack_barrier_matches(
        true,
        (id, generation, 28),
        id,
        Some(generation),
        28
    ));
    let mut arm = Some((id, "current".into(), "initial".into()));
    assert!(!take_key_arm(&mut arm, id, "old", "initial"));
    assert!(take_key_arm(&mut arm, id, "current", "initial"));
    assert!(!take_key_arm(&mut arm, id, "current", "initial"));
    arm = Some((id, "current".into(), "rollback".into()));
    assert!(!take_key_arm(&mut arm, id, "old", "rollback"));
    assert!(!take_key_arm(&mut arm, id, "current", "initial"));
    assert!(take_key_arm(&mut arm, id, "current", "rollback"));
    let mut fault = SnapshotFault {
        armed: None,
        consumed: None,
    };
    let response = serde_json::json!({"result":{"version":1,"outputSeq":"818"}});
    assert!(!fault.consume(true, "s", "g", true, &response));
    fault.armed = Some(("s".into(), "g".into()));
    for (enabled, s, g, production) in [
        (false, "s", "g", true),
        (true, "other", "g", true),
        (true, "s", "old", true),
        (true, "s", "g", false),
    ] {
        assert!(!fault.consume(enabled, s, g, production, &response));
        assert!(fault.armed.is_some());
    }
    assert!(!fault.consume(
        true,
        "s",
        "g",
        true,
        &serde_json::json!({"error":{"code":-1}})
    ));
    assert!(fault.consume(true, "s", "g", true, &response));
    assert_eq!(fault.consumed.as_ref().unwrap()["snapshotCompleted"], true);
    assert!(!fault.consume(true, "s", "g", true, &response));
}

#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
fn start_native_ui_e2e(app: &tauri::AppHandle) {
    if std::env::var_os("AEROTAB_NATIVE_UI_E2E").is_none() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let window = app.get_webview_window("main").unwrap();
        let script = r#"(async()=>{
window.__nativeUiE2eStage={stage:'eval-entered',url:location.href,readyState:document.readyState};
const harnessIdentity='retained-reverse-runtime-20261101-1530';
let reverseProbe=null;
const reverseEvents=[];
const controllerCanvases=new Set();
const identities=new Map();
const continuity=(baseline,current,identity,canvas)=>({channel:current.channelRegistrations===1,attach:current.attachCount===1,noDetachOrClose:current.detachOrCloseCount===0,core:current.coreIdentity===baseline.coreIdentity,session:current.sessionId===baseline.sessionId,controller:identity?.controllers.size===1,canvas:identity?.canvases.size===1&&identity.canvases.has(canvas)});
let reverseEpoch=0;
const beginReverse=(sessionId,label,generation,receiptStart,command)=>({label,sessionId,generation,epoch:++reverseEpoch,receiptStart,receipts:[],command});
const matchesTransaction=(t,p)=>t?.sessionId===p.sessionId&&t.sourceGeneration===p.generation&&t.epoch===p.epoch&&t.phase===p.label;
const matchingReceipt=(r,p)=>matchesTransaction(r.transaction,p)&&r.sessionId===p.sessionId&&r.generation!==p.generation&&typeof r.generation==='string'&&Number.isSafeInteger(r.revision)&&r.revision>0&&/^[0-9]+$/.test(r.appliedOutputSeq)&&BigInt(r.appliedOutputSeq)>0n&&BigInt(r.drawnOutputSeq)>=BigInt(r.appliedOutputSeq);
const timeline=[], layoutSnapshots=[];
const recordStage=label=>{if(timeline.length>=128)return;const f=[...document.querySelectorAll('button')].find(b=>b.textContent.includes('⇄'));const s=document.querySelector('[data-terminal-view-backend]');if(label!=='mutation'||window.__aerotabViewCausalTrace?.snapshot().some(e=>e.stage==='layout.sample')){if(layoutSnapshots.length<128)layoutSnapshots.push({label,at:Date.now(),trace:window.__aerotabViewCausalTrace?.snapshot()});}const next={label,at:Date.now(),footer:f?.textContent,disabled:f?.disabled,backend:s?.dataset.terminalViewBackend,sourceConnected:window.__stagingSourceCanvas?.isConnected};if(label!=='mutation'||JSON.stringify({...next,at:0})!==JSON.stringify({...timeline.at(-1),at:0}))timeline.push(next);};
new MutationObserver(()=>recordStage('mutation')).observe(document.body,{subtree:true,childList:true,attributes:true,attributeFilter:['disabled','hidden','data-terminal-view-backend']});
let inv;
try {
if(typeof window.__TAURI_INTERNALS__?.invoke!=='function') {window.__nativeUiE2eStage.stage='bridge-missing';console.error('NATIVE_UI_E2E bridge-missing',location.href);return;}
inv=window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);
window.__nativeUiE2eStage.stage='bridge-ready';
await inv('native_ui_e2e_report',{report:{stage:'bridge-ready',url:location.href,readyState:document.readyState,resources:performance.getEntriesByType('resource').map(r=>({name:r.name,duration:r.duration}))}});
const evidence=[], calls=[];let phase='startup';
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
window.__TAURI_INTERNALS__.invoke=async(cmd,args)=>{try{
 if(window.__ackCloseGate&&cmd.startsWith('native_view_'))window.__ackCloseGate.dispatched.push({cmd,sessionId:args?.sessionId});
 const method=cmd==='rpc'?args?.frame?.method:null;
 if(reverseProbe&&method==='terminal.snapshotEnvelope'){
  const p=reverseProbe;controllerCanvases.add(document.querySelector('[data-terminal-view-backend] canvas'));
  reverseEvents.push({event:'target capture requested',label:p.label,at:Date.now(),generation:p.generation});
 }
 const r=await inv(cmd,args);
 const closeGate=window.__ackCloseGate;
 if(closeGate&&!closeGate.held&&cmd==='native_view_update'&&args?.sessionId===closeGate.id&&args.cfg?.inputAck>closeGate.baseline){closeGate.held=true;await closeGate.promise;}
 if(cmd.startsWith('native_view_')||cmd==='terminal_frames'||(cmd==='rpc'&&['terminal.attach','terminal.detach','session.openLocal','session.close'].includes(args?.frame?.method)))calls.push({cmd,args:cmd==='rpc'?args.frame:args,result:r});return r;}catch(e){calls.push({cmd,error:String(e)});throw e;}};
const invoke=(cmd,args)=>window.__TAURI_INTERNALS__.invoke(cmd,args);
const pasteTrace=[];
const eventApi=window.__TAURI__?.event;
if(eventApi?.listen){const listen=eventApi.listen.bind(eventApi);eventApi.listen=async(name,callback,options)=>listen(name,event=>{
 if(name==='aerotab:native-paste-request'&&pasteTrace.length<64)pasteTrace.push({phase:'listener callback entered',at:Date.now(),sessionId:event.payload?.sessionId,generation:event.payload?.generation,requestId:event.payload?.requestId,occluded:document.body.dataset.nativeViewOccluded==='true',hidden:document.hidden});
 return callback(event);
},options);}
const drawReceipts=window.__reverseDrawReceipts=[];
// This script is compiled only in the debug Linux harness and requires E2E env.
Object.defineProperty(window,'__aerotabDebugDrawObserver',{configurable:false,writable:false,value:Object.freeze({causalTrace:true,pasteTrace:true,identity:(session,controller,canvas)=>{let row=identities.get(session);if(!row){row={controllers:new Set(),canvases:new Set()};identities.set(session,row);}row.controllers.add(controller);row.canvases.add(canvas);},receipt:r=>{
 controllerCanvases.add(document.querySelector('[data-terminal-view-backend] canvas'));
 if(drawReceipts.length>=64)return;
 const p=reverseProbe;
 const receipt=Object.freeze({...r,observedAt:Date.now(),transaction:p?Object.freeze({sessionId:p.sessionId,sourceGeneration:p.generation,epoch:p.epoch,phase:p.label}):null});
 drawReceipts.push(receipt);
 if(p)p.receipts.push(receipt);
}})});
const rpc=async(method,params={})=>{const r=await invoke('rpc',{frame:{jsonrpc:'2.0',id:90001,method,params}});if(r.error)throw Error(JSON.stringify(r.error));return r.result;};
const wait=async(fn,label,attempts=160)=>{let last;for(let i=0;i<attempts;i++){try{last=await fn();if(last)return last;}catch(e){last=String(e);}await sleep(50);}throw Error(label+': '+JSON.stringify(last));};
const surface=()=>document.querySelector('[data-terminal-view-backend]');
const footer=()=>[...document.querySelectorAll('button')].find(b=>b.textContent.includes('⇄'));
const backend=kind=>wait(()=>surface()?.dataset.terminalViewBackend===kind&&footer()&&!footer().disabled,'backend '+kind);
const text=()=>rpc('terminal.text',{id});
// Canonical byte-output watermark, never snapshot/frame seq and never Number.
const outputWatermarks=new Map();
const decimalU64=value=>{if(typeof value!=='string'||! /^(0|[1-9][0-9]*)$/.test(value))throw Error('output watermark is not a decimal string');const n=BigInt(value);if(n>18446744073709551615n)throw Error('output watermark exceeds u64');return n;};
const outputReached=s=>{const fixed=outputWatermarks.get(s.generation);if(!fixed)throw Error('uncaptured attachment output target');if(s.handoffOutputSeq!==fixed.target)throw Error('attachment output target chased live output');return !s.error&&s.drawnOutputSeq!=null&&decimalU64(s.drawnOutputSeq)>=decimalU64(fixed.target);};
const nativeDraw=async(label,previousGeneration)=>{
 const before=await wait(async()=>{const s=await invoke('native_view_state',{sessionId:id});return s.generation!==previousGeneration&&s;},label+' attached');
 if(decimalU64(before.handoffOutputSeq)<=0n)throw Error('actual native attach target must be nonzero');
 if(!outputWatermarks.has(before.generation))outputWatermarks.set(before.generation,{target:before.handoffOutputSeq,before});
 const requested=await wait(async()=>[...(await invoke('native_ui_e2e_buffer',{sessionId:id})).updates].reverse().find(c=>c.result?.generation===before.generation&&c.args?.cfg?.expectedGeneration===before.generation),label+' actual UI request');
 const after=await wait(async()=>{const s=await invoke('native_view_state',{sessionId:id});return s.generation===before.generation&&s.drawnRevision>=requested.args.cfg.revision&&outputReached(s)&&s;},label+' generation/revision/output draw');
 evidence.push({phase:'native output watermark',label,before,after,fixedTarget:before.handoffOutputSeq,requestedRevision:requested.args.cfg.revision,comparison:'decimal u64 BigInt',frameSeqUsed:false});return after;
};
const jobCommand="printf 'E2E_%s_OK\\n' START; (i=0; while [ $i -lt 15 ]; do printf 'TICK_%s\\n' $i; i=$((i+1)); sleep 1; done) & E2E_JOB=$!; printf '\\nDIAG_PID_%s\\n' $E2E_JOB\n";
const diagnose=async(label)=>{await rpc('session.write',{id,data:btoa("printf '\\nDIAG_PHASE_%s\\n' "+label+"; if kill -0 $E2E_JOB 2>/dev/null; then printf 'DIAG_ALIVE_yes\\n'; else printf 'DIAG_ALIVE_no\\n'; wait $E2E_JOB; printf 'DIAG_EXIT_%s\\n' $?; fi; ps -o stat= -p $E2E_JOB | sed 's/^/DIAG_STAT_/'; ps -o pid=,ppid=,stat=,wchan=,comm= --ppid $E2E_JOB | sed 's/^/DIAG_CHILD_/'; jobs -l | sed 's/^/DIAG_JOB_/'\n")});await sleep(200);evidence.push({phase:'producer '+label,at:Date.now(),core:await invoke('native_ui_e2e_buffer',{sessionId:id})});};
let id;
try{
await wait(()=>footer()&&!footer().disabled&&surface(),'production terminal ready');
const sessions=await rpc('session.list');if(sessions.length!==1)throw Error('expected isolated startup single session');id=sessions[0].id;
evidence.push({phase:'startup',id,backend:surface().dataset.terminalViewBackend,footer:footer().textContent});
await invoke('native_ui_e2e_buffer',{sessionId:id,arm:true});
await rpc('session.write',{id,data:btoa("printf 'E2E_%s_OK\\n' START\n")});
await wait(async()=> (await text()).text.includes('E2E_START_OK'),'shell initial output');
phase='real staging failure rollback';
window.__stagingSourceCanvas=surface().querySelector('canvas');
const sourceCore=await invoke('native_ui_e2e_buffer',{sessionId:id});
const sourceChannels=sourceCore.channelRegistrations;
await invoke('native_ui_e2e_buffer',{sessionId:id,arm:false});
recordStage('before injected failure click');footer().click();recordStage('after injected failure click');
await wait(()=>window.__stagingEvidence?.length&&footer()&&!footer().disabled,'injected target failure settled');
if(surface().dataset.terminalViewBackend!=='webview'||!window.__stagingSourceCanvas?.isConnected||window.__stagingSourceCanvas.hidden)throw Error('target failure lost live source canvas');
const sourceInput=surface().querySelector('textarea');
sourceInput.value="printf 'ROLLBACK_%s_OK\\n' WRITE\n";sourceInput.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertText',data:sourceInput.value}));
await wait(async()=> (await invoke('native_ui_e2e_buffer',{sessionId:id})).rollbackMarkerCount===1,'real source input survives rollback exactly once');
const rolledBack=await invoke('native_ui_e2e_buffer',{sessionId:id});
if(rolledBack.channelRegistrations!==sourceChannels)throw Error('rollback replaced sole Channel');
evidence.push({phase,id,sourceCanvasRetained:true,channelRegistrations:sourceChannels,markerCount:rolledBack.rollbackMarkerCount,scope:'real attach failure rollback and post-rollback input; queued-during-drain failure not injected'});
phase='native first';recordStage('before retry click');footer().click();recordStage('after retry click');await backend('native-surface');
if(location.search.includes('rollbackMinimal=1')){await inv('native_ui_e2e_report',{report:{nativePasteTrace:window.__aerotabNativePasteTrace?.snapshot()??[],layoutSnapshots,causalTrace:window.__aerotabViewCausalTrace?.snapshot(),ok:true,scope:'minimal rollback retry',harnessIdentity,uiScripts:[...document.scripts].map(s=>s.src).filter(Boolean),timeline,stagingEvents:window.__stagingEvidence,evidence,state:await invoke('native_view_state',{sessionId:id}),core:await invoke('native_ui_e2e_buffer',{sessionId:id})}});return;}
if(surface().querySelector('canvas')!==window.__stagingSourceCanvas||(await invoke('native_ui_e2e_buffer',{sessionId:id})).channelRegistrations!==sourceChannels)throw Error('successful staging replaced source canvas or Channel');
evidence.push({phase:'staging source before target attach',events:window.__stagingEvidence,sourceCanvasIdentityRetained:true,channelRegistrations:sourceChannels});
let state=await nativeDraw('first native draw');
const flowBefore=(await invoke('native_ui_e2e_buffer',{sessionId:id})).flow;
await invoke('native_view_focus',{sessionId:id});
const focusObserved=await wait(async()=>{const c=await invoke('native_ui_e2e_buffer',{sessionId:id});return c.flow?.generation===state.generation&&c.flow.focused&&c.trace.some(e=>e.generation===state.generation&&e.phase==='actual-focus-in-event')&&c.flow;},'actual GTK focus event and current generation',40);
evidence.push({phase:'actual focus request/event barrier',before:flowBefore,after:focusObserved,scope:'real grab_focus request; bounded observation, no fabricated focus'});
if(location.search.includes('ackClose=1')){
 const baseline=await invoke('native_ui_e2e_buffer',{sessionId:id});
 let releaseAck;const ackGate=new Promise(resolve=>{releaseAck=resolve;});
 window.__ackCloseGate={id,baseline:baseline.flow.reserved,held:false,promise:ackGate,dispatched:[]};
 await invoke('native_ui_e2e_buffer',{sessionId:id,inject:{generation:state.generation,phase:'initial native input'}});
 await wait(()=>window.__ackCloseGate.held,'real host ACK response held');
 footer().click();
 const closeButton=document.querySelector('button[aria-label="Close tab"],button[aria-label="关闭标签"]');
 if(!closeButton)throw Error('production close button missing');closeButton.click();
 await wait(async()=>!(await rpc('session.list')).some(s=>s.id===id),'production session removed');
 await wait(async()=>{try{await invoke('native_view_state',{sessionId:id});return false;}catch{return true;}},'closed host owner detached');
 const closedCalls=window.__ackCloseGate.dispatched.length;
 const causalStart=window.__aerotabViewCausalTrace.snapshot().length;
 const atClose=await invoke('native_ui_e2e_buffer',{sessionId:id,inject:{closedProbe:true}});
 releaseAck();
 const observationMs=3000;await sleep(observationMs);
 const after=await invoke('native_ui_e2e_buffer',{sessionId:id,inject:{closedProbe:true}});
 const lateCalls=window.__ackCloseGate.dispatched.slice(closedCalls);
 if(after.sessionWriteCount!==atClose.sessionWriteCount||after.ownerPresent||lateCalls.some(c=>c.cmd==='native_view_attach'||c.cmd==='native_view_update'||c.cmd==='native_view_detach'))throw Error('late callback wrote or mutated closed view');
 if(window.__aerotabViewCausalTrace.snapshot().slice(causalStart).some(e=>e.stage==='transaction.committed'))throw Error('closed transaction committed late');
 await inv('native_ui_e2e_report',{report:{ok:true,scope:'real close with delayed real host ACK response',observationMs,atClose,after,lateCalls,hardwareInput:false,limitations:['No replacement view mounted during observation','ACK accepted by host before response delay; not delayed transport execution']}});return;
}
if(location.search.includes('ackRecovery=1')){
 phase='production ACK rejection and recovery';
 const baseline=await invoke('native_ui_e2e_buffer',{sessionId:id});
 await invoke('native_ui_e2e_buffer',{sessionId:id,inject:{ackFault:true}});
 await invoke('native_ui_e2e_buffer',{sessionId:id,inject:{generation:state.generation,phase:'initial native input'}});
 const held=await wait(async()=>{const b=await invoke('native_ui_e2e_buffer',{sessionId:id});return b.ackFault?.rejects>=3&&b;},'real ACK rejection repeated');
 if(held.sessionWriteCount!==baseline.sessionWriteCount+1||held.flow.ack!==baseline.flow.ack||held.flow.pending<1||held.coreIdentity!==baseline.coreIdentity||held.channelRegistrations!==1||held.gdkMarkerCount>1)throw Error('ACK hold violated successful writer ownership');
 // Request a real reverse transaction, then change geometry while its source
 // drain remains blocked. This supersedes the captured target receipt.
 const traceStart=window.__aerotabViewCausalTrace.snapshot().length;
 footer().click();
 // ACK rejection also rejects hide/update; use the real transaction drain
 // boundary rather than waiting for an impossible GTK hide before release.
 await wait(()=>window.__aerotabViewCausalTrace.snapshot().slice(traceStart).some(e=>e.stage==='transaction.drain'),'reverse source drain entered');
 const capturedWidth=surface().getBoundingClientRect().width;
 surface().style.width='calc(100% - 8px)';
 await wait(()=>surface().getBoundingClientRect().width!==capturedWidth,'real geometry supersession');
 await invoke('native_ui_e2e_buffer',{sessionId:id,inject:{ackFault:false}});
 const drained=await wait(async()=>{const b=await invoke('native_ui_e2e_buffer',{sessionId:id});return b.flow?.ack===b.flow?.reserved&&b.flow.pending===0&&b;},'final real host ACK');
 await backend('native-surface');
 const restored=await invoke('native_view_state',{sessionId:id});
 if(restored.generation!==state.generation||!restored.visible||restored.inputFrozen)throw Error('superseded reverse did not restore retained native source');
 const after=await invoke('native_ui_e2e_buffer',{sessionId:id});
 const checks=continuity(baseline,after,identities.get(id),window.__stagingSourceCanvas);
 if(Object.values(checks).some(v=>!v)||after.sessionWriteCount-baseline.sessionWriteCount!==drained.flow.reserved-baseline.flow.reserved||after.gdkMarkerCount!==1)throw Error('ACK recovery duplicated write or replaced core/controller');
 await inv('native_ui_e2e_report',{report:{ok:true,scope:'real GTK ACK-only rejection recovery with geometry-superseded reverse transaction',baseline,held,drained,after,restored,checks,oneWritePerReservation:true,writerDelta:after.sessionWriteCount-baseline.sessionWriteCount,limitations:['Synthetic GTK input, not hardware','Supersession uses actual DOM geometry change, not a new renderer request','Close injection covered by Node production transaction tests, not this OS run']}});return;
}
await invoke('native_ui_e2e_buffer',{sessionId:id,inject:{generation:state.generation,phase:'initial native input'}});
await wait(async()=>(await text()).text.includes('GDK_BRIDGE_OK'),'synthetic GTK production writer marker');
const flowAfter=await wait(async()=>{const f=(await invoke('native_ui_e2e_buffer',{sessionId:id})).flow;return f?.ack>0&&f.ack===f.reserved&&f.pending===0&&f;},'host nonzero ACK drained');
if(flowAfter.generation!==state.generation||flowAfter.sessionId!==id||flowAfter.reserved<=flowBefore.reserved)throw Error('GTK flow correlation mismatch');
evidence.push({phase:'synthetic GDK writer',id,generation:state.generation,marker:true,before:flowBefore,after:flowAfter,syntheticGdk:true,hardwareInput:false});
if(location.search.includes('gtkOnly=1')){await inv('native_ui_e2e_report',{report:{ok:true,scope:'GTK small scenario',evidence}});return;}
if(location.search.includes('pasteMinimal=1')){
 phase='minimal native multiline approval';
 const generation=state.generation;
 const send=(scenario,requestId,gen=generation)=>invoke('native_ui_e2e_paste',{sessionId:id,generation:gen,requestId,scenario});
 const confirmation=()=>document.querySelector('dialog.app-confirm-dialog[open]');
 const markerCount=async()=> (await text()).text.split('\n').filter(line=>line.trim()==='PASTE_ALLOW_OK').length;
 const before=await invoke('native_ui_e2e_buffer',{sessionId:id});
 await send('reject',2001);await wait(confirmation,'minimal reject confirmation');
 await wait(async()=>!(await invoke('native_view_state',{sessionId:id})).visible,'minimal reject hide');
 confirmation().querySelector('button').click();await wait(()=>!confirmation(),'minimal reject dismissed');
 await wait(async()=>(await invoke('native_view_state',{sessionId:id})).visible,'minimal reject restored');
 const rejected=await invoke('native_ui_e2e_buffer',{sessionId:id});
 if(rejected.sessionWriteCount!==before.sessionWriteCount)throw Error('minimal rejected paste wrote');
 await send('old',2002,'00000000-0000-0000-0000-000000000000');
 await send('allow',2003);await wait(confirmation,'minimal allow confirmation');
 await wait(async()=>!(await invoke('native_view_state',{sessionId:id})).visible,'minimal allow hide');
 const hidden=await invoke('native_view_state',{sessionId:id});
 const beforeApprove=await invoke('native_ui_e2e_buffer',{sessionId:id});
 if(beforeApprove.sessionWriteCount!==before.sessionWriteCount)throw Error('minimal cancel or old generation wrote before approval');
 evidence.push({phase:'cancel and old generation isolated',writerBefore:before.sessionWriteCount,writerAfterReject:rejected.sessionWriteCount,writerBeforeApprove:beforeApprove.sessionWriteCount});
 const updateStart=(await invoke('native_ui_e2e_buffer',{sessionId:id})).updates.length;
 const buttons=confirmation().querySelectorAll('button');buttons[buttons.length-1].click();
 await wait(()=>!confirmation(),'minimal allow dismissed');
 await wait(async()=>(await invoke('native_view_state',{sessionId:id})).visible,'minimal actual show complete');
 const pasteWritten=await wait(async()=>{const c=await invoke('native_ui_e2e_buffer',{sessionId:id});return c.sessionWriteCount>before.sessionWriteCount&&c;},'minimal paste original writer before Enter');
 if(pasteWritten.sessionWriteCount!==before.sessionWriteCount+1)throw Error('minimal paste writer increment not exactly one before Enter');
 evidence.push({phase:'paste writer isolated before Enter',writerBefore:before.sessionWriteCount,writerAfterReject:rejected.sessionWriteCount,writerAfterPaste:pasteWritten.sessionWriteCount,markerBeforeEnter:await markerCount()});
 await invoke('native_view_focus',{sessionId:id});
 await wait(async()=>{const c=await invoke('native_ui_e2e_buffer',{sessionId:id});return c.flow?.generation===generation&&c.flow.focused&&c.trace.some(e=>e.generation===generation&&e.phase==='actual-focus-in-event');},'minimal submit actual focus event',40);
 await invoke('native_ui_e2e_buffer',{sessionId:id,inject:{generation,phase:'paste submit enter'}});
 await wait(async()=>await markerCount()===1,'minimal approved PTY marker once');
 const after=await wait(async()=>{const c=await invoke('native_ui_e2e_buffer',{sessionId:id});return c.flow.ack===c.flow.reserved&&c.flow.pending===0&&c;},'minimal Enter ACK drain');
 const shown=after.updates.slice(updateStart).find(c=>c.result?.generation===generation&&c.result.visible);
 if(!shown||after.sessionWriteCount!==pasteWritten.sessionWriteCount+1||after.flow.reserved!==pasteWritten.flow.reserved+1||await markerCount()!==1)throw Error('minimal show / isolated GTK Enter exactly once gate');
 await inv('native_ui_e2e_report',{report:{ok:true,scope:'minimal native multiline approval',harnessIdentity,generation,focusEventObserved:true,hiddenBeforeApproval:!hidden.visible,actualShowCompleted:true,shownRevision:shown.result.revision,writerBefore:before.sessionWriteCount,writerAfterReject:rejected.sessionWriteCount,writerAfterPaste:pasteWritten.sessionWriteCount,writerAfterEnter:after.sessionWriteCount,enterReservedBefore:pasteWritten.flow.reserved,enterReservedAfter:after.flow.reserved,approvedMarkerCount:1,rejectedBlocked:true,oldGenerationBlocked:true,clipboardTextRecorded:false}});return;
}
phase='dynamic revision barrier';
const initialRevision=await invoke('native_view_state',{sessionId:id});
const initialUpdate=[...(await invoke('native_ui_e2e_buffer',{sessionId:id})).updates].reverse().find(c=>c.cmd==='native_view_update'&&c.result?.generation===state.generation);
if(!initialUpdate)throw Error('missing actual UI revision request');
document.dispatchEvent(new KeyboardEvent('keydown',{key:'=',code:'Equal',ctrlKey:true,bubbles:true}));
const fontRequest=await wait(async()=>[...(await invoke('native_ui_e2e_buffer',{sessionId:id})).updates].reverse().find(c=>c.cmd==='native_view_update'&&c.args?.cfg?.fontSize!==initialUpdate.args.cfg.fontSize&&c.result?.generation===state.generation),'production font action update');
await wait(async()=>{const s=await invoke('native_view_state',{sessionId:id});return s.generation===state.generation&&s.drawnRevision>=fontRequest.args.cfg.revision&&outputReached(s)&&s;},'font drawn revision');
document.dispatchEvent(new KeyboardEvent('keydown',{key:'s',code:'KeyS',ctrlKey:true,altKey:true,bubbles:true}));
const geometryRequest=await wait(async()=>[...(await invoke('native_ui_e2e_buffer',{sessionId:id})).updates].reverse().find(c=>c.cmd==='native_view_update'&&c.result?.generation===state.generation&&c.args.geometry.width!==initialRevision.geometry.width),'production sidebar geometry update');
const revised=await wait(async()=>{const s=await invoke('native_view_state',{sessionId:id});const latest=[...(await invoke('native_ui_e2e_buffer',{sessionId:id})).updates].reverse().find(c=>c.cmd==='native_view_update'&&c.result?.generation===state.generation);return s.generation===state.generation&&s.revision>initialRevision.revision&&s.drawnRevision>=latest.args.cfg.revision&&s.drawnRevision>=geometryRequest.args.cfg.revision&&outputReached(s)&&s;},'latest geometry drawn revision');
const capturedOldUpdate=geometryRequest.args;
// Restore the real sidebar action before the existing settings-button gate.
document.dispatchEvent(new KeyboardEvent('keydown',{key:'s',code:'KeyS',ctrlKey:true,altKey:true,bubbles:true}));
await wait(async()=>{const s=await invoke('native_view_state',{sessionId:id});return s.geometry.width===initialRevision.geometry.width&&s.drawnRevision>=s.revision;},'sidebar geometry restored and drawn');
evidence.push({phase,id,before:initialRevision,after:revised,fontRequestedRevision:fontRequest.args.cfg.revision,geometryRequestedRevision:geometryRequest.args.cfg.revision});
const rect=surface().getBoundingClientRect();evidence.push({phase,id,state,rect:{x:rect.x,y:rect.y,width:rect.width,height:rect.height},text:await text()});
phase='reverse real target failure rollback';
const reverseFailureBefore=await invoke('native_view_state',{sessionId:id});
const reverseFailureCalls=calls.length;
reverseProbe=window.__reverseBoundaryProbe={...beginReverse(id,'reverse target capture failure',state.generation,drawReceipts.length),failCapture:true};
footer().click();
const consumed=await wait(async()=>{const c=(await invoke('native_ui_e2e_buffer',{sessionId:id})).snapshotFault;return c?.injected&&c.snapshotCompleted&&c.sessionId===id&&c.generation===state.generation&&footer()&&!footer().disabled&&c;},'reverse target failure restored native status');
reverseProbe.injected=consumed.injected;reverseEvents.push({event:'backend snapshot fault consumed',...consumed});
const reverseFailureAfter=await invoke('native_view_state',{sessionId:id});
if(surface().dataset.terminalViewBackend!=='native-surface'||reverseFailureAfter.generation!==reverseFailureBefore.generation||!reverseFailureAfter.visible||reverseFailureAfter.inputFrozen||calls.slice(reverseFailureCalls).some(c=>c.cmd==='native_view_detach'))throw Error('reverse failed target detached or lost source');
if(surface().querySelector('canvas')!==window.__stagingSourceCanvas||(await invoke('native_ui_e2e_buffer',{sessionId:id})).channelRegistrations!==sourceChannels)throw Error('reverse failure replaced controller canvas or Channel');
evidence.push({phase,before:reverseFailureBefore,after:reverseFailureAfter,events:reverseEvents.slice(),sourceRetained:true,scope:'actual snapshot capture then injected target rejection; next pending40 burst verifies restored source input'});
reverseProbe=null;
if(location.search.includes('reverseFaultMinimal=1')){
 const beforeInput=(await invoke('native_ui_e2e_buffer',{sessionId:id})).flow;
 const markerBefore=(await invoke('native_ui_e2e_buffer',{sessionId:id})).gdkMarkerCount;
 await invoke('native_view_focus',{sessionId:id});await invoke('native_ui_e2e_buffer',{sessionId:id,inject:{generation:state.generation,phase:'retained source after rollback'}});
 const afterInput=await wait(async()=>{const f=(await invoke('native_ui_e2e_buffer',{sessionId:id})).flow;return f.generation===state.generation&&f.reserved>beforeInput.reserved&&f.ack===f.reserved&&f.pending===0&&f;},'retained source real input after rollback');
 const markerAfter=await wait(async()=>{const n=(await invoke('native_ui_e2e_buffer',{sessionId:id})).gdkMarkerCount;return n===markerBefore+1&&n;},'rollback GTK PTY marker exactly once');
 evidence.push({phase:'retained source real GTK after rollback',beforeInput,afterInput,markerBefore,markerAfter});
 footer().click();await backend('webview');footer().click();await backend('native-surface');const retried=await nativeDraw('retry after consumed backend failure',state.generation);
 if(surface().querySelector('canvas')!==window.__stagingSourceCanvas||(await invoke('native_ui_e2e_buffer',{sessionId:id})).channelRegistrations!==sourceChannels)throw Error('minimal reverse replaced controller/channel');
 await inv('native_ui_e2e_report',{report:{ok:true,scope:'backend reverse fault minimal',consumed,beforeInput,afterInput,retried,evidence,causalTrace:window.__aerotabViewCausalTrace?.snapshot()}});return;
}
phase='producer overlap synchronization';
// Start the unchanged bounded 15-marker producer only when UI setup is complete.
await rpc('session.write',{id,data:btoa(jobCommand+"if kill -0 $E2E_JOB 2>/dev/null; then printf 'DIAG_OVERLAP_RUNNING_BEFORE\\n'; fi\n")});
const producerBefore=await wait(async()=>{const c=await invoke('native_ui_e2e_buffer',{sessionId:id});return c.rawTicks.length>0&&c.rawTicks.length<15&&c.buffer.includes('DIAG_OVERLAP_RUNNING_BEFORE')&&c;},'producer running and initial pump tick before switch');
evidence.push({phase:'producer overlap start barrier',observedAt:Date.now(),rawTicks:producerBefore.rawTicks,runningBefore:true});
phase='native pending handoff';
await invoke('native_view_focus',{sessionId:id});
const captureCountBefore=(await invoke('native_ui_e2e_buffer',{sessionId:id})).webviewCaptures.length;
const receiptCountBefore=drawReceipts.length;
const switchStartedAt=Date.now();
reverseProbe=window.__reverseBoundaryProbe=beginReverse(id,'pending40 retained reverse',state.generation,drawReceipts.length,"printf 'DIAG_REVERSE_%s_OK\\n' WINDOW1\n");
await invoke('native_ui_e2e_buffer',{sessionId:id,burst:true});
await backend('webview');
const reverseBoundary=await wait(async()=>{const events=(await invoke('native_ui_e2e_buffer',{sessionId:id})).reverseBoundaryEvents;return events.some(e=>e.event==='before actual source detach'&&matchesTransaction(e.transaction,reverseProbe)&&e.host?.generation===state.generation)&&events;},'reverse real detach observation');
const hide=reverseBoundary.find(e=>e.event==='actual GTK hide completed'&&matchesTransaction(e.transaction,reverseProbe)&&e.after?.generation===state.generation);
const detach=reverseBoundary.find(e=>e.event==='before actual source detach'&&matchesTransaction(e.transaction,reverseProbe)&&e.host?.generation===state.generation);
const dispatched=reverseBoundary.find(e=>e.event==='target input dispatched before real GTK hide'&&matchesTransaction(e.reply.transaction,reverseProbe));
const boundaryReceipt=dispatched?.reply.receipts.find(r=>matchingReceipt(r,reverseProbe));
if(!hide||hide.after.visible||hide.flowAfter.reserved!==hide.afterAdversarialOldWidget.reserved||detach.flow.reserved!==hide.flowAfter.reserved||detach.flow.ack!==detach.flow.reserved||detach.flow.pending!==0||!boundaryReceipt)throw Error('reversible hide closure / drain / real target draw gate failed');
reverseEvents.push({event:'verified GTK reversible hide boundary',hide,detach,receipt:boundaryReceipt});
if(!reverseProbe.inputSent||surface().querySelector('canvas')!==window.__stagingSourceCanvas||(await invoke('native_ui_e2e_buffer',{sessionId:id})).channelRegistrations!==sourceChannels)throw Error('reverse ownership/input/channel retention failed');
reverseProbe=null;
const footerReadyAt=Date.now();
const webviewCapture=await wait(async()=>{const captures=(await invoke('native_ui_e2e_buffer',{sessionId:id})).webviewCaptures;return captures.length>captureCountBefore&&captures[captureCountBefore];},'actual production snapshot envelope captured');
if(webviewCapture.error||webviewCapture.version!==1||webviewCapture.frameVersion!==1||decimalU64(webviewCapture.outputSeq)<=0n||webviewCapture.observedAtMs<switchStartedAt||webviewCapture.observedAtMs>footerReadyAt)throw Error('snapshot capture / footer readiness order invalid');
const receipt=await wait(()=>drawReceipts.slice(receiptCountBefore).find(r=>r.sessionId===id&&r.observedAt>=switchStartedAt),'actual successful WebView draw receipt');
if(typeof receipt.generation!=='string'||!receipt.generation||receipt.generation===state.generation||!Number.isSafeInteger(receipt.revision)||receipt.revision<=0||decimalU64(receipt.appliedOutputSeq)!==decimalU64(webviewCapture.outputSeq)||decimalU64(receipt.drawnOutputSeq)<decimalU64(webviewCapture.outputSeq)||receipt.observedAt>footerReadyAt)throw Error('actual render ticket / fixed target / readiness mismatch');
if(boundaryReceipt.generation!==receipt.generation||boundaryReceipt.revision!==receipt.revision||boundaryReceipt.appliedOutputSeq!==receipt.appliedOutputSeq||boundaryReceipt.drawnOutputSeq!==receipt.drawnOutputSeq)throw Error('transaction target draw identity mismatch');
evidence.push({phase:'WebView successful render receipt',capturedFixedTarget:webviewCapture.outputSeq,capture:webviewCapture,receipt,switchStartedAt,footerReadyAt,backend:surface().dataset.terminalViewBackend,footerDisabled:footer().disabled,renderTicketObserved:true,generationRevisionDrawnVerified:true,scope:'Exact generation/revision ticket accepted by production barrier after renderer.render returned; not compositor presentation'});
// Calibration is causal, not a wall-clock conversion: this pump read starts
// after capture and exact render receipt, and a later record must follow it.
const afterDrawReadStartedAt=Date.now();
const pumpAfterDraw=await invoke('native_ui_e2e_buffer',{sessionId:id});
const afterDrawReadEndedAt=Date.now();
if(pumpAfterDraw.rawTicks.length>=15)throw Error('producer ended before post-draw overlap anchor');
await rpc('session.write',{id,data:btoa("if kill -0 $E2E_JOB 2>/dev/null; then printf 'DIAG_OVERLAP_RUNNING_AFTER\\n'; fi\n")});
const overlap=await wait(async()=>{const c=await invoke('native_ui_e2e_buffer',{sessionId:id});return c.rawTicks.length>pumpAfterDraw.rawTicks.length&&c.buffer.includes('DIAG_OVERLAP_RUNNING_AFTER')&&c;},'new unique pump tick after actual capture/draw while producer running');
const postDrawTick=overlap.rawTicks[pumpAfterDraw.rawTicks.length];
const anchorTick=pumpAfterDraw.rawTicks.at(-1);
const overlapVerified=producerBefore.rawTicks.length>0&&producerBefore.rawTicks.length<15&&receipt.observedAt<=afterDrawReadStartedAt&&postDrawTick.order>=pumpAfterDraw.rawTicks.length&&(!anchorTick||postDrawTick.before_us>=anchorTick.after_us);
if(!overlapVerified)throw Error('causal pump/draw overlap not proven');
evidence.push({phase:'continuous producer overlap verified',overlapVerified,producerRunningBefore:true,producerRunningAfter:true,capturedFixedTarget:webviewCapture.outputSeq,receipt,clockCalibration:{method:'causal read barrier: receipt happens-before pump snapshot; subsequent unique pump record is after snapshot',captureWallMs:webviewCapture.observedAtMs,drawWallMs:receipt.observedAt,afterDrawReadStartedAt,afterDrawReadEndedAt,pumpClock:'engine monotonic microseconds; not numerically mixed with epoch milliseconds'},anchorTick,postDrawTick,rawTicksAtStart:producerBefore.rawTicks,rawTicksAtOverlap:overlap.rawTicks});
const raced=overlap;
const raceTrace=raced.trace.filter(e=>e.generation===state.generation);
const burstState=raceTrace.find(e=>e.phase==='burst');
const frozenState=hide.flowBefore;
const detachedState=raceTrace.find(e=>e.phase==='detach');
if(!burstState||burstState.pending!==40||burstState.reserved!==68||!frozenState||frozenState.pending!==40)throw Error('coverage insufficient: expected pending40 / watermark68 at burst and freeze');
if(!frozenState||!detachedState||hide.after.visible||detachedState.mapped||detachedState.ack!==detachedState.reserved||detachedState.pending!==0||frozenState.reserved!==burstState.reserved||hide.afterAdversarialOldWidget.reserved!==frozenState.reserved)throw Error('freeze watermark / ACK drain / detach ordering failed');
if(raced.raceExactCount!==1)throw Error('burst PTY exact ordered output not exactly once');
evidence.push({phase,id,burst:burstState,freeze:frozenState,detach:detachedState,exactCount:raced.raceExactCount,freezePendingObserved:frozenState.pending>0});
await invoke('native_ui_e2e_buffer',{sessionId:id,late:true});
phase='webview return';await wait(async()=>{try{await invoke('native_view_state',{sessionId:id});return false;}catch(e){return String(JSON.stringify(e)).includes('not_attached');}},'native detach');
let input=surface().querySelector('textarea');if(!input)throw Error('production IME input missing');input.value="printf 'E2E_%s_OK\\n' WEBVIEW\n";input.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertText',data:input.value}));
await wait(async()=> (await text()).text.includes('E2E_WEBVIEW_OK'),'webview production input');evidence.push({phase,id,text:await text(),activeElement:document.activeElement?.outerHTML?.slice(0,300)});
phase='native second';footer().click();await backend('native-surface');state=await nativeDraw('second native draw',state.generation);
const beforeStale=await invoke('native_view_state',{sessionId:id});
const updatesBeforeStale=(await invoke('native_ui_e2e_buffer',{sessionId:id})).updates;
let staleRejected=false,staleError=null;
try{await invoke('native_view_update',capturedOldUpdate);}catch(error){staleError=error;staleRejected=JSON.stringify(error).includes('config_revision');}
const afterStale=await invoke('native_view_state',{sessionId:id});
const updatesAfterStale=(await invoke('native_ui_e2e_buffer',{sessionId:id})).updates;
evidence.push({phase:'stale update diagnostic before assertion',before:beforeStale,request:capturedOldUpdate,after:afterStale,error:staleError,rejected:staleRejected,legalUpdatesBefore:updatesBeforeStale.filter(c=>c.result?.generation===beforeStale.generation),concurrentUpdates:updatesAfterStale.slice(updatesBeforeStale.length)});
const atomicStale=(await invoke('native_ui_e2e_buffer',{sessionId:id})).staleObservations.at(-1);
const unchangedAtomic=r=>r?.atomicGtkCallback&&r.error?.code==='config_revision'&&JSON.stringify(r.before)===JSON.stringify(r.after)&&JSON.stringify(r.flowBefore)===JSON.stringify(r.flowAfter);
// Negative controls: the checker must reject configuration/input corruption.
if(unchangedAtomic({...atomicStale,after:{...atomicStale?.after,revision:(atomicStale?.after?.revision??0)+1}})||unchangedAtomic({...atomicStale,after:{...atomicStale?.after,geometry:{width:-1}}})||unchangedAtomic({...atomicStale,flowAfter:{...atomicStale?.flowAfter,ack:999}}))throw Error('stale checker negative control failed');
evidence.push({phase:'atomic stale rejection protected state',observation:atomicStale,negativeControls:['revision','geometry','input ack'],asyncBefore:beforeStale,asyncAfter:afterStale});
if(!staleRejected||!unchangedAtomic(atomicStale))throw Error('old generation update not rejected without state mutation');
await wait(async()=>{const s=await invoke('native_view_state',{sessionId:id});return s.generation===afterStale.generation&&s.drawnRevision>=s.revision&&outputReached(s)&&s;},'legitimate coordinator draw settles after stale rejection');
evidence.push({phase:'captured old generation update rejected',id,rejected:true,before:beforeStale,after:afterStale});
const beforeLate=(await invoke('native_ui_e2e_buffer',{sessionId:id})).flow;
await invoke('native_ui_e2e_buffer',{sessionId:id,late:true});
const afterLate=await invoke('native_ui_e2e_buffer',{sessionId:id});
const oldWidgetLateBlocked=afterLate.raceExactCount===1&&beforeLate.reserved===0&&beforeLate.ack===0&&afterLate.flow.reserved===0&&afterLate.flow.ack===0&&afterLate.flow.generation===beforeLate.generation;
evidence.push({phase:'old GTK widget late check',id,blocked:oldWidgetLateBlocked,before:beforeLate,after:afterLate.flow});
await rpc('session.write',{id,data:btoa("printf 'E2E_%s_OK\\n' NATIVE_RETURN\n")});await wait(async()=> (await text()).text.includes('E2E_NATIVE_RETURN_OK'),'continued shell IO');evidence.push({phase,id,state,text:await text()});
phase='paste policy';
const pasteGeneration=state.generation;
const sendPaste=(scenario,requestId,generation=pasteGeneration)=>invoke('native_ui_e2e_paste',{sessionId:id,generation,requestId,scenario});
const enter=()=>rpc('session.write',{id,data:btoa('\r')});
const pasteBefore=await invoke('native_ui_e2e_buffer',{sessionId:id});
evidence.push({phase:'paste diagnostic before emit',targetGeneration:pasteGeneration,state:await invoke('native_view_state',{sessionId:id}),flow:pasteBefore.flow,writeCount:pasteBefore.sessionWriteCount,callbackObserverInstalled:!!eventApi?.listen});
await sendPaste('single',1001);await sleep(250);
const pasteAfter=await invoke('native_ui_e2e_buffer',{sessionId:id});
evidence.push({phase:'paste diagnostic after callback before enter',trace:pasteTrace,state:await invoke('native_view_state',{sessionId:id}),flow:pasteAfter.flow,writeCount:pasteAfter.sessionWriteCount,markerCount:pasteAfter.pasteSingleCount,confirmationVisible:!!document.querySelector('dialog.app-confirm-dialog[open]')});
await enter();await wait(async()=>(await text()).text.includes('PASTE_SINGLE_OK'),'targeted single line paste');
await sendPaste('duplicate',1001);await sendPaste('old',1002,'00000000-0000-0000-0000-000000000000');await sleep(300);await enter();await sleep(200);let pasteText=(await text()).text;if(pasteText.includes('PASTE_DUPLICATE_OK')||pasteText.includes('PASTE_OLDGEN_OK'))throw Error('stale paste wrote PTY');
const confirm=()=>document.querySelector('dialog.app-confirm-dialog[open]');
await sendPaste('reject',1003);await wait(confirm,'multiline reject confirmation');await wait(async()=>!(await invoke('native_view_state',{sessionId:id})).visible,'paste modal hides native');confirm().querySelector('button').click();await wait(()=>!confirm(),'reject dismissed');await sleep(250);if((await text()).text.includes('PASTE_REJECT_OK'))throw Error('rejected paste wrote PTY');await wait(async()=>(await invoke('native_view_state',{sessionId:id})).visible,'reject native restored');
const approveBefore=await invoke('native_ui_e2e_buffer',{sessionId:id});
await sendPaste('allow',1004);await wait(confirm,'multiline allow confirmation');const buttons=confirm().querySelectorAll('button');buttons[buttons.length-1].click();await wait(()=>!confirm(),'allow dismissed');
const approveWritten=await wait(async()=>{const c=await invoke('native_ui_e2e_buffer',{sessionId:id});return c.sessionWriteCount>approveBefore.sessionWriteCount&&c;},'approved paste writer before Enter');
if(approveWritten.sessionWriteCount!==approveBefore.sessionWriteCount+1)throw Error('approved paste writer not exactly once before Enter');
await wait(async()=>(await invoke('native_view_state',{sessionId:id})).visible,'approved native restored');
await invoke('native_view_focus',{sessionId:id});await wait(async()=>{const c=await invoke('native_ui_e2e_buffer',{sessionId:id});return c.flow?.generation===pasteGeneration&&c.flow.focused&&c.trace.some(e=>e.generation===pasteGeneration&&e.phase==='actual-focus-in-event');},'approved Enter real focus',40);
await invoke('native_ui_e2e_buffer',{sessionId:id,inject:{generation:pasteGeneration,phase:'paste submit enter'}});
await wait(async()=>(await text()).text.includes('PASTE_ALLOW_OK'),'approved multiline reaches PTY');
const approveAfter=await wait(async()=>{const c=await invoke('native_ui_e2e_buffer',{sessionId:id});return c.flow.ack===c.flow.reserved&&c.flow.pending===0&&c;},'approved Enter drain');
if(approveAfter.sessionWriteCount!==approveWritten.sessionWriteCount+1||approveAfter.flow.reserved!==approveWritten.flow.reserved+1)throw Error('approved Enter not isolated');
evidence.push({phase:'approved paste and GTK Enter isolated',writerBefore:approveBefore.sessionWriteCount,writerAfterPaste:approveWritten.sessionWriteCount,writerAfterEnter:approveAfter.sessionWriteCount,enterReservedBefore:approveWritten.flow.reserved,enterReservedAfter:approveAfter.flow.reserved});
evidence.push({phase,id,generation:pasteGeneration,single:true,duplicateBlocked:true,oldGenerationBlocked:true,rejectBlocked:true,approved:true,text:await text()});
reverseProbe=window.__reverseBoundaryProbe=beginReverse(id,'second retained reverse',pasteGeneration,drawReceipts.length,"printf 'DIAG_REVERSE_%s_OK\\n' WINDOW2\n");
footer().click();await backend('webview');
await wait(async()=> (await invoke('native_ui_e2e_buffer',{sessionId:id})).reverseBoundaryEvents.some(e=>e.event==='before actual source detach'&&matchesTransaction(e.transaction,reverseProbe)&&e.host?.generation===pasteGeneration),'second reverse real detach');
const secondBoundary=(await invoke('native_ui_e2e_buffer',{sessionId:id})).reverseBoundaryEvents.find(e=>e.event==='target input dispatched before real GTK hide'&&matchesTransaction(e.reply.transaction,reverseProbe));
if(!reverseProbe.inputSent||!secondBoundary?.reply.receipts.some(r=>matchingReceipt(r,reverseProbe)))throw Error('second reverse window input or transaction draw missing');reverseProbe=null;
await sendPaste('late',1005);await sleep(250);await enter();await sleep(250);if((await text()).text.includes('PASTE_LATE_OK'))throw Error('detached late paste wrote PTY');evidence.push({phase:'late paste blocked',id,text:await text()});footer().click();await backend('native-surface');await nativeDraw('new generation after paste detach',pasteGeneration);
phase='tab occlusion';const original=document.querySelector('[role=tab][aria-selected=true]');document.querySelector('button.tab-new-button').click();await wait(()=>document.querySelector('#picker-listbox [role=option]'),'shell picker');document.querySelector('#picker-listbox [role=option]').click();await wait(()=>document.querySelectorAll('[role=tab]').length===2,'second production tab');await wait(async()=>!(await invoke('native_view_state',{sessionId:id})).visible,'hidden inactive tab');evidence.push({phase,id,state:await invoke('native_view_state',{sessionId:id})});original.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,button:0}));await backend('native-surface');await wait(async()=>(await invoke('native_view_state',{sessionId:id})).visible,'restore original tab');
phase='settings modal';const settings=[...document.querySelectorAll('button')].find(b=>/^(Settings|设置)$/.test(b.getAttribute('aria-label')||''));if(!settings)throw Error('settings entry missing');settings.click();await wait(()=>document.querySelector('.settings-overlay-panel'),'real settings modal');await wait(async()=>!(await invoke('native_view_state',{sessionId:id})).visible,'native modal occlusion');evidence.push({phase,id,state:await invoke('native_view_state',{sessionId:id}),dialog:document.querySelector('[role=dialog]')?.getAttribute('aria-label'),activeElement:document.activeElement?.tagName});document.querySelector('.settings-overlay-panel header button').click();await wait(()=>!document.querySelector('.settings-overlay-panel'),'modal closed');await wait(async()=>(await invoke('native_view_state',{sessionId:id})).visible,'native restored after modal');
evidence.push({phase:'final',id,sessions:await rpc('session.list'),state:await invoke('native_view_state',{sessionId:id}),text:await text()});
await diagnose('FINAL');
await rpc('session.write',{id,data:btoa("wait $E2E_JOB; printf '\\nDIAG_WAIT_EXIT_%s\\n' $?; printf 'DIAG_JOB_DONE\\n'\n")});
await wait(async()=>{const c=await invoke('native_ui_e2e_buffer',{sessionId:id});return c.buffer.includes('DIAG_JOB_DONE');},'original producer wait completion',400);
evidence.push({phase:'producer waited',at:Date.now(),core:await invoke('native_ui_e2e_buffer',{sessionId:id})});
const observed=await invoke('native_ui_e2e_buffer',{sessionId:id});
const ticks=observed.rawTicks;
if(observed.reverseWindow1Count!==1||observed.reverseWindow2Count!==1||!observed.reverseWindow1AfterRace)throw Error('reverse queued input order/exactly once failed');
const continuityChecks=continuity(sourceCore,observed,identities.get(id),window.__stagingSourceCanvas);
const otherSessionEvidence=observed.otherSessions;
evidence.push({phase:'independent continuity checks',checks:continuityChecks,channelRegistrations:observed.channelRegistrations,globalChannelRegistrations:observed.globalChannelRegistrations,attachCount:observed.attachCount,otherSessions:otherSessionEvidence});
if(Object.values(continuityChecks).some(v=>v!==true)||otherSessionEvidence.some(s=>s.channelRegistrations!==1||s.attachCount!==1))throw Error('single controller/channel/core continuity failed');
evidence.push({phase:'two retained bidirectional round trips',events:reverseEvents,window1Count:observed.reverseWindow1Count,window2Count:observed.reverseWindow2Count,window1AfterNativeRace:observed.reverseWindow1AfterRace,channelRegistrations:observed.channelRegistrations,sameControllerCanvas:true,sameSession:id,scope:'same canvas plus Channel and terminal.attach uniqueness; no direct access to private controller identity'});
if(ticks.length!==15||ticks.some((r,i)=>r.tick!==i||r.order!==i||r.after_us<r.before_us||(i&&r.before_us<ticks[i-1].after_us)))throw Error('continued output raw integrity/order failed');
const stages=evidence.filter(e=>e.core?.rawTicks);if(stages.some((e,i)=>i&&e.core.rawTicks.length<stages[i-1].core.rawTicks.length))throw Error('continued output observation regressed');
if(!(await text()).alive||!observed.buffer.includes('DIAG_WAIT_EXIT_0'))throw Error('original core/producer continuity failed');
const outputAfterTicks=await invoke('native_view_state',{sessionId:id});
const finalFixed=outputWatermarks.get(outputAfterTicks.generation);
if(!outputReached(outputAfterTicks)||outputAfterTicks.drawnRevision<outputAfterTicks.revision)throw Error('final fixed output/revision barrier incomplete');
if(decimalU64(outputAfterTicks.drawnOutputSeq)<=decimalU64(finalFixed.target))throw Error('continuous output did not advance beyond fixed attach target');
evidence.push({phase:'fixed output target after 15 ticks',before:finalFixed.before,after:outputAfterTicks,fixedTarget:finalFixed.target,targetUnchanged:true,comparison:'decimal u64 BigInt'});
evidence.push({phase:'continued output verified',id,expected:15,duplicates:0,ordered:true,coreAlive:true,records:ticks});
evidence.push({phase:'input bridge scope',id,syntheticGdk:true,hostReserved:true,nonzeroHostAck:true,sequenceForged:false});
await inv('native_ui_e2e_report',{report:{nativePasteTrace:window.__aerotabNativePasteTrace?.snapshot()??[],layoutSnapshots,causalTrace:window.__aerotabViewCausalTrace?.snapshot(),ok:oldWidgetLateBlocked&&overlapVerified,overlapVerified,error:oldWidgetLateBlocked?null:'old GTK callback reserved input into new generation',evidence,calls,limitations:['Successful renderer.render and exact accepted ticket observed; compositor presentation is not proven','Retained bidirectional transactions exercised; persistent ACK-failure recovery, supersession and close injection remain unverified','Synthetic GDK widget emission is not hardware delivery; OS focus and IME acceptance remain untested','modal/focus visual verification pending','Windows/macOS not tested']}});
}catch(e){await inv('native_ui_e2e_report',{report:{nativePasteTrace:window.__aerotabNativePasteTrace?.snapshot()??[],layoutSnapshots,causalTrace:window.__aerotabViewCausalTrace?.snapshot(),ok:false,phase,error:String(e),harnessIdentity,uiScripts:[...document.scripts].map(s=>s.src).filter(Boolean),timeline,nativeState:await invoke('native_view_state',{sessionId:id}).catch(e=>({error:e})),core:await invoke('native_ui_e2e_buffer',{sessionId:id}).catch(e=>({error:String(e)})),evidence,calls,harnessState:{footerText:footer()?.textContent,footerDisabled:footer()?.disabled,backend:surface()?.dataset.terminalViewBackend,stagingEvents:window.__stagingEvidence,sourceCanvasConnected:window.__stagingSourceCanvas?.isConnected},dom:document.body.innerText.slice(-6000)}});}
}catch(e){window.__nativeUiE2eStage={stage:'top-level-error',error:String(e),url:location.href};console.error('NATIVE_UI_E2E',String(e));if(inv) await inv('native_ui_e2e_report',{report:{nativePasteTrace:window.__aerotabNativePasteTrace?.snapshot()??[],ok:false,...window.__nativeUiE2eStage}});}
})()"#;
        let script = if std::env::var("AEROTAB_REVERSE_FAULT_MINIMAL").as_deref() == Ok("1") {
            script.replace("location.search.includes('reverseFaultMinimal=1')", "true")
        } else {
            script.to_owned()
        };
        eprintln!("NATIVE_UI_E2E eval-dispatch url={:?}", window.url());
        let script = if std::env::var("AEROTAB_ACK_CLOSE").as_deref() == Ok("1") {
            script.replace("location.search.includes('ackClose=1')", "true")
        } else {
            script
        };
        let script = if std::env::var("AEROTAB_ACK_RECOVERY").as_deref() == Ok("1") {
            script.replace("location.search.includes('ackRecovery=1')", "true")
        } else {
            script.to_owned()
        };
        let script = if std::env::var("AEROTAB_NATIVE_GTK_ONLY").as_deref() == Ok("1") {
            script.replace("location.search.includes('gtkOnly=1')", "true")
        } else {
            script.to_owned()
        };
        let script = if std::env::var("AEROTAB_ROLLBACK_MINIMAL").as_deref() == Ok("1") {
            script.replace("location.search.includes('rollbackMinimal=1')", "true")
        } else {
            script
        };
        let script = if std::env::var("AEROTAB_PASTE_MINIMAL").as_deref() == Ok("1") {
            script.replace("location.search.includes('pasteMinimal=1')", "true")
        } else {
            script
        };
        if let Err(e) = window.eval(&script) {
            eprintln!("UI e2e eval failed: {e}");
            app.exit(6);
        } else {
            eprintln!("NATIVE_UI_E2E eval-dispatched");
        }
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
        let _=window.with_webview(|platform| {
            use webkit2gtk::WebViewExt;
            platform.inner().evaluate_javascript("JSON.stringify({url:location.href,readyState:document.readyState,bridge:typeof window.__TAURI_INTERNALS__?.invoke,stage:window.__nativeUiE2eStage})",None,None,None::<&webkit2gtk::gio::Cancellable>,|r| match r {Ok(v)=>eprintln!("NATIVE_UI_E2E webkit-probe {}",v),Err(e)=>eprintln!("NATIVE_UI_E2E webkit-probe-error {e}")});
        });
    });
}

#[cfg(all(debug_assertions, feature = "native-view", target_os = "linux"))]
fn observe_native_ui_e2e(window: &tauri::WebviewWindow) {
    if std::env::var_os("AEROTAB_NATIVE_UI_E2E").is_none() {
        return;
    }
    // Missing final JSON must never be interpreted as success by a runner.
    std::thread::spawn(|| {
        std::thread::park_timeout(std::time::Duration::from_secs(90));
        eprintln!("NATIVE_UI_E2E missing-final-report status=7");
        std::process::exit(7);
    });
    let _ = window.with_webview(|platform| {
        use webkit2gtk::{WebResourceExt, WebViewExt};
        let view = platform.inner();
        view.connect_load_changed(|_, event| eprintln!("NATIVE_UI_E2E webkit-load {event:?}"));
        view.connect_load_failed(|_, event, _, error| {
            eprintln!(
                "NATIVE_UI_E2E load-failed event={event:?} domain={:?} code={}",
                error.domain(),
                unsafe { (*error.as_ptr()).code }
            );
            false
        });
        view.connect_web_process_terminated(|_, reason| {
            eprintln!("NATIVE_UI_E2E web-process-terminated {reason:?}")
        });
        view.connect_resource_load_started(|_, resource, _| {
            // Only record local production resource paths, never query/auth data.
            let path = resource
                .uri()
                .and_then(|u| tauri::Url::parse(&u).ok())
                .filter(|u| u.host_str() == Some("localhost") || u.host_str() == Some("127.0.0.1"))
                .map(|u| u.path().to_string())
                .unwrap_or_else(|| "[non-local resource]".into());
            eprintln!("NATIVE_UI_E2E resource-start {path}");
            let failed_path = path.clone();
            resource.connect_failed(move |_, e| {
                eprintln!(
                    "NATIVE_UI_E2E resource-failed {failed_path} domain={:?} code={}",
                    e.domain(),
                    unsafe { (*e.as_ptr()).code }
                )
            });
            resource.connect_finished(move |_| eprintln!("NATIVE_UI_E2E resource-finished {path}"));
        });
    });
}

/// Probe the configured updater endpoint and return a structured status the
/// frontend can render in the Settings panel.
#[tauri::command]
async fn check_update(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(update)) => Ok(serde_json::json!({
            "available": true,
            "version": update.version,
            "current": update.current_version,
            "notes": update.body,
            "date": update.date.map(|d| d.to_string()),
        })),
        Ok(None) => Ok(serde_json::json!({
            "available": false,
        })),
        Err(e) => Err(e.to_string()),
    }
}

/// Download and install the latest update returned by `check_update`. The
/// updater plugin handles signature verification using the pubkey configured
/// in tauri.conf.json before applying the bundle.
#[tauri::command]
async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no update available".to_string())?;
    let mut downloaded: u64 = 0;
    update
        .download_and_install(
            |chunk, total| {
                downloaded += chunk as u64;
                tracing::info!(downloaded, total = ?total, "updater progress");
            },
            || tracing::info!("updater download finished"),
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn safe_relative_path(base_dir: &str, relative: &[String]) -> Result<PathBuf, String> {
    if base_dir.trim().is_empty() {
        return Err("base directory is empty".into());
    }
    let mut out = PathBuf::from(base_dir);
    for segment in relative {
        if segment.is_empty()
            || segment == "."
            || segment == ".."
            || segment.contains('/')
            || segment.contains('\\')
            || Path::new(segment).is_absolute()
        {
            return Err(format!("unsafe relative path segment: {segment}"));
        }
        out.push(segment);
    }
    Ok(out)
}

async fn write_chunk_to_path(
    path: impl AsRef<Path>,
    offset: u64,
    data: &str,
    create: bool,
) -> Result<(), String> {
    let path = path.as_ref();
    let bytes = BASE64
        .decode(data.as_bytes())
        .map_err(|e| format!("bad base64: {e}"))?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create(true);
    if create {
        options.truncate(true);
    }
    let mut file = options.open(path).await.map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(offset))
        .await
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes).await.map_err(|e| e.to_string())?;
    file.flush().await.map_err(|e| e.to_string())?;
    Ok(())
}

fn safe_drop_filename(name: &str) -> String {
    let base = Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("drop");
    let sanitized: String = base
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
        .collect();
    if sanitized.is_empty() {
        "drop".to_string()
    } else {
        sanitized
    }
}

/// Stage a drag-dropped file (macOS WKWebView has no `File.path`) for trzsz upload.
#[tauri::command]
async fn local_stage_drop_file(
    app: tauri::AppHandle,
    name: String,
    data: String,
) -> Result<String, String> {
    let safe_name = safe_drop_filename(&name);
    let temp_root = app
        .path()
        .cache_dir()
        .map_err(|e| e.to_string())?
        .join("drop-staging");
    tokio::fs::create_dir_all(&temp_root)
        .await
        .map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = temp_root.join(format!("{stamp}-{safe_name}"));
    write_chunk_to_path(&path, 0, &data, true).await?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
fn pick_save_file(default_name: Option<String>) -> Result<Option<String>, String> {
    let _guard = lock_file_dialog()?;
    let mut dialog = rfd::FileDialog::new();
    if let Some(name) = default_name.filter(|name| !name.trim().is_empty()) {
        dialog = dialog.set_file_name(name);
    }
    Ok(dialog
        .save_file()
        .map(|path| path.to_string_lossy().into_owned()))
}

#[tauri::command]
fn pick_open_files(directory: Option<bool>) -> Result<Option<Vec<String>>, String> {
    let _guard = lock_file_dialog()?;
    if directory.unwrap_or(false) {
        return Ok(rfd::FileDialog::new()
            .pick_folder()
            .map(|path| vec![path.to_string_lossy().into_owned()]));
    }
    Ok(rfd::FileDialog::new().pick_files().map(|paths| {
        paths
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect()
    }))
}

#[tauri::command]
fn pick_open_private_key_file() -> Result<Option<String>, String> {
    let _guard = lock_file_dialog()?;
    Ok(rfd::FileDialog::new()
        .add_filter(
            "SSH private key",
            &["pem", "key", "ppk", "pub", "txt", "asc"],
        )
        .add_filter("All files", &["*"])
        .pick_file()
        .map(|path| path.to_string_lossy().into_owned()))
}

#[tauri::command]
fn pick_directory() -> Result<Option<String>, String> {
    let _guard = lock_file_dialog()?;
    Ok(rfd::FileDialog::new()
        .pick_folder()
        .map(|path| path.to_string_lossy().into_owned()))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalPathInfo {
    kind: &'static str,
    size: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalDirEntry {
    name: String,
    kind: &'static str,
    size: u64,
}

#[derive(Debug, Serialize)]
struct LocalReadChunk {
    data: String,
}

#[tauri::command]
async fn local_stat(path: String) -> Result<LocalPathInfo, String> {
    let meta = tokio::fs::metadata(path).await.map_err(|e| e.to_string())?;
    let kind = if meta.is_file() {
        "file"
    } else if meta.is_dir() {
        "dir"
    } else {
        "other"
    };
    Ok(LocalPathInfo {
        kind,
        size: meta.len(),
    })
}

#[tauri::command]
async fn local_realpath(path: String) -> Result<String, String> {
    let path = tokio::fs::canonicalize(path)
        .await
        .map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
async fn local_read_dir(path: String) -> Result<Vec<String>, String> {
    let mut entries = tokio::fs::read_dir(path).await.map_err(|e| e.to_string())?;
    let mut names = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    Ok(names)
}

#[tauri::command]
fn local_home_dir() -> Result<String, String> {
    #[cfg(windows)]
    {
        if let Ok(profile) = std::env::var("USERPROFILE") {
            if !profile.is_empty() {
                return Ok(profile);
            }
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(home) = std::env::var("HOME") {
            if !home.is_empty() {
                return Ok(home);
            }
        }
    }
    Err("home directory is not available".into())
}

#[tauri::command]
async fn local_list_dir(path: String) -> Result<Vec<LocalDirEntry>, String> {
    if path == "__drives__" {
        return list_windows_drives().await;
    }
    let mut entries = tokio::fs::read_dir(&path)
        .await
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        let name = entry.file_name().to_string_lossy().into_owned();
        let meta = entry.metadata().await.map_err(|e| e.to_string())?;
        let kind = if meta.is_file() {
            "file"
        } else if meta.is_dir() {
            "dir"
        } else {
            "other"
        };
        out.push(LocalDirEntry {
            name,
            kind,
            size: meta.len(),
        });
    }
    out.sort_by(|a, b| {
        let a_dir = a.kind == "dir";
        let b_dir = b.kind == "dir";
        a_dir
            .cmp(&b_dir)
            .reverse()
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(out)
}

async fn list_windows_drives() -> Result<Vec<LocalDirEntry>, String> {
    let mut out = Vec::new();
    for letter in b'A'..=b'Z' {
        let drive = format!("{}:\\", letter as char);
        if std::path::Path::new(&drive).exists() {
            out.push(LocalDirEntry {
                name: drive,
                kind: "dir",
                size: 0,
            });
        }
    }
    Ok(out)
}

#[tauri::command]
async fn local_read_chunk(path: String, offset: u64, len: u64) -> Result<LocalReadChunk, String> {
    let len = usize::try_from(len.min(16 * 1024 * 1024)).map_err(|e| e.to_string())?;
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(offset))
        .await
        .map_err(|e| e.to_string())?;
    let mut buf = vec![0_u8; len];
    let n = file.read(&mut buf).await.map_err(|e| e.to_string())?;
    buf.truncate(n);
    Ok(LocalReadChunk {
        data: BASE64.encode(buf),
    })
}

#[tauri::command]
async fn local_mkdir(path: String) -> Result<(), String> {
    tokio::fs::create_dir_all(path)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn local_remove(path: String, recursive: Option<bool>) -> Result<bool, String> {
    let Ok(meta) = tokio::fs::metadata(&path).await else {
        return Ok(false);
    };
    if meta.is_dir() {
        if recursive.unwrap_or(false) {
            tokio::fs::remove_dir_all(path)
                .await
                .map_err(|e| e.to_string())?;
        } else {
            tokio::fs::remove_dir(path)
                .await
                .map_err(|e| e.to_string())?;
        }
    } else {
        tokio::fs::remove_file(path)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(true)
}

#[tauri::command]
async fn local_rename(from: String, to: String) -> Result<(), String> {
    tokio::fs::rename(from, to).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn local_copy_file(from: String, to: String) -> Result<(), String> {
    tokio::fs::copy(&from, &to)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn local_mkdir_relative(base_dir: String, relative: Vec<String>) -> Result<String, String> {
    let path = safe_relative_path(&base_dir, &relative)?;
    tokio::fs::create_dir_all(&path)
        .await
        .map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
async fn local_write_chunk(
    path: String,
    offset: u64,
    data: String,
    create: bool,
) -> Result<(), String> {
    write_chunk_to_path(path, offset, &data, create).await
}

#[tauri::command]
async fn local_write_relative_chunk(
    base_dir: String,
    relative: Vec<String>,
    offset: u64,
    data: String,
    create: bool,
) -> Result<(), String> {
    let path = safe_relative_path(&base_dir, &relative)?;
    write_chunk_to_path(path, offset, &data, create).await
}

fn restore_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        if let Err(e) = window.show() {
            tracing::warn!(error = %e, "failed to show main window from tray");
        }
        if let Err(e) = window.unminimize() {
            tracing::warn!(error = %e, "failed to unminimize main window from tray");
        }
        if let Err(e) = window.set_focus() {
            tracing::warn!(error = %e, "failed to focus main window from tray");
        }
    }
}

fn hide_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        if let Err(e) = window.hide() {
            tracing::warn!(error = %e, "failed to hide main window to tray");
        }
    }
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, TRAY_SHOW_ID, "Show AeroTab", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, TRAY_HIDE_ID, "Hide to Tray", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, TRAY_QUIT_ID, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &hide, &quit])?;
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("AeroTab")
        .menu(&menu)
        .show_menu_on_left_click(false);
    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }
    tray.build(app)?;
    Ok(())
}

fn spawn_minimize_to_tray_watcher(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            interval.tick().await;
            let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
                break;
            };
            let state = app.state::<DesktopWindowState>();
            let behavior = state.window_behavior();
            if !state.tray_available || !behavior.tray_enabled || !behavior.tray_minimize_to_tray {
                continue;
            }
            if matches!(window.is_minimized(), Ok(true)) {
                if let Err(e) = window.hide() {
                    tracing::warn!(error = %e, "failed to hide minimized window to tray");
                }
                if let Err(e) = window.unminimize() {
                    tracing::warn!(error = %e, "failed to clear minimized state after tray hide");
                }
            }
        }
    });
}

/// Desktop session introspection (Wayland vs X11, X11 forward availability).
#[tauri::command]
fn session_info(display: Option<String>) -> DesktopSessionInfo {
    desktop_session::desktop_session_info(display.as_deref())
}

fn runtime_diagnostics_enabled() -> bool {
    std::env::var("AEROTAB_RUNTIME_DIAGNOSTICS").as_deref() == Ok("1")
}

/// Explicitly opted-in, main-webview-only, bounded metadata sink. No debug port.
#[tauri::command]
fn runtime_diagnostic_snapshot(
    window: tauri::WebviewWindow,
    snapshot: serde_json::Value,
) -> Result<(), String> {
    if !runtime_diagnostics_enabled() || window.label() != MAIN_WINDOW_LABEL {
        return Err("diagnostics disabled".into());
    }
    let mut safe = serde_json::Map::new();
    for key in [
        "schema", "beats", "maxLagMs", "dialogs", "tabLists", "inert", "blockers",
    ] {
        if let Some(value) = snapshot.get(key).and_then(|v| v.as_u64()) {
            safe.insert(key.into(), value.min(1_000_000_000).into());
        }
    }
    for key in ["hidden", "modalOverlay", "nativeOccluded"] {
        if let Some(value) = snapshot.get(key).and_then(|v| v.as_bool()) {
            safe.insert(key.into(), value.into());
        }
    }
    if let Some(phase @ ("boot" | "mounting" | "mounted" | "mount-failed")) =
        snapshot.get("phase").and_then(|v| v.as_str())
    {
        safe.insert("phase".into(), phase.into());
    }
    let mut counts = serde_json::Map::new();
    for key in ["click", "contextmenu", "keydown"] {
        if let Some(value) = snapshot
            .get("counts")
            .and_then(|v| v.get(key))
            .and_then(|v| v.as_u64())
        {
            counts.insert(key.into(), value.min(1_000_000_000).into());
        }
    }
    safe.insert("counts".into(), counts.into());
    let events: Vec<_> = snapshot.get("events").and_then(|v| v.as_array()).into_iter().flatten().take(64).filter_map(|event| {
        let kind = event.get("kind")?.as_str()?;
        if !["error", "rejection"].contains(&kind) { return None; }
        let code = event.get("code")?.as_str()?;
        if !["effect_update_depth_exceeded", "state_unsafe_mutation", "hydration_mismatch", "initialization", "other"].contains(&code) { return None; }
        Some(serde_json::json!({"kind":kind,"code":code,"line":event.get("line").and_then(|v|v.as_u64()).unwrap_or(0),"column":event.get("column").and_then(|v|v.as_u64()).unwrap_or(0)}))
    }).collect();
    safe.insert("events".into(), events.into());
    let path = window
        .app_handle()
        .path()
        .app_local_data_dir()
        .map_err(|_| "diagnostic path unavailable")?
        .join("runtime-diagnostics.json");
    std::fs::create_dir_all(path.parent().ok_or("diagnostic path unavailable")?)
        .map_err(|_| "diagnostic directory failed")?;
    std::fs::write(
        path,
        serde_json::to_vec(&safe).map_err(|_| "diagnostic serialization failed")?,
    )
    .map_err(|_| "diagnostic write failed".into())
}

fn main() {
    // Before any worker threads or stores: smoke must never migrate/open user data.
    #[cfg(all(feature = "native-view", target_os = "linux"))]
    if std::env::var_os("AEROTAB_NATIVE_HOST_SMOKE").is_some()
        || (cfg!(debug_assertions) && std::env::var_os("AEROTAB_NATIVE_UI_E2E").is_some())
    {
        let root =
            std::env::temp_dir().join(format!("aerotab-native-smoke-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("smoke data directory");
        unsafe {
            std::env::set_var("XDG_DATA_HOME", root.join("data"));
            std::env::set_var("XDG_CONFIG_HOME", root.join("config"));
            std::env::set_var("XDG_CACHE_HOME", root.join("cache"));
        }
        eprintln!("native smoke isolated root: {}", root.display());
    }
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .compact()
        .init();

    tracing::info!(version = CORE_VERSION, "aerotab-app starting");

    // WebView2 defaults to white while the page loads. Use the app shell
    // color so Win10 does not flash a bright surface behind `transparent:true`.
    // Translucent window mode still uses CSS on #root; this only affects the
    // WebView2 surface before/outside painted HTML.
    #[cfg(target_os = "windows")]
    {
        // SAFETY: set_var is single-threaded here, before any threads spawn.
        unsafe {
            std::env::set_var("WEBVIEW2_DEFAULT_BACKGROUND_COLOR", "FF0b0d12");
        }
    }

    let diagnostic_script = if runtime_diagnostics_enabled() {
        format!(
            "window.__AEROTAB_RUNTIME_DIAGNOSTICS__ = true;\n{}",
            include_str!("../../../apps/ui/public/runtime-diagnostics.js")
        )
    } else {
        String::new()
    };
    tauri::Builder::default()
        .plugin(tauri::plugin::Builder::<tauri::Wry, ()>::new("runtime-diagnostics")
            .js_init_script(diagnostic_script)
            .build())
        .on_page_load(|window,payload| {
            #[cfg(all(debug_assertions,feature="native-view",target_os="linux"))]
            if std::env::var_os("AEROTAB_NATIVE_UI_E2E").is_some() {
                eprintln!("NATIVE_UI_E2E page-load label={} event={:?} url={}",window.label(),payload.event(),payload.url());
                if window.label()=="main" && payload.event()==tauri::webview::PageLoadEvent::Finished {start_native_ui_e2e(window.app_handle());}
            }
            #[cfg(all(debug_assertions, feature = "native-view", target_os = "windows"))]
            if window.label()=="main" && payload.event()==tauri::webview::PageLoadEvent::Finished { windows_ui_e2e::start(window.app_handle()); }
            let _=(window,payload);
        })
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("aerotab"));

            #[cfg(all(debug_assertions, feature="native-view", target_os="windows"))]
            let data_dir = if std::env::var("AEROTAB_WINDOWS_NATIVE_UI_E2E").as_deref()==Ok("1") {
                std::env::temp_dir().join(format!("aerotab-windows-ui-e2e-{}",std::process::id()))
            } else { data_dir };
            let rt = tauri::async_runtime::handle();
            let (dispatcher, core_state, settings_store) = rt.block_on(async {
                aerotab_core::core::init().await?;
                aerotab_core::ipc::init().await?;
                aerotab_core::ssh::init().await?;
                aerotab_core::terminal::init().await?;
                aerotab_core::serial::init().await?;
                aerotab_core::sync::init().await?;
                aerotab_core::plugins::init().await?;
                let d = Dispatcher::new();
                let state = AppState::new();

                std::fs::create_dir_all(&data_dir).ok();
                if std::env::var_os("AEROTAB_NATIVE_HOST_SMOKE").is_none() && !(cfg!(debug_assertions) && (std::env::var_os("AEROTAB_NATIVE_UI_E2E").is_some() || (cfg!(target_os="windows") && std::env::var("AEROTAB_WINDOWS_NATIVE_UI_E2E").as_deref()==Ok("1")))) {
                    aerotab_core::migrate::migrate_app_data_if_needed(&data_dir);
                }
                *state.notes.lock().await = Some(aerotab_core::notes::NotesStore::open(data_dir.join("notes")).map_err(std::io::Error::other)?);
                *state.scripts.lock().await = Some(Arc::new(aerotab_core::scripts::ScriptsService::new(aerotab_core::scripts::ScriptsStore::open(data_dir.join("scripts")).map_err(std::io::Error::other)?)));
                *state.network_resources.lock().await = Some(aerotab_core::network_resources::NetworkResourceStore::open(&data_dir)?);
                let profiles_path = data_dir.join("profiles.sled");
                let open_profiles = || aerotab_core::profile::ProfileStore::open(&profiles_path);
                let profile_locked = open_profiles()
                    .or_else(|e| {
                        let msg = e.to_string().to_lowercase();
                        let locked = msg.contains("lock")
                            || msg.contains("busy")
                            || msg.contains("resource")
                            || msg.contains("would block");
                        if locked {
                            tracing::info!("profile store locked; opening read-only for secondary instance");
                            aerotab_core::profile::ProfileStore::open_readonly(&profiles_path)
                        } else {
                            Err(e)
                        }
                    });
                match profile_locked {
                    Ok(s) => *state.profiles.lock().await = Some(s),
                    Err(e) => {
                        tracing::warn!(error = %e, "profile store open failed; trying recovery");
                        let backup = data_dir.join(format!(
                            "profiles.sled.bak-{}",
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0)
                        ));
                        if profiles_path.exists() {
                            let _ = std::fs::rename(&profiles_path, &backup);
                        }
                        match open_profiles() {
                            Ok(s) => {
                                tracing::info!(backup = %backup.display(), "profile store recovered");
                                *state.profiles.lock().await = Some(s);
                            }
                            Err(e2) => tracing::warn!(error = %e2, "profile store recovery failed"),
                        }
                    }
                }
                let mut desktop_settings_store = None;
                let settings_open = SettingsStore::open(&data_dir).or_else(|e| {
                    let msg = e.to_string().to_lowercase();
                    if msg.contains("lock") || msg.contains("busy") || msg.contains("resource") {
                        tracing::info!("settings store locked; opening read-only");
                        SettingsStore::open_readonly(&data_dir)
                    } else {
                        Err(e)
                    }
                });
                match settings_open {
                    Ok(s) => {
                        desktop_settings_store = Some(s.clone());
                        *state.settings.lock().await = Some(s);
                    }
                    Err(e) => tracing::warn!(error = %e, "settings store open failed"),
                }
                match aerotab_core::vault::VaultStore::open(&data_dir) {
                    Ok(s) => *state.vault.lock().await = Some(s),
                    Err(e) => tracing::warn!(error = %e, "vault store open failed"),
                }
                match aerotab_core::ssh::known_hosts::KnownHosts::open(&data_dir) {
                    Ok(s) => *state.known_hosts.lock().await = Some(s),
                    Err(e) => tracing::warn!(error = %e, "known_hosts open failed"),
                }
                let plugins_dir = data_dir.join("plugins");
                match state.wasm_host.load_dir(&plugins_dir).await {
                    Ok(n) => tracing::info!(loaded = n, "wasm plugins"),
                    Err(e) => tracing::warn!(error = %e, "wasm plugin load failed"),
                }

                register_all(&d, state.clone());
                Ok::<(Arc<Dispatcher>, Arc<AppState>, Option<SettingsStore>), anyhow::Error>((
                    Arc::new(d),
                    state,
                    desktop_settings_store,
                ))
            })?;

            let tray_settings = settings_store
                .as_ref()
                .and_then(|settings| settings.get("window").ok().flatten())
                .map(parse_window_behavior)
                .unwrap_or_default();
            let tray_available = if tray_settings.tray_enabled {
                match setup_tray(app) {
                    Ok(()) => true,
                    Err(e) => {
                        tracing::warn!(error = %e, "tray setup failed");
                        false
                    }
                }
            } else {
                false
            };

            let mcp = Arc::new(aerotab_core::mcp::McpRuntime::new());
            aerotab_core::mcp::register_rpc(&dispatcher, core_state.clone(), mcp.clone());
            let saved_mcp = settings_store
                .as_ref()
                .and_then(|store| store.get("mcp").ok().flatten())
                .map(serde_json::from_value::<aerotab_core::mcp::config::McpConfig>)
                .transpose();
            match saved_mcp {
                Ok(Some(config)) => {
                    if let Err(error) = rt.block_on(mcp.configure(
                        config,
                        core_state.clone(),
                        dispatcher.clone(),
                    )) {
                        tracing::warn!(%error, "MCP startup failed; listener remains disabled");
                    }
                }
                Ok(None) => {}
                Err(error) => tracing::warn!(%error, "Invalid saved MCP configuration"),
            }
            app.manage(AppRpc {
                dispatcher,
                state: core_state,
                _mcp: mcp,
            });
            app.manage(DesktopWindowState {
                settings: settings_store,
                tray_available,
                close_authorized: std::sync::atomic::AtomicBool::new(false),
            });
            set_app_handle(app.handle().clone());
            if tray_available {
                spawn_minimize_to_tray_watcher(app.handle().clone());
            }
            if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
                #[cfg(all(debug_assertions,feature="native-view",target_os="linux"))]
                observe_native_ui_e2e(&window);
                disable_native_webview_context_menus(&window);
                #[cfg(windows)]
                {
                    if let Ok(hwnd) = window.hwnd() {
                        set_parent_hwnd(hwnd.0 as usize);
                    }
                }
                #[cfg(all(unix, not(target_os = "macos")))]
                {
                    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                    if let Ok(wh) = window.window_handle() {
                        if let RawWindowHandle::Xlib(h) = wh.as_raw() {
                            set_parent_hwnd(h.window as usize);
                        }
                    }
                }
            }
            #[cfg(all(feature="native-view",target_os="linux"))]
            start_native_host_smoke(app.handle());
            #[cfg(all(debug_assertions, feature="native-view", target_os="windows"))]
            start_windows_native_host_smoke(app.handle());
            // Ensure the shell is visible even if the webview has not called show yet.
            if let Err(e) = reveal_main_window(app.handle()) {
                tracing::warn!(error = %e, "failed to show main window during setup");
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != MAIN_WINDOW_LABEL {
                return;
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<DesktopWindowState>();
                let behavior = state.window_behavior();
                if state.tray_available && behavior.tray_enabled {
                    api.prevent_close();
                    if let Err(e) = window.hide() {
                        tracing::warn!(error = %e, "failed to hide closing window to tray");
                    }
                } else if !state.close_authorized.swap(false, std::sync::atomic::Ordering::AcqRel) {
                    api.prevent_close();
                    if let Err(e) = window.emit("aerotab:prepare-close", ()) { tracing::warn!(error = %e, "failed to request safe window close"); }
                }
            }
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            TRAY_SHOW_ID => restore_main_window(app),
            TRAY_HIDE_ID => hide_main_window(app),
            TRAY_QUIT_ID => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|app, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
            | TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => restore_main_window(app),
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            runtime_diagnostic_snapshot,
            show_main_window,
            get_window_screen_rect,
            get_main_window_hwnd,
            session_info,
            open_file_transfer_window,
            close_current_window,
            authorize_main_window_close,
            rpc,
            terminal_frames,
            native_view_capabilities,
            #[cfg(all(debug_assertions, feature="native-view", target_os="windows"))]
            windows_native_e2e,
            #[cfg(all(debug_assertions,feature="native-view",target_os="linux"))]
            native_ui_e2e_report,
            #[cfg(all(debug_assertions,feature="native-view",target_os="linux"))]
            native_ui_e2e_paste,
            #[cfg(all(debug_assertions,feature="native-view",target_os="linux"))]
            native_ui_e2e_buffer,
            #[cfg(feature="native-view")]
            native_view_focus,
            #[cfg(feature="native-view")]
            native_view_attach,
            #[cfg(feature="native-view")]
            native_view_update,
            #[cfg(feature="native-view")]
            native_view_detach,
            #[cfg(feature="native-view")]
            native_view_state,
            check_update,
            install_update,
            pick_open_files,
            pick_open_private_key_file,
            pick_save_file,
            pick_directory,
            local_stat,
            local_realpath,
            local_read_dir,
            local_list_dir,
            local_home_dir,
            local_read_chunk,
            local_mkdir,
            local_remove,
            local_rename,
            local_copy_file,
            local_mkdir_relative,
            local_write_chunk,
            local_write_relative_chunk,
            local_stage_drop_file,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                let state = app.state::<AppRpc>().state.clone();
                tauri::async_runtime::block_on(async {
                    if let Some(service) = state.scripts.lock().await.clone() { service.shutdown().await; }
                });
            }
        });
}
