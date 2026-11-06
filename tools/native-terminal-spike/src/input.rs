use winit::keyboard::{Key, ModifiersState, NamedKey};

pub fn encode_key(
    key: &Key,
    text: Option<&str>,
    modifiers: ModifiersState,
    application_cursor: bool,
    ime_composing: bool,
) -> Vec<u8> {
    if ime_composing || modifiers.super_key() {
        return Vec::new();
    }
    let modifier = 1
        + u8::from(modifiers.shift_key())
        + 2 * u8::from(modifiers.alt_key())
        + 4 * u8::from(modifiers.control_key());
    let sequence = match key {
        Key::Named(named) => match named {
            NamedKey::Enter => "\r".into(),
            NamedKey::Backspace => "\x7f".into(),
            NamedKey::Tab if modifiers.shift_key() => "\x1b[Z".into(),
            NamedKey::Tab => "\t".into(),
            NamedKey::Escape => "\x1b".into(),
            NamedKey::ArrowUp
            | NamedKey::ArrowDown
            | NamedKey::ArrowRight
            | NamedKey::ArrowLeft
            | NamedKey::Home
            | NamedKey::End => {
                let code = match named {
                    NamedKey::ArrowUp => 'A',
                    NamedKey::ArrowDown => 'B',
                    NamedKey::ArrowRight => 'C',
                    NamedKey::ArrowLeft => 'D',
                    NamedKey::Home => 'H',
                    _ => 'F',
                };
                if modifier > 1 {
                    format!("\x1b[1;{modifier}{code}")
                } else {
                    format!("\x1b{}{code}", if application_cursor { 'O' } else { '[' })
                }
            }
            NamedKey::Delete => "\x1b[3~".into(),
            NamedKey::Insert => "\x1b[2~".into(),
            NamedKey::PageUp => "\x1b[5~".into(),
            NamedKey::PageDown => "\x1b[6~".into(),
            _ => return Vec::new(),
        },
        Key::Character(character) => {
            if modifiers.control_key() {
                let first = character
                    .chars()
                    .next()
                    .unwrap_or('\0')
                    .to_ascii_uppercase();
                let control = match first {
                    '@'..='_' => first as u8 & 0x1f,
                    ' ' | '2' => 0,
                    '?' => 127,
                    _ => return Vec::new(),
                };
                let mut result = vec![control];
                if modifiers.alt_key() {
                    result.insert(0, 27);
                }
                return result;
            }
            let mut result = text.unwrap_or(character.as_str()).as_bytes().to_vec();
            if modifiers.alt_key() {
                result.insert(0, 27);
            }
            return result;
        }
        _ => return Vec::new(),
    };
    sequence.into_bytes()
}

pub fn encode_commit(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_ctrl_alt_and_named_keys_encode_terminal_bytes() {
        let none = ModifiersState::empty();
        assert_eq!(
            encode_key(&Key::Character("中".into()), Some("中"), none, false, false),
            "中".as_bytes()
        );
        assert_eq!(
            encode_key(
                &Key::Character("c".into()),
                Some("c"),
                ModifiersState::CONTROL,
                false,
                false
            ),
            b"\x03"
        );
        assert_eq!(
            encode_key(
                &Key::Character("x".into()),
                Some("x"),
                ModifiersState::ALT,
                false,
                false
            ),
            b"\x1bx"
        );
        assert_eq!(
            encode_key(&Key::Named(NamedKey::Enter), None, none, false, false),
            b"\r"
        );
        assert_eq!(
            encode_key(&Key::Named(NamedKey::Backspace), None, none, false, false),
            b"\x7f"
        );
        assert_eq!(
            encode_key(&Key::Named(NamedKey::Escape), None, none, false, false),
            b"\x1b"
        );
    }

    #[test]
    fn cursor_modes_modifiers_and_ime_do_not_duplicate_preedit() {
        let up = Key::Named(NamedKey::ArrowUp);
        assert_eq!(
            encode_key(&up, None, ModifiersState::empty(), false, false),
            b"\x1b[A"
        );
        assert_eq!(
            encode_key(&up, None, ModifiersState::empty(), true, false),
            b"\x1bOA"
        );
        assert_eq!(
            encode_key(&up, None, ModifiersState::CONTROL, false, false),
            b"\x1b[1;5A"
        );
        assert!(
            encode_key(
                &Key::Character("a".into()),
                Some("a"),
                ModifiersState::empty(),
                false,
                true
            )
            .is_empty()
        );
        assert_eq!(encode_commit("中文"), "中文".as_bytes());
    }
}
