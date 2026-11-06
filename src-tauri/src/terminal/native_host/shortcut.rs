//! Shared DOM first-refusal bridge. The frontend remains the only binding/guard resolver.
use super::{
    input::{Key, Modifiers},
    Geometry,
};

pub(super) fn key_name(key: Key) -> String {
    match key {
        Key::Character(c) => c.to_string(),
        Key::Enter => "Enter".into(),
        Key::Backspace => "Backspace".into(),
        Key::Tab => "Tab".into(),
        Key::Escape => "Escape".into(),
        Key::Up => "ArrowUp".into(),
        Key::Down => "ArrowDown".into(),
        Key::Left => "ArrowLeft".into(),
        Key::Right => "ArrowRight".into(),
        Key::Home => "Home".into(),
        Key::End => "End".into(),
        Key::Insert => "Insert".into(),
        Key::Delete => "Delete".into(),
        Key::PageUp => "PageUp".into(),
        Key::PageDown => "PageDown".into(),
        Key::Function(n) => format!("F{n}"),
        Key::Unknown => String::new(),
    }
}
pub(super) fn shortcut_script(
    key: Key,
    m: Modifiers,
    geometry: Geometry,
    id: uuid::Uuid,
) -> String {
    let init = serde_json::json!({"key":key_name(key),"ctrlKey":m.control,"shiftKey":m.shift,"altKey":m.alt,"metaKey":m.super_key,"bubbles":true,"cancelable":true});
    let x = geometry.x + geometry.width / 2.;
    let y = geometry.y + geometry.height / 2.;
    // Topmost element only: never dispatch through a modal/occluding sibling.
    format!("(()=>{{const x=document.elementFromPoint({x},{y})?.closest('[data-native-session=\"{id}\"]');if(!x||x.dataset.nativeInputActive!=='true'||document.hidden||document.body.dataset.nativeViewOccluded==='true')return {{blocked:true}};const e=new KeyboardEvent('keydown',{init});x.dispatchEvent(e);return {{consumed:e.defaultPrevented}};}})()")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_keys_and_modifiers_match_dom() {
        assert_eq!(key_name(Key::Function(24)), "F24");
        assert_eq!(key_name(Key::Left), "ArrowLeft");
        assert_eq!(key_name(Key::Character('[')), "[");
        let js = shortcut_script(
            Key::Character('S'),
            Modifiers {
                control: true,
                alt: true,
                shift: true,
                super_key: true,
                ..Default::default()
            },
            Geometry::default(),
            uuid::Uuid::nil(),
        );
        for name in ["ctrlKey", "shiftKey", "altKey", "metaKey"] {
            assert!(js.contains(&format!("\"{name}\":true")));
        }
        assert!(js.contains("elementFromPoint"));
        assert!(!js.contains("elementsFromPoint"));
    }
}
