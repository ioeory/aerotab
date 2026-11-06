use super::{HostError, ViewConfig};
use crate::terminal::engine::frame::*;
#[derive(Clone, Debug)]
pub struct Cell {
    pub text: String,
    pub foreground: [u8; 3],
    pub background: [u8; 3],
    pub flags: u16,
    pub width: usize,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub cells: Vec<Cell>,
    pub cols: usize,
    pub rows: usize,
    pub cursor: (usize, usize),
    pub cursor_visible: bool,
    pub cursor_shape: String,
    pub seq: u32,
}
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}
impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], HostError> {
        let end = self
            .at
            .checked_add(n)
            .filter(|n| *n <= self.data.len())
            .ok_or_else(|| HostError::new("invalid_frame", "truncated frame"))?;
        let b = &self.data[self.at..end];
        self.at = end;
        Ok(b)
    }
    fn u8(&mut self) -> Result<u8, HostError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, HostError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, HostError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
}
fn color(v: u32, cfg: &ViewConfig) -> [u8; 3] {
    let n = v & 0xffffff;
    match v >> 24 {
        2 => [(n >> 16) as u8, (n >> 8) as u8, n as u8],
        0 if n < 16 => cfg.palette[n as usize],
        0 if n == 257 => cfg.background,
        0 => cfg.foreground,
        1 if n < 16 => cfg.palette[n as usize],
        1 if n < 232 => {
            let n = n - 16;
            let levels = [0, 95, 135, 175, 215, 255];
            [
                levels[(n / 36) as usize],
                levels[(n / 6 % 6) as usize],
                levels[(n % 6) as usize],
            ]
        }
        1 => [8 + (n.min(255) - 232) as u8 * 10; 3],
        _ => cfg.foreground,
    }
}
impl Snapshot {
    pub fn decode(bytes: &[u8], cfg: &ViewConfig) -> Result<Self, HostError> {
        cfg.validate()?;
        let mut r = Reader { data: bytes, at: 0 };
        if r.u8()? != FRAME_VERSION {
            return Err(HostError::new("invalid_frame", "frame version"));
        }
        if r.u8()? != KIND_FULL {
            return Err(HostError::new(
                "unsupported_frame",
                "requires existing engine full snapshot; raw bypass is not a native frame",
            ));
        }
        let cols = r.u16()? as usize;
        let rows = r.u16()? as usize;
        if cols == 0 || rows == 0 || cols * rows > 1_000_000 {
            return Err(HostError::new("invalid_frame", "grid dimensions"));
        }
        let seq = r.u32()?;
        let flags = r.u32()?;
        let col = r.u16()? as usize;
        let line = r.u16()? as i16;
        let shape = r.u8()?;
        r.take(13)?;
        if flags & FLAG_TITLE != 0 {
            let n = r.u16()? as usize;
            r.take(n)?;
        }
        if flags & FLAG_CLIPBOARD != 0 {
            let n = r.u32()? as usize;
            r.take(n)?;
        }
        let count = r.u16()? as usize;
        if count != rows {
            return Err(HostError::new("invalid_frame", "incomplete full frame"));
        }
        let blank = Cell {
            text: " ".into(),
            foreground: cfg.foreground,
            background: cfg.background,
            flags: 0,
            width: 1,
        };
        let mut cells = vec![blank; cols * rows];
        let mut seen = vec![false; rows];
        for _ in 0..count {
            let y = r.u16()? as usize;
            if y >= rows || seen[y] {
                return Err(HostError::new("invalid_frame", "duplicate/outside row"));
            }
            seen[y] = true;
            let runs = r.u16()?;
            let mut x = 0;
            for _ in 0..runs {
                let n = r.u16()? as usize;
                let fg = color(r.u32()?, cfg);
                let bg = color(r.u32()?, cfg);
                let flags = r.u16()?;
                let kind = r.u8()?;
                if n == 0 || x + n > cols || kind > 1 {
                    return Err(HostError::new("invalid_frame", "invalid run"));
                }
                for _ in 0..n {
                    let c = if kind == 0 {
                        ' '
                    } else {
                        char::from_u32(r.u32()?)
                            .ok_or_else(|| HostError::new("invalid_frame", "invalid codepoint"))?
                    };
                    let mut foreground = fg;
                    let mut background = bg;
                    if flags & 1 != 0 {
                        std::mem::swap(&mut foreground, &mut background);
                    }
                    if flags & CELL_SELECTED != 0 {
                        background = [60, 85, 120];
                    }
                    cells[y * cols + x] = Cell {
                        text: if flags & 256 != 0 {
                            " ".into()
                        } else {
                            c.to_string()
                        },
                        foreground,
                        background,
                        flags,
                        width: if flags & 64 != 0 {
                            0
                        } else if flags & 32 != 0 {
                            2
                        } else {
                            1
                        },
                    };
                    x += 1;
                }
            }
            if x != cols {
                return Err(HostError::new("invalid_frame", "short row"));
            }
        }
        if r.at != bytes.len() {
            return Err(HostError::new("invalid_frame", "trailing data"));
        }
        Ok(Self {
            cells,
            cols,
            rows,
            cursor: (col, line.max(0) as usize),
            cursor_visible: shape != 3 && line >= 0 && (line as usize) < rows && col < cols,
            cursor_shape: match shape {
                0 => "Block",
                2 => "Beam",
                _ => "Underline",
            }
            .into(),
            seq,
        })
    }
}
