//! Native terminal engine: VT parsing and grid state live in Rust
//! (`alacritty_terminal`), the webview only renders cells.
//!
//! One [`TerminalEngine`] is attached per interactive session. A pump task
//! owns the session's output receiver, feeds bytes into the [`Term`], and
//! pushes **binary damage frames** (see [`frame`]) to a [`FrameSink`] at most
//! once per [`FRAME_INTERVAL`]. All UI-driven mutations (resize, scroll,
//! selection, search) go through the engine and request a redraw via
//! [`TerminalEngine::request_redraw`].
//!
//! Transfer bypass: when transfer detection is enabled and a `trzsz` magic
//! header shows up in the stream, the engine stops parsing and forwards raw
//! bytes (`KIND_RAW` frames) so the frontend `trzsz` filter can drive the
//! transfer; display output produced by the filter is written back through
//! [`TerminalEngine::feed`]. The frontend clears bypass when the transfer
//! finishes.

pub mod frame;

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::search::RegexSearch;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermDamage};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Processor, Rgb};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex as AsyncMutex, Notify};
use uuid::Uuid;

use frame::{FrameHeader, FrameWriter, RowRuns};

/// Receives encoded frames (binary). Must be cheap and non-blocking.
pub type FrameSink = Arc<dyn Fn(Vec<u8>) + Send + Sync>;
/// Session output receiver shared with the legacy `session.poll` path.
pub type SharedOutput = Arc<AsyncMutex<mpsc::Receiver<Vec<u8>>>>;

/// Minimum spacing between two frames (≈60 Hz).
pub const FRAME_INTERVAL: Duration = Duration::from_millis(16);
/// Spacing between activity-only frames for hidden panes.
const ACTIVITY_INTERVAL: Duration = Duration::from_millis(250);

const TRZSZ_MAGIC: &[u8] = b"::TRZSZ:TRANSFER:";
const ZMODEM_MAGIC: &[u8] = b"**\x18B0";

#[derive(Clone, Default)]
struct EventQueue(Arc<StdMutex<Vec<Event>>>);

