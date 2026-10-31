/**
 * Binary frame decoder for the Rust terminal engine
 * (see `src-tauri/src/terminal/engine/frame.rs` for the layout).
 *
 * Cell state is kept in flat typed arrays so a frame can be applied with
 * plain index arithmetic and the renderer can read it without allocation.
 */

export const FRAME_VERSION = 1;

export const KIND_FULL = 0;
export const KIND_PARTIAL = 1;
export const KIND_ACTIVITY = 2;
export const KIND_RAW = 3;

export const FLAG_BELL = 1 << 0;
export const FLAG_TITLE = 1 << 1;
export const FLAG_CLIPBOARD = 1 << 2;
export const FLAG_EXITED = 1 << 3;
export const FLAG_ZMODEM_HINT = 1 << 4;
export const FLAG_RAW_BYPASS = 1 << 5;
export const FLAG_TRZSZ_DETECTED = 1 << 6;

// alacritty_terminal cell flags (u16) + our SELECTED bit.
export const CF_INVERSE = 1 << 0;
export const CF_BOLD = 1 << 1;
export const CF_ITALIC = 1 << 2;
export const CF_UNDERLINE = 1 << 3;
export const CF_WRAPLINE = 1 << 4;
export const CF_WIDE_CHAR = 1 << 5;
export const CF_WIDE_CHAR_SPACER = 1 << 6;
export const CF_DIM = 1 << 7;
export const CF_HIDDEN = 1 << 8;
export const CF_STRIKEOUT = 1 << 9;
export const CF_LEADING_WIDE_CHAR_SPACER = 1 << 10;
export const CF_DOUBLE_UNDERLINE = 1 << 11;
export const CF_UNDERCURL = 1 << 12;
export const CF_DOTTED_UNDERLINE = 1 << 13;
export const CF_DASHED_UNDERLINE = 1 << 14;
export const CF_SELECTED = 1 << 15;
export const CF_ANY_UNDERLINE =
  CF_UNDERLINE | CF_DOUBLE_UNDERLINE | CF_UNDERCURL | CF_DOTTED_UNDERLINE | CF_DASHED_UNDERLINE;

// alacritty TermMode bits.
export const MODE_SHOW_CURSOR = 1;
export const MODE_APP_CURSOR = 1 << 1;
export const MODE_APP_KEYPAD = 1 << 2;
export const MODE_MOUSE_REPORT_CLICK = 1 << 3;
export const MODE_BRACKETED_PASTE = 1 << 4;
export const MODE_SGR_MOUSE = 1 << 5;
export const MODE_MOUSE_MOTION = 1 << 6;
export const MODE_LINE_WRAP = 1 << 7;
export const MODE_LINE_FEED_NEW_LINE = 1 << 8;
export const MODE_ORIGIN = 1 << 9;
export const MODE_INSERT = 1 << 10;
export const MODE_FOCUS_IN_OUT = 1 << 11;
export const MODE_ALT_SCREEN = 1 << 12;
export const MODE_MOUSE_DRAG = 1 << 13;
export const MODE_UTF8_MOUSE = 1 << 14;
export const MODE_ALTERNATE_SCROLL = 1 << 15;
export const MODE_VI = 1 << 16;
export const MODE_ANY_MOUSE = MODE_MOUSE_REPORT_CLICK | MODE_MOUSE_MOTION | MODE_MOUSE_DRAG;

export const CURSOR_BLOCK = 0;
export const CURSOR_UNDERLINE = 1;
export const CURSOR_BEAM = 2;
export const CURSOR_HIDDEN = 3;
export const CURSOR_HOLLOW = 4;

export const HEADER_LEN = 32;

export interface FrameHeader {
  kind: number;
  cols: number;
  rows: number;
  seq: number;
  flags: number;
  cursorCol: number;
  cursorLine: number;
  cursorShape: number;
  displayOffset: number;
  historySize: number;
  mode: number;
  title: string | null;
  clipboard: Uint8Array | null;
}

/** Flat cell storage for `cols × rows`. */
export class GridState {
  cols = 0;
  rows = 0;
  chars = new Uint32Array(0);
  fg = new Uint32Array(0);
  bg = new Uint32Array(0);
  flags = new Uint16Array(0);
  /** Rows touched since the renderer last consumed them. */
  dirtyRows = new Set<number>();
  allDirty = true;

