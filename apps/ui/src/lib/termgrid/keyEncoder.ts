/**
 * Keyboard → PTY bytes. Mirrors xterm.js application-cursor / keypad /
 * bracketed-paste basics so SSH apps see familiar sequences.
 */

import {
  MODE_APP_CURSOR,
  MODE_APP_KEYPAD,
  MODE_BRACKETED_PASTE,
} from './frame';

export interface KeyEncodeOptions {
  mode: number;
  /** Treat Alt as Meta (ESC-prefix) — Tabby `altIsMeta`. */
  altIsMeta: boolean;
}

const APP_CURSOR: Record<string, string> = {
  ArrowUp: '\x1bOA',
  ArrowDown: '\x1bOB',
  ArrowRight: '\x1bOC',
  ArrowLeft: '\x1bOD',
};

const NORM_CURSOR: Record<string, string> = {
  ArrowUp: '\x1b[A',
  ArrowDown: '\x1b[B',
  ArrowRight: '\x1b[C',
  ArrowLeft: '\x1b[D',
};

const APP_KEYPAD: Record<string, string> = {
  Numpad0: '\x1bOp',
  Numpad1: '\x1bOq',
  Numpad2: '\x1bOr',
  Numpad3: '\x1bOs',
  Numpad4: '\x1bOt',
  Numpad5: '\x1bOu',
  Numpad6: '\x1bOv',
  Numpad7: '\x1bOw',
  Numpad8: '\x1bOx',
  Numpad9: '\x1bOy',
  NumpadDecimal: '\x1bOn',
  NumpadAdd: '\x1bOk',
  NumpadSubtract: '\x1bOm',
  NumpadMultiply: '\x1bOj',
  NumpadDivide: '\x1bOo',
  NumpadEnter: '\x1bOM',
};

/** Returns bytes to send, or `null` when the event should not be consumed. */
export function encodeKeyEvent(ev: KeyboardEvent, opts: KeyEncodeOptions): string | null {
  if (ev.isComposing) return null;
  const appCursor = (opts.mode & MODE_APP_CURSOR) !== 0;
  const appKeypad = (opts.mode & MODE_APP_KEYPAD) !== 0;

  if (ev.key === 'Enter') return '\r';
  if (ev.key === 'Tab') return '\t';
  if (ev.key === 'Escape') return '\x1b';
  if (ev.key === 'Backspace') return '\x7f';
  if (ev.key === 'Delete') return '\x1b[3~';
  if (ev.key === 'Insert') return '\x1b[2~';
  if (ev.key === 'Home') return appCursor ? '\x1bOH' : '\x1b[H';
  if (ev.key === 'End') return appCursor ? '\x1bOF' : '\x1b[F';
  if (ev.key === 'PageUp') return '\x1b[5~';
  if (ev.key === 'PageDown') return '\x1b[6~';

  if (ev.key in APP_CURSOR || ev.key in NORM_CURSOR) {
    const table = appCursor ? APP_CURSOR : NORM_CURSOR;
    const base = table[ev.key];
    if (!base) return null;
    return modifyArrow(base, ev);
  }

  if (appKeypad && ev.code in APP_KEYPAD && !ev.shiftKey && !ev.ctrlKey && !ev.altKey && !ev.metaKey) {
    return APP_KEYPAD[ev.code] ?? null;
  }

  if (ev.key.startsWith('F')) {
    const n = parseInt(ev.key.slice(1), 10);
    if (n >= 1 && n <= 12) return functionKey(n);
  }

  // Ctrl + letter / symbol (no Shift — Ctrl+Shift+C is app "copy", not ^C)
  if (ev.ctrlKey && !ev.altKey && !ev.metaKey && !ev.shiftKey && ev.key.length === 1) {
    const c = ev.key.toLowerCase();
    if (c >= 'a' && c <= 'z') return String.fromCharCode(c.charCodeAt(0) - 96);
    if (c === '@') return '\x00';
    if (c === '[') return '\x1b';
    if (c === '\\') return '\x1c';
    if (c === ']') return '\x1d';
    if (c === '^' || c === '6') return '\x1e';
    if (c === '_' || c === '-') return '\x1f';
    if (c === '?') return '\x7f';
    if (c === ' ') return '\x00';
  }

  // Alt / Meta prefix
  if ((ev.altKey || (opts.altIsMeta && ev.altKey) || ev.metaKey) && ev.key.length === 1 && !ev.ctrlKey) {
    return `\x1b${ev.key}`;
  }

  if (ev.key.length === 1 && !ev.ctrlKey && !ev.altKey && !ev.metaKey) {
    return ev.key;
  }

  return null;
}

export function encodePaste(text: string, mode: number): string {
  if ((mode & MODE_BRACKETED_PASTE) !== 0) {
    return `\x1b[200~${text}\x1b[201~`;
  }
  return text;
}

function functionKey(n: number): string {
  // xterm-compatible F1–F12
  const map = [
    '', '\x1bOP', '\x1bOQ', '\x1bOR', '\x1bOS',
    '\x1b[15~', '\x1b[17~', '\x1b[18~', '\x1b[19~',
    '\x1b[20~', '\x1b[21~', '\x1b[23~', '\x1b[24~',
  ];
  return map[n] ?? '';
}

/** CSI u / modifier for arrows when Shift/Ctrl/Alt held. */
function modifyArrow(base: string, ev: KeyboardEvent): string {
  const mod =
    1
    + (ev.shiftKey ? 1 : 0)
    + (ev.altKey ? 2 : 0)
    + (ev.ctrlKey ? 4 : 0)
    + (ev.metaKey ? 8 : 0);
  if (mod === 1) return base;
  // Convert ESC [ A / ESC O A → ESC [ 1 ; mod A
  if (base.startsWith('\x1bO')) {
    return `\x1b[1;${mod}${base.slice(2)}`;
  }
  if (base.startsWith('\x1b[') && base.length === 3) {
    return `\x1b[1;${mod}${base.slice(2)}`;
  }
  return base;
}
