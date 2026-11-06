use native_terminal_core::{Terminal, input::InputGate};
#[test]
fn full_style_and_cursor_modes() {
    let mut t = Terminal::new(40, 4);
    t.feed(b"\x1b[1;2;3;4;7;8;9mX\x1b[5 q\x1b[?2004h");
    let s = t.snapshot();
    assert_ne!(s.cells[0].flags, 0);
    assert_eq!(s.cursor_shape, "Beam");
    assert!(s.cursor_blinking);
    assert_ne!(s.mode, 0);
}
#[test]
fn history_viewport_and_selection_share_core() {
    let mut t = Terminal::new(20, 3);
    for i in 0..20 {
        t.feed(format!("OLD{i:02}\r\n").as_bytes());
    }
    t.scroll_lines(10);
    let s = t.snapshot();
    assert!(s.display_offset > 0);
    assert!(s.history > 0);
    let text = t.select_cells(0, 4);
    assert!(text.starts_with("OLD"));
    t.scroll_bottom();
    assert_eq!(t.snapshot().display_offset, 0);
}
#[test]
fn gate_rejects_stale_hidden_modal_and_composition_key() {
    let g = InputGate {
        epoch: 4,
        focused: true,
        hidden: false,
        modal: false,
    };
    assert!(g.accepts(4));
    assert!(!g.accepts(3));
    assert!(!InputGate { modal: true, ..g }.accepts(4));
    assert!(!InputGate { hidden: true, ..g }.accepts(4));
}
