pub mod host_policy;
pub mod input;
pub mod layout;
pub mod native_input;
pub mod pty;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub text: String,
    pub foreground: [u8; 3],
    pub background: [u8; 3],
    pub width: usize,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<Cell>,
    pub cursor: (usize, usize),
    pub cursor_visible: bool,
}

use alacritty_terminal::{
    event::{Event, EventListener, WindowSize},
    grid::Dimensions,
    index::{Column, Line, Point},
    term::{Config, Term, cell::Flags, test::TermSize},
    vte::ansi::{Color, NamedColor, Processor},
};

#[derive(Clone, Default)]
struct Events(std::sync::Arc<std::sync::Mutex<Vec<Event>>>);
impl EventListener for Events {
    fn send_event(&self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
}

pub struct Terminal {
    term: Term<Events>,
    processor: Processor,
    events: Events,
}

impl Terminal {
    pub fn new(cols: usize, rows: usize) -> Self {
        let events = Events::default();
        Self {
            term: Term::new(
                Config::default(),
                &TermSize::new(cols.max(2), rows.max(1)),
                events.clone(),
            ),
            events,
            processor: Processor::new(),
        }
    }
    pub fn feed(&mut self, bytes: &[u8]) {
        self.processor.advance(&mut self.term, bytes);
    }
    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.term.resize(TermSize::new(cols.max(2), rows.max(1)));
    }
    pub fn take_replies(&mut self) -> Vec<Vec<u8>> {
        let events = std::mem::take(&mut *self.events.0.lock().unwrap());
        events
            .into_iter()
            .filter_map(|event| match event {
                Event::PtyWrite(text) => Some(text.into_bytes()),
                Event::TextAreaSizeRequest(format) => Some(
                    format(WindowSize {
                        num_lines: self.term.screen_lines() as u16,
                        num_cols: self.term.columns() as u16,
                        cell_width: 11,
                        cell_height: 23,
                    })
                    .into_bytes(),
                ),
                Event::ColorRequest(index, format) => {
                    let rgb = self.term.colors()[index]
                        .map(|rgb| [rgb.r, rgb.g, rgb.b])
                        .unwrap_or_else(|| {
                            if index < 256 {
                                self.resolve(Color::Indexed(index as u8))
                            } else if index == 257 {
                                [24, 28, 36]
                            } else {
                                [216, 222, 233]
                            }
                        });
                    Some(
                        format(alacritty_terminal::vte::ansi::Rgb {
                            r: rgb[0],
                            g: rgb[1],
                            b: rgb[2],
                        })
                        .into_bytes(),
                    )
                }
                // Clipboard access is intentionally disabled in this isolated spike.
                _ => None,
            })
            .collect()
    }
    pub fn application_cursor(&self) -> bool {
        self.term
            .mode()
            .contains(alacritty_terminal::term::TermMode::APP_CURSOR)
    }
    pub fn snapshot(&self) -> Snapshot {
        let cols = self.term.columns();
        let rows = self.term.screen_lines();
        let mut cells = Vec::with_capacity(cols * rows);
        for row in 0..rows {
            for col in 0..cols {
                let cell = &self.term.grid()[Point::new(Line(row as i32), Column(col))];
                let mut text = cell.c.to_string();
                if let Some(combining) = cell.zerowidth() {
                    text.extend(combining);
                }
                let mut foreground = self.resolve(cell.fg);
                let mut background = self.resolve(cell.bg);
                if cell.flags.contains(Flags::INVERSE) {
                    std::mem::swap(&mut foreground, &mut background);
                }
                if cell.flags.contains(Flags::HIDDEN) {
                    text = " ".into();
                }
                let width = if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    0
                } else if cell.flags.contains(Flags::WIDE_CHAR) {
                    2
                } else {
                    1
                };
                cells.push(Cell {
                    text,
                    foreground,
                    background,
                    width,
                });
            }
        }
        let cursor = self.term.grid().cursor.point;
        Snapshot {
            cols,
            rows,
            cells,
            cursor: (cursor.column.0, cursor.line.0.max(0) as usize),
            cursor_visible: self
                .term
                .mode()
                .contains(alacritty_terminal::term::TermMode::SHOW_CURSOR),
        }
    }

    fn resolve(&self, color: Color) -> [u8; 3] {
        let index = match color {
            Color::Spec(rgb) => return [rgb.r, rgb.g, rgb.b],
            Color::Indexed(index) => index as usize,
            Color::Named(named) => named as usize,
        };
        if let Some(rgb) = self.term.colors()[index] {
            return [rgb.r, rgb.g, rgb.b];
        }
        match index {
            0..=15 => PALETTE[index],
            16..=231 => {
                let n = index - 16;
                let level = |v: usize| if v == 0 { 0 } else { (55 + v * 40) as u8 };
                [level(n / 36), level(n / 6 % 6), level(n % 6)]
            }
            232..=255 => [(8 + (index - 232) * 10) as u8; 3],
            n if n == NamedColor::Background as usize => [24, 28, 36],
            _ => [216, 222, 233],
        }
    }
}

