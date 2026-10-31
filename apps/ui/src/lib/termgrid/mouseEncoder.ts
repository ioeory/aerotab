/**
 * Encode pointer events into xterm-compatible mouse report sequences
 * (normal X10 / UTF-8 / SGR). Coordinates are 0-based cell indices.
 */

import {
  MODE_MOUSE_DRAG,
  MODE_MOUSE_MOTION,
  MODE_MOUSE_REPORT_CLICK,
  MODE_SGR_MOUSE,
  MODE_UTF8_MOUSE,
} from './frame';

export type MouseButton = 0 | 1 | 2 | 3 | 64 | 65;

export interface MouseEncodeOpts {
  mode: number;
  /** 0=left, 1=middle, 2=right, 3=release/none, 64/65=wheel up/down */
  button: MouseButton;
  col: number;
  row: number;
  pressed: boolean;
  shift?: boolean;
  meta?: boolean;
  ctrl?: boolean;
  /** Motion while button held (drag) or any-motion mode. */
  motion?: boolean;
}

/** True when the PTY wants mouse events instead of local selection. */
export function mouseReportingActive(mode: number): boolean {
  return (mode & (MODE_MOUSE_REPORT_CLICK | MODE_MOUSE_MOTION | MODE_MOUSE_DRAG)) !== 0;
}

export function encodeMouseEvent(opts: MouseEncodeOpts): string | null {
  const {
    mode,
    button,
    col,
    row,
    pressed,
    shift = false,
    meta = false,
    ctrl = false,
    motion = false,
  } = opts;

  if (!mouseReportingActive(mode)) return null;

  // Click-only mode: ignore pure motion / drag-without-button.
  if ((mode & MODE_MOUSE_REPORT_CLICK) !== 0
      && (mode & (MODE_MOUSE_MOTION | MODE_MOUSE_DRAG)) === 0
      && motion) {
    return null;
  }
  // Drag mode without motion: only report when a button is held or this is press/release.
  if ((mode & MODE_MOUSE_DRAG) !== 0
      && (mode & MODE_MOUSE_MOTION) === 0
      && motion
      && button === 3) {
    return null;
  }

  let b = button;
  if (motion && button < 3) b = (button + 32) as MouseButton;
  if (shift) b = (b + 4) as MouseButton;
  if (meta) b = (b + 8) as MouseButton;
  if (ctrl) b = (b + 16) as MouseButton;

  const x = Math.max(0, col) + 1;
  const y = Math.max(0, row) + 1;

  if ((mode & MODE_SGR_MOUSE) !== 0) {
    const final = pressed || motion ? 'M' : 'm';
    // SGR release keeps the original button code; button 3 is unused.
    const sb = !pressed && !motion ? (button === 3 ? 0 : button) : b;
    return `\x1b[<${sb};${x};${y}${final}`;
  }

  // Legacy / UTF-8 mouse: release is button 3.
  const legacyBtn = pressed || motion ? b : 3;
  const cx = x + 32;
  const cy = y + 32;
  const cb = legacyBtn + 32;
  if ((mode & MODE_UTF8_MOUSE) !== 0) {
    return `\x1b[M${String.fromCharCode(cb)}${encodeUtf8Coord(cx)}${encodeUtf8Coord(cy)}`;
  }
  // Clamp to 223 (255-32) for classic X10.
  if (cx > 255 || cy > 255) return null;
  return `\x1b[M${String.fromCharCode(cb)}${String.fromCharCode(Math.min(cx, 255))}${String.fromCharCode(Math.min(cy, 255))}`;
}

function encodeUtf8Coord(v: number): string {
  if (v < 128) return String.fromCharCode(v);
  // Two-byte UTF-8 for larger coordinates (xterm utf8 mouse).
  const lo = 0x80 | (v & 0x3f);
  const hi = 0xc0 | (v >> 6);
  return String.fromCharCode(hi, lo);
}

export function wheelButton(deltaY: number): MouseButton {
  return deltaY < 0 ? 64 : 65;
}
