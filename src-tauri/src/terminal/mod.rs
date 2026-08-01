//! PTY-backed terminal channel.
//!
//! A [`PtyChannel`] owns a child process attached to a pseudo-terminal. The
//! reader thread runs blocking I/O off the async runtime and forwards bytes
//! into a [`tokio::sync::mpsc`] channel. Writes go in through an
//! [`std::io::Write`] handle protected by a [`std::sync::Mutex`].

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize as PortablePtySize};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PtySize {
    pub rows: u16,
    pub cols: u16,
}

impl Default for PtySize {
    fn default() -> Self {
        Self { rows: 24, cols: 80 }
    }
}

impl From<PtySize> for PortablePtySize {
    fn from(s: PtySize) -> Self {
        PortablePtySize {
            rows: s.rows,
            cols: s.cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("pty open: {0}")]
    Open(String),
    #[error("spawn: {0}")]
    Spawn(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("resize: {0}")]
    Resize(String),
    #[error("channel closed")]
    Closed,
}

pub struct PtyChannel {
    #[allow(dead_code)]
    master: Box<dyn MasterPty + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    output_rx: Option<mpsc::Receiver<Vec<u8>>>,
}

/// Build a shell [`CommandBuilder`], adding login/interactive flags when appropriate.
///
/// GUI apps on macOS inherit a minimal `PATH` from launchd. Terminal.app starts a
/// **login** shell so `/etc/zprofile` and `~/.zprofile` run (Homebrew, nvm, etc.).
/// AeroTab must do the same (`zsh -l`, `bash -l`, `fish --login`).
pub fn build_shell_command(program: &str, args: &[String]) -> CommandBuilder {
    let mut cmd = CommandBuilder::new(program);
    if let Some(flag) = login_shell_flag(program) {
        if should_prepend_login_flag(args) {
            cmd.arg(flag);
        }
    }
    for a in args {
        cmd.arg(a);
    }
    apply_terminal_env(&mut cmd);
    cmd
}

/// Terminal type advertised to local PTY children; matches the xterm.js frontend.
pub const DEFAULT_TERM: &str = "xterm-256color";

/// Set the terminal environment on a locally spawned PTY command.
///
/// A GUI process launched from Finder/launchd has no `TERM`, so children inherit
/// none and tools such as `clear`, `tput`, or `less` fail with
/// "TERM environment variable not set."
pub fn apply_terminal_env(cmd: &mut CommandBuilder) {
    cmd.env("TERM", DEFAULT_TERM);
    cmd.env("COLORTERM", "truecolor");
    cmd.env("TERM_PROGRAM", "AeroTab");
    cmd.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
}

#[cfg(unix)]
fn login_shell_flag(program: &str) -> Option<&'static str> {
    let base = std::path::Path::new(program)
        .file_name()
        .and_then(|s| s.to_str())?;
    match base {
        "zsh" | "bash" => Some("-l"),
        "fish" => Some("--login"),
        _ => None,
    }
}

#[cfg(not(unix))]
fn login_shell_flag(_program: &str) -> Option<&'static str> {
    None
}

#[cfg(unix)]
fn should_prepend_login_flag(args: &[String]) -> bool {
    if args.iter().any(|a| a == "-l" || a == "--login") {
        return false;
    }
    // One-shot / non-interactive invocations must not force login mode.
    if args
        .iter()
        .any(|a| a == "-c" || a == "--command" || a.starts_with("-c"))
    {
        return false;
    }
    true
}

#[cfg(not(unix))]
fn should_prepend_login_flag(_args: &[String]) -> bool {
    false
}

/// Merge macOS system paths via `path_helper` into this process environment.
///
/// Child PTYs inherit the parent env; fixing PATH here helps shells and tools
/// (git, ssh) even before login-shell startup files run.
#[cfg(target_os = "macos")]
pub fn prepare_process_environment() {
    use std::process::Command;

    let Ok(output) = Command::new("/usr/libexec/path_helper").arg("-s").output() else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    if let Some(path) = parse_path_helper_path(&text) {
        // SAFETY: called from main/setup before worker threads use env.
        unsafe {
            std::env::set_var("PATH", path);
        }
    }
}

