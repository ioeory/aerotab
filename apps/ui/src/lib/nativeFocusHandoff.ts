/**
 * Coordinate modal DOM focus with Win32 native-surface OS focus handoff.
 *
 * Under native-gpu, the terminal HWND holds GetFocus(). Hiding it returns focus to
 * the WebView parent via SetFocus(parent), but that only happens after
 * native_view_update(visible:false) IPC. Calling input.focus() beforehand looks
 * focused in the DOM while keystrokes still go to the native HWND.
 *
 * Multiple native surfaces can complete hide at different times — wait until every
 * mounted native-surface has reported handoff (or timeout), then callers should
 * also invoke desktop_focus_webview so MoveFocus/SetFocus wins even if a sibling
 * HWND still races.
 */

export const NATIVE_OS_FOCUS_HANDOFF_EVENT = 'aerotab:native-os-focus-handoff';

let epoch = 0;
let completedEpoch = 0;
let expecting = false;
let pendingHides = 0;
const waiters = new Set<() => void>();

function hasNativeSurface(): boolean {
  if (typeof document === 'undefined') return false;
  return !!document.querySelector('[data-terminal-view-backend="native-surface"]');
}

function nativeSurfaceCount(): number {
  if (typeof document === 'undefined') return 0;
  return document.querySelectorAll('[data-terminal-view-backend="native-surface"]').length;
}

function settleWaiters(): void {
  const pending = [...waiters];
  waiters.clear();
  for (const finish of pending) finish();
}

/** Call when a modal/overlay opens and may need OS focus returned to the WebView. */
export function expectNativeFocusHandoff(): void {
  epoch += 1;
  const count = nativeSurfaceCount();
  if (count === 0 && !hasNativeSurface()) {
    expecting = false;
    pendingHides = 0;
    completedEpoch = epoch;
    settleWaiters();
    return;
  }
  expecting = true;
  // Every mounted native surface must report hide/already-hidden handoff.
  pendingHides = Math.max(1, count);
}

/** Call after native_view_update hide (or when already hidden) finishes SetFocus(parent). */
export function completeNativeFocusHandoff(): void {
  if (!expecting || completedEpoch === epoch) return;
  pendingHides = Math.max(0, pendingHides - 1);
  if (pendingHides > 0) return;
  completedEpoch = epoch;
  expecting = false;
  settleWaiters();
  if (typeof document === 'undefined') return;
  document.dispatchEvent(new CustomEvent(NATIVE_OS_FOCUS_HANDOFF_EVENT));
}

export function isNativeFocusHandoffPending(): boolean {
  return expecting && completedEpoch !== epoch;
}

export function nativeFocusHandoffPendingCount(): number {
  return isNativeFocusHandoffPending() ? pendingHides : 0;
}

/** Resolve once hide handoff completes, or after timeout (no native / stuck IPC). */
export function waitForNativeOsFocusHandoff(timeoutMs = 450): Promise<void> {
  if (!isNativeFocusHandoffPending()) return Promise.resolve();
  if (typeof window === 'undefined') return Promise.resolve();
  return new Promise((resolve) => {
    let settled = false;
    const finish = () => {
      if (settled) return;
      settled = true;
      waiters.delete(finish);
      window.clearTimeout(timer);
      resolve();
    };
    const timer = window.setTimeout(finish, timeoutMs);
    waiters.add(finish);
  });
}
