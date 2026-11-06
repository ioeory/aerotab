//! JSONL adapter for a real alacritty core and the existing broker. Test-only host;
//! headless parser ACK is real, presentation ACK is explicitly host-modeled.
use alacritty_terminal::{
    event::{Event as CoreEvent, EventListener},
    grid::Dimensions,
    index::{Column, Line, Point},
    term::{Config, Term, TermMode, cell::Flags, test::TermSize},
    vte::ansi::{Color, NamedColor, Processor},
};
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};
use std::sync::{Arc, Mutex};
use terminal_handoff_spike::{Broker, Consumer, Event, Record, Switch};

#[derive(Clone, Default)]
struct Listener(Arc<Mutex<Vec<String>>>, Arc<Mutex<Vec<String>>>);
impl EventListener for Listener {
    fn send_event(&self, event: CoreEvent) {
        match event {
            CoreEvent::PtyWrite(bytes) => self.0.lock().unwrap().push(bytes),
            CoreEvent::Title(title) => self.1.lock().unwrap().push(title),
            _ => {}
        }
    }
}
fn color(color: Color) -> Value {
    match color {
        Color::Spec(rgb) => json!([
            "rgb",
            (u32::from(rgb.r) << 16) | (u32::from(rgb.g) << 8) | u32::from(rgb.b)
        ]),
        Color::Indexed(n) => json!(["palette", n]),
        Color::Named(NamedColor::Foreground | NamedColor::Background) => json!(["default", 0]),
        Color::Named(n) => json!(["palette", n as usize]),
    }
}
fn snapshot(term: &Term<Listener>) -> Value {
    let grid = term.grid();
    let mut lines = Vec::new();
    let mut cells = Vec::new();
    for row in -(grid.history_size() as i32)..term.screen_lines() as i32 {
        let mut text = String::new();
        let mut row_cells = Vec::new();
        for col in 0..term.columns() {
            let cell = &grid[Point::new(Line(row), Column(col))];
            let width = if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                0
            } else if cell.flags.contains(Flags::WIDE_CHAR) {
                2
            } else {
                1
            };
            let mut chars = if width == 0 {
                String::new()
            } else {
                cell.c.to_string()
            };
            if let Some(extra) = cell.zerowidth() {
                chars.extend(extra);
            }
            text.push_str(&chars);
            if row >= 0 {
                let flags = [
                    Flags::BOLD,
                    Flags::ITALIC,
                    Flags::UNDERLINE,
                    Flags::INVERSE,
                    Flags::HIDDEN,
                    Flags::STRIKEOUT,
                    Flags::DIM,
                ]
                .map(|flag| cell.flags.contains(flag));
                row_cells.push(json!([chars, width, color(cell.fg), color(cell.bg), flags]));
            }
        }
        lines.push(text.trim_end_matches(' ').to_string());
        if row >= 0 {
            cells.push(row_cells);
        }
    }
    let mode = term.mode();
    json!({"type": if mode.contains(TermMode::ALT_SCREEN) {"alternate"} else {"normal"},
        "cursor": [grid.cursor.point.column.0, grid.cursor.point.line.0],
        "history": grid.history_size(), "lines": lines, "cells": cells,
        "modes": {"applicationCursorKeysMode": mode.contains(TermMode::APP_CURSOR),
            "applicationKeypadMode": mode.contains(TermMode::APP_KEYPAD),
            "bracketedPasteMode": mode.contains(TermMode::BRACKETED_PASTE),
            "insertMode": mode.contains(TermMode::INSERT), "originMode": mode.contains(TermMode::ORIGIN),
            "sendFocusMode": mode.contains(TermMode::FOCUS_IN_OUT), "wraparoundMode": mode.contains(TermMode::LINE_WRAP)},
        "cursorVisible": mode.contains(TermMode::SHOW_CURSOR)})
}
fn bytes(v: &Value) -> Vec<u8> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_u64().unwrap() as u8)
        .collect()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = Listener::default();
    let mut term = Term::new(
        Config {
            scrolling_history: 20_000,
            ..Config::default()
        },
        &TermSize::new(80, 24),
        listener.clone(),
    );
    let mut parser: Processor = Processor::new();
    let mut broker = Broker::new(2026, 4096);
    let mut inflight: Option<Record> = None;
    let mut switch: Option<Switch> = None;
    let mut pty = Vec::<String>::new();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for line in io::stdin().lock().lines() {
        let cmd: Value = serde_json::from_str(&line?)?;
        let result = match cmd["op"].as_str().unwrap() {
            "append" => {
                let event = if cmd.get("bytes").is_some() {
                    Event::Bytes(bytes(&cmd["bytes"]))
                } else if cmd.get("cols").is_some() {
                    Event::Resize {
                        cols: cmd["cols"].as_u64().unwrap() as u16,
                        rows: cmd["rows"].as_u64().unwrap() as u16,
                    }
                } else {
                    Event::Eof
                };
                match broker.try_append(event) {
                    Err((e, _)) => json!({"error": format!("{e:?}")}),
                    Ok(seq) => {
                        let record = broker.next(Consumer::Native)?.unwrap();
                        match record.event {
                            Event::Bytes(ref b) => parser.advance(&mut term, b),
                            Event::Resize { cols, rows } => {
                                term.resize(TermSize::new(cols as usize, rows as usize))
                            }
                            Event::Eof => {}
                        }
                        broker.ack(Consumer::Native, seq)?;
                        let responses = std::mem::take(&mut *listener.0.lock().unwrap());
                        let routed = broker.route_responses(
                            Consumer::Native,
                            seq,
                            responses.concat().into_bytes(),
                        )?;
                        if !routed.is_empty() {
                            pty.push(String::from_utf8(routed)?);
                        }
                        json!({"seq":seq})
                    }
                }
            }
            "next" => match broker.next(Consumer::ModeledXterm) {
                Ok(Some(record)) => {
                    let seq = record.seq;
                    inflight = Some(record);
                    json!({"seq":seq})
                }
                Ok(None) => json!({"empty":true}),
                Err(e) => json!({"error":format!("{e:?}")}),
            },
            "inflight" => match inflight.as_ref() {
                Some(Record {
                    seq,
                    event: Event::Bytes(b),
                }) => json!({"seq":seq,"bytes":b}),
                Some(Record {
                    seq,
                    event: Event::Resize { cols, rows },
                }) => json!({"seq":seq,"cols":cols,"rows":rows}),
                Some(Record {
                    seq,
                    event: Event::Eof,
                }) => json!({"seq":seq,"eof":true}),
                None => json!({"empty":true}),
            },
            "ack" => {
                let seq = cmd["seq"].as_u64().unwrap();
                let ack = broker.ack_decoded(
                    Consumer::ModeledXterm,
                    seq,
                    cmd["pendingUtf8"].as_u64().unwrap_or(0) as u8,
                );
                if ack.is_ok() {
                    inflight = None;
                }
                json!({"ok":ack.is_ok(),"error":ack.err().map(|e|format!("{e:?}"))})
            }
            "begin" => {
                switch = Some(broker.begin_switch(if cmd["native"] == true {
                    Consumer::Native
                } else {
                    Consumer::ModeledXterm
                })?);
                json!({"ok":true})
            }
            "commit" => {
                let result = broker.commit(switch.unwrap());
                json!({"ok":result.is_ok(),"error":result.err().map(|e|format!("{e:?}"))})
            }
            "host_frame_model" => {
                let result = broker.first_frame(switch.unwrap(), cmd["seq"].as_u64().unwrap());
                json!({"ok":result.is_ok()})
            }
            "secondary_response" => {
                let result = broker.route_responses(
                    Consumer::ModeledXterm,
                    cmd["seq"].as_u64().unwrap(),
                    bytes(&cmd["bytes"]),
                );
                json!({"rejected":result.is_err()})
            }
            "snapshot" => snapshot(&term),
            "status" => {
                json!({"session":broker.session_id(),"retained":broker.retained_bytes(),"active":format!("{:?}",broker.active()),"pty":pty,"titles":*listener.1.lock().unwrap(),"unicode":unicode_width::UNICODE_VERSION})
            }
            _ => json!({"error":"unknown command"}),
        };
        serde_json::to_writer(&mut out, &result)?;
        writeln!(out)?;
        out.flush()?;
    }
    Ok(())
}
