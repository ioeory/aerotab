use native_host_spike::{
    Terminal,
    interaction::{AtlasSlots, Selection, hit_cell, route_commit},
};
#[test]
fn selection_dpi_wide_and_combining_copy() {
    let mut t = Terminal::new(12, 2);
    t.feed("A中e\u{301} Z".as_bytes());
    let s = t.snapshot();
    assert_eq!(hit_cell(&s, 35.0 * 2.0, 13.0 * 2.0, 2.0), Some(1));
    let mut selection = Selection::default();
    selection.begin(&s, 24.0, 13.0, 1.0);
    selection.extend(&s, 46.0, 13.0, 1.0);
    assert_eq!(selection.text(&s), "中e\u{301}");
    let (a, b) = selection.range().unwrap();
    assert_eq!(t.select_cells(a, b), "中e\u{301}");
    assert_eq!(hit_cell(&s, 0.0, 0.0, 1.0), None);
    assert_eq!(hit_cell(&s, f64::NAN, 13.0, 1.0), None);
    assert_eq!(hit_cell(&s, 13.0, 13.0, 0.0), None);
}
#[test]
fn focus_and_modal_route_only_matching_pane() {
    assert!(route_commit(0, Some(0), false, false));
    assert!(!route_commit(1, Some(0), false, false));
    assert!(!route_commit(0, Some(0), true, false));
    assert!(route_commit(1, Some(1), false, false));
    assert!(!route_commit(0, Some(0), false, true));
}
#[test]
fn atlas_exhaustion_requires_flush_then_reuses_bounded_page() {
    let mut atlas = AtlasSlots::new(2);
    assert!(!atlas.needs_flush("a"));
    assert_eq!(atlas.insert("a"), 0);
    assert_eq!(atlas.insert("b"), 1);
    assert!(atlas.needs_flush("c"));
    atlas.clear_page();
    assert_eq!(atlas.insert("c"), 0);
    assert_eq!(atlas.len(), 1);
    assert_eq!(atlas.pages, 1);
    for i in 0..100 {
        let key = i.to_string();
        if atlas.needs_flush(&key) {
            atlas.clear_page();
        }
        atlas.insert(&key);
        assert!(atlas.len() <= 2);
    }
}
