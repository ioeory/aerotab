use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex},
};

#[derive(Default)]
struct Output {
    chunks: VecDeque<Vec<u8>>,
    bytes: usize,
    eof: bool,
    stopped: bool,
    error: Option<String>,
}

#[derive(Clone, Default)]
pub struct OutputQueue(Arc<(Mutex<Output>, Condvar)>);

impl OutputQueue {
    pub fn push(&self, bytes: Vec<u8>) -> bool {
        if bytes.len() > 256 * 1024 {
            return false;
        }
        let (lock, wake) = &*self.0;
        let mut output = lock.lock().unwrap();
        while output.bytes + bytes.len() > 256 * 1024 && !output.stopped && !output.eof {
            output = wake.wait(output).unwrap();
        }
        if output.stopped || output.eof {
            return false;
        }
        output.bytes += bytes.len();
        output.chunks.push_back(bytes);
        true
    }
    pub fn finish(&self, error: Option<String>) {
        let mut output = self.0.0.lock().unwrap();
        output.eof = true;
        output.error = error;
        self.0.1.notify_all();
    }
    pub fn drain(&self) -> (Vec<Vec<u8>>, bool, Option<String>) {
        let mut output = self.0.0.lock().unwrap();
        let chunks = output.chunks.drain(..).collect();
        output.bytes = 0;
        self.0.1.notify_all();
        (chunks, output.eof, output.error.clone())
    }
    pub fn stop(&self) {
        self.0.0.lock().unwrap().stopped = true;
        self.0.1.notify_all();
    }
}

pub fn grid_size(width: u32, height: u32) -> (u16, u16) {
    (
        ((width.saturating_sub(24) / 11).max(2)).min(u16::MAX as u32) as u16,
        ((height.saturating_sub(24) / 23).max(1)).min(u16::MAX as u32) as u16,
    )
}

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};
use std::{
    io::{Read, Write},
    sync::atomic::{AtomicBool, Ordering},
    thread::JoinHandle,
};

type Failure = Box<dyn std::error::Error>;

pub struct Session {
    master: Option<Box<dyn MasterPty + Send>>,
    #[cfg(windows)]
    closer: Option<JoinHandle<()>>,
    writer: Option<Box<dyn Write + Send>>,
    child: Box<dyn Child + Send + Sync>,
    reader: Option<JoinHandle<()>>,
    stopped: Arc<AtomicBool>,
    pub output: OutputQueue,
    pub pid: u32,
    pub bytes_written: usize,
    pub resize_count: usize,
    pub exit_code: Option<u32>,
}

