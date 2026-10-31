//! Binary frame format pushed from the Rust terminal engine to the webview.
//!
//! All integers are little-endian. Layout (byte offsets):
//!
//! ```text
//!  0  u8   version (= 1)
//!  1  u8   kind: 0 full | 1 partial | 2 activity | 3 raw bytes
//!  2  u16  cols
//!  4  u16  rows
//!  6  u32  seq
//! 10  u32  flags (FLAG_*)
//! 14  u16  cursor col
//! 16  i16  cursor line (viewport-relative; may be outside 0..rows)
//! 18  u8   cursor shape: 0 block | 1 underline | 2 beam | 3 hidden | 4 hollow
//! 19  u8   reserved
//! 20  u32  display offset (lines scrolled back)
//! 24  u32  history size (scrollback lines available)
//! 28  u32  terminal mode bits (alacritty `TermMode`)
//! 32  [u16 len + utf8]   title        (if FLAG_TITLE)
//!     [u32 len + bytes]  clipboard    (if FLAG_CLIPBOARD)
//!     kind 3: u32 len + raw bytes
//!     kind 0/1: u16 row_count, then rows:
//!        u16 y, u16 run_count, runs:
//!           u16 count, u32 fg, u32 bg, u16 cell_flags, u8 text_kind
//!           text_kind 0 = all blanks (no payload)
//!           text_kind 1 = `count` × u32 codepoint
//! ```
//!
//! Colours are `kind << 24 | value`: kind 0 = named (0–15 ANSI, 256 fg,
//! 257 bg, 258 cursor, 259–266 dim, 267 bright fg, 268 dim fg), kind 1 =
//! indexed 0–255, kind 2 = packed 0xRRGGBB. The palette lives in the
//! frontend so themes switch without a round-trip; OSC 4/10/11 overrides
//! are already resolved to kind 2 on the Rust side.

pub const FRAME_VERSION: u8 = 1;

pub const KIND_FULL: u8 = 0;
pub const KIND_PARTIAL: u8 = 1;
pub const KIND_ACTIVITY: u8 = 2;
pub const KIND_RAW: u8 = 3;

pub const FLAG_BELL: u32 = 1 << 0;
pub const FLAG_TITLE: u32 = 1 << 1;
pub const FLAG_CLIPBOARD: u32 = 1 << 2;
pub const FLAG_EXITED: u32 = 1 << 3;
pub const FLAG_ZMODEM_HINT: u32 = 1 << 4;
pub const FLAG_RAW_BYPASS: u32 = 1 << 5;
pub const FLAG_TRZSZ_DETECTED: u32 = 1 << 6;

/// Cell flag bit (outside alacritty's `Flags` range) marking a selected cell.
pub const CELL_SELECTED: u16 = 1 << 15;

pub const COLOR_NAMED: u32 = 0;
pub const COLOR_INDEXED: u32 = 1 << 24;
pub const COLOR_RGB: u32 = 2 << 24;

pub const HEADER_LEN: usize = 32;

/// Header fields shared by every frame kind.
#[derive(Debug, Clone, Default)]
pub struct FrameHeader {
    pub kind: u8,
    pub cols: u16,
    pub rows: u16,
    pub seq: u32,
    pub flags: u32,
    pub cursor_col: u16,
    pub cursor_line: i16,
    pub cursor_shape: u8,
    pub display_offset: u32,
    pub history_size: u32,
    pub mode: u32,
    pub title: Option<String>,
    pub clipboard: Option<Vec<u8>>,
}

/// One run of cells sharing fg / bg / flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub fg: u32,
    pub bg: u32,
    pub flags: u16,
    /// Codepoints; all `' '` ⇒ encoded as blank run without payload.
    pub chars: Vec<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct RowRuns {
    pub y: u16,
    pub runs: Vec<Run>,
}

pub struct FrameWriter {
    buf: Vec<u8>,
}

