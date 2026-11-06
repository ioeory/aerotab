//! AppKit-independent policy, exercised on every development platform.
use super::{Geometry, HostError};
pub(super) struct Placement {
    pub frame: [f64; 4],
    pub pixels: [u32; 2],
}
pub(super) fn placement(
    g: Geometry,
    parent_height: f64,
    flipped: bool,
    scale: f64,
) -> Result<Placement, HostError> {
    g.validate()?;
    if !scale.is_finite()
        || !(0.5..=8.).contains(&scale)
        || !parent_height.is_finite()
        || parent_height < 0.
    {
        return Err(HostError::new(
            "invalid_backing_scale",
            "invalid AppKit bounds or backing scale",
        ));
    }
    Ok(Placement {
        frame: [
            g.x,
            if flipped {
                g.y
            } else {
                parent_height - g.y - g.height
            },
            g.width,
            g.height,
        ],
        pixels: [
            (g.width * scale).round().max(1.) as u32,
            (g.height * scale).round().max(1.) as u32,
        ],
    })
}
#[derive(Clone, Default)]
pub(super) struct Preedit {
    pub text: String,
    pub selected: (usize, usize),
}
impl Preedit {
    pub fn clear(&mut self) {
        self.text.clear();
        self.selected = (0, 0);
    }
    fn boundary(&self, index: usize, round_up: bool) -> usize {
        let mut at = 0;
        for c in self.text.chars() {
            let end = at + c.len_utf16();
            if index < end {
                return if round_up && index > at { end } else { at };
            }
            at = end;
        }
        at
    }
    pub fn set(&mut self, text: String, location: usize, length: usize) {
        self.text = text;
        let start = self.boundary(location, false);
        let end = self.boundary(location.saturating_add(length), true);
        self.selected = (start, end.saturating_sub(start));
    }
    /// Only marked text is an addressable document. Never synthesize remote edits.
    pub fn replacement_supported(&self, range: Option<(usize, usize)>) -> bool {
        match range {
            None => true,
            Some((start, len)) => start
                .checked_add(len)
                .is_some_and(|end| end <= utf16_len(&self.text)),
        }
    }
    pub fn commit(&self, text: &str, range: Option<(usize, usize)>) -> Option<String> {
        if !self.replacement_supported(range) {
            return None;
        }
        let Some(range) = range else {
            return Some(text.into());
        };
        let mut result = self.clone();
        result.replace(text.into(), 0, 0, Some(range));
        Some(result.text)
    }
    pub fn replace(
        &mut self,
        text: String,
        location: usize,
        length: usize,
        replacement: Option<(usize, usize)>,
    ) {
        if !self.replacement_supported(replacement) {
            return;
        }
        let Some((from, count)) = replacement else {
            self.set(text, location, length);
            return;
        };
        let start = self.boundary(from, false);
        let end = self.boundary(from.saturating_add(count), true);
        let mut units: Vec<_> = self.text.encode_utf16().collect();
        units.splice(start..end, text.encode_utf16());
        self.set(
            String::from_utf16(&units).expect("UTF16 scalar boundaries"),
            start.saturating_add(location),
            length,
        );
    }
    pub fn substring(&self, location: usize, length: usize) -> Option<(String, (usize, usize))> {
        let n = utf16_len(&self.text);
        if location >= n {
            return None;
        }
        let start = self.boundary(location, false);
        let end = self.boundary(location.saturating_add(length), true);
        let units: Vec<_> = self.text.encode_utf16().collect();
        Some((
            String::from_utf16(&units[start..end]).ok()?,
            (start, end - start),
        ))
    }
}
pub(super) fn terminal_button(button: isize) -> Option<u8> {
    match button {
        0 => Some(0),
        1 => Some(2),
        2 => Some(1),
        _ => None,
    }
}
pub(super) fn wheel_delta(delta: f64, precise: bool, line_height: f64) -> f64 {
    if !delta.is_finite() {
        return 0.;
    }
    -delta / if precise { line_height.max(1.) } else { 1. }
}
pub(super) fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}
#[derive(Clone, Debug)]
pub(super) struct GlyphBox {
    pub start: usize,
    pub end: usize,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub rtl: bool,
}
#[derive(Default)]
pub(super) struct PreeditLayout {
    pub glyphs: Vec<GlyphBox>,
    pub end: usize,
}
impl PreeditLayout {
    pub fn hit(&self, x: f64, y: f64) -> Option<usize> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        self.glyphs
            .iter()
            .find(|g| x >= g.x && x <= g.x + g.w && y >= g.y && y < g.y + g.h)
            .map(|g| {
                if (x < g.x + g.w / 2.) != g.rtl {
                    g.start
                } else {
                    g.end
                }
            })
    }
    pub fn caret(&self, at: usize) -> Option<(f64, f64, f64)> {
        let at = at.min(self.end);
        let g = self
            .glyphs
            .iter()
            .find(|g| g.start <= at && at < g.end)
            .or_else(|| self.glyphs.iter().rev().find(|g| g.end <= at))?;
        let trailing = g.end > g.start && at >= g.end;
        Some((g.x + if trailing != g.rtl { g.w } else { 0. }, g.y, g.h))
    }
}
pub(super) fn composition_shift(y: f64, line: f64, height: f64) -> f64 {
    if y < 12. {
        12. - y
    } else {
        (height - 12. - line - y).min(0.)
    }
}
pub(super) fn selection_kind(clicks: isize, block: bool) -> &'static str {
    if block {
        "block"
    } else if clicks >= 3 {
        "line"
    } else if clicks == 2 {
        "word"
    } else {
        "simple"
    }
}
pub(super) fn edge_scroll(y: f64, height: f64, line: f64) -> i32 {
    if !y.is_finite() || !height.is_finite() {
        return 0;
    }
    if y < 12. {
        (1. + (12. - y) / line.max(1.)).floor().min(8.) as i32
    } else if y > height - 12. {
        -((1. + (y - height + 12.) / line.max(1.)).floor().min(8.) as i32)
    } else {
        0
    }
}
/// A receipt belongs to one live attachment AND one applied geometry/configuration.
pub(super) struct Barrier {
    generation: u128,
    live: bool,
    pub revision: u32,
    pub drawn: Option<(u32, u64)>,
}
impl Barrier {
    pub fn new(generation: u128) -> Self {
        Self {
            generation,
            live: true,
            revision: 0,
            drawn: None,
        }
    }
    pub fn apply(&mut self, generation: u128, revision: u32) -> bool {
        if !self.live || self.generation != generation || revision < self.revision {
            return false;
        }
        self.revision = revision;
        true
    }
    pub fn complete(
        &mut self,
        generation: u128,
        revision: u32,
        output: u64,
        gpu_complete: bool,
    ) -> bool {
        if !self.live
            || self.generation != generation
            || self.revision != revision
            || !gpu_complete
            || self.drawn.is_some_and(|(_, previous)| output < previous)
        {
            return false;
        }
        self.drawn = Some((revision, output));
        true
    }
    pub fn revoke(&mut self) {
        self.live = false;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::native_host::Geometry;
    #[test]
    fn css_points_and_backing_pixels_are_distinct() {
        let p = placement(
            Geometry {
                x: 1.25,
                y: 3.5,
                width: 101.5,
                height: 51.25,
            },
            600.,
            false,
            2.,
        )
        .unwrap();
        assert_eq!(p.frame, [1.25, 545.25, 101.5, 51.25]);
        assert_eq!(p.pixels, [203, 103]);
        assert_eq!(
            placement(Geometry::default(), 800., true, 1.)
                .unwrap()
                .frame[1],
            0.
        );
        assert!(placement(Geometry::default(), 800., false, f64::NAN).is_err());
    }
    #[test]
    fn receipt_requires_live_generation_revision_and_completed_gpu() {
        let mut b = Barrier::new(7);
        assert!(!b.complete(7, 0, 15, false));
        assert_eq!(b.drawn, None);
        assert!(b.apply(7, 1));
        assert!(!b.complete(7, 0, 16, true));
        assert!(!b.complete(8, 1, 16, true));
        assert!(b.complete(7, 1, 16, true));
        assert!(!b.complete(7, 1, 15, true));
        assert!(b.apply(7, 1)); // exact retry can drain ACK; caller checks payload equality
        assert!(!b.apply(7, 0));
        b.revoke();
        assert!(!b.complete(7, 1, 17, true));
        assert!(!b.apply(7, 2));
        assert_eq!(b.drawn, Some((1, 16)));
    }
    #[test]
    fn preedit_clamps_utf16_selection_and_never_slices_surrogates() {
        let mut p = Preedit::default();
        p.set("中🙂a".into(), 2, usize::MAX);
        assert_eq!(p.selected, (1, 3));
        assert_eq!(p.substring(2, 1), Some(("🙂".into(), (1, 2))));
        p.clear();
        assert_eq!(p.selected, (0, 0));
        assert!(p.text.is_empty());
    }
    #[test]
    fn preedit_replacement_respects_utf16_range() {
        let mut p = Preedit::default();
        p.set("a🙂b".into(), 3, 0);
        p.replace("中".into(), 0, 1, Some((1, 2)));
        assert_eq!(p.text, "a中b");
        assert_eq!(p.selected, (1, 1));
        p.replace("x".into(), 0, 0, None);
        assert_eq!(p.text, "x");
    }
    #[test]
    fn pointer_buttons_follow_xterm_not_appkit_numbering() {
        assert_eq!(terminal_button(0), Some(0));
        assert_eq!(terminal_button(1), Some(2));
        assert_eq!(terminal_button(2), Some(1));
        assert_eq!(terminal_button(3), None);
        assert_eq!(wheel_delta(20., true, 20.), -1.);
        assert_eq!(wheel_delta(1., false, 20.), -1.);
    }
    #[test]
    fn multiline_layout_hit_and_candidate_share_utf16_clusters() {
        let layout = PreeditLayout {
            glyphs: vec![
                GlyphBox {
                    start: 0,
                    end: 1,
                    x: 12.,
                    y: 12.,
                    w: 10.,
                    h: 20.,
                    rtl: false,
                },
                GlyphBox {
                    start: 1,
                    end: 3,
                    x: 12.,
                    y: 32.,
                    w: 20.,
                    h: 20.,
                    rtl: false,
                },
            ],
            end: 3,
        };
        assert_eq!(layout.hit(13., 33.), Some(1));
        assert_eq!(layout.hit(30., 33.), Some(3));
        assert_eq!(layout.hit(90., 90.), None);
        assert_eq!(layout.caret(2), Some((12., 32., 20.)));
        assert_eq!(layout.caret(3), Some((32., 32., 20.)));
        assert_eq!(selection_kind(2, false), "word");
        assert_eq!(selection_kind(3, false), "line");
        assert_eq!(edge_scroll(-20., 200., 20.), 2);
        assert_eq!(edge_scroll(230., 200., 20.), -3);
        assert_eq!(edge_scroll(60., 200., 20.), 0);
    }
    #[test]
    fn oversized_preedit_scrolls_selected_row_into_view() {
        assert_eq!(composition_shift(112., 20., 100.), -44.);
        assert_eq!(composition_shift(-8., 20., 100.), 20.);
        assert_eq!(composition_shift(32., 20., 100.), 0.);
        let mut p = Preedit::default();
        p.set("🙂".into(), 0, 0);
        assert!(!p.replacement_supported(Some((usize::MAX, 1))));
        assert_eq!(p.commit("x", Some((0, 2))), Some("x".into()));
    }
    #[test]
    fn trailing_newline_has_own_candidate_and_hit_box() {
        let layout = PreeditLayout {
            glyphs: vec![
                GlyphBox {
                    start: 0,
                    end: 1,
                    x: 12.,
                    y: 12.,
                    w: 10.,
                    h: 20.,
                    rtl: false,
                },
                GlyphBox {
                    start: 2,
                    end: 2,
                    x: 12.,
                    y: 32.,
                    w: 1.,
                    h: 20.,
                    rtl: false,
                },
            ],
            end: 2,
        };
        assert_eq!(layout.caret(2), Some((12., 32., 20.)));
        assert_eq!(layout.hit(12., 33.), Some(2));
    }
    #[test]
    fn rtl_clusters_never_return_half_surrogates() {
        let layout = PreeditLayout {
            glyphs: vec![GlyphBox {
                start: 0,
                end: 2,
                x: 10.,
                y: 20.,
                w: 20.,
                h: 20.,
                rtl: true,
            }],
            end: 2,
        };
        assert_eq!(layout.hit(11., 21.), Some(2));
        assert_eq!(layout.hit(29., 21.), Some(0));
        assert_eq!(layout.caret(0), Some((30., 20., 20.)));
        assert_eq!(layout.hit(f64::NAN, 21.), None);
    }
    #[test]
    fn committed_replacement_is_rejected_without_mutating_preedit() {
        let mut p = Preedit::default();
        p.set("中🙂".into(), 3, 0);
        assert!(!p.replacement_supported(Some((2, 4))));
        assert!(!p.replacement_supported(Some((100, 0))));
        assert!(p.replacement_supported(Some((1, 2))));
        assert_eq!(p.commit("a", Some((1, 2))), Some("中a".into()));
        assert_eq!(p.commit("a", Some((100, 1))), None);
        assert_eq!(p.text, "中🙂");
    }
    #[test]
    fn ime_range_uses_utf16_not_unicode_scalar_or_utf8_length() {
        assert_eq!(utf16_len("中🙂"), 3);
    }
}
