use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
#[cfg(windows)]
use tauri::Manager;

static APP_HANDLE: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

pub fn set_app_handle(handle: tauri::AppHandle) {
    let _ = APP_HANDLE.set(handle);
}

/// Kept for backward compat — no longer used internally.
pub fn set_parent_hwnd(_hwnd: usize) {}

#[cfg(windows)]
fn get_app_handle() -> Option<&'static tauri::AppHandle> {
    APP_HANDLE.get()
}

#[cfg(windows)]
const MAIN_WINDOW_LABEL: &str = "main";

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbedRectDip {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub device_pixel_ratio: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbedInstanceMeta {
    pub instance_id: String,
    pub pid: u32,
    pub program: String,
    pub platform: &'static str,
    pub spawned_ms: i64,
    pub has_native_window: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbedResult {
    pub instance_id: String,
    pub pid: u32,
    pub program: String,
    pub platform: &'static str,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbedCapabilities {
    pub embed_supported: bool,
    pub platform: &'static str,
    pub note: String,
    pub programs: Vec<NativeProgramInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeProgramInfo {
    pub id: String,
    pub path: String,
}

#[derive(Debug)]
pub struct EmbedInstance {
    pub instance_id: String,
    pub child: Mutex<Child>,
    pub pid: u32,
    pub program: String,
    pub spawned_ms: i64,
}

#[derive(Debug)]
pub enum EmbedError {
    Spawn(String),
    Platform(String),
    InstanceNotFound,
}

impl std::fmt::Display for EmbedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(s) => write!(f, "spawn: {s}"),
            Self::Platform(s) => write!(f, "{s}"),
            Self::InstanceNotFound => write!(f, "instance not found"),
        }
    }
}

#[derive(Default)]
pub struct EmbedRegistry {
    inner: Mutex<Vec<EmbedInstance>>,
}

impl EmbedRegistry {
    pub fn new() -> Self {
        Self::default()
    }
}

pub struct EmbeddedTerminalRegistry {
    registry: EmbedRegistry,
}

impl Default for EmbeddedTerminalRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl EmbeddedTerminalRegistry {
    pub fn new() -> Self {
        Self {
            registry: EmbedRegistry::new(),
        }
    }

    pub fn embed_capabilities(&self) -> EmbedCapabilities {
        capabilities()
    }

    pub fn embed_start(
        &self,
        program: Option<&str>,
        title: &str,
        argv: &[String],
        rect: &EmbedRectDip,
    ) -> Result<EmbedResult, EmbedError> {
        embed_start(&self.registry, program, title, argv, rect)
    }

    pub fn embed_sync_geometry(
        &self,
        instance_id: &str,
        rect: &EmbedRectDip,
    ) -> Result<(), EmbedError> {
        embed_sync_geometry(&self.registry, instance_id, rect)
    }

    pub fn embed_end(&self, instance_id: &str) -> Result<bool, EmbedError> {
        embed_end(&self.registry, instance_id)
    }

    pub fn embed_list(&self) -> Vec<EmbedInstanceMeta> {
        embed_list(&self.registry)
    }
}

// ---------------------------------------------------------------------------
// Program detection
// ---------------------------------------------------------------------------

fn detect_programs() -> Vec<NativeProgramInfo> {
    let candidates: &[(&str, &str)] = &[
        ("alacritty", "alacritty"),
        ("ghostty", "ghostty"),
        ("kitty", "kitty"),
        ("wt", "wt"),
        ("wezterm", "wezterm"),
    ];
    let mut out = Vec::new();
    for (id, exe) in candidates {
        let exe_with_ext = if cfg!(windows) {
            format!("{exe}.exe")
        } else {
            exe.to_string()
        };
        if let Some(path) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path) {
                let full = dir.join(&exe_with_ext);
                if full.is_file() {
                    out.push(NativeProgramInfo {
                        id: id.to_string(),
                        path: full.to_string_lossy().to_string(),
                    });
                    break;
                }
            }
        }
    }
    out
}

fn resolve_program(program: Option<&str>) -> Result<(String, PathBuf), EmbedError> {
    let programs = detect_programs();
    if programs.is_empty() {
        return Err(EmbedError::Platform(
            "no native terminal emulators found on PATH".into(),
        ));
    }
    if let Some(name) = program {
        for p in &programs {
            if p.id == name {
                return Ok((p.id.clone(), PathBuf::from(&p.path)));
            }
        }
        return Err(EmbedError::Platform(format!(
            "requested terminal '{name}' not found on PATH"
        )));
    }
    let first = &programs[0];
    Ok((first.id.clone(), PathBuf::from(&first.path)))
}

fn default_shell_argv() -> Vec<String> {
    #[cfg(windows)]
    {
        if let Ok(comspec) = std::env::var("ComSpec") {
            return vec![comspec];
        }
        vec!["cmd.exe".to_string()]
    }
    #[cfg(not(windows))]
    {
        if let Ok(shell) = std::env::var("SHELL") {
            return vec![shell, "-l".to_string()];
        }
        vec!["/bin/sh".to_string(), "-l".to_string()]
    }
}