impl Session {
    pub fn spawn(command: CommandBuilder, cols: u16, rows: u16) -> Result<Self, Failure> {
        let pair = portable_pty::native_pty_system().openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let mut input = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        let pid = child.process_id().ok_or("PTY child has no PID")?;
        let output = OutputQueue::default();
        let producer = output.clone();
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        #[cfg(unix)]
        let fd = pair.master.as_raw_fd().ok_or("PTY has no pollable fd")?;
        let reader = std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            while !stop.load(Ordering::Acquire) {
                #[cfg(unix)]
                {
                    let mut poll = libc::pollfd {
                        fd,
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    // The master stays alive until this thread has joined.
                    let ready = unsafe { libc::poll(&mut poll, 1, 50) };
                    if ready == 0 {
                        continue;
                    }
                    if ready < 0 {
                        let error = std::io::Error::last_os_error();
                        if error.kind() == std::io::ErrorKind::Interrupted {
                            continue;
                        }
                        producer.finish(Some(error.to_string()));
                        return;
                    }
                }
                match input.read(&mut buffer) {
                    Ok(0) => {
                        producer.finish(None);
                        return;
                    }
                    Ok(count) => {
                        if !producer.push(buffer[..count].to_vec()) {
                            return;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    // Linux PTY EIO after the final slave closes is an EOF, not lost output.
                    Err(error) if cfg!(unix) && error.raw_os_error() == Some(5) => {
                        producer.finish(None);
                        return;
                    }
                    Err(error) => {
                        producer.finish(Some(error.to_string()));
                        return;
                    }
                }
            }
        });
        Ok(Self {
            master: Some(pair.master),
            #[cfg(windows)]
            closer: None,
            writer: Some(writer),
            child,
            reader: Some(reader),
            stopped,
            output,
            pid,
            bytes_written: 0,
            resize_count: 0,
            exit_code: None,
        })
    }
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), Failure> {
        self.writer
            .as_mut()
            .ok_or("PTY writer closed")?
            .write_all(bytes)?;
        self.bytes_written += bytes.len();
        Ok(())
    }
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), Failure> {
        self.master.as_ref().ok_or("PTY closed")?.resize(PtySize {
            rows,
            cols,
            pixel_width: cols.saturating_mul(11),
            pixel_height: rows.saturating_mul(23),
        })?;
        let actual = self.master.as_ref().ok_or("PTY closed")?.get_size()?;
        if (actual.cols, actual.rows) != (cols, rows) {
            return Err("PTY resize readback differs".into());
        }
        self.resize_count += 1;
        Ok(())
    }
    pub fn poll_exit(&mut self) -> Result<(), Failure> {
        if self.exit_code.is_none() {
            self.exit_code = self.child.try_wait()?.map(|status| status.exit_code());
        }
        #[cfg(windows)]
        if self.exit_code.is_some()
            && self.closer.is_none()
            && let Some(master) = self.master.take()
        {
            self.writer.take();
            // Closing ConPTY is what ends its output pipe, even after the child exited.
            // Close on another thread while the reader continues draining to avoid deadlock.
            self.closer = Some(std::thread::spawn(move || drop(master)));
        }
        Ok(())
    }
    pub fn shutdown(&mut self) -> Result<(), Failure> {
        if self.reader.is_none() {
            return Ok(());
        }
        self.stopped.store(true, Ordering::Release);
        self.output.stop();
        // Also terminate a foreground job in a distinct job-control process group.
        #[cfg(unix)]
        if let Some(fd) = self.master.as_ref().and_then(|master| master.as_raw_fd()) {
            let foreground = unsafe { libc::tcgetpgrp(fd) };
            if foreground > 1 && foreground != unsafe { libc::getpgrp() } {
                unsafe {
                    libc::kill(-foreground, libc::SIGKILL);
                }
            }
        }
        // Kill the PTY session group as well as its direct child on Unix.
        #[cfg(unix)]
        if let Some(group) = self
            .master
            .as_ref()
            .and_then(|master| master.process_group_leader())
            && group > 1
        {
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
        }
        self.poll_exit()?;
        if self.exit_code.is_none() {
            self.child.kill()?;
            self.exit_code = Some(self.child.wait()?.exit_code());
        }
        self.writer.take();
        #[cfg(windows)]
        if self.closer.is_none()
            && let Some(master) = self.master.take()
        {
            self.closer = Some(std::thread::spawn(move || drop(master)));
        }
        if let Some(reader) = self.reader.take() {
            #[cfg(windows)]
            {
                use std::os::windows::io::AsRawHandle;
                use windows_sys::Win32::System::IO::CancelSynchronousIo;
                // Reissue cancellation to cover the race between the stop check and ReadFile.
                // Do not drop the ConPTY master before joining: ClosePseudoConsole can block.
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
                while !reader.is_finished() {
                    unsafe {
                        CancelSynchronousIo(reader.as_raw_handle());
                    }
                    if std::time::Instant::now() >= deadline {
                        self.reader = Some(reader);
                        return Err("Windows PTY reader cancellation exceeded deadline".into());
                    }
                    let lock = self.output.0.0.lock().unwrap();
                    let _ = self
                        .output
                        .0
                        .1
                        .wait_timeout(lock, std::time::Duration::from_millis(10))
                        .unwrap();
                }
            }
            reader.join().map_err(|_| "PTY reader panicked")?;
        }
        #[cfg(windows)]
        if let Some(closer) = self.closer.take() {
            closer.join().map_err(|_| "ConPTY closer panicked")?;
        }
        Ok(())
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

pub fn smoke_command() -> Result<CommandBuilder, Failure> {
    let mut command = CommandBuilder::new(std::env::current_exe()?);
    command.arg("--pty-child");
    Ok(command)
}

pub fn interactive_command() -> CommandBuilder {
    #[cfg(unix)]
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    #[cfg(windows)]
    let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into());
    let mut command = CommandBuilder::new(shell);
    #[cfg(windows)]
    command.arg("/d"); // Disable cmd AutoRun registry hooks in the standalone prototype.
    command.env("TERM", "xterm-256color");
    command
}