impl FrameWriter {
    pub fn new(header: &FrameHeader) -> Self {
        let mut buf = Vec::with_capacity(4096);
        buf.push(FRAME_VERSION);
        buf.push(header.kind);
        put_u16(&mut buf, header.cols);
        put_u16(&mut buf, header.rows);
        put_u32(&mut buf, header.seq);
        let mut flags = header.flags;
        if header.title.is_some() {
            flags |= FLAG_TITLE;
        }
        if header.clipboard.is_some() {
            flags |= FLAG_CLIPBOARD;
        }
        put_u32(&mut buf, flags);
        put_u16(&mut buf, header.cursor_col);
        put_u16(&mut buf, header.cursor_line as u16);
        buf.push(header.cursor_shape);
        buf.push(0);
        put_u32(&mut buf, header.display_offset);
        put_u32(&mut buf, header.history_size);
        put_u32(&mut buf, header.mode);
        debug_assert_eq!(buf.len(), HEADER_LEN);
        if let Some(t) = &header.title {
            let bytes = t.as_bytes();
            let n = bytes.len().min(u16::MAX as usize);
            put_u16(&mut buf, n as u16);
            buf.extend_from_slice(&bytes[..n]);
        }
        if let Some(c) = &header.clipboard {
            put_u32(&mut buf, c.len() as u32);
            buf.extend_from_slice(c);
        }
        Self { buf }
    }

    pub fn raw(mut self, bytes: &[u8]) -> Vec<u8> {
        put_u32(&mut self.buf, bytes.len() as u32);
        self.buf.extend_from_slice(bytes);
        self.buf
    }

    pub fn rows(mut self, rows: &[RowRuns]) -> Vec<u8> {
        put_u16(&mut self.buf, rows.len() as u16);
        for row in rows {
            put_u16(&mut self.buf, row.y);
            put_u16(&mut self.buf, row.runs.len() as u16);
            for run in &row.runs {
                put_u16(&mut self.buf, run.chars.len() as u16);
                put_u32(&mut self.buf, run.fg);
                put_u32(&mut self.buf, run.bg);
                put_u16(&mut self.buf, run.flags);
                if run.chars.iter().all(|&c| c == ' ' as u32) {
                    self.buf.push(0);
                } else {
                    self.buf.push(1);
                    for &c in &run.chars {
                        put_u32(&mut self.buf, c);
                    }
                }
            }
        }
        self.buf
    }

    pub fn finish(self) -> Vec<u8> {
        self.buf
    }
}

/// Builds runs for one row from per-cell tuples, merging adjacent cells with
/// identical attributes.
pub fn build_runs(cells: impl Iterator<Item = (u32, u32, u32, u16)>) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for (ch, fg, bg, flags) in cells {
        match runs.last_mut() {
            Some(last) if last.fg == fg && last.bg == bg && last.flags == flags => {
                last.chars.push(ch);
            }
            _ => runs.push(Run {
                fg,
                bg,
                flags,
                chars: vec![ch],
            }),
        }
    }
    runs
}

#[inline]
fn put_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_le_bytes());
}

#[inline]
fn put_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

/// Minimal reader used by tests and the CLI bench harness.
pub mod decode {
    use super::*;

    pub struct Decoded {
        pub header: FrameHeader,
        pub rows: Vec<RowRuns>,
        pub raw: Vec<u8>,
    }

    fn u16_at(b: &[u8], o: usize) -> u16 {
        u16::from_le_bytes([b[o], b[o + 1]])
    }
    fn u32_at(b: &[u8], o: usize) -> u32 {
        u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
    }

