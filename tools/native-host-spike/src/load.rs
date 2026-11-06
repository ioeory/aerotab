use std::time::Instant;
pub const ROUNDS: usize = 12;
pub fn payload(pane: usize, round: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    for line in 0..2048 {
        bytes.extend_from_slice(format!("\x1b[38;2;240;40;20mLOAD:{pane}:{round}:{line:04} ASCII 中文 e\u{301} {}\x1b[0m\r\n","abcdefghijklmnopqrstuvwxyz0123456789".repeat(3)).as_bytes());
    }
    bytes.extend_from_slice(format!("END:{pane}:{round}\r\n").as_bytes());
    bytes
}
pub fn verify(trace: &[u8], pane: usize, round: usize) -> bool {
    let expected = payload(pane, round);
    trace.windows(expected.len()).any(|w| w == expected)
}
pub fn percentile(values: &[u64], percent: usize) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    Some(sorted[(sorted.len() * percent.clamp(1, 100)).div_ceil(100) - 1])
}
#[derive(Default)]
pub struct Metrics {
    pub frame_us: Vec<u64>,
    pub update_us: Vec<u64>,
    pub pending: Option<Instant>,
    pub parsed: usize,
}
impl Metrics {
    pub fn parsed(&mut self, bytes: usize) {
        self.parsed += bytes;
        self.pending.get_or_insert_with(Instant::now);
    }
    pub fn frame(&mut self, start: Instant) {
        self.frame_us.push(start.elapsed().as_micros() as u64);
        if let Some(pending) = self.pending.take() {
            self.update_us.push(pending.elapsed().as_micros() as u64);
        }
    }
    pub fn summary(&self) -> serde_json::Value {
        let q = |v: &[u64]| serde_json::json!({"samples":v.len(),"p50_us":percentile(v,50),"p95_us":percentile(v,95),"p99_us":percentile(v,99)});
        serde_json::json!({"parsed_bytes":self.parsed,"frames":self.frame_us.len(),"draw_callback_elapsed":q(&self.frame_us),"parse_start_to_next_draw_complete":q(&self.update_us),"input_to_photon_latency_measured":false,"gpu_present_latency_measured":false})
    }
}
pub fn usage() -> serde_json::Value {
    let mut r = std::mem::MaybeUninit::<libc::rusage>::uninit();
    assert_eq!(
        unsafe { libc::getrusage(libc::RUSAGE_SELF, r.as_mut_ptr()) },
        0
    );
    let r = unsafe { r.assume_init() };
    serde_json::json!({"self_user_cpu_s":r.ru_utime.tv_sec as f64+r.ru_utime.tv_usec as f64/1e6,"self_system_cpu_s":r.ru_stime.tv_sec as f64+r.ru_stime.tv_usec as f64/1e6,"self_maxrss_kib_linux":r.ru_maxrss,"scope":"host process only; excludes WebKit subprocesses and PTY children"})
}
