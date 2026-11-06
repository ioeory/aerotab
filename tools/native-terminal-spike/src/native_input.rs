use crate::input::encode_key;
use winit::keyboard::{Key, ModifiersState, NamedKey};

#[derive(Default)]
pub struct NativeInput {
    pub enabled: bool,
    pub focused: bool,
    pub composing: bool,
    high: Option<u16>,
}
impl NativeInput {
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
            .map(|c| crate::input::encode_commit(&c.to_string()))
            .unwrap_or_default()
    }
    pub fn commit(&mut self, units: &[u16]) -> Vec<u8> {
        self.composing = false;
        self.high = None;
        if !self.enabled || !self.focused {
            return Vec::new();
        }
        crate::input::encode_commit(&String::from_utf16_lossy(units))
    }
    pub fn key(&mut self, key: NamedKey, mods: ModifiersState, app: bool) -> Vec<u8> {
        if !self.enabled || !self.focused {
            return Vec::new();
        }
        encode_key(&Key::Named(key), None, mods, app, self.composing)
    }
    pub fn gate(&mut self, enabled: bool, focused: bool) {
        self.enabled = enabled;
        self.focused = focused;
        if !enabled || !focused {
            self.high = None;
            self.composing = false;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn surrogate_pair_is_one_utf8_scalar_and_gate_discards_partial_input() {
        let mut i = NativeInput::default();
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
        let mut i = NativeInput::default();
        i.gate(true, true);
        i.composing = true;
        assert!(i.character(0x4e2d).is_empty());
        assert_eq!(i.commit(&[0x4e2d, 0x6587]), "中文".as_bytes());
        // WndProc consumes WM_IME_CHAR and default IME translation; WM_CHAR after a result
        // is a distinct new key, not an automatic second path through commit.
        assert!(!i.composing);
        assert!(i.commit(&[]).is_empty());
        i.gate(true, false);
        assert!(i.commit(&[65]).is_empty());
    }
    #[test]
    fn native_named_keys_use_shared_encoder_and_gating() {
        let mut i = NativeInput::default();
        i.gate(true, true);
        assert_eq!(
            i.key(NamedKey::ArrowUp, ModifiersState::empty(), true),
            b"\x1bOA"
        );
        assert_eq!(
            i.key(NamedKey::Enter, ModifiersState::empty(), false),
            b"\r"
        );
        i.composing = true;
        assert!(
            i.key(NamedKey::Enter, ModifiersState::empty(), false)
                .is_empty()
        );
        i.gate(false, true);
        assert!(
            i.key(NamedKey::ArrowUp, ModifiersState::empty(), false)
                .is_empty()
        );
    }
}
