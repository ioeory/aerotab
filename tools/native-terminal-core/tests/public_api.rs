use native_terminal_core::Terminal;
#[test]
fn selection_is_public_clamped_and_clearable() {
    let mut t = Terminal::new(12, 2);
    t.feed("A中e\u{301}Z".as_bytes());
    assert_eq!(t.select_cells(1, 3), "中e\u{301}");
    assert_eq!(t.select_cells(3, 1), "中e\u{301}");
    assert_eq!(t.selection(), Some((1, 3)));
    t.clear_selection();
    assert_eq!(t.selection(), None);
    assert_eq!(t.select_cells(usize::MAX, usize::MAX), "");
}
#[test]
fn snapshot_and_replies_share_one_parser() {
    let mut t = Terminal::new(8, 2);
    t.feed("中".as_bytes());
    t.feed(b"\x1b[6n");
    assert_eq!(t.take_replies(), [b"\x1b[1;3R".to_vec()]);
    assert!(t.take_replies().is_empty());
    assert_eq!(t.snapshot().cells[0].width, 2);
}