    pub fn decode(b: &[u8]) -> Decoded {
        assert_eq!(b[0], FRAME_VERSION);
        let mut h = FrameHeader {
            kind: b[1],
            cols: u16_at(b, 2),
            rows: u16_at(b, 4),
            seq: u32_at(b, 6),
            flags: u32_at(b, 10),
            cursor_col: u16_at(b, 14),
            cursor_line: u16_at(b, 16) as i16,
            cursor_shape: b[18],
            display_offset: u32_at(b, 20),
            history_size: u32_at(b, 24),
            mode: u32_at(b, 28),
            title: None,
            clipboard: None,
        };
        let mut o = HEADER_LEN;
        if h.flags & FLAG_TITLE != 0 {
            let n = u16_at(b, o) as usize;
            o += 2;
            h.title = Some(String::from_utf8_lossy(&b[o..o + n]).into_owned());
            o += n;
        }
        if h.flags & FLAG_CLIPBOARD != 0 {
            let n = u32_at(b, o) as usize;
            o += 4;
            h.clipboard = Some(b[o..o + n].to_vec());
            o += n;
        }
        let mut rows = Vec::new();
        let mut raw = Vec::new();
        match h.kind {
            KIND_RAW => {
                let n = u32_at(b, o) as usize;
                o += 4;
                raw = b[o..o + n].to_vec();
            }
            KIND_FULL | KIND_PARTIAL => {
                let row_count = u16_at(b, o) as usize;
                o += 2;
                for _ in 0..row_count {
                    let y = u16_at(b, o);
                    let run_count = u16_at(b, o + 2) as usize;
                    o += 4;
                    let mut runs = Vec::with_capacity(run_count);
                    for _ in 0..run_count {
                        let count = u16_at(b, o) as usize;
                        let fg = u32_at(b, o + 2);
                        let bg = u32_at(b, o + 6);
                        let flags = u16_at(b, o + 10);
                        let text_kind = b[o + 12];
                        o += 13;
                        let chars = if text_kind == 0 {
                            vec![' ' as u32; count]
                        } else {
                            let mut v = Vec::with_capacity(count);
                            for _ in 0..count {
                                v.push(u32_at(b, o));
                                o += 4;
                            }
                            v
                        };
                        runs.push(Run {
                            fg,
                            bg,
                            flags,
                            chars,
                        });
                    }
                    rows.push(RowRuns { y, runs });
                }
            }
            _ => {}
        }
        Decoded {
            header: h,
            rows,
            raw,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::decode::decode;
    use super::*;

    #[test]
    fn header_roundtrip_with_title_and_clipboard() {
        let h = FrameHeader {
            kind: KIND_ACTIVITY,
            cols: 80,
            rows: 24,
            seq: 7,
            flags: FLAG_BELL,
            cursor_col: 3,
            cursor_line: -2,
            cursor_shape: 2,
            display_offset: 5,
            history_size: 100,
            mode: 0x1234,
            title: Some("héllo".into()),
            clipboard: Some(b"clip".to_vec()),
        };
        let bytes = FrameWriter::new(&h).finish();
        let d = decode(&bytes);
        assert_eq!(d.header.kind, KIND_ACTIVITY);
        assert_eq!(d.header.cols, 80);
        assert_eq!(d.header.seq, 7);
        assert!(d.header.flags & FLAG_BELL != 0);
        assert_eq!(d.header.cursor_line, -2);
        assert_eq!(d.header.title.as_deref(), Some("héllo"));
        assert_eq!(d.header.clipboard.as_deref(), Some(&b"clip"[..]));
        assert_eq!(d.header.mode, 0x1234);
    }

    #[test]
    fn runs_merge_and_blank_runs_have_no_payload() {
        let cells = [
            (' ' as u32, 1, 2, 0),
            (' ' as u32, 1, 2, 0),
            ('a' as u32, 1, 2, 0),
            ('b' as u32, 9, 2, 0),
        ];
        let runs = build_runs(cells.into_iter());
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].chars, vec![' ' as u32, ' ' as u32, 'a' as u32]);
        let blank_only = build_runs([(' ' as u32, 1, 2, 0u16); 5].into_iter());
        let h = FrameHeader {
            kind: KIND_FULL,
            ..Default::default()
        };
        let bytes = FrameWriter::new(&h).rows(&[RowRuns {
            y: 0,
            runs: blank_only.clone(),
        }]);
        // header + row_count + row header + one run header (13 bytes), no chars.
        assert_eq!(bytes.len(), HEADER_LEN + 2 + 4 + 13);
        let d = decode(&bytes);
        assert_eq!(d.rows[0].runs, blank_only);
    }

    #[test]
    fn raw_frame_roundtrip() {
        let h = FrameHeader {
            kind: KIND_RAW,
            ..Default::default()
        };
        let bytes = FrameWriter::new(&h).raw(b"\x1b[31mhi");
        let d = decode(&bytes);
        assert_eq!(d.raw, b"\x1b[31mhi");
    }
}