fn build_command(_prog_id: &str, path: &Path, title: &str, argv: &[String]) -> Command {
    let mut cmd = Command::new(path);
    cmd.args(["-e"]);
    cmd.arg(title);
    cmd.args(["--"]);
    cmd.args(argv);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd
}

fn unix_ms_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn platform_name() -> &'static str {
    #[cfg(windows)]
    {
        "win32"
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if std::env::var("WAYLAND_DISPLAY")
            .ok()
            .filter(|s| !s.is_empty())
            .is_some()
        {
            "wayland"
        } else {
            "x11"
        }
    }
    #[cfg(target_os = "macos")]
    {
        "macos"
    }
}

// ---------------------------------------------------------------------------
// Capabilities
// ---------------------------------------------------------------------------

pub fn capabilities() -> EmbedCapabilities {
    #[cfg(windows)]
    {
        EmbedCapabilities {
            embed_supported: true,
            platform: "win32",
            note: "Win32 CreateProcess + STARTUPINFO positioning.".into(),
            programs: detect_programs(),
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let is_wayland = std::env::var("WAYLAND_DISPLAY")
            .ok()
            .filter(|s| !s.is_empty())
            .is_some();
        if is_wayland {
            EmbedCapabilities {
                embed_supported: false,
                platform: "wayland",
                note: "Wayland prohibits foreign window reparenting.".into(),
                programs: detect_programs(),
            }
        } else {
            EmbedCapabilities {
                embed_supported: true,
                platform: "x11",
                note: "X11 tiling.".into(),
                programs: detect_programs(),
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        EmbedCapabilities {
            embed_supported: false,
            platform: "macos",
            note: "macOS embedding not implemented.".into(),
            programs: detect_programs(),
        }
    }
}

// ---------------------------------------------------------------------------
// Start / sync / end / list
// ---------------------------------------------------------------------------

#[cfg_attr(not(windows), allow(unused_mut, unused_variables))]
pub fn embed_start(
    registry: &EmbedRegistry,
    program: Option<&str>,
    title: &str,
    argv: &[String],
    rect: &EmbedRectDip,
) -> Result<EmbedResult, EmbedError> {
    let (prog_id, path) = resolve_program(program)?;
    let argv = if argv.is_empty() {
        default_shell_argv()
    } else {
        argv.to_vec()
    };

    #[cfg(windows)]
    let (found_hwnd, pid, child) = {
        let (hwnd, pid, child) = spawn_and_position(&prog_id, &path, title, &argv, rect)?;
        (hwnd, pid, child)
    };
    #[cfg(not(windows))]
    let (found_hwnd, pid, child) = {
        let mut cmd = build_command(&prog_id, &path, title, &argv);
        let child = cmd
            .spawn()
            .map_err(|e| EmbedError::Spawn(format!("{prog_id}: {e}")))?;
        (0isize, child.id(), child)
    };

    let spawned_ms = unix_ms_now();
    let instance_id = uuid::Uuid::new_v4().to_string();

    let mut guard = registry
        .inner
        .lock()
        .map_err(|e| EmbedError::Platform(e.to_string()))?;
    guard.push(EmbedInstance {
        instance_id: instance_id.clone(),
        child: Mutex::new(child),
        pid,
        program: prog_id.clone(),
        spawned_ms,
    });

    #[cfg(windows)]
    let diag = {
        let app = get_app_handle();
        let inner_str = if let Some(app) = app {
            if let Some(w) = app.get_webview_window(MAIN_WINDOW_LABEL) {
                if let Ok(pos) = w.inner_position() {
                    let s = w.scale_factor().unwrap_or(1.0);
                    let tx = (rect.x * s) as i32;
                    let ty = (rect.y * s) as i32;
                    format!(" inner=({},{}) target=({},{})", pos.x, pos.y, tx, ty)
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };
        format!(
            "pid={pid} hwnd={found_hwnd}{inner_str} pane=({:.0},{:.0} {}x{}@{:.1}) embed=true",
            rect.x, rect.y, rect.width, rect.height, rect.device_pixel_ratio
        )
    };
    #[cfg(not(windows))]
    let diag = format!("pid={pid} embed=true");

    Ok(EmbedResult {
        instance_id,
        pid,
        program: prog_id.clone(),
        platform: platform_name(),
        message: Some(diag),
    })
}

pub fn embed_sync_geometry(
    registry: &EmbedRegistry,
    _instance_id: &str,
    _rect: &EmbedRectDip,
) -> Result<(), EmbedError> {
    let _guard = registry
        .inner
        .lock()
        .map_err(|e| EmbedError::Platform(e.to_string()))?;
    Ok(())
}

pub fn embed_end(registry: &EmbedRegistry, instance_id: &str) -> Result<bool, EmbedError> {
    let mut guard = registry
        .inner
        .lock()
        .map_err(|e| EmbedError::Platform(e.to_string()))?;
    let pos = guard
        .iter()
        .position(|inst| inst.instance_id == instance_id);

    if let Some(idx) = pos {
        let inst = guard.remove(idx);
        let mut child = inst
            .child
            .into_inner()
            .map_err(|_| EmbedError::Platform("mutex poisoned".into()))?;
        let _ = child.kill();
        let _ = child.wait();
        return Ok(true);
    }
    Ok(false)
}

pub fn embed_list(registry: &EmbedRegistry) -> Vec<EmbedInstanceMeta> {
    let guard = match registry.inner.lock() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };
    guard
        .iter()
        .map(|inst| EmbedInstanceMeta {
            instance_id: inst.instance_id.clone(),
            pid: inst.pid,
            program: inst.program.clone(),
            platform: platform_name(),
            spawned_ms: inst.spawned_ms,
            has_native_window: false,
        })
        .collect()
}

pub fn embed_cleanup(registry: &EmbedRegistry) {
    let mut guard = match registry.inner.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    guard.retain(|inst| {
        let mut child = match inst.child.lock() {
            Ok(c) => c,
            Err(_) => return false,
        };
        child.try_wait().is_ok_and(|s| s.is_some())
    });
}

// ---------------------------------------------------------------------------
// Win32 — CreateProcess with STARTUPINFO position
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn spawn_and_position(
    prog_id: &str,
    path: &Path,
    title: &str,
    argv: &[String],
    rect: &EmbedRectDip,
) -> Result<(isize, u32, Child), EmbedError> {
    use std::os::windows::process::CommandExt;
    use std::process::Command as StdCommand;
    use tauri::Manager;
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, SetWindowPos, HWND_TOPMOST, SWP_SHOWWINDOW,
    };

    let app = get_app_handle().ok_or_else(|| EmbedError::Platform("no AppHandle".into()))?;
    let window = app
        .get_webview_window(MAIN_WINDOW_LABEL)
        .ok_or_else(|| EmbedError::Platform("main window not found".into()))?;
    let scale = window.scale_factor().unwrap_or(rect.device_pixel_ratio);
    let inner = window
        .inner_position()
        .map_err(|e| EmbedError::Platform(format!("inner_position(): {e}")))?;

    let screen_x = inner.x + (rect.x * scale) as i32;
    let screen_y = inner.y + (rect.y * scale) as i32;
    let px_w = (rect.width * scale) as i32;
    let px_h = (rect.height * scale) as i32;

    let mut cmd = build_command(prog_id, path, title, argv);
    let mut child = cmd
        .spawn()
        .map_err(|e| EmbedError::Spawn(format!("{prog_id}: {e}")))?;
    let pid = child.id();

    // Find the window (EnumWindows with retry)
    let search = Mutex::new(Win32SearchState {
        target_pid: pid,
        found_hwnd: 0,
    });
    let mut found: isize = 0;
    for _ in 0..50 {
        search.lock().unwrap().found_hwnd = 0;
        unsafe {
            let lparam = LPARAM(&search as *const _ as isize);
            let _ = EnumWindows(Some(win32_enum_proc), lparam);
        }
        found = search.lock().unwrap().found_hwnd;
        if found != 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    if found == 0 {
        // Kill the process, it didn't create a window we can find
        let _ = child.kill();
        return Err(EmbedError::Platform(
            "terminal window not found after 5s".into(),
        ));
    }

    // NO owner, NO style changes — just HWND_TOPMOST + SetWindowPos
    unsafe {
        let child_hwnd = HWND(found as *mut std::ffi::c_void);
        let _ = SetWindowPos(
            child_hwnd,
            HWND_TOPMOST,
            screen_x,
            screen_y,
            px_w,
            px_h,
            SWP_SHOWWINDOW,
        );
    }

    Ok((found, pid, child))
}

#[cfg(windows)]
struct Win32SearchState {
    target_pid: u32,
    found_hwnd: isize,
}

#[cfg(windows)]
unsafe extern "system" fn win32_enum_proc(
    hwnd: windows::Win32::Foundation::HWND,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::BOOL {
    use windows::Win32::Foundation::BOOL;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindow, GetWindowThreadProcessId, IsWindowVisible, GW_OWNER,
    };

    let state: &Mutex<Win32SearchState> = &*(lparam.0 as *const Mutex<Win32SearchState>);
    let mut s = match state.lock() {
        Ok(s) => s,
        Err(_) => return BOOL(0),
    };

    let owner = GetWindow(hwnd, GW_OWNER);
    if let Ok(owner_hwnd) = owner {
        if !owner_hwnd.0.is_null() {
            return BOOL(1);
        }
    }

    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }

    let mut proc_id: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut proc_id));

    if proc_id == s.target_pid {
        s.found_hwnd = hwnd.0 as isize;
        return BOOL(0);
    }

    BOOL(1)
}
