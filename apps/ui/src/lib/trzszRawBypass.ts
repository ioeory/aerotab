/**
 * Decides when the native engine may leave raw bypass after a trzsz magic key.
 *
 * trzsz.js detects the magic key asynchronously (`setTimeout(…, 10)` inside
 * `processServerOutput`), so `isTransferringFiles()` is still false right after
 * the first raw chunk. Releasing bypass then routes the server's `#CFG:` reply
 * into the VT parser and the transfer hangs; keep bypass until the filter has
 * had time to start and then finish its transfer.
 */
export interface RawBypassGateOptions {
  isTransferring: () => boolean;
  release: () => void;
  settleMs?: number;
  pollMs?: number;
  setTimer?: (fn: () => void, ms: number) => unknown;
  clearTimer?: (id: unknown) => void;
}

export interface RawBypassGate {
  onServerOutput(): void;
  dispose(): void;
}

export function createRawBypassGate(opts: RawBypassGateOptions): RawBypassGate {
  const settleMs = opts.settleMs ?? 250;
  const pollMs = opts.pollMs ?? 100;
  const setTimer = opts.setTimer ?? ((fn, ms) => setTimeout(fn, ms));
  const clearTimer = opts.clearTimer ?? ((id) => clearTimeout(id as ReturnType<typeof setTimeout>));
  let timer: unknown = null;

  const cancel = () => {
    if (timer != null) clearTimer(timer);
    timer = null;
  };
  const schedule = (ms: number) => {
    cancel();
    timer = setTimer(check, ms);
  };
  function check() {
    timer = null;
    if (opts.isTransferring()) schedule(pollMs);
    else opts.release();
  }

  return {
    onServerOutput: () => schedule(settleMs),
    dispose: cancel,
  };
}
