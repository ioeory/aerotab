//! GTK-independent legacy xterm key encoding. Text composition stays with the IME.

/// Configuration/geometry version only; never an engine output watermark.
#[cfg(any(not(target_os = "macos"), test))]
#[derive(Default)]
pub(super) struct ConfigRevision {
    pub applied: u32,
    pub drawn: Option<u32>,
}
#[cfg(any(not(target_os = "macos"), test))]
impl ConfigRevision {
    pub fn apply(
        &mut self,
        generation: u128,
        expected: Option<u128>,
        revision: u32,
    ) -> Result<(), &'static str> {
        if expected != Some(generation) {
            return Err("stale attachment epoch");
        }
        if revision <= self.applied {
            return Err("configuration revision must increase");
        }
        self.applied = revision;
        Ok(())
    }
    pub fn draw_complete(&mut self, ticket: u32) -> bool {
        if ticket != self.applied {
            return false;
        }
        self.drawn = Some(ticket);
        true
    }
}

/// Every callback retains its original immutable identity, never a session-only lookup.
#[cfg(any(not(target_os = "macos"), test))]
#[derive(Clone)]
pub(super) struct AttachmentLifetime<G, W> {
    generation: G,
    widget: W,
    active: std::rc::Rc<std::cell::Cell<bool>>,
}
#[cfg(any(not(target_os = "macos"), test))]
impl<G: PartialEq, W: PartialEq> AttachmentLifetime<G, W> {
    pub fn new(generation: G, widget: W) -> Self {
        Self {
            generation,
            widget,
            active: std::rc::Rc::new(std::cell::Cell::new(true)),
        }
    }
    pub fn matches(&self, generation: G, widget: W) -> bool {
        self.active.get() && self.generation == generation && self.widget == widget
    }
    pub fn invalidate(&self) {
        self.active.set(false);
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Key {
    Character(char),
    Enter,
    Backspace,
    Tab,
    Escape,
    Up,
    Down,
    Right,
    Left,
    Home,
    End,
    Insert,
    Delete,
    PageUp,
    PageDown,
    Function(u8),
    Unknown,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
    pub super_key: bool,
    /// AltGr / level-three combinations belong to the input method.
    pub level_three: bool,
}

pub(super) fn encode(key: Key, modifiers: Modifiers, app_cursor: bool) -> Option<String> {
    let m = modifiers;
    // Super chords and Ctrl+Shift text are reserved for application shortcuts.
    // In particular, copy/paste must never become ETX/SYN on the remote host.
    if m.super_key || m.level_three {
        return None;
    }
    let parameter = 1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.control);
    let cursor = match key {
        Key::Up => Some('A'),
        Key::Down => Some('B'),
        Key::Right => Some('C'),
        Key::Left => Some('D'),
        Key::Home => Some('H'),
        Key::End => Some('F'),
        _ => None,
    };
    if let Some(suffix) = cursor {
        return Some(if parameter != 1 {
            format!("\x1b[1;{parameter}{suffix}")
        } else if app_cursor {
            format!("\x1bO{suffix}")
        } else {
            format!("\x1b[{suffix}")
        });
    }
    if let Key::Function(n @ 1..=4) = key {
        let suffix = (b'P' + n - 1) as char;
        return Some(if parameter == 1 {
            format!("\x1bO{suffix}")
        } else {
            format!("\x1b[1;{parameter}{suffix}")
        });
    }
    let tilde = match key {
        Key::Insert => Some(2),
        Key::Delete => Some(3),
        Key::PageUp => Some(5),
        Key::PageDown => Some(6),
        Key::Function(n @ 5..=12) => Some([15, 17, 18, 19, 20, 21, 23, 24][n as usize - 5]),
        _ => None,
    };
    if let Some(code) = tilde {
        return Some(if parameter == 1 {
            format!("\x1b[{code}~")
        } else {
            format!("\x1b[{code};{parameter}~")
        });
    }
    let text = match key {
        Key::Enter => "\r".to_owned(),
        Key::Backspace => "\x7f".to_owned(),
        Key::Escape => "\x1b".to_owned(),
        Key::Tab if m.shift => {
            return Some(if parameter == 2 {
                "\x1b[Z".to_owned()
            } else {
                format!("\x1b[1;{parameter}Z")
            })
        }
        Key::Tab => "\t".to_owned(),
        Key::Character(_) if m.control && m.shift => return None,
        Key::Character(c) if m.control => {
            let c = c.to_ascii_lowercase();
            let control = match c {
                'a'..='z' => (c as u8 - b'a' + 1) as char,
                '@' | ' ' => '\0',
                '[' => '\x1b',
                '\\' => '\x1c',
                ']' => '\x1d',
                '^' | '6' => '\x1e',
                '_' | '-' => '\x1f',
                '?' => '\x7f',
                _ => return None,
            };
            control.to_string()
        }
        Key::Character(c) if m.alt && !c.is_control() => c.to_string(),
        // Plain Unicode, dead keys and modifier-only keys must go through IME.
        _ => return None,
    };
    Some(if m.alt { format!("\x1b{text}") } else { text })
}

/// Legacy safety policy retained only to document the pre-bridge regression.
#[cfg(test)]
pub(super) fn reserved_shortcut(key: Key, m: Modifiers) -> bool {
    if m.level_three {
        return false;
    }
    if m.super_key || (m.control && m.shift) {
        return true;
    }
    if m.control {
        return match key {
            Key::Tab => true,
            Key::Character(c) if m.alt => matches!(c.to_ascii_lowercase(), 'f' | 'e' | 's'),
            Key::Character(c) => matches!(
                c.to_ascii_lowercase(),
                't' | 'w' | 'f' | ',' | '=' | '+' | '-'
            ),
            _ => false,
        };
    }
    m.alt
        && !m.shift
        && matches!(
            key,
            Key::Up | Key::Down | Key::Left | Key::Right | Key::Character('z' | 'Z' | '[' | ']')
        )
}

/// Preedit candidates own keys until IM has had the chance to filter them.
pub(super) fn shortcut_before_ime(m: Modifiers, composing: bool) -> bool {
    !composing && !m.level_three && (m.control || m.alt)
}

#[derive(Default)]
pub(super) struct InputFlow {
    pub ready: bool,
    pub seq: u32,
    pub ack: u32,
    pub frozen: bool,
    pending: std::collections::VecDeque<usize>,
    bytes: usize,
}
impl InputFlow {
    pub fn freeze(&mut self) {
        self.frozen = true;
        self.ready = false;
    }
    pub fn acknowledge(&mut self, ready: bool, ack: u32) -> bool {
        if ack > self.seq {
            return false;
        }
        while self.ack < ack {
            self.bytes -= self.pending.pop_front().unwrap_or(0);
            self.ack += 1;
        }
        self.ready = ready && !self.frozen;
        true
    }
    pub fn reserve(&mut self, bytes: usize) -> Option<u32> {
        if !self.ready || self.pending.len() >= 256 || bytes > 65536 || self.bytes + bytes > 262144
        {
            return None;
        }
        self.seq = self.seq.checked_add(1)?;
        self.bytes += bytes;
        self.pending.push_back(bytes);
        Some(self.seq)
    }
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn clipboard_callback_valid(
    expected_epoch: u64,
    current_epoch: u64,
    same_attachment: bool,
    visible: bool,
    mapped: bool,
    focused: bool,
) -> bool {
    expected_epoch == current_epoch && same_attachment && visible && mapped && focused
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ClipboardAction {
    Copy,
    Paste,
}
pub(super) fn clipboard_action(key: Key, m: Modifiers, composing: bool) -> Option<ClipboardAction> {
    if composing || m.alt || m.super_key || m.level_three || !m.shift {
        return None;
    }
    match key {
        Key::Character('c' | 'C') if m.control => Some(ClipboardAction::Copy),
        Key::Character('v' | 'V') if m.control => Some(ClipboardAction::Paste),
        Key::Insert if !m.control => Some(ClipboardAction::Paste),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct MouseMode {
    pub click: bool,
    pub drag: bool,
    pub motion: bool,
    pub sgr: bool,
    pub utf8: bool,
}
impl MouseMode {
    pub fn active(self) -> bool {
        self.click || self.drag || self.motion
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CellPosition {
    pub col: u16,
    pub row: u16,
}
pub(super) fn cell_position(
    x: f64,
    y: f64,
    cw: u16,
    ch: u16,
    cols: u16,
    rows: u16,
) -> CellPosition {
    fn index(value: f64, size: u16, count: u16) -> u16 {
        if !value.is_finite() {
            return 0;
        }
        (((value - 12.).max(0.) / f64::from(size.max(1))) as u16).min(count.saturating_sub(1))
    }
    CellPosition {
        col: index(x, cw, cols),
        row: index(y, ch, rows),
    }
}
#[derive(Clone, Copy, Debug)]
pub(super) enum MouseAction {
    Press(u8),
    Release(u8),
    Motion(Option<u8>),
    Wheel(u8),
}
pub(super) fn encode_mouse(
    mode: MouseMode,
    action: MouseAction,
    pos: CellPosition,
    m: Modifiers,
) -> Option<Vec<u8>> {
    if !mode.active() || m.shift {
        return None;
    }
    let modifiers = 8 * u8::from(m.alt || m.super_key) + 16 * u8::from(m.control);
    let (button, release) = match action {
        MouseAction::Press(b @ 0..=2) => (b, false),
        MouseAction::Release(b @ 0..=2) => (b, true),
        MouseAction::Wheel(b @ 64..=67) => (b, false),
        MouseAction::Motion(button) => {
            if !mode.motion && (!mode.drag || button.is_none()) {
                return None;
            }
            match button {
                Some(b @ 0..=2) => (b + 32, false),
                None => (35, false),
                _ => return None,
            }
        }
        _ => return None,
    };
    let x = u32::from(pos.col) + 1;
    let y = u32::from(pos.row) + 1;
    if mode.sgr {
        let suffix = if release { 'm' } else { 'M' };
        return Some(format!("\x1b[<{};{x};{y}{suffix}", button + modifiers).into_bytes());
    }
    let button = if release { 3 } else { button } + modifiers + 32;
    let x = x + 32;
    let y = y + 32;
    let mut bytes = vec![27, b'[', b'M', button];
    if mode.utf8 {
        // 1005 uses at most two UTF-8 bytes per coordinate (2015 cells).
        if x > 2047 || y > 2047 {
            return None;
        }
        for coordinate in [x, y] {
            let mut encoded = [0; 4];
            bytes.extend_from_slice(
                char::from_u32(coordinate)?
                    .encode_utf8(&mut encoded)
                    .as_bytes(),
            );
        }
    } else {
        if x > 255 || y > 255 {
            return None;
        }
        bytes.extend_from_slice(&[x as u8, y as u8]);
    }
    Some(bytes)
}
#[derive(Default)]
pub(super) struct PointerState {
    selecting: bool,
    reported: u8,
}
impl PointerState {
    pub fn press(&mut self, button: u8, report: bool) -> bool {
        if button > 2 {
            return false;
        }
        if report {
            self.reported |= 1 << button;
        }
        if button == 0 {
            self.selecting = !report;
        }
        button == 0 && !report
    }
    pub fn selecting(&self, left_held: bool) -> bool {
        self.selecting && left_held
    }
    pub fn release(&mut self, button: u8) -> (bool, bool) {
        if button > 2 {
            return (false, false);
        }
        let local = button == 0 && self.selecting;
        if button == 0 {
            self.selecting = false;
        }
        let reported = self.reported & (1 << button) != 0;
        self.reported &= !(1 << button);
        (local, reported)
    }
}
#[derive(Default)]
pub(super) struct WheelState {
    remainder: f64,
    reporting: bool,
}
impl WheelState {
    /// GDK vertical delta is negative upwards; positive result means down.
    pub fn steps(&mut self, delta: f64, reporting: bool) -> i32 {
        if !delta.is_finite() {
            return 0;
        }
        if self.reporting != reporting || (delta != 0. && self.remainder.signum() != delta.signum())
        {
            self.remainder = 0.;
        }
        self.reporting = reporting;
        // Bound a single event's work, including pathological device deltas.
        self.remainder += delta.clamp(-32., 32.);
        let steps = self.remainder.trunc() as i32;
        self.remainder -= f64::from(steps);
        steps
    }
}

#[cfg(test)]
mod mouse_tests {
    use super::*;
    fn mode() -> MouseMode {
        MouseMode {
            click: true,
            sgr: true,
            ..MouseMode::default()
        }
    }
    fn pos() -> CellPosition {
        CellPosition { col: 4, row: 2 }
    }
    #[test]
    fn sgr_buttons_release_modifiers_and_wheel() {
        let m = Modifiers {
            alt: true,
            control: true,
            ..Modifiers::default()
        };
        for button in 0..3 {
            assert_eq!(
                encode_mouse(mode(), MouseAction::Press(button), pos(), m),
                Some(format!("\x1b[<{};5;3M", button + 24).into_bytes())
            );
            assert_eq!(
                encode_mouse(mode(), MouseAction::Release(button), pos(), m),
                Some(format!("\x1b[<{};5;3m", button + 24).into_bytes())
            );
        }
        for button in 64..=67 {
            assert_eq!(
                encode_mouse(
                    mode(),
                    MouseAction::Wheel(button),
                    pos(),
                    Modifiers::default()
                ),
                Some(format!("\x1b[<{button};5;3M").into_bytes())
            );
        }
        assert!(encode_mouse(
            mode(),
            MouseAction::Press(0),
            pos(),
            Modifiers { shift: true, ..m }
        )
        .is_none());
        assert!(encode_mouse(MouseMode::default(), MouseAction::Press(0), pos(), m).is_none());
        assert!(encode_mouse(mode(), MouseAction::Press(7), pos(), m).is_none());
    }
    #[test]
    fn click_drag_and_all_motion_are_distinct() {
        let m = Modifiers::default();
        assert!(encode_mouse(mode(), MouseAction::Motion(Some(0)), pos(), m).is_none());
        let drag = MouseMode {
            drag: true,
            click: false,
            ..mode()
        };
        assert!(encode_mouse(drag, MouseAction::Motion(None), pos(), m).is_none());
        assert_eq!(
            encode_mouse(drag, MouseAction::Motion(Some(2)), pos(), m),
            Some(b"\x1b[<34;5;3M".to_vec())
        );
        let motion = MouseMode {
            motion: true,
            ..drag
        };
        assert_eq!(
            encode_mouse(motion, MouseAction::Motion(None), pos(), m),
            Some(b"\x1b[<35;5;3M".to_vec())
        );
    }
    #[test]
    fn legacy_is_raw_bytes_and_utf8_encodes_once() {
        let legacy = MouseMode {
            sgr: false,
            ..mode()
        };
        let m = Modifiers {
            control: true,
            ..Modifiers::default()
        };
        let edge = CellPosition { col: 222, row: 94 };
        assert_eq!(
            encode_mouse(legacy, MouseAction::Press(0), edge, m),
            Some(vec![27, 91, 77, 48, 255, 127])
        );
        assert_eq!(
            encode_mouse(legacy, MouseAction::Release(2), pos(), m),
            Some(vec![27, 91, 77, 51, 37, 35])
        );
        assert!(encode_mouse(
            legacy,
            MouseAction::Press(0),
            CellPosition { col: 223, row: 0 },
            m
        )
        .is_none());
        let utf8 = MouseMode {
            utf8: true,
            ..legacy
        };
        assert_eq!(
            encode_mouse(utf8, MouseAction::Press(0), edge, m),
            Some(vec![27, 91, 77, 48, 195, 191, 127])
        );
        assert!(encode_mouse(
            utf8,
            MouseAction::Press(0),
            CellPosition { col: 2015, row: 0 },
            m
        )
        .is_none());
    }
    #[test]
    fn coordinates_clamp_to_grid_and_ignore_invalid_geometry() {
        assert_eq!(
            cell_position(-9., -1., 8, 16, 80, 24),
            CellPosition { col: 0, row: 0 }
        );
        assert_eq!(
            cell_position(28., 44., 8, 16, 80, 24),
            CellPosition { col: 2, row: 2 }
        );
        assert_eq!(
            cell_position(99999., 99999., 8, 16, 80, 24),
            CellPosition { col: 79, row: 23 }
        );
        assert_eq!(
            cell_position(f64::NAN, f64::INFINITY, 0, 0, 0, 0),
            CellPosition { col: 0, row: 0 }
        );
    }
    #[test]
    fn only_left_local_press_selects_and_reported_release_is_paired() {
        let mut s = PointerState::default();
        assert!(!s.press(2, false));
        assert!(!s.selecting(true));
        assert!(s.press(0, false));
        assert!(s.selecting(true));
        assert!(!s.selecting(false));
        assert_eq!(s.release(2), (false, false));
        assert_eq!(s.release(0), (true, false));
        assert!(!s.selecting(true));
        assert!(!s.press(1, true));
        assert_eq!(s.release(1), (false, true));
        assert_eq!(s.release(1), (false, false));
    }
    #[test]
    fn smooth_wheel_accumulates_reverses_and_resets_between_routes() {
        let mut w = WheelState::default();
        assert_eq!(w.steps(-0.4, false), 0);
        assert_eq!(w.steps(0., false), 0);
        assert_eq!(w.steps(-0.7, false), -1);
        assert_eq!(w.steps(0.5, false), 0);
        assert_eq!(w.steps(0.6, false), 1);
        assert_eq!(w.steps(0.7, true), 0);
        assert_eq!(w.steps(0.4, true), 1);
        assert_eq!(w.steps(f64::NAN, true), 0);
        assert_eq!(w.steps(-2., true), -2);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn config_revision_rejects_stale_epoch_and_stale_draw_completion() {
        let mut barrier = super::ConfigRevision::default();
        assert!(barrier.apply(7, Some(6), 1).is_err());
        assert!(barrier.apply(7, None, 1).is_err());
        assert!(barrier.apply(7, Some(7), 1).is_ok());
        let ticket = barrier.applied;
        assert!(barrier.apply(7, Some(7), 2).is_ok());
        assert!(!barrier.draw_complete(ticket));
        assert_eq!(barrier.drawn, None);
        assert!(barrier.draw_complete(2));
        assert_eq!(barrier.drawn, Some(2));
        assert!(barrier.apply(7, Some(7), 2).is_err());
        assert!(barrier.apply(7, Some(7), 1).is_err());
        assert_eq!(barrier.applied, 2);
        assert_eq!(barrier.drawn, Some(2));
    }
    #[test]
    fn attachment_lifetime_rejects_old_generation_widget_and_revoked_callbacks() {
        let lifetime = super::AttachmentLifetime::new(7u64, 11usize);
        assert!(lifetime.matches(7, 11));
        assert!(!lifetime.matches(8, 11));
        assert!(!lifetime.matches(7, 12));
        let callback = lifetime.clone();
        let mut flow = super::InputFlow::default();
        assert!(flow.acknowledge(true, 0));
        assert_eq!(flow.reserve(3), Some(1));
        flow.freeze();
        // Quiescing input is not revoking the attachment: accepted ACKs still drain.
        assert!(callback.matches(7, 11));
        assert!(flow.acknowledge(false, 1));
        assert_eq!(flow.ack, flow.seq);
        lifetime.invalidate();
        assert!(!callback.matches(7, 11));
        let replacement = super::AttachmentLifetime::new(8u64, 12usize);
        assert!(replacement.matches(8, 12));
        assert!(!callback.matches(8, 12));
    }
    #[test]
    fn frozen_flow_keeps_watermark_pending_and_cannot_be_reopened_by_late_ack() {
        let mut flow = super::InputFlow::default();
        assert!(flow.acknowledge(true, 0));
        assert_eq!(flow.reserve(3), Some(1));
        assert_eq!(flow.reserve(4), Some(2));
        flow.freeze();
        assert_eq!(flow.seq, 2);
        assert_eq!(flow.pending.len(), 2);
        assert!(flow.acknowledge(true, 1));
        assert_eq!(flow.reserve(1), None);
        assert!(flow.acknowledge(false, 2));
        assert_eq!(flow.ack, 2);
        assert!(flow.pending.is_empty());
    }
    #[test]
    fn input_flow_requires_handshake_bounds_and_monotonic_ack() {
        use super::*;
        let mut flow = InputFlow::default();
        assert_eq!(flow.reserve(1), None);
        assert!(flow.acknowledge(true, 0));
        for seq in 1..=256 {
            assert_eq!(flow.reserve(1), Some(seq));
        }
        assert_eq!(flow.reserve(1), None);
        assert!(!flow.acknowledge(true, 257));
        assert!(flow.acknowledge(true, 128));
        assert!(flow.acknowledge(true, 0)); // stale config cannot rewind ack
        assert_eq!(flow.reserve(65536), Some(257));
        assert!(flow.acknowledge(false, 257));
        assert_eq!(flow.reserve(1), None);
    }
    #[test]
    fn clipboard_late_callback_requires_uninterrupted_attachment_visibility() {
        use super::*;
        assert!(clipboard_callback_valid(1, 1, true, true, true, true));
        assert!(!clipboard_callback_valid(1, 2, true, true, true, true)); // hide then show
        assert!(!clipboard_callback_valid(1, 1, false, true, true, true)); // detach ABA
        assert!(!clipboard_callback_valid(1, 1, true, false, true, true));
        assert!(!clipboard_callback_valid(1, 1, true, true, false, true));
        assert!(!clipboard_callback_valid(1, 1, true, true, true, false));
    }
    #[test]
    fn clipboard_chords_are_exact_and_preedit_has_priority() {
        use super::*;
        let m = Modifiers {
            control: true,
            shift: true,
            ..Modifiers::default()
        };
        assert_eq!(
            clipboard_action(Key::Character('C'), m, false),
            Some(ClipboardAction::Copy)
        );
        assert_eq!(
            clipboard_action(Key::Character('v'), m, false),
            Some(ClipboardAction::Paste)
        );
        assert_eq!(
            clipboard_action(
                Key::Insert,
                Modifiers {
                    shift: true,
                    ..Modifiers::default()
                },
                false
            ),
            Some(ClipboardAction::Paste)
        );
        assert_eq!(clipboard_action(Key::Character('v'), m, true), None);
        assert_eq!(
            clipboard_action(Key::Character('v'), Modifiers { alt: true, ..m }, false),
            None
        );
        assert_eq!(
            clipboard_action(
                Key::Character('c'),
                Modifiers {
                    control: true,
                    ..Modifiers::default()
                },
                false
            ),
            None
        );
    }
    #[test]
    fn terminal_chords_bypass_simple_im_but_not_active_preedit() {
        use super::*;
        let ctrl = Modifiers {
            control: true,
            ..Modifiers::default()
        };
        let alt = Modifiers {
            alt: true,
            ..Modifiers::default()
        };
        assert!(shortcut_before_ime(ctrl, false));
        assert!(shortcut_before_ime(alt, false));
        assert!(!shortcut_before_ime(ctrl, true));
        assert!(!shortcut_before_ime(alt, true));
        assert!(!shortcut_before_ime(Modifiers::default(), false));
        assert!(!shortcut_before_ime(
            Modifiers {
                level_three: true,
                ..alt
            },
            false
        ));
    }

    #[test]
    fn default_application_shortcuts_do_not_become_remote_input() {
        use super::*;
        let ctrl = Modifiers {
            control: true,
            ..Modifiers::default()
        };
        for key in [
            Key::Character('t'),
            Key::Character('w'),
            Key::Character('f'),
            Key::Character(','),
            Key::Character('='),
            Key::Character('-'),
            Key::Tab,
        ] {
            assert!(reserved_shortcut(key, ctrl), "{key:?}");
        }
        for key in [
            Key::Up,
            Key::Down,
            Key::Left,
            Key::Right,
            Key::Character('z'),
            Key::Character('['),
            Key::Character(']'),
        ] {
            assert!(reserved_shortcut(
                key,
                Modifiers {
                    alt: true,
                    ..Modifiers::default()
                }
            ));
        }
        assert!(reserved_shortcut(
            Key::Enter,
            Modifiers {
                shift: true,
                ..ctrl
            }
        ));
        assert!(reserved_shortcut(
            Key::Character('f'),
            Modifiers { alt: true, ..ctrl }
        ));
        assert!(!reserved_shortcut(
            Key::Character('f'),
            Modifiers {
                control: true,
                alt: true,
                level_three: true,
                ..Modifiers::default()
            }
        ));
        for c in ['c', 'd', 'z', 'l'] {
            assert!(!reserved_shortcut(Key::Character(c), ctrl));
        }
    }

    use super::*;

    #[test]
    fn cursor_and_home_end_follow_application_mode() {
        for (key, suffix) in [
            (Key::Up, 'A'),
            (Key::Down, 'B'),
            (Key::Right, 'C'),
            (Key::Left, 'D'),
            (Key::Home, 'H'),
            (Key::End, 'F'),
        ] {
            for app in [false, true] {
                let prefix = if app { "\x1bO" } else { "\x1b[" };
                assert_eq!(
                    encode(key, Modifiers::default(), app),
                    Some(format!("{prefix}{suffix}"))
                );
            }
        }
    }

    #[test]
    fn navigation_and_function_keys_use_xterm_sequences() {
        for (key, expected) in [
            (Key::Insert, "\x1b[2~"),
            (Key::Delete, "\x1b[3~"),
            (Key::PageUp, "\x1b[5~"),
            (Key::PageDown, "\x1b[6~"),
            (Key::Enter, "\r"),
            (Key::Backspace, "\x7f"),
            (Key::Tab, "\t"),
            (Key::Escape, "\x1b"),
        ] {
            assert_eq!(
                encode(key, Modifiers::default(), false).as_deref(),
                Some(expected)
            );
        }
        for (index, expected) in [
            "\x1bOP", "\x1bOQ", "\x1bOR", "\x1bOS", "\x1b[15~", "\x1b[17~", "\x1b[18~", "\x1b[19~",
            "\x1b[20~", "\x1b[21~", "\x1b[23~", "\x1b[24~",
        ]
        .iter()
        .enumerate()
        {
            assert_eq!(
                encode(Key::Function(index as u8 + 1), Modifiers::default(), false).as_deref(),
                Some(*expected)
            );
        }
    }

    #[test]
    fn control_ascii_and_alt_control_are_encoded() {
        let ctrl = Modifiers {
            control: true,
            ..Modifiers::default()
        };
        for c in 'a'..='z' {
            assert_eq!(
                encode(Key::Character(c), ctrl, false),
                Some(((c as u8 - b'a' + 1) as char).to_string())
            );
        }
        for (c, expected) in [
            ('C', '\x03'),
            ('@', '\0'),
            (' ', '\0'),
            ('[', '\x1b'),
            ('\\', '\x1c'),
            (']', '\x1d'),
            ('6', '\x1e'),
            ('^', '\x1e'),
            ('-', '\x1f'),
            ('_', '\x1f'),
            ('?', '\x7f'),
        ] {
            assert_eq!(
                encode(Key::Character(c), ctrl, false),
                Some(expected.to_string())
            );
        }
        assert_eq!(
            encode(Key::Character('c'), Modifiers { alt: true, ..ctrl }, false).as_deref(),
            Some("\x1b\x03")
        );
    }

    #[test]
    fn modifiers_use_csi_parameters_or_alt_prefix() {
        let shift = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        let alt = Modifiers {
            alt: true,
            ..Modifiers::default()
        };
        let all = Modifiers {
            shift: true,
            alt: true,
            control: true,
            ..Modifiers::default()
        };
        assert_eq!(encode(Key::Tab, shift, false).as_deref(), Some("\x1b[Z"));
        assert_eq!(encode(Key::Up, all, true).as_deref(), Some("\x1b[1;8A"));
        assert_eq!(encode(Key::Home, alt, true).as_deref(), Some("\x1b[1;3H"));
        assert_eq!(
            encode(Key::Delete, all, false).as_deref(),
            Some("\x1b[3;8~")
        );
        assert_eq!(
            encode(Key::Function(1), shift, false).as_deref(),
            Some("\x1b[1;2P")
        );
        assert_eq!(
            encode(Key::Function(12), alt, false).as_deref(),
            Some("\x1b[24;3~")
        );
        assert_eq!(
            encode(Key::Character('é'), alt, false).as_deref(),
            Some("\x1bé")
        );
        assert_eq!(encode(Key::Enter, alt, false).as_deref(), Some("\x1b\r"));
    }

    #[test]
    fn text_unknown_and_application_shortcut_chords_remain_unconsumed() {
        for key in [
            Key::Character('a'),
            Key::Character('中'),
            Key::Unknown,
            Key::Function(13),
        ] {
            assert_eq!(encode(key, Modifiers::default(), false), None);
        }
        for modifiers in [
            Modifiers {
                super_key: true,
                ..Modifiers::default()
            },
            Modifiers {
                level_three: true,
                alt: true,
                ..Modifiers::default()
            },
            Modifiers {
                shift: true,
                control: true,
                ..Modifiers::default()
            },
        ] {
            assert_eq!(encode(Key::Character('C'), modifiers, false), None);
        }
        assert_eq!(
            encode(
                Key::Character('中'),
                Modifiers {
                    control: true,
                    ..Modifiers::default()
                },
                false
            ),
            None
        );
    }
}
