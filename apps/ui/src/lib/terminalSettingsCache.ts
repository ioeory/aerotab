/** Shared terminal settings loader — one settings burst per app, not per pane. */

let cached: unknown = null;
let inflight: Promise<unknown> | null = null;

export async function getTerminalSettings<T>(load: () => Promise<T>): Promise<T> {
  if (cached !== null) return cached as T;
  if (inflight) return inflight as Promise<T>;
  inflight = load().then((value) => {
    cached = value;
    inflight = null;
    return value;
  });
  return inflight as Promise<T>;
}

export function invalidateTerminalSettingsCache(): void {
  cached = null;
  inflight = null;
}
