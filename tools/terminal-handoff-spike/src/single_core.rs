//! Acceptance route: ONE persistent alacritty parser, two presentation endpoints.
//! No second parser, byte replay, byte journal or cross-core equivalence required.
//! Frames/first_frame are protocol models, not actual GPU presentation evidence.
use alacritty_terminal::{
    event::{Event, EventListener},
    grid::Dimensions,
    index::{Column, Line, Point},
    term::{Config, Term, TermMode, test::TermSize},
    vte::ansi::Processor,
};
use std::sync::{Arc, Mutex};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Native,
    Webview,
}
impl View {
    fn index(self) -> usize {
        match self {
            Self::Native => 0,
            Self::Webview => 1,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    NotReady,
    Stale,
    Detached,
    ActiveView,
    Backpressure,
    Timeout,
    Ended,
    InFlight,
    InvalidSize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub rows: Vec<String>,
    pub cells: Vec<Vec<String>>,
    pub render_rows: Vec<Vec<serde_json::Value>>,
    pub cursor: (usize, i32),
    pub mode: u32,
    pub history: usize,
    pub history_rows: Vec<String>,
    pub colors: Vec<String>,
    pub cols: usize,
    pub lines: usize,
    pub ended: bool,
}
#[derive(Clone, Debug)]
pub struct Frame {
    pub epoch: u64,
    pub revision: u64,
    pub wire_seq: u32,
    pub output_seq: u64,
    pub full: bool,
    pub base_revision: Option<u64>,
    pub changed_rows: Vec<usize>,
    pub snapshot: Snapshot,
}
impl Frame {
    /// Public state-only JSON adapter shared by host fixtures.
    pub fn to_wire(&self, session: u64, view: View) -> serde_json::Value {
        let s = &self.snapshot;
        serde_json::json!({"version":1,"session":session.to_string(),"view":match view {View::Native=>"native",View::Webview=>"webview"},"epoch":self.epoch.to_string(),"revision":self.revision.to_string(),"output_seq":self.output_seq.to_string(),"wire_seq":self.wire_seq,"full":self.full,"base_revision":self.base_revision.map(|r|r.to_string()),"cols":s.cols,"lines":s.lines,"cursor":{"col":s.cursor.0,"line":s.cursor.1,"visible":s.mode & TermMode::SHOW_CURSOR.bits()!=0},"mode":s.mode,"history":s.history,"ended":s.ended,"rows":self.changed_rows.iter().map(|&y|serde_json::json!({"y":y,"cells":s.render_rows[y]})).collect::<Vec<_>>()})
    }
}
#[derive(Default)]
struct Attachment {
    epoch: u64,
    attached: bool,
    full: bool,
    ack: Option<u64>,
    presented: Option<u64>,
    flight: Option<Frame>,
    baseline: Option<Snapshot>,
}
#[derive(Clone, Default)]
struct Listener(Arc<Mutex<Vec<String>>>);
impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        if let Event::PtyWrite(s) = event {
            self.0.lock().unwrap().push(s);
        }
    }
}
struct Pending {
    target: View,
    deadline: u64,
}
pub struct CanonicalSession {
    id: u64,
    term: Box<Term<Listener>>,
    parser: Processor,
    listener: Listener,
    views: [Attachment; 2],
    active: View,
    revision: u64,
    output_seq: u64,
    feeds: u64,
    wire_seq: u32,
    ended: bool,
    pending: Option<Pending>,
    input_queue: Vec<u8>,
    transport_input: Vec<u8>,
}
impl CanonicalSession {
    pub fn new(id: u64, cols: u16, rows: u16) -> Self {
        assert!(cols >= 2 && rows > 0);
        let listener = Listener::default();
        Self {
            id,
            term: Box::new(Term::new(
                Config {
                    scrolling_history: 5000,
                    ..Config::default()
                },
                &TermSize::new(cols as usize, rows as usize),
                listener.clone(),
            )),
            parser: Processor::new(),
            listener,
            views: Default::default(),
            active: View::Native,
            revision: 0,
            output_seq: 0,
            feeds: 0,
            wire_seq: 0,
            ended: false,
            pending: None,
            input_queue: Vec::new(),
            transport_input: Vec::new(),
        }
    }
    pub fn core_identity(&self) -> usize {
        std::ptr::from_ref(&*self.term) as usize
    }
    pub fn session_id(&self) -> u64 {
        self.id
    }
    pub fn active(&self) -> View {
        self.active
    }
    pub fn feed_count(&self) -> u64 {
        self.feeds
    }
    pub fn output_seq(&self) -> u64 {
        self.output_seq
    }
    pub fn snapshot(&self) -> Snapshot {
        let g = self.term.grid();
        let mut rows = Vec::new();
        let mut cells = Vec::new();
        for row in 0..self.term.screen_lines() {
            let mut text = String::new();
            let mut line = Vec::new();
            for col in 0..self.term.columns() {
                let c = &g[Point::new(Line(row as i32), Column(col))];
                text.push(c.c);
                line.push(format!("{c:?}"));
            }
            rows.push(text);
            cells.push(line);
        }
        let history_rows = (-(g.history_size() as i32)..0)
            .map(|row| {
                (0..self.term.columns())
                    .map(|col| g[Point::new(Line(row), Column(col))].c)
                    .collect()
            })
            .collect();
        let colors = (0..269)
            .map(|i| format!("{:?}", self.term.colors()[i]))
            .collect();
        use alacritty_terminal::{term::cell::Flags, vte::ansi::Color};
        let color = |c: Color| match c {
            Color::Spec(rgb) => {
                serde_json::json!({"kind":"rgb","value":(u32::from(rgb.r)<<16)|(u32::from(rgb.g)<<8)|u32::from(rgb.b)})
            }
            Color::Indexed(i) => serde_json::json!({"kind":"indexed","value":i}),
            Color::Named(n) => serde_json::json!({"kind":"named","value":n as u32}),
        };
        let render_rows = (0..self.term.screen_lines()).map(|row|(0..self.term.columns()).map(|col|{
            let c=&g[Point::new(Line(row as i32),Column(col))];
            let mut text=c.c.to_string();if let Some(extra)=c.zerowidth(){text.extend(extra);}
            let resolve=|v:Color|match v{Color::Named(n) if (n as usize)<269=>self.term.colors()[n as usize].map(Color::Spec).unwrap_or(v),_=>v};
            serde_json::json!({"text":text,"width":if c.flags.contains(Flags::WIDE_CHAR_SPACER){0}else if c.flags.contains(Flags::WIDE_CHAR){2}else{1},"fg":color(resolve(c.fg)),"bg":color(resolve(c.bg)),"flags":c.flags.bits()})
        }).collect()).collect();
        Snapshot {
            rows,
            cells,
            render_rows,
            history_rows,
            colors,
            cursor: (g.cursor.point.column.0, g.cursor.point.line.0),
            mode: self.term.mode().bits(),
            history: g.history_size(),
            cols: self.term.columns(),
            lines: self.term.screen_lines(),
            ended: self.ended,
        }
    }
    pub fn output(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if self.ended {
            return Err(Error::Ended);
        }
        self.parser.advance(&mut *self.term, bytes);
        self.feeds += 1;
        self.output_seq += 1;
        self.revision += 1;
        Ok(())
    }
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), Error> {
        if cols < 2 || rows == 0 {
            return Err(Error::InvalidSize);
        }
        self.term
            .resize(TermSize::new(cols as usize, rows as usize));
        self.output_seq += 1;
        self.revision += 1;
        for v in &mut self.views {
            v.full = true;
        }
        Ok(())
    }
    pub fn eof(&mut self) {
        self.ended = true;
        self.output_seq += 1;
        self.revision += 1;
    }
    pub fn attach(&mut self, view: View) -> Result<(), Error> {
        let a = &mut self.views[view.index()];
        a.epoch += 1;
        a.attached = true;
        a.full = true;
        a.ack = None;
        a.presented = None;
        a.flight = None;
        a.baseline = None;
        Ok(())
    }
    pub fn detach(&mut self, view: View) -> Result<(), Error> {
        if view == self.active {
            return Err(Error::ActiveView);
        }
        if self.pending.as_ref().is_some_and(|p| p.target == view) {
            self.cancel()?;
        }
        let a = &mut self.views[view.index()];
        a.attached = false;
        a.flight = None;
        a.baseline = None;
        Ok(())
    }
    pub fn request_full(&mut self, view: View) -> Result<(), Error> {
        let a = &mut self.views[view.index()];
        if !a.attached {
            return Err(Error::Detached);
        }
        a.full = true;
        Ok(())
    }
    /// One in-flight immutable snapshot per view. No revision journal: a lagging
    /// view receives a freshly coalesced diff versus its acknowledged baseline,
    /// or full snapshot after resize/reattach/request_full. History stays in core.
    pub fn frame(&mut self, view: View) -> Result<Option<Frame>, Error> {
        let a = &self.views[view.index()];
        if !a.attached {
            return Err(Error::Detached);
        }
        if a.flight.is_some() {
            return Err(Error::InFlight);
        }
        if a.ack == Some(self.revision) && !a.full {
            return Ok(None);
        }
        let snapshot = self.snapshot();
        let a = &mut self.views[view.index()];
        let changed_rows = (0..snapshot.lines)
            .filter(|row| {
                a.full
                    || a.baseline
                        .as_ref()
                        .is_none_or(|b| b.cells.get(*row) != snapshot.cells.get(*row))
            })
            .collect();
        self.wire_seq = self.wire_seq.wrapping_add(1);
        let frame = Frame {
            epoch: a.epoch,
            revision: self.revision,
            wire_seq: self.wire_seq,
            output_seq: self.output_seq,
            full: a.full,
            base_revision: if a.full { None } else { a.ack },
            changed_rows,
            snapshot,
        };
        a.flight = Some(frame.clone());
        Ok(Some(frame))
    }
    pub fn ack(&mut self, view: View, epoch: u64, revision: u64) -> Result<(), Error> {
        let a = &mut self.views[view.index()];
        if !a.attached {
            return Err(Error::Detached);
        }
        if a.epoch != epoch || a.flight.as_ref().is_none_or(|f| f.revision != revision) {
            return Err(Error::Stale);
        }
        let f = a.flight.take().unwrap();
        a.ack = Some(revision);
        a.baseline = Some(f.snapshot);
        a.full = a.ack != Some(self.revision)
            && a.baseline.as_ref().is_some_and(|b| {
                b.cols != self.term.columns() || b.lines != self.term.screen_lines()
            });
        Ok(())
    }
    pub fn first_frame(&mut self, view: View, epoch: u64, revision: u64) -> Result<(), Error> {
        let a = &mut self.views[view.index()];
        if a.epoch != epoch || a.ack != Some(revision) {
            return Err(Error::Stale);
        }
        a.presented = Some(revision);
        Ok(())
    }
    fn ready(&self, view: View) -> bool {
        let a = &self.views[view.index()];
        a.attached && a.ack == Some(self.revision) && a.presented == Some(self.revision)
    }
    /// Do not freeze input during attachment/catchup; only this short commit
    /// window. Caller-supplied monotonic clock makes timeout deterministic.
    pub fn begin_commit(&mut self, target: View, now: u64, timeout: u64) -> Result<(), Error> {
        if !self.ready(target) {
            return Err(Error::NotReady);
        }
        self.pending = Some(Pending {
            target,
            deadline: now.saturating_add(timeout),
        });
        Ok(())
    }
    pub fn commit(&mut self, now: u64) -> Result<(), Error> {
        let p = self.pending.as_ref().ok_or(Error::NotReady)?;
        if now > p.deadline {
            self.cancel()?;
            return Err(Error::Timeout);
        }
        if !self.ready(p.target) {
            return Err(Error::NotReady);
        }
        self.active = p.target;
        self.cancel()?;
        Ok(())
    }
    pub fn cancel(&mut self) -> Result<(), Error> {
        self.pending = None;
        self.transport_input.append(&mut self.input_queue);
        Ok(())
    }
    /// Bounded commit queue; return Backpressure instead of dropping/replaying.
    /// transport_input is a bounded stand-in drained by the caller, not a PTY.
    pub fn input(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if bytes.len()
            > 128_usize.saturating_sub(self.input_queue.len() + self.transport_input.len())
        {
            return Err(Error::Backpressure);
        }
        let queue = if self.pending.is_some() {
            &mut self.input_queue
        } else {
            &mut self.transport_input
        };
        if bytes.len() > 128_usize.saturating_sub(queue.len()) {
            return Err(Error::Backpressure);
        }
        queue.extend_from_slice(bytes);
        Ok(())
    }
    pub fn take_input(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.transport_input)
    }
    pub fn take_responses(&self) -> Vec<String> {
        std::mem::take(&mut *self.listener.0.lock().unwrap())
    }
    pub fn retained_frames(&self) -> usize {
        self.views.iter().filter(|a| a.flight.is_some()).count()
    }
    pub fn alternate_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }
}