const PALETTE: [[u8; 3]; 16] = [
    [46, 52, 64],
    [191, 97, 106],
    [163, 190, 140],
    [235, 203, 139],
    [129, 161, 193],
    [180, 142, 173],
    [136, 192, 208],
    [229, 233, 240],
    [76, 86, 106],
    [235, 122, 132],
    [183, 210, 160],
    [255, 223, 159],
    [149, 181, 213],
    [200, 162, 193],
    [156, 212, 228],
    [236, 239, 244],
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_replies_have_one_drainable_source_and_cursor_mode_tracks_vt() {
        let mut terminal = Terminal::new(80, 24);
        terminal.feed(b"\x1b[6n\x1b[?1h");
        assert_eq!(terminal.take_replies(), [b"\x1b[1;1R".to_vec()]);
        assert!(terminal.take_replies().is_empty());
        assert!(terminal.application_cursor());
        terminal.feed(b"\x1b[?1l");
        assert!(!terminal.application_cursor());
    }

    #[test]
    fn grid_is_fixed_and_unicode_wide_cells_keep_their_columns() {
        let mut terminal = Terminal::new(8, 3);
        terminal.feed("A中e\u{301}".as_bytes());
        let grid = terminal.snapshot();
        assert_eq!((grid.cols, grid.rows, grid.cells.len()), (8, 3, 24));
        assert_eq!(grid.cells[0].text, "A");
        assert_eq!(grid.cells[1].text, "中");
        assert_eq!(grid.cells[1].width, 2);
        assert_eq!(grid.cells[2].width, 0);
        assert_eq!(grid.cells[3].text, "e\u{301}");
        assert_eq!(grid.cursor, (4, 0));
    }

    #[test]
    fn ansi_truecolor_indexed_and_inverse_are_resolved() {
        let mut terminal = Terminal::new(8, 2);
        terminal.feed(b"\x1b[38;2;12;34;56;48;5;196mX\x1b[7mY");
        let grid = terminal.snapshot();
        assert_eq!(grid.cells[0].foreground, [12, 34, 56]);
        assert_eq!(grid.cells[0].background, [255, 0, 0]);
        assert_eq!(grid.cells[1].foreground, [255, 0, 0]);
        assert_eq!(grid.cells[1].background, [12, 34, 56]);
    }

    #[test]
    fn ansi_palette_and_reset_restore_defaults() {
        let mut terminal = Terminal::new(8, 2);
        terminal.feed(b"\x1b[31mR\x1b[0mD");
        let grid = terminal.snapshot();
        assert_ne!(grid.cells[0].foreground, grid.cells[1].foreground);
        assert_eq!(grid.cells[1].foreground, [216, 222, 233]);
        assert_eq!(grid.cells[1].background, [24, 28, 36]);
    }

    #[test]
    fn cursor_addressing_and_visibility_follow_ansi() {
        let mut terminal = Terminal::new(8, 4);
        terminal.feed(b"\x1b[3;5H@\x1b[?25l");
        let grid = terminal.snapshot();
        assert_eq!(grid.cells[2 * 8 + 4].text, "@");
        assert_eq!(grid.cursor, (5, 2));
        assert!(!grid.cursor_visible);
        terminal.feed(b"\x1b[?25h");
        assert!(terminal.snapshot().cursor_visible);
    }

    #[test]
    fn resizing_preserves_content_and_changes_grid_dimensions() {
        let mut terminal = Terminal::new(8, 3);
        terminal.feed(b"hello");
        terminal.resize(12, 5);
        let grid = terminal.snapshot();
        assert_eq!((grid.cols, grid.rows, grid.cells.len()), (12, 5, 60));
        assert_eq!(grid.cells[0].text, "h");
        assert_eq!(grid.cursor, (5, 0));
    }

    #[test]
    fn zero_size_is_clamped_and_partial_utf8_is_buffered() {
        let mut terminal = Terminal::new(8, 3);
        let bytes = "中".as_bytes();
        terminal.feed(&bytes[..1]);
        terminal.feed(&bytes[1..]);
        assert_eq!(terminal.snapshot().cells[0].text, "中");
        terminal.resize(0, 0);
        let grid = terminal.snapshot();
        assert_eq!((grid.cols, grid.rows, grid.cells.len()), (2, 1, 2));
    }
}