  resize(cols: number, rows: number) {
    if (cols === this.cols && rows === this.rows) return;
    const n = Math.max(0, cols * rows);
    this.cols = cols;
    this.rows = rows;
    this.chars = new Uint32Array(n).fill(0x20);
    this.fg = new Uint32Array(n);
    this.bg = new Uint32Array(n).fill(0x101); // named background (kind 0, 257)
    this.flags = new Uint16Array(n);
    this.dirtyRows.clear();
    this.allDirty = true;
  }

  clearDirty() {
    this.dirtyRows.clear();
    this.allDirty = false;
  }

  index(col: number, row: number): number {
    return row * this.cols + col;
  }
}

export interface DecodedFrame {
  header: FrameHeader;
  /** Raw bytes for `KIND_RAW` frames. */
  raw: Uint8Array | null;
  /** Viewport rows whose cells were updated in this frame. */
  updatedRows: number[];
}

const textDecoder = new TextDecoder('utf-8');

/**
 * Parse one frame and apply its rows to `grid`. Returns the header plus
 * which rows changed. The grid is resized when the frame geometry differs.
 */
export function applyFrame(buf: ArrayBuffer, grid: GridState): DecodedFrame {
  const dv = new DataView(buf);
  const bytes = new Uint8Array(buf);
  if (dv.getUint8(0) !== FRAME_VERSION) {
    throw new Error(`termgrid: unsupported frame version ${dv.getUint8(0)}`);
  }
  const header: FrameHeader = {
    kind: dv.getUint8(1),
    cols: dv.getUint16(2, true),
    rows: dv.getUint16(4, true),
    seq: dv.getUint32(6, true),
    flags: dv.getUint32(10, true),
    cursorCol: dv.getUint16(14, true),
    cursorLine: dv.getInt16(16, true),
    cursorShape: dv.getUint8(18),
    displayOffset: dv.getUint32(20, true),
    historySize: dv.getUint32(24, true),
    mode: dv.getUint32(28, true),
    title: null,
    clipboard: null,
  };
  let o = HEADER_LEN;
  if (header.flags & FLAG_TITLE) {
    const n = dv.getUint16(o, true);
    o += 2;
    header.title = textDecoder.decode(bytes.subarray(o, o + n));
    o += n;
  }
  if (header.flags & FLAG_CLIPBOARD) {
    const n = dv.getUint32(o, true);
    o += 4;
    header.clipboard = bytes.slice(o, o + n);
    o += n;
  }
  const out: DecodedFrame = { header, raw: null, updatedRows: [] };
  if (header.kind === KIND_RAW) {
    const n = dv.getUint32(o, true);
    o += 4;
    out.raw = bytes.slice(o, o + n);
    return out;
  }
  if (header.kind === KIND_ACTIVITY) return out;

  if (header.cols !== grid.cols || header.rows !== grid.rows) {
    grid.resize(header.cols, header.rows);
  }
  if (header.kind === KIND_FULL) grid.allDirty = true;
  const cols = grid.cols;
  const rowCount = dv.getUint16(o, true);
  o += 2;
  for (let r = 0; r < rowCount; r++) {
    const y = dv.getUint16(o, true);
    const runCount = dv.getUint16(o + 2, true);
    o += 4;
    let x = 0;
    const base = y * cols;
    for (let ri = 0; ri < runCount; ri++) {
      const count = dv.getUint16(o, true);
      const fg = dv.getUint32(o + 2, true);
      const bg = dv.getUint32(o + 6, true);
      const flags = dv.getUint16(o + 10, true);
      const textKind = dv.getUint8(o + 12);
      o += 13;
      if (y < grid.rows) {
        const end = Math.min(x + count, cols);
        if (textKind === 0) {
          for (let i = x; i < end; i++) {
            const idx = base + i;
            grid.chars[idx] = 0x20;
            grid.fg[idx] = fg;
            grid.bg[idx] = bg;
            grid.flags[idx] = flags;
          }
        } else {
          for (let i = 0; i < count; i++) {
            const cp = dv.getUint32(o, true);
            o += 4;
            const cx = x + i;
            if (cx >= cols) continue;
            const idx = base + cx;
            grid.chars[idx] = cp;
            grid.fg[idx] = fg;
            grid.bg[idx] = bg;
            grid.flags[idx] = flags;
          }
        }
      } else if (textKind !== 0) {
        o += 4 * count;
      }
      x += count;
    }
    if (y < grid.rows) {
      grid.dirtyRows.add(y);
      out.updatedRows.push(y);
    }
  }
  return out;
}
