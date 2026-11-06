use crate::{Terminal, pty::Session};
use std::io::{Read, Write};

pub fn grid_size(width: u32, height: u32) -> (u16, u16) {
    (
        ((width.saturating_sub(24) / 11).max(2)).min(u16::MAX as u32) as u16,
        ((height.saturating_sub(24) / 23).max(1)).min(u16::MAX as u32) as u16,
    )
}

pub struct NativeSession {
    pub terminal: Terminal,
    pub second: Option<Box<NativeSession>>,
    pub pty: Session,
    pub raw: Vec<u8>,
    pub replies: usize,
    pub blocked: usize,
    pub eof: bool,
    pub initial_pid: u32,
    pub synthetic_commits: usize,
    pub metrics: crate::load::Metrics,
    pub revision: u64,
    pub output_seq: u64,
}
impl NativeSession {
    pub fn spawn() -> Self {
        let mut first = Self::spawn_one();
        first.second = Some(Box::new(Self::spawn_one()));
        first
    }
    fn spawn_one() -> Self {
        let mut command = portable_pty::CommandBuilder::new(std::env::current_exe().unwrap());
        command.arg("--native-pty-child");
        let pty = Session::spawn(command, 80, 24).unwrap();
        let pid = pty.pid;
        Self {
            terminal: Terminal::new(80, 24),
            second: None,
            pty,
            raw: Vec::new(),
            replies: 0,
            blocked: 0,
            eof: false,
            initial_pid: pid,
            synthetic_commits: 0,
            metrics: crate::load::Metrics::default(),
            revision: 0,
            output_seq: 0,
        }
    }
    pub fn poll(&mut self) {
        if let Some(second) = &mut self.second {
            second.poll();
        }
        self.poll_local();
    }
    pub fn poll_local(&mut self) {
        let (chunks, eof, error) = self.pty.output.drain();
        assert!(error.is_none(), "{error:?}");
        for bytes in chunks {
            self.metrics.parsed(bytes.len());
            self.raw.extend(&bytes);
            self.terminal.feed(&bytes);
            self.revision += 1;
            self.output_seq += 1;
            for reply in self.terminal.take_replies() {
                self.replies += 1;
                self.pty.write(&reply).unwrap();
            }
        }
        self.eof = eof;
        self.pty.poll_exit().unwrap();
    }
    pub fn input(&mut self, text: &str, blocked: bool) {
        if blocked {
            self.blocked += 1;
            return;
        }
        self.pty.write(text.as_bytes()).unwrap();
    }
    pub fn resize(&mut self, index: usize, width: i32, height: i32) {
        self.pane_mut(index).revision += 1;
        let (cols, rows) = grid_size(width.max(0) as u32, height.max(0) as u32);
        if index == 0 {
            self.terminal.resize(cols as usize, rows as usize);
            self.pty.resize(cols, rows).unwrap();
        } else {
            let second = self.second.as_mut().unwrap();
            second.terminal.resize(cols as usize, rows as usize);
            second.pty.resize(cols, rows).unwrap();
        }
    }
    pub fn pane(&self, index: usize) -> &Self {
        if index == 0 {
            self
        } else {
            self.second.as_ref().unwrap()
        }
    }
    pub fn pane_mut(&mut self, index: usize) -> &mut Self {
        if index == 0 {
            self
        } else {
            self.second.as_mut().unwrap()
        }
    }
    pub fn contains(&self, text: &str) -> bool {
        String::from_utf8_lossy(&self.raw).contains(text)
    }
}

pub fn child() -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        let mut termios = std::mem::zeroed();
        if libc::tcgetattr(0, &mut termios) != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        libc::cfmakeraw(&mut termios);
        libc::tcsetattr(0, libc::TCSANOW, &termios);
    }
    let mut out = std::io::stdout();
    let mut input = std::io::stdin();
    write!(
        out,
        "\x1b[38;2;240;40;20mGPU GL glyph atlas\x1b[0m\r\nREADY:{}\r\n中文 e\u{301}\r\n\x1b[6n",
        std::process::id()
    )?;
    out.flush()?;
    let mut bytes = Vec::new();
    let mut b = [0u8];
    loop {
        if input.read(&mut b)? == 0 {
            return Err("EOF before DSR".into());
        }
        bytes.push(b[0]);
        if b[0] == b'R' {
            break;
        }
        if bytes.len() > 64 {
            return Err("oversized DSR".into());
        }
    }
    if bytes != b"\x1b[4;1R" {
        return Err(format!("wrong DSR {bytes:?}").into());
    }
    write!(out, "DSR_EXACTLY_ONCE\r\n")?;
    out.flush()?;
    bytes.clear();
    loop {
        if input.read(&mut b)? == 0 {
            break;
        }
        if b[0] != b'\n' {
            bytes.push(b[0]);
            continue;
        }
        match bytes.as_slice() {
            b"NATIVE_COMMIT" => write!(out, "COMMIT_OK\r\n")?,
            b"RESIZE" => {
                let mut size = std::mem::MaybeUninit::<libc::winsize>::uninit();
                if unsafe { libc::ioctl(0, libc::TIOCGWINSZ, size.as_mut_ptr()) } != 0 {
                    return Err("ioctl".into());
                }
                let size = unsafe { size.assume_init() };
                write!(
                    out,
                    "RESIZE_OK:{}:{}:{}\r\n",
                    std::process::id(),
                    size.ws_col,
                    size.ws_row
                )?;
            }
            b"PARTIAL" => {
                out.write_all(b"\xe4")?;
            }
            b"FINISH" => {
                out.write_all(b"\xb8\xad\x1b[38;2;")?;
            }
            b"STYLE" => {
                out.write_all(b"12;34;56mX\x1b[1;3;4mSTYLED\x1b[0m\r\n")?;
            }
            b"ALT" => {
                out.write_all(b"\x1b[?1049hALT SCREEN \xe4\xb8\xad e\xcc\x81\x1b[3;5H!")?;
            }
            b"MAIN" => {
                out.write_all(b"\x1b[?1049l")?;
            }
            command if command.starts_with(b"LOAD:") => {
                let command = std::str::from_utf8(command)?;
                let mut fields = command.split(':').skip(1);
                let pane = fields.next().ok_or("pane")?.parse()?;
                let round = fields.next().ok_or("round")?.parse()?;
                let payload = crate::load::payload(pane, round);
                out.write_all(&payload)?;
                write!(out, "PRODUCED:{pane}:{round}:{}\r\n", payload.len())?;
            }
            b"EXIT" => {
                write!(out, "CHILD_DONE\r\n")?;
                out.flush()?;
                return Ok(());
            }
            command if command.starts_with(b"PULSE:") => {
                write!(out, "{}\r\n", String::from_utf8_lossy(command))?
            }
            _ => return Err(format!("unexpected/duplicate input {bytes:?}").into()),
        }
        bytes.clear();
        out.flush()?;
    }
    Ok(())
}
