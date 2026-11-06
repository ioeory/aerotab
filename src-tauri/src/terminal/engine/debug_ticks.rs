//! Debug-only, explicitly armed fixed-marker observer. Never retains raw output.
use serde::Serialize;
#[derive(Default)]
pub struct Observer {
    session: Option<uuid::Uuid>,
    candidate: Vec<u8>,
    escape: u8,
    pub records: Vec<Record>,
}
#[derive(Clone, Serialize)]
pub struct Record {
    pub tick: u8,
    pub order: usize,
    pub before_us: u64,
    pub after_us: u64,
}
impl Observer {
    pub fn arm(&mut self, session: uuid::Uuid, enabled: bool) {
        *self = Self::default();
        if enabled {
            self.session = Some(session);
        }
    }
    pub fn observe(&mut self, bytes: &[u8], before_us: u64, after_us: u64) {
        if self.session.is_none() {
            return;
        }
        for &b in bytes {
            // Ignore ANSI control-string payloads too: marker-looking OSC/DCS
            // content is not terminal output text. No payload is retained.
            match self.escape {
                1 => {
                    self.escape = match b {
                        b'[' => 2,
                        b']' | b'P' | b'_' | b'^' => 3,
                        _ => 0,
                    };
                    continue;
                }
                2 => {
                    if (0x40..=0x7e).contains(&b) {
                        self.escape = 0;
                    }
                    continue;
                }
                3 => {
                    if b == 7 {
                        self.escape = 0;
                    } else if b == 27 {
                        self.escape = 4;
                    }
                    continue;
                }
                4 => {
                    self.escape = if b == b'\\' { 0 } else { 3 };
                    continue;
                }
                _ => {}
            }
            if b == 27 {
                self.candidate.clear();
                self.escape = 1;
                continue;
            }
            if b == b'T' {
                self.candidate.clear();
                self.candidate.push(b);
                continue;
            }
            if self.candidate.is_empty() {
                continue;
            }
            self.candidate.push(b);
            let n = self.candidate.len();
            let valid = if n <= 5 {
                b"TICK_".starts_with(&self.candidate)
            } else {
                let tail = &self.candidate[5..];
                let digits = tail.iter().take_while(|b| b.is_ascii_digit()).count();
                digits > 0
                    && digits <= 2
                    && (digits == tail.len()
                        || tail[digits..] == *b"\r"
                        || tail[digits..] == *b"\n"
                        || tail[digits..] == *b"\r\n")
            };
            if !valid || n > 9 {
                self.candidate.clear();
                continue;
            }
            if b == b'\n' {
                let digits = &self.candidate[5..];
                let tick = digits
                    .iter()
                    .take_while(|b| b.is_ascii_digit())
                    .fold(0u8, |v, b| v * 10 + b - b'0');
                if tick < 15 && self.records.len() < 64 {
                    self.records.push(Record {
                        tick,
                        order: self.records.len(),
                        before_us,
                        after_us,
                    });
                }
                self.candidate.clear();
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chunked_and_ansi() {
        let mut o = Observer::default();
        o.arm(uuid::Uuid::nil(), true);
        for b in b"\x1b[31mTICK_0\r\nTICK_1\nTICK_\x1b[0m2\nTICK_99\n\x1b]2;TICK_3\n\x07\x1bPTICK_4\n\x1b\\" {
            o.observe(&[*b], 1, 2);
        }
        assert_eq!(o.records.iter().map(|r| r.tick).collect::<Vec<_>>(), [0, 1]);
        assert!(o.candidate.len() <= 9);
    }
    #[test]
    fn disabled_records_nothing() {
        let mut o = Observer::default();
        o.arm(uuid::Uuid::nil(), false);
        o.observe(b"TICK_0\n", 1, 2);
        assert!(o.records.is_empty());
        assert!(o.candidate.is_empty());
    }
}