/// Controlled child executable: no shell profiles, terminal echo disabled, actual stdin roundtrip.
pub fn smoke_child() -> Result<(), Failure> {
    #[cfg(unix)]
    {
        let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
        if unsafe { libc::tcgetattr(0, termios.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut termios = unsafe { termios.assume_init() };
        unsafe {
            libc::cfmakeraw(&mut termios);
        }
        if unsafe { libc::tcsetattr(0, libc::TCSANOW, &termios) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::*;
        let input = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        let output = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
        // No echo/line editing; preserve VT input so replies arrive as escape bytes.
        if unsafe { SetConsoleMode(input, ENABLE_VIRTUAL_TERMINAL_INPUT) } == 0
            || unsafe {
                SetConsoleMode(
                    output,
                    ENABLE_PROCESSED_OUTPUT
                        | ENABLE_VIRTUAL_TERMINAL_PROCESSING
                        | DISABLE_NEWLINE_AUTO_RETURN,
                )
            } == 0
            || unsafe { SetConsoleCP(65001) } == 0
            || unsafe { SetConsoleOutputCP(65001) } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    {
        let mut stdout = std::io::stdout();
        let mut stdin = std::io::stdin();
        write!(stdout, "READY:{}\r\n", std::process::id())?;
        stdout.flush()?;
        let mut line = Vec::new();
        loop {
            let mut byte = [0];
            if stdin.read(&mut byte)? == 0 {
                break;
            }
            if byte[0] != b'\n' {
                line.push(byte[0]);
                continue;
            }
            match line.as_slice() {
                b"ROUNDTRIP_NATIVE_42" => {
                    write!(
                        stdout,
                        "ACK:ROUNDTRIP_NATIVE_42 \x1b[38;2;12;34;56m中文\x1b[0m\r\n\x1b[6n"
                    )?;
                    stdout.flush()?;
                    let mut reply = Vec::new();
                    while stdin.read(&mut byte)? == 1 {
                        reply.push(byte[0]);
                        if byte[0] == b'R' {
                            break;
                        }
                        if reply.len() > 64 {
                            return Err("oversized cursor reply".into());
                        }
                    }
                    if reply != b"\x1b[3;1R" {
                        return Err(format!("unexpected DSR reply: {reply:?}").into());
                    }
                    write!(stdout, "PROTOCOL_REPLY_OK\r\n")?;
                }
                b"RESIZE" => {
                    let (cols, rows) = child_console_size()?;
                    write!(
                        stdout,
                        "RESIZED:{}:{}:{}\r\n",
                        std::process::id(),
                        cols,
                        rows
                    )?;
                }
                bytes if bytes == "Native中🙂".as_bytes() => {
                    write!(stdout, "NATIVE_ACK:Native中🙂\r\n")?;
                }
                b"EXIT" => {
                    write!(stdout, "CHILD_DONE\r\n")?;
                    stdout.flush()?;
                    return Ok(());
                }
                _ => return Err("unexpected smoke input".into()),
            }
            line.clear();
            stdout.flush()?;
        }
        Ok(())
    }
}

fn child_console_size() -> Result<(u16, u16), Failure> {
    #[cfg(unix)]
    {
        let mut size = std::mem::MaybeUninit::<libc::winsize>::uninit();
        if unsafe { libc::ioctl(0, libc::TIOCGWINSZ, size.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let size = unsafe { size.assume_init() };
        Ok((size.ws_col, size.ws_row))
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::*;
        let mut info = std::mem::MaybeUninit::<CONSOLE_SCREEN_BUFFER_INFO>::uninit();
        if unsafe { GetConsoleScreenBufferInfo(GetStdHandle(STD_OUTPUT_HANDLE), info.as_mut_ptr()) }
            == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let info = unsafe { info.assume_init() };
        Ok((
            (info.srWindow.Right - info.srWindow.Left + 1) as u16,
            (info.srWindow.Bottom - info.srWindow.Top + 1) as u16,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn real_shell_resize_and_shutdown_keep_pid_then_reap_child_and_reader() {
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-c", "printf SHELL_READY; read value"]);
        let mut session = Session::spawn(command, 80, 24).unwrap();
        let pid = session.pid;
        session.resize(100, 30).unwrap();
        assert_eq!(session.child.process_id(), Some(pid));
        assert_eq!(
            (
                session.master.as_ref().unwrap().get_size().unwrap().cols,
                session.master.as_ref().unwrap().get_size().unwrap().rows
            ),
            (100, 30)
        );
        session.shutdown().unwrap();
        assert!(session.reader.is_none());
        assert!(session.child.try_wait().unwrap().is_some());
        assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
        session.shutdown().unwrap();
    }

    #[test]
    fn oversized_output_is_rejected_instead_of_deadlocking() {
        let queue = OutputQueue::default();
        assert!(!queue.push(vec![0; 256 * 1024 + 1]));
    }

    #[test]
    fn ordered_output_drains_before_eof_and_rejects_late_data() {
        let queue = OutputQueue::default();
        assert!(queue.push(b"one".to_vec()));
        assert!(queue.push(b"two".to_vec()));
        queue.finish(None);
        assert!(!queue.push(b"late".to_vec()));
        let (chunks, eof, error) = queue.drain();
        assert_eq!(chunks, [b"one".to_vec(), b"two".to_vec()]);
        assert!(eof);
        assert!(error.is_none());
        assert!(queue.drain().0.is_empty());
    }

    #[test]
    fn stopped_queue_wakes_a_backpressured_producer() {
        let queue = OutputQueue::default();
        assert!(queue.push(vec![0; 256 * 1024]));
        let producer = queue.clone();
        let thread = std::thread::spawn(move || producer.push(vec![1]));
        queue.stop();
        assert!(!thread.join().unwrap());
    }

    #[test]
    fn resize_matches_physical_grid_and_clamps_minimum() {
        assert_eq!(grid_size(1060, 560), (94, 23));
        assert_eq!(grid_size(904, 484), (80, 20));
        assert_eq!(grid_size(0, 0), (2, 1));
    }
}
