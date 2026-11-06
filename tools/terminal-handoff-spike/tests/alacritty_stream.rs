//! Same-core streaming checks only. Secondary role here uses a SECOND ALACRITTY
//! instance as an explicitly modeled consumer; this is NOT real xterm verification.
use alacritty_terminal::{
    event::{Event as CoreEvent, EventListener},
    grid::Dimensions,
    index::{Column, Line, Point},
    term::{Config, Term, TermMode, test::TermSize},
    vte::ansi::Processor,
};
use std::sync::{Arc, Mutex};
use terminal_handoff_spike::{Broker, Consumer, Error, Event};

#[derive(Clone, Default)]
struct Listener(Arc<Mutex<Vec<String>>>);
impl EventListener for Listener {
    fn send_event(&self, event: CoreEvent) {
        if let CoreEvent::PtyWrite(bytes) = event {
            self.0.lock().unwrap().push(bytes);
        }
    }
}
struct Core {
    term: Term<Listener>,
    parser: Processor,
    events: Listener,
    eof: bool,
}
impl Core {
    fn new() -> Self {
        let events = Listener::default();
        let config = Config {
            scrolling_history: 20_000,
            ..Config::default()
        };
        Self {
            term: Term::new(config, &TermSize::new(80, 24), events.clone()),
            parser: Processor::new(),
            events,
            eof: false,
        }
    }
    fn apply(&mut self, event: &Event) {
        assert!(!self.eof);
        match event {
            Event::Bytes(bytes) => self.parser.advance(&mut self.term, bytes),
            Event::Resize { cols, rows } => self
                .term
                .resize(TermSize::new(*cols as usize, *rows as usize)),
            Event::Eof => self.eof = true,
        }
    }
    fn state(&self) -> String {
        let mut state = format!(
            "{:?}|{:?}|{:?}|{}",
            self.term.grid().cursor,
            self.term.mode(),
            self.term.grid().history_size(),
            self.eof
        );
        for index in 0..269 {
            state.push_str(&format!("{:?}", self.term.colors()[index]));
        }
        for line in -(self.term.grid().history_size() as i32)..self.term.screen_lines() as i32 {
            for col in 0..self.term.columns() {
                state.push_str(&format!(
                    "{:?}",
                    self.term.grid()[Point::new(Line(line), Column(col))]
                ));
            }
        }
        state
    }
}
fn consume(b: &mut Broker, role: Consumer, core: &mut Core) {
    while let Some(record) = b.next(role).unwrap() {
        core.apply(&record.event);
        b.ack(role, record.seq).unwrap();
    }
}
fn publish(b: &mut Broker, mut event: Event, native: &mut Core, modeled: &mut Core) {
    loop {
        match b.try_append(event) {
            Ok(_) => return,
            Err((Error::Backpressure, retry)) => {
                consume(b, Consumer::Native, native);
                consume(b, Consumer::ModeledXterm, modeled);
                event = retry;
            }
            other => panic!("unexpected append: {other:?}"),
        }
    }
}

