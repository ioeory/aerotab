use native_host_spike::Terminal;
use native_host_spike::terminal_gl::{Lifecycle, cell_rect, effective_colors};

#[test]
fn fixed_cells_keep_wide_combining_and_color() {
    let mut terminal = Terminal::new(12, 3);
    terminal.feed("\u{1b}[38;2;240;40;20mA中e\u{301}".as_bytes());
    let s = terminal.snapshot();
    assert_eq!(cell_rect(1, 12, s.cells[1].width), (23.0, 12.0, 22.0, 23.0));
    assert_eq!(s.cells[2].width, 0);
    assert_eq!(cell_rect(3, 12, 1).0, 45.0);
    assert_eq!(s.cells[3].text, "e\u{301}");
    assert_eq!(effective_colors(&s.cells[0], false).0, [240, 40, 20]);
    assert_eq!(effective_colors(&s.cells[0], true).1, [60, 85, 120]);
}
#[test]
fn renderer_lifecycle_does_not_recreate_on_hide_or_resize() {
    let mut life = Lifecycle::default();
    life.realize();
    life.frame();
    life.frame();
    assert_eq!((life.created, life.frames, life.deleted), (1, 2, 0));
    life.unrealize();
    assert_eq!(life.deleted, 1);
    life.unrealize();
    assert_eq!(life.deleted, 1);
    life.realize();
    assert_eq!(life.created, 2);
}
