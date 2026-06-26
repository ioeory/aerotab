//! Desktop session introspection (Wayland vs X11, X11 forward availability).

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopSessionInfo {
    pub platform: String,
    pub wayland: bool,
    pub display: Option<String>,
    pub x11_forward_available: bool,
}

/// Inspect the current desktop session. `display_override` mirrors Settings → SSH `x11Display`.
pub fn desktop_session_info(display_override: Option<&str>) -> DesktopSessionInfo {
    #[cfg(unix)]
    {
        let wayland = std::env::var("WAYLAND_DISPLAY")
            .ok()
            .filter(|s| !s.is_empty())
            .is_some();
        let display = effective_display(display_override);
        let x11_forward_available = crate::ssh::resolve_x11_socket_path(display_override).is_some();
        let platform = if wayland {
            "wayland".to_string()
        } else if display.is_some() {
            "x11".to_string()
        } else {
            "linux".to_string()
        };
        DesktopSessionInfo {
            platform,
            wayland,
            display,
            x11_forward_available,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = display_override;
        DesktopSessionInfo {
            platform: std::env::consts::OS.to_string(),
            wayland: false,
            display: None,
            x11_forward_available: false,
        }
    }
}

fn effective_display(display_override: Option<&str>) -> Option<String> {
    display_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| std::env::var("DISPLAY").ok().filter(|s| !s.is_empty()))
}
