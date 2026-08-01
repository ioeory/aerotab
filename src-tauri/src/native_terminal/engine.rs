use std::io::{Read, Write};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use serde::Serialize;
use uuid::Uuid;
use vt100::Parser;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CellInfo {
    pub c: String,
    pub fg: String,
    pub bg: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub cursor: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineCells {
    pub row: usize,
    pub cells: Vec<CellInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenFrame {
    pub cols: usize,
    pub rows: usize,
    pub cursor_x: usize,
    pub cursor_y: usize,
    pub lines: Vec<LineCells>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineResult {
    pub engine_id: String,
}

pub struct NativeTerminalEngine {
    pub engine_id: String,
    parser: Arc<Mutex<Parser>>,
    master: Mutex<Box<dyn portable_pty::MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    close_tx: mpsc::Sender<()>,
    _reader_thread: Option<thread::JoinHandle<()>>,
}

impl NativeTerminalEngine {
    pub fn new_local(rows: u16, cols: u16, shell_cmd: Option<&[String]>) -> Result<Self, String> {
        let pty_system = NativePtySystem::default();
        let pty_size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pty_pair = pty_system
            .openpty(pty_size)
            .map_err(|e| format!("openpty: {e}"))?;

        let default_shell = if cfg!(windows) { "cmd.exe" } else { "/bin/sh" };
        let mut cmd =
            CommandBuilder::new(shell_cmd.map(|a| a[0].as_str()).unwrap_or(default_shell));
        if let Some(args) = shell_cmd {
            for a in &args[1..] {
                cmd.arg(a);
            }
        }
        #[cfg(not(windows))]
        if shell_cmd.is_none() {
            cmd.arg("-l");
        }
        crate::terminal::apply_terminal_env(&mut cmd);

        pty_pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("spawn: {e}"))?;

        let mut reader = pty_pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("try_clone_reader: {e}"))?;
        let writer = pty_pair
            .master
            .take_writer()
            .map_err(|e| format!("take_writer: {e}"))?;

        let parser = Arc::new(Mutex::new(Parser::new(rows, cols, 0)));
        let parser_clone = parser.clone();

        let (close_tx, close_rx) = mpsc::channel();

        let reader_thread = thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        parser_clone.lock().unwrap().process(&buf[..n]);
                    }
                    Err(_) => break,
                }
                if close_rx.try_recv().is_ok() {
                    break;
                }
            }
        });

        Ok(Self {
            engine_id: Uuid::new_v4().to_string(),
            parser,
            master: Mutex::new(pty_pair.master),
            writer: Mutex::new(writer),
            close_tx,
            _reader_thread: Some(reader_thread),
        })
    }

    pub fn write(&self, data: &[u8]) -> Result<(), String> {
        let mut w = self.writer.lock().unwrap();
        w.write_all(data).map_err(|e| format!("write: {e}"))
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<(), String> {
        let pty_size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        let master = self.master.lock().unwrap();
        master
            .resize(pty_size)
            .map_err(|e| format!("resize: {e}"))?;
        drop(master);
        let mut p = self.parser.lock().unwrap();
        p.screen_mut().set_size(rows, cols);
        Ok(())
    }

    pub fn snapshot(&self) -> ScreenFrame {
        let p = self.parser.lock().unwrap();
        let screen = p.screen();
        let (rows, cols) = screen.size();
        let (cursor_row, cursor_col) = screen.cursor_position();

        let mut lines = Vec::with_capacity(rows as usize);
        for row in 0..rows {
            let mut cells = Vec::with_capacity(cols as usize);
            for col in 0..cols {
                let is_cursor = row == cursor_row && col == cursor_col;
                match screen.cell(row, col) {
                    Some(cell) => cells.push(CellInfo {
                        c: cell.contents().to_string(),
                        fg: color_to_hex(cell.fgcolor()),
                        bg: color_to_hex(cell.bgcolor()),
                        bold: cell.bold(),
                        italic: cell.italic(),
                        underline: cell.underline(),
                        cursor: is_cursor,
                    }),
                    None => cells.push(CellInfo {
                        c: " ".into(),
                        fg: "#cccccc".into(),
                        bg: "#000000".into(),
                        bold: false,
                        italic: false,
                        underline: false,
                        cursor: is_cursor,
                    }),
                }
            }
            lines.push(LineCells {
                row: row as usize,
                cells,
            });
        }

        ScreenFrame {
            cols: cols as usize,
            rows: rows as usize,
            cursor_x: cursor_col as usize,
            cursor_y: cursor_row as usize,
            lines,
        }
    }

    pub fn close(&self) {
        let _ = self.close_tx.send(());
    }
}

impl Drop for NativeTerminalEngine {
    fn drop(&mut self) {
        let _ = self.close_tx.send(());
    }
}

fn color_to_hex(c: vt100::Color) -> String {
    match c {
        vt100::Color::Default => "#cccccc".into(),
        vt100::Color::Idx(i) => XTERM_COLORS
            .get(i as usize)
            .copied()
            .unwrap_or("#cccccc")
            .into(),
        vt100::Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
    }
}

const XTERM_COLORS: &[&str] = &[
    "#000000", "#800000", "#008000", "#808000", "#000080", "#800080", "#008080", "#c0c0c0",
    "#808080", "#ff0000", "#00ff00", "#ffff00", "#0000ff", "#ff00ff", "#00ffff", "#ffffff",
];

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

pub struct EngineRegistry {
    engines: Mutex<Vec<Arc<NativeTerminalEngine>>>,
}

impl Default for EngineRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineRegistry {
    pub fn new() -> Self {
        Self {
            engines: Mutex::new(Vec::new()),
        }
    }

    pub fn create_local(
        &self,
        rows: u16,
        cols: u16,
        shell_cmd: Option<&[String]>,
    ) -> Result<EngineResult, String> {
        let engine = NativeTerminalEngine::new_local(rows, cols, shell_cmd)?;
        let result = EngineResult {
            engine_id: engine.engine_id.clone(),
        };
        self.engines.lock().unwrap().push(Arc::new(engine));
        Ok(result)
    }

    pub fn write(&self, engine_id: &str, data: &[u8]) -> Result<(), String> {
        let guard = self.engines.lock().unwrap();
        let engine = guard
            .iter()
            .find(|e| e.engine_id == engine_id)
            .ok_or_else(|| "engine not found".to_string())?
            .clone();
        drop(guard);
        engine.write(data)
    }

    pub fn snapshot(&self, engine_id: &str) -> Result<ScreenFrame, String> {
        let guard = self.engines.lock().unwrap();
        let engine = guard
            .iter()
            .find(|e| e.engine_id == engine_id)
            .ok_or_else(|| "engine not found".to_string())?
            .clone();
        drop(guard);
        Ok(engine.snapshot())
    }

    pub fn resize(&self, engine_id: &str, rows: u16, cols: u16) -> Result<(), String> {
        let guard = self.engines.lock().unwrap();
        let engine = guard
            .iter()
            .find(|e| e.engine_id == engine_id)
            .ok_or_else(|| "engine not found".to_string())?
            .clone();
        drop(guard);
        engine.resize(rows, cols)
    }

    pub fn close(&self, engine_id: &str) -> bool {
        let mut guard = self.engines.lock().unwrap();
        let len_before = guard.len();
        guard.retain(|e| e.engine_id != engine_id);
        guard.len() < len_before
    }
}
