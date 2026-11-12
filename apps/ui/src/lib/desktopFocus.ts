/**
 * Force OS keyboard focus into the Tauri WebView, stealing it from any native
 * child HWND. DOM element.focus() alone is not enough under native-gpu.
 */

export async function focusWebviewDesktop(): Promise<void> {
  if (typeof window === 'undefined') return;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const w = window as any;
  const invoke = w.__TAURI__?.core?.invoke ?? w.__TAURI__?.invoke ?? w.__TAURI_INTERNALS__?.invoke;
  if (typeof invoke !== 'function') return;
  try {
    await invoke.call(w.__TAURI__?.core ?? w.__TAURI__ ?? w.__TAURI_INTERNALS__, 'desktop_focus_webview');
  } catch {
    // Non-Windows / older shells: DOM focus still runs after this helper.
  }
}
