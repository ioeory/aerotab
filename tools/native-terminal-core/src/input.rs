//! Backend-neutral key encoding; platform adapters translate their event enums here.
#[derive(Clone, Copy)]
pub struct InputGate {
    pub epoch: u64,
    pub focused: bool,
    pub hidden: bool,
    pub modal: bool,
}
impl InputGate {
    pub fn accepts(self, epoch: u64) -> bool {
        epoch == self.epoch && self.focused && !self.hidden && !self.modal
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Key<'a> {
    Text(&'a str),
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
    Delete,
    Insert,
    PageUp,
    PageDown,
}
#[derive(Clone, Copy, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
    pub super_key: bool,
}
pub fn encode_key(
    key: Key<'_>,
    text: Option<&str>,
    m: Modifiers,
    application_cursor: bool,
    composing: bool,
) -> Vec<u8> {
    if composing || m.super_key {
        return Vec::new();
    }
    let modifier = 1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.control);
    let s = match key {
        Key::Enter => "\r".into(),
        Key::Backspace => "\x7f".into(),
        Key::Tab if m.shift => "\x1b[Z".into(),
        Key::Tab => "\t".into(),
        Key::Escape => "\x1b".into(),
        Key::Up | Key::Down | Key::Right | Key::Left | Key::Home | Key::End => {
            let c = match key {
                Key::Up => 'A',
                Key::Down => 'B',
                Key::Right => 'C',
                Key::Left => 'D',
                Key::Home => 'H',
                _ => 'F',
            };
            if modifier > 1 {
                format!("\x1b[1;{modifier}{c}")
            } else {
                format!("\x1b{}{c}", if application_cursor { 'O' } else { '[' })
            }
        }
        Key::Delete => "\x1b[3~".into(),
        Key::Insert => "\x1b[2~".into(),
        Key::PageUp => "\x1b[5~".into(),
        Key::PageDown => "\x1b[6~".into(),
        Key::Text(character) => {
            let mut bytes = if m.control {
                let c = character
                    .chars()
                    .next()
                    .unwrap_or('\0')
                    .to_ascii_uppercase();
                vec![match c {
                    '@'..='_' => c as u8 & 31,
                    ' ' | '2' => 0,
                    '?' => 127,
                    _ => return Vec::new(),
                }]
            } else {
                text.unwrap_or(character).as_bytes().to_vec()
            };
            if m.alt {
                bytes.insert(0, 27);
            }
            return bytes;
        }
    };
    s.into_bytes()
}
pub fn encode_commit(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}
#[derive(Default)]
pub struct InputState {
    pub enabled: bool,
    pub focused: bool,
    pub composing: bool,
    high: Option<u16>,
}
impl InputState {
    pub fn gate(&mut self, enabled: bool, focused: bool) {
        self.enabled = enabled;
        self.focused = focused;
        if !enabled || !focused {
            self.high = None;
            self.composing = false;
        }
    }
    pub fn character(&mut self, unit: u16) -> Vec<u8> {
        if !self.enabled || !self.focused || self.composing {
            return Vec::new();
        }
        if (0xd800..=0xdbff).contains(&unit) {
            self.high = Some(unit);
            return Vec::new();
        }
        let scalar = if (0xdc00..=0xdfff).contains(&unit) {
            let Some(high) = self.high.take() else {
                return Vec::new();
            };
            0x10000 + ((high as u32 - 0xd800) << 10) + (unit as u32 - 0xdc00)
        } else {
            self.high = None;
            unit as u32
        };
        if scalar == 8 {
            return vec![127];
        }
        self.unicode(scalar)
    }
    pub fn unicode(&mut self, scalar: u32) -> Vec<u8> {
        if !self.enabled || !self.focused || self.composing {
            return Vec::new();
        }
        char::from_u32(scalar)
            .map(|c| encode_commit(&c.to_string()))
            .unwrap_or_default()
    }
    pub fn commit(&mut self, units: &[u16]) -> Vec<u8> {
        self.composing = false;
        self.high = None;
        if !self.enabled || !self.focused {
            return Vec::new();
        }
        encode_commit(&String::from_utf16_lossy(units))
    }
    pub fn key(&mut self, key: Key<'_>, m: Modifiers, app: bool) -> Vec<u8> {
        if !self.enabled || !self.focused {
            return Vec::new();
        }
        encode_key(key, None, m, app, self.composing)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_ctrl_alt_and_named_keys_encode_terminal_bytes() {
        let m = Modifiers::default();
        assert_eq!(
            encode_key(Key::Text("中"), Some("中"), m, false, false),
            "中".as_bytes()
        );
        assert_eq!(
            encode_key(
                Key::Text("c"),
                None,
                Modifiers { control: true, ..m },
                false,
                false
            ),
            b"\x03"
        );
        assert_eq!(
            encode_key(
                Key::Text("x"),
                None,
                Modifiers { alt: true, ..m },
                false,
                false
            ),
            b"\x1bx"
        );
        for (k, b) in [
            (Key::Enter, b"\r".as_slice()),
            (Key::Backspace, b"\x7f"),
            (Key::Escape, b"\x1b"),
        ] {
            assert_eq!(encode_key(k, None, m, false, false), b);
        }
    }
    #[test]
    fn cursor_modes_modifiers_and_ime_do_not_duplicate_preedit() {
        let m = Modifiers::default();
        assert_eq!(encode_key(Key::Up, None, m, false, false), b"\x1b[A");
        assert_eq!(encode_key(Key::Up, None, m, true, false), b"\x1bOA");
        assert_eq!(
            encode_key(
                Key::Up,
                None,
                Modifiers { control: true, ..m },
                false,
                false
            ),
            b"\x1b[1;5A"
        );
        assert!(encode_key(Key::Text("a"), None, m, false, true).is_empty());
        assert_eq!(encode_commit("中文"), "中文".as_bytes());
    }
    #[test]
    fn surrogate_pair_is_one_utf8_scalar_and_gate_discards_partial_input() {
        let mut i = InputState::default();
        i.gate(true, true);
        assert!(i.character(0xd83d).is_empty());
        assert_eq!(i.character(0xde42), "🙂".as_bytes());
        i.character(0xd83d);
        i.gate(false, false);
        assert!(i.character(65).is_empty());
        i.gate(true, true);
        assert_eq!(i.character(66), b"B");
        assert_eq!(i.unicode(0x4e2d), "中".as_bytes());
        assert!(i.unicode(0x110000).is_empty());
    }
    #[test]
    fn ime_preedit_never_writes_and_commit_exactly_once_without_char_duplicate() {
        let mut i = InputState::default();
        i.gate(true, true);
        i.composing = true;
        assert!(i.character(0x4e2d).is_empty());
        assert_eq!(i.commit(&[0x4e2d, 0x6587]), "中文".as_bytes());
        assert!(!i.composing);
        assert!(i.commit(&[]).is_empty());
        i.gate(true, false);
        assert!(i.commit(&[65]).is_empty());
    }
    #[test]
    fn native_named_keys_use_shared_encoder_and_gating() {
        let mut i = InputState::default();
        i.gate(true, true);
        assert_eq!(i.key(Key::Up, Modifiers::default(), true), b"\x1bOA");
        assert_eq!(i.key(Key::Enter, Modifiers::default(), false), b"\r");
        i.composing = true;
        assert!(i.key(Key::Enter, Modifiers::default(), false).is_empty());
        i.gate(false, true);
        assert!(i.key(Key::Up, Modifiers::default(), false).is_empty());
    }
}
