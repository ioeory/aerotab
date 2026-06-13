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
    sync::Arc,
    time::Duration,
};

#[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
use aerotab_core::commands::set_parent_hwnd;
use aerotab_core::commands::{register_all, set_app_handle, AppState};
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

const MAIN_WINDOW_LABEL: &str = "main";
const TRAY_ID: &str = "main";
const TRAY_SHOW_ID: &str = "tray-show";
const TRAY_HIDE_ID: &str = "tray-hide";
const TRAY_QUIT_ID: &str = "tray-quit";

struct AppRpc {
    dispatcher: Arc<Dispatcher>,
}

struct DesktopWindowState {
    settings: Option<SettingsStore>,
    tray_available: bool,
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
fn close_current_window(window: tauri::WebviewWindow) -> Result<(), String> {
    window.close().map_err(|e| e.to_string())
}

#[tauri::command]
async fn rpc(
    state: tauri::State<'_, AppRpc>,
    frame: serde_json::Value,
) -> Result<serde_json::Value, String> {
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
    let resp = state.dispatcher.dispatch(req).await;
    serde_json::to_value(resp).map_err(|e| e.to_string())
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

#[tauri::command]
fn pick_save_file(default_name: Option<String>) -> Result<Option<String>, String> {
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

fn main() {
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

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("aerotab"));

            let rt = tauri::async_runtime::handle();
            let (dispatcher, settings_store) = rt.block_on(async {
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
                aerotab_core::migrate::migrate_app_data_if_needed(&data_dir);
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

                register_all(&d, state);
                Ok::<(Arc<Dispatcher>, Option<SettingsStore>), anyhow::Error>((
                    Arc::new(d),
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

            app.manage(AppRpc { dispatcher });
            app.manage(DesktopWindowState {
                settings: settings_store,
                tray_available,
            });
            set_app_handle(app.handle().clone());
            if tray_available {
                spawn_minimize_to_tray_watcher(app.handle().clone());
            }
            if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
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
            show_main_window,
            get_window_screen_rect,
            get_main_window_hwnd,
            open_file_transfer_window,
            close_current_window,
            rpc,
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
            local_mkdir_relative,
            local_write_chunk,
            local_write_relative_chunk,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
