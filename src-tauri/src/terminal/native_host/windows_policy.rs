//! Win32 placement policy: CSS coordinates are never passed directly to HWND APIs.
use super::{Geometry, HostError};
pub(super) fn physical(g: Geometry, scale: f64) -> Result<[i32; 4], HostError> {
    g.validate()?;
    if !scale.is_finite() || !(0.5..=8.).contains(&scale) {
        return Err(HostError::new("invalid_dpi", "invalid window scale factor"));
    }
    let x = (g.x * scale).round() as i32;
    let y = (g.y * scale).round() as i32;
    Ok([
        x,
        y,
        ((g.x + g.width) * scale).round() as i32 - x,
        ((g.y + g.height) * scale).round() as i32 - y,
    ])
}
/// Keep the child HWND enabled while it remains visible and healthy.
/// "Not ready" / frozen input is discarded by `Attachment::active()` (`flow.ready`),
/// so `EnableWindow(FALSE)` is reserved for hide/error/destroy. Disabling the focused
/// child sets `GetFocus()` to NULL and routes `WM_SYSKEY*` into the tao top-level window.
pub(super) fn surface_input_enabled(visible: bool, has_error: bool) -> bool {
    visible && !has_error
}

/// Hiding, disabling or destroying the Win32 focus window leaves keystrokes with
/// no target; the parent's WM_SETFOCUS makes wry move focus into WebView2.
pub(super) fn release_focus_to_parent(had_focus: bool, enabled: bool) -> bool {
    had_focus && !enabled
}
#[derive(Default)]
pub(super) struct TextInput {
    high: Option<u16>,
}
impl TextInput {
    pub fn reset(&mut self) {
        self.high = None;
    }
    pub fn character(&mut self, unit: u16, active: bool, composing: bool) -> Option<String> {
        if !active || composing {
            self.reset();
            return None;
        }
        if (0xd800..=0xdbff).contains(&unit) {
            self.high = Some(unit);
            return None;
        }
        if (0xdc00..=0xdfff).contains(&unit) {
            return self
                .high
                .take()
                .and_then(|high| {
                    char::from_u32(0x10000 + ((high as u32 - 0xd800) << 10) + unit as u32 - 0xdc00)
                })
                .map(|c| c.to_string());
        }
        self.reset();
        char::from_u32(unit as u32).map(|c| c.to_string())
    }
}
#[cfg(test)]
mod tests {

    use super::*;
    use crate::terminal::native_host::Geometry;
    #[test]
    fn fractional_dpi_rounds_edges_not_width_independently() {
        let p = physical(
            Geometry {
                x: 1.,
                y: 3.,
                width: 101.,
                height: 51.,
            },
            1.25,
        )
        .unwrap();
        assert_eq!(p, [1, 4, 127, 64]);
        assert!(physical(Geometry::default(), f64::NAN).is_err());
        assert!(physical(Geometry::default(), 0.).is_err());
    }
    #[test]
    fn not_ready_keeps_hwnd_enabled_so_disable_does_not_null_focus() {
        // Visible + healthy stays enabled; readiness is enforced by Attachment::active().
        assert!(surface_input_enabled(true, false));
        assert!(!surface_input_enabled(false, false));
        assert!(!surface_input_enabled(true, true));
    }
    #[test]
    fn hiding_focused_view_hands_keyboard_focus_back_to_webview() {
        assert!(release_focus_to_parent(true, false));
        assert!(!release_focus_to_parent(true, true));
        assert!(!release_focus_to_parent(false, false));
        assert!(!release_focus_to_parent(false, true));
    }
    #[test]
    fn focus_handoff_runs_when_surface_stops_accepting_os_focus() {
        let enabled = surface_input_enabled(false, false);
        assert!(release_focus_to_parent(true, enabled));
        let enabled = surface_input_enabled(true, false);
        assert!(!release_focus_to_parent(true, enabled));
    }
    #[test]
    fn utf16_preedit_focus_loss_and_surrogates_do_not_leak() {
        let mut text = TextInput::default();
        assert_eq!(text.character(0xd83d, true, false), None);
        assert_eq!(text.character(0xde42, true, false).as_deref(), Some("🙂"));
        assert_eq!(text.character(0xd83d, true, false), None);
        assert_eq!(text.character(0xde42, false, false), None);
        assert_eq!(text.character(b'x' as u16, true, true), None);
        assert_eq!(
            text.character(b'y' as u16, true, false).as_deref(),
            Some("y")
        );
    }
}
