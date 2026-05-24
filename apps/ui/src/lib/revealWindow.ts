/** Show the Tauri main window after the first UI frame (startup is `visible: false`). */

type TauriWindow = { show: () => Promise<void> };

export function revealMainWindow(): void {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const w = window as any;
  const invoke = w.__TAURI__?.core?.invoke ?? w.__TAURI__?.invoke;
  if (typeof invoke === 'function') {
    void invoke('show_main_window');
    return;
  }
  const getWin = w.__TAURI__?.webviewWindow?.getCurrentWebviewWindow;
  if (typeof getWin === 'function') {
    void (getWin() as TauriWindow).show();
  }
}
