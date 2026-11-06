pub use native_terminal_core::{Cell, Snapshot, Terminal, input, pty};
pub mod handoff;
pub mod interaction;
pub mod load;
pub mod terminal_gl;
pub mod terminal_session;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    pub hidden: bool,
    pub modal: bool,
}

impl Policy {
    pub fn native_visible(self) -> bool {
        !self.hidden && !self.modal
    }
    pub fn native_accepts_input(self) -> bool {
        self.native_visible()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CssRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

pub fn map_css_rect(rect: CssRect, dpr: f64, gtk_scale: i32) -> Option<(i32, i32, i32, i32)> {
    if gtk_scale <= 0 || !dpr.is_finite() || dpr <= 0.0 {
        return None;
    }
    let factor = dpr / f64::from(gtk_scale);
    let values = [rect.x, rect.y, rect.width, rect.height];
    if values
        .iter()
        .any(|v| !v.is_finite() || *v < 0.0 || *v * factor > 32768.0)
        || rect.width <= 0.0
        || rect.height <= 0.0
    {
        return None;
    }
    Some((
        (rect.x * factor).round() as i32,
        (rect.y * factor).round() as i32,
        (rect.width * factor).round() as i32,
        (rect.height * factor).round() as i32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modal_obscures_native_and_blocks_input_until_closed() {
        let mut policy = Policy::default();
        assert!(policy.native_visible());
        policy.modal = true;
        assert!(!policy.native_visible());
        assert!(!policy.native_accepts_input());
        policy.modal = false;
        assert!(policy.native_visible());
        assert!(policy.native_accepts_input());
    }

    #[test]
    fn modal_does_not_undo_user_hidden_state() {
        let mut policy = Policy {
            hidden: true,
            modal: true,
        };
        policy.modal = false;
        assert!(!policy.native_visible());
        assert!(!policy.native_accepts_input());
        policy.hidden = false;
        assert!(policy.native_visible());
    }
}
