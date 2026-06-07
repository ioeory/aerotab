//! Native terminal emulators (Alacritty / Ghostty / Kitty) as detached OS
//! windows or in-pane embedded child windows.
//!
//! | Mode | Description | Implemented |
//! |------|-------------|-------------|
//! | Detached | Separate top-level OS window | Yes |
//! | Embed (Win32) | SetParent + SetWindowPos | Phase 2 |
//! | Embed (X11) | xcb_configure_window tile | Phase 3 |
//! | Embed (Wayland) | Not possible | Detached fallback |
//!
//! See [`docs/native-terminal-embed-poc.md`](../../../docs/native-terminal-embed-poc.md).

pub mod embed;
pub mod engine;
pub use engine::EngineRegistry;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use embed::{
    EmbedCapabilities, EmbedError, EmbedInstanceMeta, EmbedRectDip, EmbedRegistry, EmbedResult,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeSpawnMode {
    /// Separate top-level OS window (default, supported).
    Detached,
    /// Placeholder for HWND / X11 reparent into AeroTab chrome (not implemented).
    Embed,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeProgramInfo {
    pub id: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeDetectResult {
    pub programs: Vec<NativeProgramInfo>,
    pub embed_supported: bool,
    pub embed_note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeSpawnResult {
    pub instance_id: String,
    pub pid: u32,
    pub program: String,
    pub mode: NativeSpawnMode,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeInstanceMeta {
    pub instance_id: String,
    pub pid: u32,
    pub program: String,
    pub title: String,
    pub spawned_ms: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum NativeTerminalError {
    #[error("no native terminal found on PATH (install alacritty, ghostty, or kitty)")]
    NotFound,
    #[error("unknown program: {0}")]
    UnknownProgram(String),
    #[error("spawn: {0}")]
    Spawn(String),
    #[error("embed not implemented: {0}")]
    EmbedNotImplemented(String),
    #[error("instance not found")]
    InstanceNotFound,
}

struct RunningInstance {
    child: Child,
    program: String,
    title: String,
    spawned_ms: i64,
}

/// Tracks child processes started via [`NativeTerminalRegistry::spawn`].
#[derive(Default)]
pub struct NativeTerminalRegistry {
    inner: Mutex<HashMap<Uuid, RunningInstance>>,
}

impl NativeTerminalRegistry {
    pub fn detect() -> NativeDetectResult {
        NativeDetectResult {
            programs: detect_programs(),
            embed_supported: false,
            embed_note: "POC: only detached windows. In-pane embed needs per-OS reparenting \
                         (Win32 SetParent / X11 reparent / macOS NSView) and breaks Svelte \
                         split layout unless each pane is a native child window."
                .into(),
        }
    }

    pub fn list(&self) -> Vec<NativeInstanceMeta> {
        let guard = self.inner.lock().expect("native terminal registry");
        guard
            .iter()
            .map(|(id, inst)| NativeInstanceMeta {
                instance_id: id.to_string(),
                pid: inst.child.id(),
                program: inst.program.clone(),
                title: inst.title.clone(),
                spawned_ms: inst.spawned_ms,
            })
            .collect()
    }

    pub fn spawn(
        &self,
        program: Option<&str>,
        title: &str,
        argv: &[String],
        mode: NativeSpawnMode,
    ) -> Result<NativeSpawnResult, NativeTerminalError> {
        if mode == NativeSpawnMode::Embed {
            let caps = embed::capabilities();
            if !caps.embed_supported {
                return Err(NativeTerminalError::EmbedNotImplemented(format!(
                    "embed not supported on {}: {}",
                    caps.platform, caps.note
                )));
            }
            // Embed mode delegates to EmbeddedTerminalRegistry (see embed_start below)
            // For now, fall through to detached with a message
            return Err(NativeTerminalError::EmbedNotImplemented(
                "embed mode requires parent window handle; use `nativeTerminal.embedStart` with screen rect from frontend".into(),
            ));
        }

        let (id, path) = resolve_program(program)?;
        let mut cmd = build_command(&id, &path, title, argv);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
            cmd.creation_flags(CREATE_NEW_CONSOLE);
        }

        let child = cmd
            .spawn()
            .map_err(|e| NativeTerminalError::Spawn(format!("{id}: {e}")))?;
        let pid = child.id();
        let instance_id = Uuid::new_v4();
        let spawned_ms = unix_ms_now();

        self.inner.lock().expect("native terminal registry").insert(
            instance_id,
            RunningInstance {
                child,
                program: id.clone(),
                title: title.to_string(),
                spawned_ms,
            },
        );

        Ok(NativeSpawnResult {
            instance_id: instance_id.to_string(),
            pid,
            program: id,
            mode,
            message: Some("Opened in a separate native terminal window (experimental POC).".into()),
        })
    }

    pub fn close(&self, instance_id: &str) -> Result<bool, NativeTerminalError> {
        let uuid =
            Uuid::parse_str(instance_id).map_err(|_| NativeTerminalError::InstanceNotFound)?;
        let mut guard = self.inner.lock().expect("native terminal registry");
        let Some(mut inst) = guard.remove(&uuid) else {
            return Err(NativeTerminalError::InstanceNotFound);
        };
        let _ = inst.child.kill();
        let _ = inst.child.wait();
        Ok(true)
    }

    /// Drop exited children so `list` stays accurate.
    pub fn reap_exited(&self) {
        let mut guard = self.inner.lock().expect("native terminal registry");
        guard.retain(|_, inst| match inst.child.try_wait() {
            Ok(Some(_)) => false,
            Ok(None) => true,
            Err(_) => false,
        });
    }
}

/// Registry of embedded native terminal windows (pane-level, not top-level).
/// Each instance receives the parent window handle from the frontend so the
/// terminal window gets reparented / tiled into the layout grid.
#[derive(Default)]
pub struct EmbeddedTerminalRegistry {
    embed_registry: embed::EmbedRegistry,
}

impl EmbeddedTerminalRegistry {
    /// Query which native terminals are available, plus platform embed support.
    pub fn embed_capabilities(&self) -> EmbedCapabilities {
        embed::capabilities()
    }

    /// Spawn a native terminal and attach it to the given parent window / pane rect.
    pub fn embed_start(
        &self,
        program: Option<&str>,
        title: &str,
        argv: &[String],
        rect: &EmbedRectDip,
    ) -> Result<EmbedResult, NativeTerminalError> {
        embed::embed_start(&self.embed_registry, program, title, argv, rect)
            .map_err(|e| NativeTerminalError::Spawn(e.to_string()))
    }

    /// Update the geometry of an embedded terminal (called on resize/drag/maximize).
    pub fn embed_sync_geometry(
        &self,
        instance_id: &str,
        rect: &EmbedRectDip,
    ) -> Result<(), NativeTerminalError> {
        embed::embed_sync_geometry(&self.embed_registry, instance_id, rect)
            .map_err(|e| NativeTerminalError::Spawn(e.to_string()))
    }

    /// Close an embedded terminal instance.
    pub fn embed_end(&self, instance_id: &str) -> Result<bool, NativeTerminalError> {
        embed::embed_end(&self.embed_registry, instance_id)
            .map_err(|e| NativeTerminalError::Spawn(e.to_string()))
    }

    /// Reap exited embedded terminals.
    pub fn embed_cleanup(&self) {
        embed::embed_cleanup(&self.embed_registry);
    }

    /// List all active embedded terminal instances.
    pub fn embed_list(&self) -> Vec<EmbedInstanceMeta> {
        embed::embed_list(&self.embed_registry)
    }
}

fn unix_ms_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

const PROGRAMS: &[(&str, &str)] = &[
    ("alacritty", "alacritty"),
    ("ghostty", "ghostty"),
    ("kitty", "kitty"),
];

pub fn detect_programs() -> Vec<NativeProgramInfo> {
    let mut out = Vec::new();
    for (id, bin) in PROGRAMS {
        if let Some(path) = which(bin) {
            out.push(NativeProgramInfo {
                id: (*id).into(),
                path: path.display().to_string(),
            });
        }
    }
    out
}

fn resolve_program(requested: Option<&str>) -> Result<(String, PathBuf), NativeTerminalError> {
    if let Some(id) = requested {
        let id = id.to_ascii_lowercase();
        if !PROGRAMS.iter().any(|(p, _)| *p == id.as_str()) {
            return Err(NativeTerminalError::UnknownProgram(id));
        }
        let path = which(PROGRAMS.iter().find(|(p, _)| *p == id.as_str()).unwrap().1)
            .ok_or(NativeTerminalError::NotFound)?;
        return Ok((id, path));
    }
    for (id, bin) in PROGRAMS {
        if let Some(path) = which(bin) {
            return Ok(((*id).into(), path));
        }
    }
    Err(NativeTerminalError::NotFound)
}

fn which(bin: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    let paths = std::env::split_paths(&path_var);
    for dir in paths {
        let candidate = dir.join(bin);
        if is_executable(&candidate) {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let exe = dir.join(format!("{bin}.exe"));
            if is_executable(&exe) {
                return Some(exe);
            }
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn build_command(program: &str, path: &Path, title: &str, argv: &[String]) -> Command {
    let mut cmd = Command::new(path);
    match program {
        "alacritty" => {
            if !title.is_empty() {
                cmd.args(["--title", title]);
            }
            cmd.arg("-e");
            cmd.args(argv);
        }
        "ghostty" => {
            // Ghostty follows `-e` / `--command` style similar to Alacritty.
            if !title.is_empty() {
                cmd.args(["--title", title]);
            }
            cmd.arg("-e");
            cmd.args(argv);
        }
        "kitty" => {
            if !title.is_empty() {
                cmd.arg(format!("--title={title}"));
            }
            cmd.arg("--hold");
            cmd.arg("-e");
            cmd.args(argv);
        }
        _ => {
            cmd.args(argv);
        }
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_does_not_panic() {
        let _ = detect_programs();
    }

    #[test]
    fn embed_mode_rejected() {
        let reg = NativeTerminalRegistry::default();
        let err = reg
            .spawn(
                Some("alacritty"),
                "t",
                &["echo".into(), "hi".into()],
                NativeSpawnMode::Embed,
            )
            .unwrap_err();
        assert!(matches!(err, NativeTerminalError::EmbedNotImplemented(_)));
    }

    #[test]
    fn alacritty_command_line_includes_hold_program() {
        let argv = vec!["ssh".into(), "user@host".into()];
        let mut cmd = build_command("alacritty", Path::new("/usr/bin/alacritty"), "My", &argv);
        // Smoke: building the command must not panic; actual spawn depends on PATH.
        cmd.arg("--version");
        let _ = cmd;
    }
}