#[test]
fn split_utf8_csi_osc_and_alternate_screen_preserve_persistent_parser_state() {
    let bytes = "main\r\n中e\u{301}\x1b[38;2;11;22;33mR\x1b]4;1;rgb:12/34/56\x1b\\\x1b[?1049hvim\x1b[2;5H中\x1b[?25l\x1b[6n".as_bytes();
    let mut reference = Core::new();
    reference.apply(&Event::Bytes(bytes.to_vec()));
    for split in 0..=bytes.len() {
        let mut b = Broker::new(12, 1024);
        let (mut native, mut modeled) = (Core::new(), Core::new());
        let mut forwarded = vec![];
        for (index, part) in [&bytes[..split], &bytes[split..]].into_iter().enumerate() {
            publish(
                &mut b,
                Event::Bytes(part.to_vec()),
                &mut native,
                &mut modeled,
            );
            consume(&mut b, Consumer::Native, &mut native);
            consume(&mut b, Consumer::ModeledXterm, &mut modeled);
            let responses = std::mem::take(&mut *native.events.0.lock().unwrap()).concat();
            let routed = b
                .route_responses(Consumer::Native, index as u64 + 1, responses.into_bytes())
                .unwrap();
            forwarded.extend(routed);
            let token = b.begin_switch(Consumer::ModeledXterm).unwrap();
            b.first_frame(token, index as u64 + 1).unwrap();
            b.commit(token).unwrap();
            // The frame acknowledgement is modeled, not an actual GPU presentation.
        }
        assert_eq!(native.state(), reference.state(), "native split {split}");
        assert_eq!(
            modeled.state(),
            reference.state(),
            "MODELED secondary split {split}"
        );
        assert!(native.term.mode().contains(TermMode::ALT_SCREEN));
        assert!(!native.term.mode().contains(TermMode::SHOW_CURSOR));
        assert!(
            !forwarded.is_empty(),
            "actual alacritty DSR response required"
        );
        assert_eq!(
            forwarded,
            reference.events.0.lock().unwrap().concat().into_bytes()
        );
        assert_eq!(
            b.route_responses(
                Consumer::ModeledXterm,
                2,
                modeled.events.0.lock().unwrap().concat().into_bytes()
            ),
            Err(Error::NotAuthority)
        );
        // Persistent parsers retain the saved primary screen through return.
        let leave = Event::Bytes(b"\x1b[?1049l\x1b[?25h".to_vec());
        reference.apply(&leave);
        publish(&mut b, leave, &mut native, &mut modeled);
        consume(&mut b, Consumer::Native, &mut native);
        consume(&mut b, Consumer::ModeledXterm, &mut modeled);
        assert_eq!(native.state(), reference.state());
        assert_eq!(modeled.state(), reference.state());
        // Reset reference for the next independent split.
        reference = Core::new();
        reference.apply(&Event::Bytes(bytes.to_vec()));
    }
}

#[test]
fn more_than_256kib_survives_small_log_resize_switches_and_eof() {
    let mut b = Broker::new(2026, 4096);
    let (mut native, mut modeled, mut reference) = (Core::new(), Core::new(), Core::new());
    let mut bytes = 0;
    for n in 0..4000 {
        let event =
            Event::Bytes(format!("line-{n:05}: {}\r\n", "0123456789".repeat(8)).into_bytes());
        if let Event::Bytes(chunk) = &event {
            bytes += chunk.len();
        }
        reference.apply(&event);
        publish(&mut b, event, &mut native, &mut modeled);
        assert!(b.retained_bytes() <= 4096);
        if n == 1500 || n == 3000 {
            let resize = Event::Resize { cols: 93, rows: 30 };
            reference.apply(&resize);
            publish(&mut b, resize, &mut native, &mut modeled);
            consume(&mut b, Consumer::Native, &mut native);
            consume(&mut b, Consumer::ModeledXterm, &mut modeled);
            let target = if n == 1500 {
                Consumer::ModeledXterm
            } else {
                Consumer::Native
            };
            let switch = b.begin_switch(target).unwrap();
            let seq = if n == 1500 { 1502 } else { 3003 };
            b.first_frame(switch, seq).unwrap();
            b.commit(switch).unwrap();
            assert_eq!(b.session_id(), 2026);
        }
    }
    assert!(bytes > 256 * 1024, "actual streamed output: {bytes}");
    reference.apply(&Event::Eof);
    publish(&mut b, Event::Eof, &mut native, &mut modeled);
    consume(&mut b, Consumer::Native, &mut native);
    consume(&mut b, Consumer::ModeledXterm, &mut modeled);
    assert_eq!(b.retained_bytes(), 0);
    assert!(native.term.grid().history_size() > 3900);
    assert_eq!(native.state(), reference.state());
    assert_eq!(modeled.state(), reference.state());
    println!(
        "streamed_bytes={bytes}; budget=4096; secondary=modeled_alacritty_not_xterm; history_lines={}",
        native.term.grid().history_size()
    );
}