/// Extract the `PATH` value from `path_helper -s` output.
///
/// The shell syntax is `PATH="a:b:c"; export PATH;`, so only the quoted span may
/// be kept. Leaking the trailing `"; export PATH` into `PATH` produces an
/// unbalanced quote that breaks `eval $(path_helper -s)` in `/etc/zprofile`.
#[cfg(target_os = "macos")]
fn parse_path_helper_path(output: &str) -> Option<String> {
    for line in output.lines() {
        let Some(rest) = line.trim().strip_prefix("PATH=") else {
            continue;
        };
        let value = if let Some(quoted) = rest.strip_prefix('"') {
            quoted.split('"').next().unwrap_or_default()
        } else {
            rest.split(';').next().unwrap_or_default()
        };
        let value = value.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
pub fn prepare_process_environment() {}

impl PtyChannel {
    pub fn spawn_default_shell(size: PtySize) -> Result<Self, PtyError> {
        let program = default_shell();
        let cmd = build_shell_command(&program, &[]);
        Self::spawn(cmd, size)
    }

    pub fn spawn(cmd: CommandBuilder, size: PtySize) -> Result<Self, PtyError> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(size.into())
            .map_err(|e| PtyError::Open(e.to_string()))?;
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| PtyError::Spawn(e.to_string()))?;
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::Open(e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| PtyError::Open(e.to_string()))?;

        let (tx, rx) = mpsc::channel::<Vec<u8>>(64);
        std::thread::Builder::new()
            .name("pty-reader".into())
            .spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if tx.blocking_send(buf[..n].to_vec()).is_err() {
                                break;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
            })
            .map_err(PtyError::Io)?;

        Ok(Self {
            master: pair.master,
            writer: Arc::new(Mutex::new(writer)),
            child,
            output_rx: Some(rx),
        })
    }

    pub fn take_output(&mut self) -> Option<mpsc::Receiver<Vec<u8>>> {
        self.output_rx.take()
    }

    pub fn write(&self, data: &[u8]) -> Result<(), PtyError> {
        let mut w = self.writer.lock().map_err(|_| PtyError::Closed)?;
        w.write_all(data)?;
        w.flush()?;
        Ok(())
    }

    pub fn resize(&self, size: PtySize) -> Result<(), PtyError> {
        self.master
            .resize(size.into())
            .map_err(|e| PtyError::Resize(e.to_string()))
    }

    pub fn kill(&mut self) -> Result<(), PtyError> {
        self.child
            .kill()
            .map_err(|e| PtyError::Spawn(e.to_string()))
    }

    pub fn try_wait(&mut self) -> Result<Option<portable_pty::ExitStatus>, PtyError> {
        self.child
            .try_wait()
            .map_err(|e| PtyError::Spawn(e.to_string()))
    }
}

#[cfg(unix)]
fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
}

#[cfg(windows)]
fn default_shell() -> String {
    std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into())
}

pub async fn init() -> crate::Result<()> {
    prepare_process_environment();
    tracing::debug!("terminal::init");
    Ok(())
}

#[cfg(test)]
mod login_shell_tests {
    use super::*;

    #[test]
    fn zsh_and_bash_use_dash_l() {
        assert_eq!(login_shell_flag("/bin/zsh"), Some("-l"));
        assert_eq!(login_shell_flag("/opt/homebrew/bin/bash"), Some("-l"));
    }

    #[test]
    fn fish_uses_login_long_flag() {
        assert_eq!(login_shell_flag("/usr/bin/fish"), Some("--login"));
    }

    #[test]
    fn sh_has_no_login_flag() {
        assert_eq!(login_shell_flag("/bin/sh"), None);
    }

    #[test]
    fn skips_duplicate_or_script_args() {
        assert!(!should_prepend_login_flag(&["-l".into()]));
        assert!(!should_prepend_login_flag(&["--login".into()]));
        assert!(!should_prepend_login_flag(&["-c".into(), "echo".into()]));
    }

    #[test]
    fn build_shell_command_sets_term() {
        let cmd = build_shell_command("/bin/zsh", &[]);
        let term = cmd.get_env("TERM").and_then(|v| v.to_str());
        assert_eq!(term, Some(DEFAULT_TERM));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parses_quoted_path_helper_output() {
        let out = concat!(
            "PATH=\"/usr/local/bin:/usr/bin:/bin\"; export PATH;\n",
            "MANPATH=\"/usr/share/man\"; export MANPATH;\n"
        );
        assert_eq!(
            parse_path_helper_path(out).as_deref(),
            Some("/usr/local/bin:/usr/bin:/bin")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parses_unquoted_path_helper_output() {
        assert_eq!(
            parse_path_helper_path("PATH=/usr/bin:/bin; export PATH;").as_deref(),
            Some("/usr/bin:/bin")
        );
        assert_eq!(parse_path_helper_path("MANPATH=\"/x\";"), None);
    }

    #[test]
    fn build_shell_command_inserts_login_once() {
        let cmd = build_shell_command("/bin/zsh", &[]);
        let args: Vec<String> = cmd
            .get_argv()
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert!(args.windows(2).any(|w| w == ["/bin/zsh", "-l"]));
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn spawn_echo_and_read() {
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.args(["-c", "printf hello-tabby"]);
        let mut ch = PtyChannel::spawn(cmd, PtySize::default()).unwrap();
        let mut rx = ch.take_output().unwrap();
        let mut collected = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            tokio::select! {
                v = rx.recv() => match v {
                    Some(chunk) => collected.extend_from_slice(&chunk),
                    None => break,
                },
                _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {
                    if std::time::Instant::now() >= deadline { break; }
                }
            }
        }
        let s = String::from_utf8_lossy(&collected);
        assert!(s.contains("hello-tabby"), "got {s:?}");
    }

    #[test]
    fn resize_does_not_error() {
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.args(["-c", "sleep 0.2"]);
        let ch = PtyChannel::spawn(cmd, PtySize::default()).unwrap();
        ch.resize(PtySize {
            rows: 40,
            cols: 120,
        })
        .unwrap();
    }
}