impl EventListener for EventQueue {
    fn send_event(&self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EngineConfig {
    pub cols: u16,
    pub rows: u16,
    pub scrollback: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            cols: 80,
            rows: 24,
            scrollback: 10_000,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ScrollCmd {
    Delta(i32),
    PageUp,
    PageDown,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SelectionKind {
    Simple,
    Block,
    Word,
    Line,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CellSide {
    Left,
    Right,
}

impl From<CellSide> for Side {
    fn from(s: CellSide) -> Self {
        match s {
            CellSide::Left => Side::Left,
            CellSide::Right => Side::Right,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SearchDirection {
    Next,
    Prev,
}

/// Viewport-relative match bounds (inclusive).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub start_col: usize,
    pub start_line: i32,
    pub end_col: usize,
    pub end_line: i32,
}

struct Inner {
    term: Term<EventQueue>,
    events: EventQueue,
    processor: Processor,
    seq: u32,
    /// Grid content changed since the last frame.
    dirty: bool,
    /// Next frame must carry every row.
    full_pending: bool,
    title: Option<String>,
    title_dirty: bool,
    bell: bool,
    clipboard: Option<Vec<u8>>,
    alive: bool,
    exit_pending: bool,
    visible: bool,
    last_activity_emit: Option<std::time::Instant>,
    raw_bypass: bool,
    transfer_detect: bool,
    trzsz_detected: bool,
    zmodem_hint: bool,
    raw_pending: Vec<u8>,
    tail: Vec<u8>,
    cell_px: (u16, u16),
    search_cache: Option<(String, RegexSearch)>,
}

pub struct TerminalEngine {
    inner: StdMutex<Inner>,
    redraw: Notify,
    sink: StdMutex<Option<FrameSink>>,
    pty_tx: mpsc::UnboundedSender<Vec<u8>>,
    pump: StdMutex<Option<tokio::task::JoinHandle<()>>>,
}

impl TerminalEngine {
    /// `pty_tx` receives bytes the terminal must write back to the session
    /// (DA / CPR / colour query replies …).
    pub fn new(cfg: EngineConfig, pty_tx: mpsc::UnboundedSender<Vec<u8>>) -> Arc<Self> {
        let events = EventQueue::default();
        let config = Config {
            scrolling_history: cfg.scrollback,
            ..Config::default()
        };
        let size = TermSize::new(cfg.cols.max(1) as usize, cfg.rows.max(1) as usize);
        let term = Term::new(config, &size, events.clone());
        Arc::new(Self {
            inner: StdMutex::new(Inner {
                term,
                events,
                processor: Processor::new(),
                seq: 0,
                dirty: true,
                full_pending: true,
                title: None,
                title_dirty: false,
                bell: false,
                clipboard: None,
                alive: true,
                exit_pending: false,
                visible: true,
                last_activity_emit: None,
                raw_bypass: false,
                transfer_detect: false,
                trzsz_detected: false,
                zmodem_hint: false,
                raw_pending: Vec::new(),
                tail: Vec::new(),
                cell_px: (0, 0),
                search_cache: None,
            }),
            redraw: Notify::new(),
            sink: StdMutex::new(None),
            pty_tx,
            pump: StdMutex::new(None),
        })
    }

    /// Spawn the output pump. Must be called from within a tokio runtime.
    pub fn start(self: &Arc<Self>, output: SharedOutput) {
        let me = self.clone();
        let handle = tokio::spawn(async move {
            let mut rx = output.lock().await;
            let mut closed = false;
            loop {
                tokio::select! {
                    chunk = rx.recv(), if !closed => match chunk {
                        Some(bytes) => me.ingest(&bytes),
                        None => { closed = true; me.mark_exited(); }
                    },
                    _ = me.redraw.notified() => {}
                }
                if !closed {
                    let deadline = tokio::time::Instant::now() + FRAME_INTERVAL;
                    loop {
                        tokio::select! {
                            biased;
                            chunk = rx.recv() => match chunk {
                                Some(bytes) => me.ingest(&bytes),
                                None => { closed = true; me.mark_exited(); break; }
                            },
                            _ = tokio::time::sleep_until(deadline) => break,
                        }
                    }
                }
                me.emit();
            }
        });
        *self.pump.lock().unwrap() = Some(handle);
    }

    pub fn set_sink(&self, sink: Option<FrameSink>) {
        *self.sink.lock().unwrap() = sink;
        {
            let mut g = self.inner.lock().unwrap();
            g.full_pending = true;
            g.dirty = true;
        }
        self.redraw.notify_one();
    }

    pub fn has_sink(&self) -> bool {
        self.sink.lock().unwrap().is_some()
    }

    pub fn request_redraw(&self) {
        self.redraw.notify_one();
    }

    pub fn request_full(&self) {
        {
            let mut g = self.inner.lock().unwrap();
            g.full_pending = true;
            g.dirty = true;
        }
        self.redraw.notify_one();
    }

    pub fn is_alive(&self) -> bool {
        self.inner.lock().unwrap().alive
    }

    pub fn seq(&self) -> u32 {
        self.inner.lock().unwrap().seq
    }

    fn mark_exited(&self) {
        let mut g = self.inner.lock().unwrap();
        if g.alive {
            g.alive = false;
            g.exit_pending = true;
            g.dirty = true;
        }
    }

    /// Feed bytes into the parser directly (used by the frontend transfer
    /// filter to display its own output while raw bypass is active).
    pub fn feed(&self, bytes: &[u8]) {
        {
            let mut g = self.inner.lock().unwrap();
            Self::advance(&mut g, bytes);
            Self::drain_events(&mut g, &self.pty_tx);
            g.dirty = true;
        }
        self.redraw.notify_one();
    }

    fn ingest(&self, bytes: &[u8]) {
        let mut g = self.inner.lock().unwrap();
        if g.raw_bypass {
            g.raw_pending.extend_from_slice(bytes);
            g.dirty = true;
            return;
        }
        if g.transfer_detect {
            let mut joined = Vec::with_capacity(g.tail.len() + bytes.len());
            joined.extend_from_slice(&g.tail);
            joined.extend_from_slice(bytes);
            if find(&joined, ZMODEM_MAGIC).is_some() {
                g.zmodem_hint = true;
            }
            if let Some(pos) = find(&joined, TRZSZ_MAGIC) {
                let tail_len = g.tail.len();
                let parse_upto = pos.saturating_sub(tail_len);
                if parse_upto > 0 {
                    Self::advance(&mut g, &bytes[..parse_upto]);
                }
                g.raw_bypass = true;
                g.trzsz_detected = true;
                g.raw_pending = joined[pos..].to_vec();
                g.tail.clear();
                g.dirty = true;
                Self::drain_events(&mut g, &self.pty_tx);
                return;
            }
            let keep = TRZSZ_MAGIC.len().max(ZMODEM_MAGIC.len()) - 1;
            let start = joined.len().saturating_sub(keep);
            g.tail = joined[start..].to_vec();
        }
        Self::advance(&mut g, bytes);
        Self::drain_events(&mut g, &self.pty_tx);
        g.dirty = true;
    }

    fn advance(g: &mut Inner, bytes: &[u8]) {
        let Inner {
            term, processor, ..
        } = g;
        processor.advance(term, bytes);
    }

    fn drain_events(g: &mut Inner, pty_tx: &mpsc::UnboundedSender<Vec<u8>>) {
        let events: Vec<Event> = std::mem::take(&mut *g.events.0.lock().unwrap());
        for ev in events {
            match ev {
                Event::Title(t) => {
                    g.title = Some(t);
                    g.title_dirty = true;
                }
                Event::ResetTitle => {
                    g.title = Some(String::new());
                    g.title_dirty = true;
                }
                Event::Bell => g.bell = true,
                Event::ClipboardStore(_, text) => g.clipboard = Some(text.into_bytes()),
                Event::ClipboardLoad(_, _) => {
                    // Reading the clipboard from the remote side is not allowed.
                }
                Event::PtyWrite(text) => {
                    let _ = pty_tx.send(text.into_bytes());
                }
                Event::ColorRequest(index, fmt) => {
                    let rgb = g.term.colors()[index].unwrap_or_else(|| default_rgb(index));
                    let _ = pty_tx.send(fmt(rgb).into_bytes());
                }
                Event::TextAreaSizeRequest(fmt) => {
                    let ws = WindowSize {
                        num_lines: g.term.screen_lines() as u16,
                        num_cols: g.term.columns() as u16,
                        cell_width: g.cell_px.0,
                        cell_height: g.cell_px.1,
                    };
                    let _ = pty_tx.send(fmt(ws).into_bytes());
                }
                Event::Wakeup
                | Event::MouseCursorDirty
                | Event::CursorBlinkingChange
                | Event::Exit
                | Event::ChildExit(_) => {}
            }
        }
    }

    /// Encode and push a frame if anything changed.
    pub fn emit(&self) {
        let frame = {
            let mut g = self.inner.lock().unwrap();
            Self::build_frame(&mut g)
        };
        if let Some(bytes) = frame {
            let sink = self.sink.lock().unwrap().clone();
            if let Some(sink) = sink {
                sink(bytes);
            }
        }
    }

    /// Encode a full snapshot regardless of damage state (used by tests and
    /// by the bench harness). Does not touch the sink.
    pub fn snapshot_frame(&self) -> Vec<u8> {
        let mut g = self.inner.lock().unwrap();
        g.full_pending = true;
        g.dirty = true;
        Self::build_frame(&mut g).unwrap_or_default()
    }

    fn header(g: &mut Inner, kind: u8) -> FrameHeader {
        g.seq = g.seq.wrapping_add(1);
        let content = g.term.renderable_content();
        let display_offset = content.display_offset;
        let cursor_line = content.cursor.point.line.0 + display_offset as i32;
        let cursor_shape = match content.cursor.shape {
            CursorShape::Block => 0,
            CursorShape::Underline => 1,
            CursorShape::Beam => 2,
            CursorShape::Hidden => 3,
            CursorShape::HollowBlock => 4,
        };
        let mode = content.mode.bits();
        let mut flags = 0u32;
        if g.bell {
            flags |= frame::FLAG_BELL;
        }
        if g.exit_pending || !g.alive {
            flags |= frame::FLAG_EXITED;
        }
        if g.zmodem_hint {
            flags |= frame::FLAG_ZMODEM_HINT;
        }
        if g.raw_bypass {
            flags |= frame::FLAG_RAW_BYPASS;
        }
        if g.trzsz_detected {
            flags |= frame::FLAG_TRZSZ_DETECTED;
        }
        let header = FrameHeader {
            kind,
            cols: g.term.columns() as u16,
            rows: g.term.screen_lines() as u16,
            seq: g.seq,
            flags,
            cursor_col: content.cursor.point.column.0 as u16,
            cursor_line: cursor_line.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            cursor_shape,
            display_offset: display_offset as u32,
            history_size: g.term.grid().history_size() as u32,
            mode,
            title: if g.title_dirty { g.title.clone() } else { None },
            clipboard: g.clipboard.take(),
        };
        g.bell = false;
        g.title_dirty = false;
        g.exit_pending = false;
        g.zmodem_hint = false;
        g.trzsz_detected = false;
        header
    }

    fn build_frame(g: &mut Inner) -> Option<Vec<u8>> {
        if !g.raw_pending.is_empty() {
            let raw = std::mem::take(&mut g.raw_pending);
            let header = Self::header(g, frame::KIND_RAW);
            return Some(FrameWriter::new(&header).raw(&raw));
        }
        let has_meta = g.title_dirty || g.bell || g.clipboard.is_some() || g.exit_pending;
        if !g.dirty && !g.full_pending && !has_meta {
            return None;
        }
        if !g.visible {
            let now = std::time::Instant::now();
            let due = match g.last_activity_emit {
                None => true,
                Some(t) => now.duration_since(t) >= ACTIVITY_INTERVAL,
            };
            if !due && !has_meta {
                return None;
            }
            g.last_activity_emit = Some(now);
            g.dirty = false;
            // Keep full_pending so the first visible frame carries everything.
            g.full_pending = true;
            let header = Self::header(g, frame::KIND_ACTIVITY);
            return Some(FrameWriter::new(&header).finish());
        }

        let rows_total = g.term.screen_lines();
        let mut full = g.full_pending;
        let damaged: Vec<usize> = if full {
            (0..rows_total).collect()
        } else {
            match g.term.damage() {
                TermDamage::Full => {
                    full = true;
                    (0..rows_total).collect()
                }
                TermDamage::Partial(iter) => iter.map(|b| b.line).collect(),
            }
        };
        let rows = Self::encode_rows(&g.term, &damaged);
        g.term.reset_damage();
        let kind = if full {
            frame::KIND_FULL
        } else {
            frame::KIND_PARTIAL
        };
        g.dirty = false;
        g.full_pending = false;
        let header = Self::header(g, kind);
        Some(FrameWriter::new(&header).rows(&rows))
    }

    fn encode_rows(term: &Term<EventQueue>, lines: &[usize]) -> Vec<RowRuns> {
        let grid = term.grid();
        let display_offset = grid.display_offset() as i32;
        let cols = grid.columns();
        let colors = term.colors();
        let selection = term.selection.as_ref().and_then(|s| s.to_range(term));
        let mut out = Vec::with_capacity(lines.len());
        for &y in lines {
            if y >= grid.screen_lines() {
                continue;
            }
            let line = Line(y as i32 - display_offset);
            let row = &grid[line];
            let cells = (0..cols).map(|x| {
                let cell = &row[Column(x)];
                let point = Point::new(line, Column(x));
                let mut flags = cell.flags.bits() & !frame::CELL_SELECTED;
                if selection.as_ref().is_some_and(|r| r.contains(point)) {
                    flags |= frame::CELL_SELECTED;
                }
                let ch = if cell.flags.contains(Flags::WIDE_CHAR_SPACER)
                    || cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    ' ' as u32
                } else {
                    cell.c as u32
                };
                (
                    ch,
                    encode_color(cell.fg, colors),
                    encode_color(cell.bg, colors),
                    flags,
                )
            });
            out.push(RowRuns {
                y: y as u16,
                runs: frame::build_runs(cells),
            });
        }
        out
    }

    // ----- UI-driven mutations -------------------------------------------

    pub fn resize(&self, cols: u16, rows: u16, cell_w: u16, cell_h: u16) {
        {
            let mut g = self.inner.lock().unwrap();
            g.cell_px = (cell_w, cell_h);
            let size = TermSize::new(cols.max(1) as usize, rows.max(1) as usize);
            g.term.resize(size);
            g.full_pending = true;
            g.dirty = true;
        }
        self.redraw.notify_one();
    }

    pub fn scroll(&self, cmd: ScrollCmd) {
        {
            let mut g = self.inner.lock().unwrap();
            let scroll = match cmd {
                ScrollCmd::Delta(n) => Scroll::Delta(n),
                ScrollCmd::PageUp => Scroll::PageUp,
                ScrollCmd::PageDown => Scroll::PageDown,
                ScrollCmd::Top => Scroll::Top,
                ScrollCmd::Bottom => Scroll::Bottom,
            };
            g.term.scroll_display(scroll);
            g.full_pending = true;
            g.dirty = true;
        }
        self.redraw.notify_one();
    }

    pub fn set_visible(&self, visible: bool) {
        {
            let mut g = self.inner.lock().unwrap();
            if visible && !g.visible {
                g.full_pending = true;
                g.dirty = true;
            }
            g.visible = visible;
        }
        if visible {
            self.redraw.notify_one();
        }
    }

    pub fn set_transfer_detect(&self, on: bool) {
        let mut g = self.inner.lock().unwrap();
        g.transfer_detect = on;
        if !on {
            g.tail.clear();
        }
    }

    pub fn set_raw_bypass(&self, on: bool) {
        {
            let mut g = self.inner.lock().unwrap();
            g.raw_bypass = on;
            if !on {
                g.tail.clear();
                g.full_pending = true;
                g.dirty = true;
            }
        }
        self.redraw.notify_one();
    }

    pub fn raw_bypass(&self) -> bool {
        self.inner.lock().unwrap().raw_bypass
    }

    fn viewport_point(g: &Inner, col: usize, line: i32) -> Point {
        let cols = g.term.columns();
        let rows = g.term.screen_lines() as i32;
        let display_offset = g.term.grid().display_offset() as i32;
        let col = Column(col.min(cols.saturating_sub(1)));
        let line = line.clamp(0, rows - 1) - display_offset;
        let topmost = g.term.topmost_line().0;
        Point::new(Line(line.max(topmost)), col)
    }

    pub fn selection_start(&self, col: usize, line: i32, kind: SelectionKind, side: CellSide) {
        {
            let mut g = self.inner.lock().unwrap();
            let point = Self::viewport_point(&g, col, line);
            let ty = match kind {
                SelectionKind::Simple => SelectionType::Simple,
                SelectionKind::Block => SelectionType::Block,
                SelectionKind::Word => SelectionType::Semantic,
                SelectionKind::Line => SelectionType::Lines,
            };
            g.term.selection = Some(Selection::new(ty, point, side.into()));
            g.full_pending = true;
            g.dirty = true;
        }
        self.redraw.notify_one();
    }

    pub fn selection_update(&self, col: usize, line: i32, side: CellSide) {
        {
            let mut g = self.inner.lock().unwrap();
            let point = Self::viewport_point(&g, col, line);
            if let Some(sel) = g.term.selection.as_mut() {
                sel.update(point, side.into());
            }
            g.full_pending = true;
            g.dirty = true;
        }
        self.redraw.notify_one();
    }

    pub fn selection_clear(&self) {
        {
            let mut g = self.inner.lock().unwrap();
            if g.term.selection.take().is_some() {
                g.full_pending = true;
                g.dirty = true;
            }
        }
        self.redraw.notify_one();
    }

    pub fn select_all(&self) {
        {
            let mut g = self.inner.lock().unwrap();
            let topmost = g.term.topmost_line();
            let bottom = g.term.bottommost_line();
            let last_col = g.term.last_column();
            let mut sel = Selection::new(
                SelectionType::Simple,
                Point::new(topmost, Column(0)),
                Side::Left,
            );
            sel.update(Point::new(bottom, last_col), Side::Right);
            g.term.selection = Some(sel);
            g.full_pending = true;
            g.dirty = true;
        }
        self.redraw.notify_one();
    }

    pub fn selection_text(&self) -> Option<String> {
        let g = self.inner.lock().unwrap();
        g.term.selection_to_string()
    }

    pub fn has_selection(&self) -> bool {
        let g = self.inner.lock().unwrap();
        g.term.selection.as_ref().is_some_and(|s| !s.is_empty())
    }

    /// Text of the whole visible viewport (for diagnostics / tests).
    pub fn viewport_text(&self) -> String {
        let g = self.inner.lock().unwrap();
        let display_offset = g.term.grid().display_offset() as i32;
        let rows = g.term.screen_lines() as i32;
        let last = g.term.last_column();
        let start = Point::new(Line(-display_offset), Column(0));
        let end = Point::new(Line(rows - 1 - display_offset), last);
        g.term.bounds_to_string(start, end)
    }

    /// Full scrollback + viewport as plain text (frontend handoff when leaving native).
    pub fn buffer_text(&self) -> String {
        let g = self.inner.lock().unwrap();
        let topmost = g.term.topmost_line();
        let bottom = g.term.bottommost_line();
        let last_col = g.term.last_column();
        g.term
            .bounds_to_string(Point::new(topmost, Column(0)), Point::new(bottom, last_col))
    }

    /// Find the next/previous match, select it and scroll it into view.
    pub fn search(
        &self,
        query: &str,
        is_regex: bool,
        case_sensitive: bool,
        direction: SearchDirection,
    ) -> Result<Option<SearchHit>, String> {
        if query.is_empty() {
            return Ok(None);
        }
        let mut pattern = if is_regex {
            query.to_string()
        } else {
            escape_regex(query)
        };
        if !case_sensitive {
            pattern = format!("(?i){pattern}");
        }
        let mut g = self.inner.lock().unwrap();
        let need_new = match g.search_cache.as_ref() {
            None => true,
            Some((p, _)) => p != &pattern,
        };
        if need_new {
            let rs = RegexSearch::new(&pattern).map_err(|e| e.to_string())?;
            g.search_cache = Some((pattern.clone(), rs));
        }
        let (dir, side) = match direction {
            SearchDirection::Next => (Direction::Right, Side::Left),
            SearchDirection::Prev => (Direction::Left, Side::Right),
        };
        let origin = match g.term.selection.as_ref().and_then(|s| s.to_range(&g.term)) {
            // Step one cell past the current selection so repeated searches advance.
            Some(range) => match direction {
                SearchDirection::Next => range.end.add(&g.term, Boundary::None, 1),
                SearchDirection::Prev => range.start.sub(&g.term, Boundary::None, 1),
            },
            None => {
                let display_offset = g.term.grid().display_offset() as i32;
                match direction {
                    // No selection yet: scan forward from the viewport top, or
                    // backward from the viewport bottom.
                    SearchDirection::Next => Point::new(Line(-display_offset), Column(0)),
                    SearchDirection::Prev => Point::new(
                        Line(g.term.screen_lines() as i32 - 1 - display_offset),
                        g.term.last_column(),
                    ),
                }
            }
        };
        let Inner {
            term, search_cache, ..
        } = &mut *g;
        let regex = &mut search_cache.as_mut().unwrap().1;
        let found = term.search_next(regex, origin, dir, side, None);
        let Some(m) = found else {
            return Ok(None);
        };
        let (start, end) = (*m.start(), *m.end());
        let mut sel = Selection::new(SelectionType::Simple, start, Side::Left);
        sel.update(end, Side::Right);
        term.selection = Some(sel);
        term.scroll_to_point(start);
        let display_offset = term.grid().display_offset() as i32;
        g.full_pending = true;
        g.dirty = true;
        drop(g);
        self.redraw.notify_one();
        Ok(Some(SearchHit {
            start_col: start.column.0,
            start_line: start.line.0 + display_offset,
            end_col: end.column.0,
            end_line: end.line.0 + display_offset,
        }))
    }

    /// OSC 8 hyperlink or an implicitly detected URL under the given cell.
    pub fn hyperlink_at(&self, col: usize, line: i32) -> Option<String> {
        let g = self.inner.lock().unwrap();
        let point = Self::viewport_point(&g, col, line);
        let grid = g.term.grid();
        let cell = &grid[point];
        if let Some(h) = cell.hyperlink() {
            return Some(h.uri().to_string());
        }
        let cols = grid.columns();
        let row = &grid[point.line];
        let chars: Vec<char> = (0..cols)
            .map(|x| {
                let c = &row[Column(x)];
                if c.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    '\u{0}'
                } else {
                    c.c
                }
            })
            .collect();
        detect_url(&chars, point.column.0)
    }

    pub fn mode_bits(&self) -> u32 {
        self.inner.lock().unwrap().term.mode().bits()
    }

    pub fn title(&self) -> Option<String> {
        self.inner.lock().unwrap().title.clone()
    }

    pub fn dims(&self) -> (u16, u16) {
        let g = self.inner.lock().unwrap();
        (g.term.columns() as u16, g.term.screen_lines() as u16)
    }

    pub fn close(&self) {
        if let Some(h) = self.pump.lock().unwrap().take() {
            h.abort();
        }
        *self.sink.lock().unwrap() = None;
    }
}

impl Drop for TerminalEngine {
    fn drop(&mut self) {
        if let Some(h) = self.pump.lock().unwrap().take() {
            h.abort();
        }
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

fn encode_color(c: Color, colors: &alacritty_terminal::term::color::Colors) -> u32 {
    match c {
        Color::Spec(rgb) => frame::COLOR_RGB | pack_rgb(rgb),
        Color::Indexed(i) => match colors[i as usize] {
            Some(rgb) => frame::COLOR_RGB | pack_rgb(rgb),
            None => frame::COLOR_INDEXED | i as u32,
        },
        Color::Named(n) => match colors[n] {
            Some(rgb) => frame::COLOR_RGB | pack_rgb(rgb),
            None => frame::COLOR_NAMED | (n as u32),
        },
    }
}

#[inline]
fn pack_rgb(rgb: Rgb) -> u32 {
    ((rgb.r as u32) << 16) | ((rgb.g as u32) << 8) | rgb.b as u32
}

/// Default xterm-256 palette used only for OSC colour *queries* when the
/// application has not overridden the slot.
pub fn default_rgb(index: usize) -> Rgb {
    const ANSI: [u32; 16] = [
        0x000000, 0xcd0000, 0x00cd00, 0xcdcd00, 0x0000ee, 0xcd00cd, 0x00cdcd, 0xe5e5e5, 0x7f7f7f,
        0xff0000, 0x00ff00, 0xffff00, 0x5c5cff, 0xff00ff, 0x00ffff, 0xffffff,
    ];
    let packed = match index {
        0..=15 => ANSI[index],
        16..=231 => {
            let i = index - 16;
            let r = i / 36;
            let gg = (i % 36) / 6;
            let b = i % 6;
            let f = |v: usize| -> u32 {
                if v == 0 {
                    0
                } else {
                    (55 + v * 40) as u32
                }
            };
            (f(r) << 16) | (f(gg) << 8) | f(b)
        }
        232..=255 => {
            let v = (8 + (index - 232) * 10) as u32;
            (v << 16) | (v << 8) | v
        }
        i if i == NamedColor::Background as usize => 0x000000,
        i if i == NamedColor::Cursor as usize => 0xd8d8d8,
        _ => 0xd8d8d8,
    };
    Rgb {
        r: (packed >> 16) as u8,
        g: (packed >> 8) as u8,
        b: packed as u8,
    }
}

fn escape_regex(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

const URL_SCHEMES: &[&str] = &[
    "http://", "https://", "ftp://", "ftps://", "file://", "ssh://", "git://", "sftp://",
];

fn is_url_stop(c: char) -> bool {
    c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`' | '\u{0}')
}

/// Scan the row around `col` for something that looks like a URL.
pub fn detect_url(chars: &[char], col: usize) -> Option<String> {
    if col >= chars.len() || is_url_stop(chars[col]) {
        return None;
    }
    let mut start = col;
    while start > 0 && !is_url_stop(chars[start - 1]) {
        start -= 1;
    }
    let mut end = col;
    while end + 1 < chars.len() && !is_url_stop(chars[end + 1]) {
        end += 1;
    }
    let mut token: String = chars[start..=end].iter().collect();
    // Trim trailing punctuation that is unlikely to be part of the URL.
    while token
        .chars()
        .last()
        .is_some_and(|c| matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}'))
    {
        token.pop();
    }
    // Allow a leading bracket/paren that wrapped the URL.
    let token = token.trim_start_matches(['(', '[', '{']).to_string();
    let lower = token.to_ascii_lowercase();
    if URL_SCHEMES.iter().any(|s| lower.starts_with(s)) {
        return Some(token);
    }
    if lower.starts_with("www.") && lower.len() > 4 {
        return Some(format!("https://{token}"));
    }
    None
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Per-session engine registry owned by `AppState`.
#[derive(Default)]
pub struct TerminalEngineRegistry {
    engines: StdMutex<HashMap<Uuid, Arc<TerminalEngine>>>,
}

impl TerminalEngineRegistry {
    pub fn get(&self, id: Uuid) -> Option<Arc<TerminalEngine>> {
        self.engines.lock().unwrap().get(&id).cloned()
    }

    pub fn insert(&self, id: Uuid, engine: Arc<TerminalEngine>) {
        if let Some(old) = self.engines.lock().unwrap().insert(id, engine) {
            old.close();
        }
    }

    pub fn remove(&self, id: Uuid) -> Option<Arc<TerminalEngine>> {
        let removed = self.engines.lock().unwrap().remove(&id);
        if let Some(e) = &removed {
            e.close();
        }
        removed
    }

    pub fn contains(&self, id: Uuid) -> bool {
        self.engines.lock().unwrap().contains_key(&id)
    }

    pub fn len(&self) -> usize {
        self.engines.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
impl TerminalEngine {
    /// Build whatever frame is pending without forcing a full redraw.
    fn snapshot_frame_any_for_test(&self) -> Vec<u8> {
        let mut g = self.inner.lock().unwrap();
        Self::build_frame(&mut g).unwrap_or_default()
    }

    fn snapshot_frame_partial_for_test(&self) -> Vec<u8> {
        let mut g = self.inner.lock().unwrap();
        g.full_pending = false;
        g.dirty = true;
        Self::build_frame(&mut g).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::frame::decode::decode;
    use super::*;

    fn engine(cols: u16, rows: u16) -> (Arc<TerminalEngine>, mpsc::UnboundedReceiver<Vec<u8>>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let e = TerminalEngine::new(
            EngineConfig {
                cols,
                rows,
                scrollback: 100,
            },
            tx,
        );
        (e, rx)
    }

    fn row_text(frame: &super::frame::decode::Decoded, y: u16) -> String {
        frame
            .rows
            .iter()
            .find(|r| r.y == y)
            .map(|r| {
                r.runs
                    .iter()
                    .flat_map(|run| run.chars.iter().map(|&c| char::from_u32(c).unwrap_or('?')))
                    .collect::<String>()
            })
            .unwrap_or_default()
    }

    #[test]
    fn full_frame_contains_text_and_cursor() {
        let (e, _rx) = engine(20, 4);
        e.feed(b"hello \x1b[31mred\x1b[0m\r\nline2");
        let bytes = e.snapshot_frame();
        let d = decode(&bytes);
        assert_eq!(d.header.kind, frame::KIND_FULL);
        assert_eq!(d.header.cols, 20);
        assert_eq!(d.header.rows, 4);
        assert_eq!(d.rows.len(), 4);
        assert!(row_text(&d, 0).starts_with("hello red"));
        assert!(row_text(&d, 1).starts_with("line2"));
        assert_eq!(d.header.cursor_line, 1);
        assert_eq!(d.header.cursor_col, 5);
        // "red" run uses named colour 1.
        let red = d.rows[0]
            .runs
            .iter()
            .find(|r| r.fg == frame::COLOR_NAMED | 1)
            .expect("red run");
        assert_eq!(red.chars, vec!['r' as u32, 'e' as u32, 'd' as u32]);
    }

    #[test]
    fn partial_frame_only_carries_damaged_rows() {
        let (e, _rx) = engine(10, 3);
        e.feed(b"a\r\nb\r\nc");
        let _ = e.snapshot_frame(); // clears damage
        e.feed(b"\x1b[1;1Hz");
        let d = decode(&e.snapshot_frame_partial_for_test());
        assert_eq!(d.header.kind, frame::KIND_PARTIAL);
        let ys: Vec<u16> = d.rows.iter().map(|r| r.y).collect();
        assert!(ys.contains(&0));
        assert!(!ys.contains(&1));
    }

    #[test]
    fn scrollback_and_scroll_commands() {
        let (e, _rx) = engine(10, 2);
        for i in 0..10 {
            e.feed(format!("line{i}\r\n").as_bytes());
        }
        let d = decode(&e.snapshot_frame());
        assert!(d.header.history_size >= 8);
        e.scroll(ScrollCmd::Delta(3));
        let d = decode(&e.snapshot_frame());
        assert_eq!(d.header.display_offset, 3);
        assert!(row_text(&d, 0).starts_with("line6"));
        e.scroll(ScrollCmd::Bottom);
        let d = decode(&e.snapshot_frame());
        assert_eq!(d.header.display_offset, 0);
    }

    #[test]
    fn selection_marks_cells_and_extracts_text() {
        let (e, _rx) = engine(10, 2);
        e.feed(b"hello world");
        e.selection_start(0, 0, SelectionKind::Simple, CellSide::Left);
        e.selection_update(4, 0, CellSide::Right);
        assert_eq!(e.selection_text().as_deref(), Some("hello"));
        let d = decode(&e.snapshot_frame());
        let selected: usize = d.rows[0]
            .runs
            .iter()
            .filter(|r| r.flags & frame::CELL_SELECTED != 0)
            .map(|r| r.chars.len())
            .sum();
        assert_eq!(selected, 5);
        e.selection_clear();
        assert!(e.selection_text().is_none());
    }

    #[test]
    fn search_selects_match_and_reports_viewport_coords() {
        let (e, _rx) = engine(20, 3);
        e.feed(b"foo bar\r\nbaz Foo\r\nqux");
        let hit = e
            .search("foo", false, false, SearchDirection::Next)
            .unwrap()
            .expect("hit");
        assert_eq!((hit.start_line, hit.start_col), (0, 0));
        let hit2 = e
            .search("foo", false, false, SearchDirection::Next)
            .unwrap()
            .expect("second hit");
        assert_eq!((hit2.start_line, hit2.start_col), (1, 4));
        assert!(e
            .search("nomatch", false, true, SearchDirection::Next)
            .unwrap()
            .is_none());
        assert!(e.search("(", true, true, SearchDirection::Next).is_err());
    }

    #[test]
    fn title_bell_and_pty_write_events() {
        let (e, mut rx) = engine(10, 2);
        e.feed(b"\x1b]0;My Title\x07\x07");
        let d = decode(&e.snapshot_frame());
        assert_eq!(d.header.title.as_deref(), Some("My Title"));
        assert!(d.header.flags & frame::FLAG_BELL != 0);
        // Device attributes request → reply written back to the pty.
        e.feed(b"\x1b[c");
        let reply = rx.try_recv().expect("DA reply");
        assert!(reply.starts_with(b"\x1b[?"));
        // Flags are one-shot.
        let d = decode(&e.snapshot_frame());
        assert!(d.header.title.is_none());
        assert!(d.header.flags & frame::FLAG_BELL == 0);
    }

    #[test]
    fn trzsz_magic_switches_to_raw_bypass_across_chunks() {
        let (e, _rx) = engine(40, 4);
        e.set_transfer_detect(true);
        e.ingest(b"prompt$ ::TRZ");
        e.ingest(b"SZ:TRANSFER:S:1.1.0:1234\r\n");
        assert!(e.raw_bypass());
        let d = decode(&e.snapshot_frame());
        assert_eq!(d.header.kind, frame::KIND_RAW);
        assert!(d.header.flags & frame::FLAG_TRZSZ_DETECTED != 0);
        assert!(d.raw.starts_with(b"::TRZSZ:TRANSFER:S:1.1.0:1234"));
        e.ingest(b"more raw");
        let d = decode(&e.snapshot_frame());
        assert_eq!(d.raw, b"more raw");
        e.set_raw_bypass(false);
        let d = decode(&e.snapshot_frame());
        assert_eq!(d.header.kind, frame::KIND_FULL);
        assert!(row_text(&d, 0).starts_with("prompt$ "));
    }

    #[test]
    fn hidden_pane_gets_activity_frames_then_full_on_show() {
        let (e, _rx) = engine(10, 2);
        e.set_visible(false);
        e.feed(b"x");
        let d = decode(&e.snapshot_frame_any_for_test());
        assert_eq!(d.header.kind, frame::KIND_ACTIVITY);
        e.set_visible(true);
        let d = decode(&e.snapshot_frame_any_for_test());
        assert_eq!(d.header.kind, frame::KIND_FULL);
    }

    #[test]
    fn url_detection() {
        let row: Vec<char> = "see https://example.com/a?b=1. and (www.foo.org)"
            .chars()
            .collect();
        assert_eq!(
            detect_url(&row, 10).as_deref(),
            Some("https://example.com/a?b=1")
        );
        assert_eq!(detect_url(&row, 1), None);
        assert_eq!(
            detect_url(&row, row.len() - 3).as_deref(),
            Some("https://www.foo.org")
        );
    }

    #[test]
    fn wide_chars_mark_spacer() {
        let (e, _rx) = engine(10, 1);
        e.feed("中a".as_bytes());
        let d = decode(&e.snapshot_frame());
        let cells: Vec<(u32, u16)> = d.rows[0]
            .runs
            .iter()
            .flat_map(|r| r.chars.iter().map(move |&c| (c, r.flags)))
            .collect();
        assert_eq!(cells[0].0, '中' as u32);
        assert!(cells[0].1 & Flags::WIDE_CHAR.bits() != 0);
        assert!(cells[1].1 & Flags::WIDE_CHAR_SPACER.bits() != 0);
        assert_eq!(cells[2].0, 'a' as u32);
    }

    #[tokio::test]
    async fn pump_pushes_frames_and_reports_exit() {
        let (e, _pty_rx) = engine(10, 2);
        let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>(8);
        let shared: SharedOutput = Arc::new(AsyncMutex::new(out_rx));
        let (frame_tx, mut frame_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        e.set_sink(Some(Arc::new(move |f| {
            let _ = frame_tx.send(f);
        })));
        e.start(shared);
        out_tx.send(b"hi".to_vec()).await.unwrap();
        let mut saw_text = false;
        for _ in 0..5 {
            let f = tokio::time::timeout(Duration::from_secs(2), frame_rx.recv())
                .await
                .expect("frame in time")
                .expect("frame");
            let d = decode(&f);
            if d.header.kind == frame::KIND_FULL && row_text(&d, 0).starts_with("hi") {
                saw_text = true;
                break;
            }
        }
        assert!(saw_text);
        drop(out_tx);
        let mut saw_exit = false;
        for _ in 0..5 {
            let f = tokio::time::timeout(Duration::from_secs(2), frame_rx.recv())
                .await
                .expect("frame in time")
                .expect("frame");
            let d = decode(&f);
            if d.header.flags & frame::FLAG_EXITED != 0 {
                saw_exit = true;
                break;
            }
        }
        assert!(saw_exit);
        assert!(!e.is_alive());
        // Scrollback still served after exit.
        e.request_full();
        let f = tokio::time::timeout(Duration::from_secs(2), frame_rx.recv())
            .await
            .expect("frame in time")
            .expect("frame");
        assert_eq!(decode(&f).header.kind, frame::KIND_FULL);
        e.close();
    }
}
