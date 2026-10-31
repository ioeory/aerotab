/** Shared terminal settings loader — one settings burst per app, not per pane. */

let cached: unknown = null;
let inflight: Promise<unknown> | null = null;
/** Bumped on invalidate so stale in-flight loads cannot repopulate the cache. */
let generation = 0;
let listening = false;

function ensureInvalidationListener(): void {
  if (listening || typeof document === 'undefined') return;
  listening = true;
  // Invalidate even when no TerminalPane is mounted (Save while empty),
  // so the next new pane loads fresh settings instead of a stale snapshot.
  document.addEventListener('aerotab:settings-changed', () => {
    invalidateTerminalSettingsCache();
  });
}

export async function getTerminalSettings<T>(load: () => Promise<T>): Promise<T> {
  ensureInvalidationListener();
  if (cached !== null) return cached as T;
  if (inflight) return inflight as Promise<T>;
  const gen = generation;
  const pending = load().then((value) => {
    if (gen === generation) cached = value;
    return value;
  }).finally(() => {
    if (inflight === pending) inflight = null;
  });
  inflight = pending;
  return pending as Promise<T>;
}

export function invalidateTerminalSettingsCache(): void {
  cached = null;
  inflight = null;
  generation += 1;
}
