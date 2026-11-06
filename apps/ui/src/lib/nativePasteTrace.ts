/** Opt-in, synchronous and text-free evidence. Independent from layout trace eviction. */
const stages = ['request', 'token', 'confirm', 'policy', 'host-wait', 'host-check', 'authorization', 'writer', 'finish', 'visibility-update'];
const reasons = ['received', 'accepted', 'disposed', 'session', 'generation', 'request', 'duplicate', 'pending', 'ineligible',
  'approved', 'denied', 'start', 'end', 'cancelled-or-timeout', 'suspended', 'detached', 'not-ready', 'frozen', 'hidden',
  'changed-during-state', 'update-during-state', 'host-generation', 'host-frozen', 'host-error', 'exception', 'invalid', 'not-native', 'completed'];
type Row = { order: number; stage: string; reason: string; request?: number; revision?: number; shown?: boolean };
function createBuffer() {
  const rows: Row[] = []; let order = 0;
  return Object.freeze({
    record(stage: string, reason: string, request?: number, update?: { revision: number; shown: boolean }) {
      const row: Row = { order: ++order, stage: stages.includes(stage) ? stage : 'unknown', reason: reasons.includes(reason) ? reason : 'unknown' };
      if (Number.isSafeInteger(request)) row.request = request;
      if (update && Number.isSafeInteger(update.revision) && typeof update.shown === 'boolean') {
        row.revision = update.revision; row.shown = update.shown;
      }
      rows.push(Object.freeze(row)); if (rows.length > 256) rows.shift();
    },
    snapshot: () => rows.map(row => ({ ...row })),
  });
}
export function tracePaste(stage: string, reason: string, request?: number, update?: { revision: number; shown: boolean }): void {
  try {
    if (typeof window === 'undefined') return;
    const host = window as unknown as {
      __aerotabDebugDrawObserver?: { pasteTrace?: boolean };
      __aerotabNativePasteTrace?: ReturnType<typeof createBuffer>;
    };
    if (host.__aerotabDebugDrawObserver?.pasteTrace !== true) return;
    if (!host.__aerotabNativePasteTrace) Object.defineProperty(host, '__aerotabNativePasteTrace', {
      value: createBuffer(), configurable: false, writable: false,
    });
    host.__aerotabNativePasteTrace!.record(stage, reason, request, update);
  } catch { /* Evidence must never change paste semantics. */ }
}
