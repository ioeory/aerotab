/** Desktop session introspection (Wayland vs X11). */

export interface DesktopSessionInfo {
  platform: string;
  wayland: boolean;
  display: string | null;
  x11ForwardAvailable: boolean;
}

function tauriInvoke(): ((cmd: string, args?: Record<string, unknown>) => Promise<unknown>) | null {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const w = window as any;
  const invoke = w.__TAURI__?.core?.invoke ?? w.__TAURI__?.invoke;
  return typeof invoke === 'function' ? invoke : null;
}

/** Prefer Tauri invoke; fall back to JSON-RPC when running outside the desktop shell. */
export async function getSessionInfo(
  rpc: { call<T>(method: string, params?: unknown): Promise<T> },
  display?: string,
): Promise<DesktopSessionInfo> {
  const invoke = tauriInvoke();
  if (invoke) {
    return invoke('session_info', { display: display ?? null }) as Promise<DesktopSessionInfo>;
  }
  return rpc.call<DesktopSessionInfo>('desktop.sessionInfo', {
    display: display ?? null,
  });
}

export function isLinuxDesktop(): boolean {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const w = window as any;
  const os = w.__TAURI_INTERNALS__?.metadata?.currentPlatform
    ?? w.__TAURI__?.os?.platform?.();
  if (typeof os === 'string') return os === 'linux';
  return navigator.userAgent.toLowerCase().includes('linux');
}
