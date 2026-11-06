//! Bounded transport independent of windows, views and input adapters.
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};
use std::{
    collections::VecDeque,
    io::{Read, Write},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};
type Failure = Box<dyn std::error::Error>;
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
        let mut o = self.0.0.lock().unwrap();
        o.eof = true;
        o.error = error;
        self.0.1.notify_all();
    }
    pub fn drain(&self) -> (Vec<Vec<u8>>, bool, Option<String>) {
        let mut o = self.0.0.lock().unwrap();
        let chunks = o.chunks.drain(..).collect();
        o.bytes = 0;
        self.0.1.notify_all();
        (chunks, o.eof, o.error.clone())
    }
    pub fn stop(&self) {
        self.0.0.lock().unwrap().stopped = true;
        self.0.1.notify_all();
    }
}
pub struct Session {
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Box<dyn Write + Send>>,
    child: Box<dyn Child + Send + Sync>,
    reader: Option<JoinHandle<()>>,
    stopped: Arc<AtomicBool>,
    #[cfg(windows)]
    closer: Option<JoinHandle<()>>,
    pub output: OutputQueue,
    pub pid: u32,
    pub bytes_written: usize,
    pub resize_count: usize,
    pub exit_code: Option<u32>,
}
impl Session {
    pub fn spawn(command: CommandBuilder, cols: u16, rows: u16) -> Result<Self, Failure> {
        let pair = portable_pty::native_pty_system().openpty(PtySize {
            cols,
            rows,
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
            writer: Some(writer),
            child,
            reader: Some(reader),
            stopped,
            output,
            pid,
            bytes_written: 0,
            resize_count: 0,
            exit_code: None,
            #[cfg(windows)]
            closer: None,
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
            cols,
            rows,
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
            self.exit_code = self.child.try_wait()?.map(|s| s.exit_code());
        }
        #[cfg(windows)]
        if self.exit_code.is_some()
            && self.closer.is_none()
            && let Some(master) = self.master.take()
        {
            self.writer.take();
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
        #[cfg(unix)]
        {
            if let Some(fd) = self.master.as_ref().and_then(|m| m.as_raw_fd()) {
                let foreground = unsafe { libc::tcgetpgrp(fd) };
                if foreground > 1 && foreground != unsafe { libc::getpgrp() } {
                    unsafe {
                        libc::kill(-foreground, libc::SIGKILL);
                    }
                }
            }
            if let Some(group) = self.master.as_ref().and_then(|m| m.process_group_leader())
                && group > 1
            {
                unsafe {
                    libc::kill(-group, libc::SIGKILL);
                }
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
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
                while !reader.is_finished() {
                    unsafe {
                        windows_sys::Win32::System::IO::CancelSynchronousIo(reader.as_raw_handle());
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(unix)]
    fn real_shell_resize_and_shutdown_keep_pid_then_reap_child_and_reader() {
        let mut c = CommandBuilder::new("/bin/sh");
        c.args(["-c", "printf SHELL_READY; read value"]);
        let mut s = Session::spawn(c, 80, 24).unwrap();
        let pid = s.pid;
        s.resize(100, 30).unwrap();
        assert_eq!(s.child.process_id(), Some(pid));
        let size = s.master.as_ref().unwrap().get_size().unwrap();
        assert_eq!((size.cols, size.rows), (100, 30));
        s.shutdown().unwrap();
        assert!(s.reader.is_none());
        assert!(s.child.try_wait().unwrap().is_some());
        assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
        s.shutdown().unwrap();
    }
    #[test]
    fn oversized_output_is_rejected_instead_of_deadlocking() {
        assert!(!OutputQueue::default().push(vec![0; 256 * 1024 + 1]));
    }
    #[test]
    fn ordered_output_drains_before_eof_and_rejects_late_data() {
        let q = OutputQueue::default();
        assert!(q.push(b"one".to_vec()));
        assert!(q.push(b"two".to_vec()));
        q.finish(None);
        assert!(!q.push(b"late".to_vec()));
        let (chunks, eof, error) = q.drain();
        assert_eq!(chunks, [b"one".to_vec(), b"two".to_vec()]);
        assert!(eof);
        assert!(error.is_none());
        assert!(q.drain().0.is_empty());
    }
    #[test]
    fn stopped_queue_wakes_a_backpressured_producer() {
        let q = OutputQueue::default();
        assert!(q.push(vec![0; 256 * 1024]));
        let p = q.clone();
        let t = std::thread::spawn(move || p.push(vec![1]));
        q.stop();
        assert!(!t.join().unwrap());
    }
}
