//! Slot-bounded Windows glyph layouts; no history of previously seen characters.
use std::sync::Arc;
const MAX_SLOTS: usize = 16_384;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LayoutKey {
    pub text: String,
    pub family: String,
    pub font_size: u64,
    pub scale: u64,
    pub cell: (u16, u16),
    pub width: usize,
    pub weight: u16,
    pub italic: bool,
    pub grid: (usize, usize),
    // All current shaping choices: Advanced, Wrap::None, per-cell clipping.
    pub shaping_policy: u8,
}
pub(super) struct Slots<T> {
    entries: Vec<Option<(LayoutKey, Arc<T>)>>,
}
impl<T> Slots<T> {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
    pub fn resize(&mut self, len: usize) {
        self.entries.resize_with(len.min(MAX_SLOTS), || None);
    }
    pub fn layout(
        &mut self,
        slot: usize,
        key: LayoutKey,
        build: impl FnOnce() -> T,
    ) -> (Arc<T>, bool) {
        if let Some(Some((existing, value))) = self.entries.get(slot) {
            if existing == &key {
                return (Arc::clone(value), true);
            }
        }
        let value = Arc::new(build());
        if let Some(entry) = self.entries.get_mut(slot) {
            *entry = Some((key, Arc::clone(&value)));
        }
        (value, false)
    }
    pub fn clear_slot(&mut self, slot: usize) {
        if let Some(entry) = self.entries.get_mut(slot) {
            *entry = None;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn key() -> LayoutKey {
        LayoutKey {
            text: "界e\u{301}".into(),
            family: "monospace".into(),
            font_size: 13f64.to_bits(),
            scale: 1f64.to_bits(),
            cell: (8, 17),
            width: 2,
            weight: 400,
            italic: false,
            grid: (100, 35),
            shaping_policy: 0,
        }
    }
    #[test]
    fn reuse_and_all_shaping_inputs_invalidate() {
        let mut s = Slots::new();
        s.resize(1);
        let k = key();
        let (first, hit) = s.layout(0, k.clone(), || 1);
        assert!(!hit);
        let (same, hit) = s.layout(0, k.clone(), || 2);
        assert!(hit);
        assert!(Arc::ptr_eq(&first, &same));
        let variants = [
            LayoutKey {
                text: "界".into(),
                ..k.clone()
            },
            LayoutKey {
                family: "other".into(),
                ..k.clone()
            },
            LayoutKey {
                font_size: 14f64.to_bits(),
                ..k.clone()
            },
            LayoutKey {
                scale: 2f64.to_bits(),
                ..k.clone()
            },
            LayoutKey {
                cell: (9, 18),
                ..k.clone()
            },
            LayoutKey {
                width: 1,
                ..k.clone()
            },
            LayoutKey {
                weight: 700,
                ..k.clone()
            },
            LayoutKey {
                italic: true,
                ..k.clone()
            },
            LayoutKey {
                grid: (80, 24),
                ..k.clone()
            },
            LayoutKey {
                shaping_policy: 1,
                ..k.clone()
            },
        ];
        for changed in variants {
            s.layout(0, k.clone(), || 0);
            assert!(!s.layout(0, changed, || 3).1);
        }
    }
    #[test]
    fn resize_clear_and_capacity_bound() {
        let mut s = Slots::new();
        s.resize(usize::MAX);
        assert_eq!(s.entries.len(), MAX_SLOTS);
        s.layout(0, key(), || 1);
        s.clear_slot(0);
        assert!(!s.layout(0, key(), || 2).1);
        s.resize(0);
        s.resize(1);
        assert!(!s.layout(0, key(), || 3).1);
        assert!(!s.layout(MAX_SLOTS, key(), || 4).1);
        assert_eq!(s.entries.len(), 1);
    }
    #[test]
    fn selection_and_position_are_not_shaping_inputs() {
        // Selection/foreground/background and position are supplied fresh to TextArea/rects.
        let mut s = Slots::new();
        s.resize(1);
        s.layout(0, key(), || 1);
        assert!(s.layout(0, key(), || 2).1);
    }
}
