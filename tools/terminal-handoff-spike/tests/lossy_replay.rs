//! Executable counterexamples to text export and last-256KiB reconstruction.
//! These use real alacritty on both sides, not xterm.js.
use alacritty_terminal::{
    event::VoidListener,
    index::{Column, Line, Point},
    term::{Config, Term, TermMode, test::TermSize},
    vte::ansi::Processor,
};

fn terminal() -> (Term<VoidListener>, Processor) {
    (
        Term::new(Config::default(), &TermSize::new(80, 24), VoidListener),
        Processor::new(),
    )
}

#[test]
fn plain_text_cannot_recover_alternate_screen_cursor_or_pending_utf8() {
    let (mut persistent, mut parser) = terminal();
    parser.advance(
        &mut persistent,
        b"main\x1b[?1049hvim\x1b[3;9H\x1b[?25l\xe4\xb8",
    );
    let (mut exported, mut fresh_parser) = terminal();
    // Equivalent visible text content still omits modes, cursor and parser state.
    fresh_parser.advance(&mut exported, b"vim");
    assert!(persistent.mode().contains(TermMode::ALT_SCREEN));
    assert!(!exported.mode().contains(TermMode::ALT_SCREEN));
    assert_ne!(persistent.grid().cursor.point, exported.grid().cursor.point);
    parser.advance(&mut persistent, b"\xad");
    fresh_parser.advance(&mut exported, b"\xad");
    assert_eq!(persistent.grid()[Point::new(Line(2), Column(8))].c, '中');
    assert_ne!(exported.grid()[Point::new(Line(2), Column(8))].c, '中');
    parser.advance(&mut persistent, b"\x1b[?1049l");
    assert_eq!(persistent.grid()[Point::new(Line(0), Column(0))].c, 'm');
}

#[test]
fn last_256kib_loses_earlier_mode_even_when_truncation_is_ascii_aligned() {
    let mut bytes = b"\x1b[?1049h\x1b[?25l".to_vec();
    bytes.extend(std::iter::repeat_n(b'x', 300 * 1024));
    let (mut full, mut full_parser) = terminal();
    full_parser.advance(&mut full, &bytes);
    let (mut ring, mut ring_parser) = terminal();
    ring_parser.advance(&mut ring, &bytes[bytes.len() - 256 * 1024..]);
    assert!(full.mode().contains(TermMode::ALT_SCREEN));
    assert!(!ring.mode().contains(TermMode::ALT_SCREEN));
    assert!(!full.mode().contains(TermMode::SHOW_CURSOR));
    assert!(ring.mode().contains(TermMode::SHOW_CURSOR));
}
