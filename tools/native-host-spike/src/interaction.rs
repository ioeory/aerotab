use crate::Snapshot;
use std::collections::HashMap;
pub fn route_commit(pane: usize, focused: Option<usize>, modal: bool, hidden: bool) -> bool {
    focused == Some(pane) && !modal && !hidden
}
pub fn hit_cell(s: &Snapshot, x: f64, y: f64, scale: f64) -> Option<usize> {
    if !x.is_finite()
        || !y.is_finite()
        || !scale.is_finite()
        || scale <= 0.0
        || x / scale < 12.0
        || y / scale < 12.0
    {
        return None;
    }
    let col = ((x / scale - 12.0) / 11.0) as usize;
    let row = ((y / scale - 12.0) / 23.0) as usize;
    if col >= s.cols || row >= s.rows {
        return None;
    }
    let index = row * s.cols + col;
    Some(if s.cells[index].width == 0 && col > 0 {
        index - 1
    } else {
        index
    })
}
#[derive(Default, Clone)]
pub struct Selection {
    pub anchor: Option<usize>,
    pub end: Option<usize>,
    pub dragging: bool,
}
impl Selection {
    pub fn begin(&mut self, s: &Snapshot, x: f64, y: f64, scale: f64) {
        self.anchor = hit_cell(s, x, y, scale);
        self.end = self.anchor;
        self.dragging = self.anchor.is_some();
    }
    pub fn extend(&mut self, s: &Snapshot, x: f64, y: f64, scale: f64) {
        if self.dragging
            && let Some(index) = hit_cell(s, x, y, scale)
        {
            self.end = Some(index);
        }
    }
    pub fn range(&self) -> Option<(usize, usize)> {
        self.anchor.zip(self.end)
    }
    pub fn text(&self, s: &Snapshot) -> String {
        let Some((a, b)) = self.range() else {
            return String::new();
        };
        let mut out = String::new();
        for i in a.min(b)..=a.max(b).min(s.cells.len() - 1) {
            if i > a.min(b) && i % s.cols == 0 {
                out.push('\n');
            }
            if s.cells[i].width > 0 {
                out.push_str(&s.cells[i].text);
            }
        }
        out
    }
}
pub struct AtlasSlots {
    slots: HashMap<String, usize>,
    capacity: usize,
    pub pages: usize,
}
impl AtlasSlots {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        Self {
            slots: HashMap::new(),
            capacity,
            pages: 0,
        }
    }
    pub fn needs_flush(&self, key: &str) -> bool {
        !self.slots.contains_key(key) && self.slots.len() == self.capacity
    }
    pub fn get(&self, key: &str) -> Option<usize> {
        self.slots.get(key).copied()
    }
    pub fn insert(&mut self, key: &str) -> usize {
        if let Some(index) = self.get(key) {
            return index;
        }
        assert!(self.slots.len() < self.capacity);
        let index = self.slots.len();
        self.slots.insert(key.to_owned(), index);
        index
    }
    pub fn clear_page(&mut self) {
        self.slots.clear();
        self.pages += 1;
    }
    pub fn len(&self) -> usize {
        self.slots.len()
    }
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}
