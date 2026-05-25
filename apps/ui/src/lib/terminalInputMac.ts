/** Workarounds for xterm.js + WKWebView on macOS (Tauri desktop). */

const SUPPRESS_SPURIOUS_INPUT_MS = 200;

let suppressSpuriousInputUntil = 0;

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

/** Track Backspace; do not block xterm — it must emit the real erase sequence for vim/zsh. */
export function trackMacBackspaceKeydown(ev: KeyboardEvent): void {
  if (!isMacDesktop() || ev.type !== 'keydown' || ev.key !== 'Backspace' || ev.metaKey) return;
  suppressSpuriousInputUntil = Date.now() + SUPPRESS_SPURIOUS_INPUT_MS;
}

function inSuppressWindow(): boolean {
  return Date.now() <= suppressSpuriousInputUntil;
}

/** WKWebView may emit a stray space after Backspace — drop it in onData/onBinary. */
export function shouldSuppressMacSpuriousInput(data: string): boolean {
  if (!isMacDesktop() || !inSuppressWindow()) return false;
  if (data === ' ') {
    suppressSpuriousInputUntil = 0;
    return true;
  }
  return false;
}

/**
 * Block textarea `insertText(' ')` leaks while xterm still handles Backspace keydown.
 * Do not block deleteContentBackward — that breaks xterm/vim key encoding.
 */
export function installMacTextareaInputGuard(
  textarea: HTMLTextAreaElement | undefined,
): () => void {
  if (!isMacDesktop() || !textarea) return () => {};
  const onBeforeInput = (ev: Event) => {
    if (!inSuppressWindow()) return;
    const ie = ev as InputEvent;
    if (ie.inputType === 'insertText' && ie.data === ' ') {
      ev.preventDefault();
      ev.stopImmediatePropagation();
    }
  };
  textarea.addEventListener('beforeinput', onBeforeInput, { capture: true });
  return () => textarea.removeEventListener('beforeinput', onBeforeInput, { capture: true });
}
