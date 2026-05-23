//! Tauri 2 desktop shell.
//!
//! This binary embeds the same JSON-RPC [`Dispatcher`] used by the stdio
//! host (`tabby`), and exposes it to the webview through a single
//! `invoke('rpc', { frame })` command. That keeps every call site, stdio
//! tests, bench harness, and the live UI, going through the same code path.

use std::{
    io::SeekFrom,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use tabby_core::commands::{register_all, AppState};
use tabby_core::ipc::{Dispatcher, ErrorCode, Request, Response, RpcError};
use tabby_core::settings::SettingsStore;
use tabby_core::CORE_VERSION;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};
use tauri_plugin_updater::UpdaterExt;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
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
fn pick_directory() -> Result<Option<String>, String> {
    Ok(rfd::FileDialog::new()
        .pick_folder()
        .map(|path| path.to_string_lossy().into_owned()))
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
    let show = MenuItem::with_id(app, TRAY_SHOW_ID, "Show Tabby v2", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, TRAY_HIDE_ID, "Hide to Tray", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, TRAY_QUIT_ID, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &hide, &quit])?;
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Tabby v2")
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

    tracing::info!(version = CORE_VERSION, "tabby-app starting");

    // Make WebView2's default surface transparent on Windows so the
    // tauri.conf.json `transparent: true` flag actually shows through.
    // Without this the WebView paints its own opaque background over the
    // transparent native window and the opacity slider has no visible effect.
    // Format is ARGB hex; 00 alpha = fully transparent.
    #[cfg(target_os = "windows")]
    {
        // SAFETY: set_var is single-threaded here, before any threads spawn.
        unsafe {
            std::env::set_var("WEBVIEW2_DEFAULT_BACKGROUND_COLOR", "00FFFFFF");
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("tabby-v2"));

            let rt = tauri::async_runtime::handle();
            let (dispatcher, settings_store) = rt.block_on(async {
                tabby_core::core::init().await?;
                tabby_core::ipc::init().await?;
                tabby_core::ssh::init().await?;
                tabby_core::terminal::init().await?;
                tabby_core::serial::init().await?;
                tabby_core::sync::init().await?;
                tabby_core::plugins::init().await?;
                let d = Dispatcher::new();
                let state = AppState::new();

                std::fs::create_dir_all(&data_dir).ok();
                let profiles_path = data_dir.join("profiles.sled");
                match tabby_core::profile::ProfileStore::open(&profiles_path) {
                    Ok(s) => *state.profiles.lock().await = Some(s),
                    Err(e) => tracing::warn!(error = %e, "profile store open failed"),
                }
                let mut desktop_settings_store = None;
                match SettingsStore::open(&data_dir) {
                    Ok(s) => {
                        desktop_settings_store = Some(s.clone());
                        *state.settings.lock().await = Some(s);
                    }
                    Err(e) => tracing::warn!(error = %e, "settings store open failed"),
                }
                match tabby_core::vault::VaultStore::open(&data_dir) {
                    Ok(s) => *state.vault.lock().await = Some(s),
                    Err(e) => tracing::warn!(error = %e, "vault store open failed"),
                }
                match tabby_core::ssh::known_hosts::KnownHosts::open(&data_dir) {
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
            if tray_available {
                spawn_minimize_to_tray_watcher(app.handle().clone());
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
            rpc,
            check_update,
            install_update,
            pick_save_file,
            pick_directory,
            local_mkdir_relative,
            local_write_chunk,
            local_write_relative_chunk,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
