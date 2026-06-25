/** Workarounds for xterm.js + WebView2 on Windows desktop (Tauri). */

import type { Terminal } from '@xterm/xterm';

/** True on Windows desktop shells (not mobile). */
export function isWindowsDesktop(): boolean {
  if (typeof navigator === 'undefined') return false;
  return /Windows/i.test(navigator.userAgent);
}

/**
 * Hide the helper textarea caret that xterm positions on the cell cursor.
 * On WebView2 the native caret can blink visibly even with opacity: 0.
 */
export function installWindowsTerminalCaretFix(
  textarea: HTMLTextAreaElement | undefined,
): () => void {
  if (!textarea) return () => {};
  textarea.style.caretColor = 'transparent';
  textarea.style.color = 'transparent';
  textarea.style.backgroundColor = 'transparent';
  return () => {};
}

/**
 * Re-init xterm's cursor blink timer after the canvas/webgl renderer attaches.
 * setRenderer() does not call handleFocus(), so blink can stay paused when the
 * terminal was already focused before the async addon loaded.
 */
export function refreshTerminalCursorBlink(term: Terminal): void {
  if (!term.options.cursorBlink) return;
  term.options.cursorBlink = false;
  term.options.cursorBlink = true;
}

export function refreshCursorBlinkIfFocused(term: Terminal | null): void {
  if (!term || !isWindowsDesktop()) return;
  const active = term.textarea;
  if (!active || active.ownerDocument.activeElement !== active) return;
  refreshTerminalCursorBlink(term);
}
