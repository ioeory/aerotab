//! Opt-in Windows focus metadata. The UI thread only uses a bounded try_send;
//! file creation and writing happen on a dedicated thread, never in WndProc.
use serde::Serialize;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc::{sync_channel, SyncSender},
    OnceLock,
};
use std::time::Instant;
use uuid::Uuid;

const CAPACITY: usize = 2048;
const MAX_BYTES: usize = 8 * 1024 * 1024;
static SINK: OnceLock<Sink> = OnceLock::new();
static REQUEST: AtomicU64 = AtomicU64::new(1);
struct Sink {
    sender: SyncSender<Record>,
    started: Instant,
    sequence: AtomicU64,
    dropped: AtomicU64,
}
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    Requested,
    Executing,
    Result,
    MouseDown,
    MouseActivate,
    SetFocus,
    KillFocus,
    Update,
    ReleaseParent,
    ReleasedParent,
    PlacementResult,
    Destroy,
}
#[derive(Serialize)]
struct Record {
    seq: u64,
    monotonic_us: u128,
    dropped: u64,
    phase: Phase,
    session_id: Uuid,
    generation: Option<Uuid>,
    request: u64,
    thread: u32,
    hwnd: usize,
    parent: usize,
    focus: usize,
    foreground: usize,
    result: Option<isize>,
}
fn enabled(flag: Option<&str>, path: Option<&std::path::Path>) -> bool {
    flag == Some("1") && path.is_some_and(std::path::Path::is_absolute)
}

/// Call once during startup, before native windows are created. An explicit
/// absolute path and create_new prevent accidental replacement of user files.
pub fn init() {
    let path = std::env::var_os("AEROTAB_WINDOWS_FOCUS_LOG").map(std::path::PathBuf::from);
    if !enabled(
        std::env::var("AEROTAB_WINDOWS_FOCUS_DIAGNOSTICS")
            .ok()
            .as_deref(),
        path.as_deref(),
    ) {
        return;
    }
    let (sender, receiver) = sync_channel::<Record>(CAPACITY);
    let path = path.unwrap();
    let worker = std::thread::Builder::new().name("native-focus-log".into()).spawn(move || {
        use std::io::Write;
        let mut file = match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(_) => { eprintln!("Windows focus diagnostic sink unavailable (new file and existing parent required)"); return; }
        };
        eprintln!("Windows focus diagnostic sink: {} (8 MiB limit)", path.display());
        let mut written = 0;
        while let Ok(record) = receiver.recv() {
            let Ok(mut bytes) = serde_json::to_vec(&record) else { continue; };
            bytes.push(b'\n');
            if written + bytes.len() > MAX_BYTES { eprintln!("Windows focus diagnostic size limit reached"); break; }
            if file.write_all(&bytes).is_err() { break; }
            written += bytes.len();
        }
    });
    if worker.is_ok() {
        let _ = SINK.set(Sink {
            sender,
            started: Instant::now(),
            sequence: AtomicU64::new(1),
            dropped: AtomicU64::new(0),
        });
    }
}
pub fn is_enabled() -> bool {
    SINK.get().is_some()
}
pub fn next_request() -> u64 {
    if SINK.get().is_some() {
        REQUEST.fetch_add(1, Ordering::Relaxed)
    } else {
        0
    }
}
#[allow(clippy::too_many_arguments)]
pub fn record(
    phase: Phase,
    session_id: Uuid,
    generation: Option<Uuid>,
    hwnd: usize,
    parent: usize,
    request: u64,
    result: Option<isize>,
) {
    let Some(sink) = SINK.get() else {
        return;
    };
    #[cfg(target_os = "windows")]
    let (thread, focus, foreground) = unsafe {
        (
            windows_sys::Win32::System::Threading::GetCurrentThreadId(),
            windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus() as usize,
            windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() as usize,
        )
    };
    #[cfg(not(target_os = "windows"))]
    let (thread, focus, foreground) = (0, 0, 0);
    let record = Record {
        seq: sink.sequence.fetch_add(1, Ordering::Relaxed),
        monotonic_us: sink.started.elapsed().as_micros(),
        dropped: sink.dropped.load(Ordering::Relaxed),
        phase,
        session_id,
        generation,
        request,
        thread,
        hwnd,
        parent,
        focus,
        foreground,
        result,
    };
    if sink.sender.try_send(record).is_err() {
        sink.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_explicit_flag_and_absolute_path() {
        let absolute = std::env::temp_dir().join("focus.jsonl");
        assert!(!enabled(None, Some(&absolute)));
        assert!(!enabled(Some("true"), Some(&absolute)));
        assert!(!enabled(Some("1"), None));
        assert!(!enabled(
            Some("1"),
            Some(std::path::Path::new("relative.jsonl"))
        ));
        assert!(enabled(Some("1"), Some(&absolute)));
    }
    #[test]
    fn schema_is_metadata_only_and_phase_is_allowlisted() {
        let value = serde_json::to_value(Record {
            seq: 1,
            monotonic_us: 2,
            dropped: 0,
            phase: Phase::MouseActivate,
            session_id: Uuid::nil(),
            generation: None,
            request: 3,
            thread: 4,
            hwnd: 5,
            parent: 6,
            focus: 7,
            foreground: 8,
            result: Some(1),
        })
        .unwrap();
        let mut keys: Vec<_> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "dropped",
                "focus",
                "foreground",
                "generation",
                "hwnd",
                "monotonic_us",
                "parent",
                "phase",
                "request",
                "result",
                "seq",
                "session_id",
                "thread"
            ]
        );
        assert_eq!(value["phase"], "mouse-activate");
    }
    #[test]
    fn bounded_channel_never_waits_for_writer() {
        let (sender, _receiver) = sync_channel(1);
        sender.try_send(1).unwrap();
        assert!(matches!(
            sender.try_send(2),
            Err(std::sync::mpsc::TrySendError::Full(2))
        ));
    }
}
