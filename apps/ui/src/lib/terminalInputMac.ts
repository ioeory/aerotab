/** Workarounds for xterm.js + WKWebView on macOS (Tauri desktop). */

const SUPPRESS_SPURIOUS_SPACE_MS = 100;

let suppressSpuriousSpaceUntil = 0;

/** True on macOS desktop (including Apple Silicon). */
export function isMacDesktop(): boolean {
  if (typeof navigator === 'undefined') return false;
  const platform = navigator.platform ?? '';
  const ua = navigator.userAgent;
  return (
    platform === 'MacIntel'
    || platform === 'MacARM64'
    || /Mac/i.test(platform)
    || (/Macintosh/i.test(ua) && !/Mobile|iPhone|iPad/i.test(ua))
  );
}

/** Backspace keydown on macOS should be handled explicitly (return false to xterm). */
export function shouldHandleMacBackspace(ev: KeyboardEvent): boolean {
  return (
    isMacDesktop()
    && ev.type === 'keydown'
    && ev.key === 'Backspace'
    && !ev.metaKey
  );
}

/**
 * Bytes to send for erase on macOS.
 *
 * zsh (and xterm terminfo `kbs`) expect ^H; sending ^? often does not run
 * `backward-delete-char` and a stray WKWebView space then appears as a literal
 * space. bash/readline usually accept both — the “only zsh” report matches this.
 */
export function macBackspaceEraseByte(ctrlKey: boolean): string {
  // Ctrl+Backspace: send DEL for word-kill style on some setups.
  if (ctrlKey) return '\x7f';
  return '\x08';
}

export function markMacBackspaceHandled(): void {
  suppressSpuriousSpaceUntil = Date.now() + SUPPRESS_SPURIOUS_SPACE_MS;
}

/** WKWebView may emit a stray space `input` after Backspace — drop it. */
export function shouldSuppressMacSpuriousSpace(data: string): boolean {
  if (!isMacDesktop() || data !== ' ') return false;
  if (Date.now() > suppressSpuriousSpaceUntil) return false;
  suppressSpuriousSpaceUntil = 0;
  return true;
}

/** Block native textarea delete paths that fight our explicit Backspace handling. */
export function installMacTextareaBackspaceGuard(
  textarea: HTMLTextAreaElement | undefined,
): () => void {
  if (!isMacDesktop() || !textarea) return () => {};
  const onBeforeInput = (ev: Event) => {
    const ie = ev as InputEvent;
    if (ie.inputType === 'deleteContentBackward') {
      ev.preventDefault();
      ev.stopImmediatePropagation();
    }
  };
  textarea.addEventListener('beforeinput', onBeforeInput, { capture: true });
  return () => textarea.removeEventListener('beforeinput', onBeforeInput, { capture: true });
}
