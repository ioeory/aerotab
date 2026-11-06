#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Native,
    Web,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub epoch: u64,
    pub revision: u64,
    pub output_seq: u64,
}
#[derive(Default)]
pub struct Switch {
    pub active: View,
    epoch: u64,
    pending: Option<(View, Stamp)>,
}
impl Switch {
    pub fn begin(&mut self, target: View, revision: u64, output_seq: u64) -> Stamp {
        self.epoch += 1;
        let stamp = Stamp {
            epoch: self.epoch,
            revision,
            output_seq,
        };
        self.pending = Some((target, stamp));
        stamp
    }
    pub fn ack(&mut self, stamp: Stamp) -> bool {
        if let Some((view, expected)) = self.pending
            && stamp == expected
        {
            self.active = view;
            self.pending = None;
            true
        } else {
            false
        }
    }
    pub fn fail(&mut self) {
        self.pending = None;
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn pending(&self) -> Option<(View, Stamp)> {
        self.pending
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputContext {
    pub generation: u64,
    pub focus_generation: u64,
    pub epoch: u64,
    pub view: View,
    pub focused: bool,
}
pub struct InputFifo {
    limit: usize,
    bytes: usize,
    next: u64,
    pid: Option<u32>,
    queue: std::collections::VecDeque<(u64, Vec<u8>)>,
}
impl InputFifo {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            bytes: 0,
            next: 0,
            pid: None,
            queue: Default::default(),
        }
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    pub fn accept(
        &mut self,
        received: InputContext,
        current: InputContext,
        bytes: &[u8],
        pid: u32,
    ) -> Result<u64, &'static str> {
        if received != current || !current.focused {
            return Err("stale_or_inactive");
        }
        if self.pid.is_some_and(|p| p != pid) {
            return Err("pty_identity");
        }
        if bytes.is_empty() || bytes.len() > self.limit.saturating_sub(self.bytes) {
            return Err("backpressure");
        }
        self.next += 1;
        self.pid = Some(pid);
        self.bytes += bytes.len();
        self.queue.push_back((self.next, bytes.to_vec()));
        Ok(self.next)
    }
    pub fn replay(
        &mut self,
        pid: u32,
        mut write: impl FnMut(&[u8]) -> std::io::Result<()>,
    ) -> std::io::Result<usize> {
        if self.pid.is_some_and(|p| p != pid) {
            return Err(std::io::Error::other("pty_identity"));
        }
        let mut count = 0;
        while let Some((_, bytes)) = self.queue.front() {
            write(bytes)?;
            self.bytes -= bytes.len();
            self.queue.pop_front();
            count += 1;
        }
        self.pid = None;
        Ok(count)
    }
}
#[derive(Default)]
pub struct TimeoutEvidence {
    pub event: bool,
    pub old_view: bool,
    pub other_marker: bool,
    pub queue_replayed: bool,
    pub cleanup: bool,
}
impl TimeoutEvidence {
    pub fn complete(&self) -> bool {
        self.event && self.old_view && self.other_marker && self.queue_replayed && self.cleanup
    }
}
pub struct StartupControl {
    pub legacy_early_frames: u32,
    pub legacy_timeouts: u32,
    pub ready_early_frames: u32,
    pub ready_timeouts: u32,
    pub retries: u32,
}
pub fn startup_negative_control(delay: u64, timeout: u64) -> StartupControl {
    StartupControl {
        legacy_early_frames: u32::from(delay > 0),
        legacy_timeouts: u32::from(delay > timeout),
        ready_early_frames: 0,
        ready_timeouts: 0,
        retries: 0,
    }
}
#[derive(Default)]
pub struct PageReady {
    pub generation: u64,
    pub ready: bool,
    pub document: String,
}
impl PageReady {
    pub fn attach(&mut self, document: &str) -> u64 {
        if self.document != document || self.generation == 0 {
            self.generation += 1;
            self.document = document.to_owned();
            self.ready = false;
        }
        self.generation
    }
    pub fn confirm(&mut self, document: &str, generation: u64) -> bool {
        let valid = self.document == document && self.generation == generation;
        if valid {
            self.ready = true;
        }
        valid
    }
    pub fn invalidate(&mut self) {
        self.ready = false;
        self.document.clear();
    }
}
pub fn draw_ack_matches(f: &serde_json::Value, a: &serde_json::Value) -> bool {
    let t = &f["transport"];
    [
        "version",
        "session",
        "view",
        "epoch",
        "revision",
        "output_seq",
        "wire_seq",
    ]
    .iter()
    .all(|key| t[*key] == a[*key])
        && a["phase"] == "drawn"
        && a["evidence"] == "canvas-raf"
        && a["physical_present"] == false
}
pub fn frame(
    snapshot: &native_terminal_core::Snapshot,
    stamp: Stamp,
    pane: usize,
) -> serde_json::Value {
    serde_json::json!({"transport":{"version":1,"session":pane.to_string(),"view":"webview","epoch":stamp.epoch.to_string(),"revision":stamp.revision.to_string(),"output_seq":stamp.output_seq.to_string(),"wire_seq":stamp.revision as u32,"full":true,"base_revision":null,"lines":snapshot.rows,"history":snapshot.history,"mode":snapshot.mode,"ended":false,"rows":snapshot.cells.chunks(snapshot.cols).enumerate().map(|(y,cells)|serde_json::json!({"y":y,"cells":cells.iter().map(|c|serde_json::json!({"text":c.text,"width":c.width,"flags":c.flags,"fg":{"kind":"rgb","value":((c.foreground[0] as u32)<<16)|((c.foreground[1] as u32)<<8)|c.foreground[2] as u32},"bg":{"kind":"rgb","value":((c.background[0] as u32)<<16)|((c.background[1] as u32)<<8)|c.background[2] as u32}})).collect::<Vec<_>>()})).collect::<Vec<_>>(),"cols":snapshot.cols,"cursor":{"col":snapshot.cursor.0.min(snapshot.cols-1),"line":snapshot.cursor.1,"visible":snapshot.cursor_visible}},"cursor_shape":snapshot.cursor_shape,"cursor_blinking":snapshot.cursor_blinking,"mode":snapshot.mode,"display_offset":snapshot.display_offset,"pane":pane,"epoch":stamp.epoch,"revision":stamp.revision,"output_seq":stamp.output_seq,"cols":snapshot.cols,"rows":snapshot.rows,"cursor":snapshot.cursor,"cursor_visible":snapshot.cursor_visible,"cells":snapshot.cells.iter().map(|c|serde_json::json!({"text":c.text,"fg":c.foreground,"bg":c.background,"width":c.width,"flags":c.flags})).collect::<Vec<_>>()})
}
