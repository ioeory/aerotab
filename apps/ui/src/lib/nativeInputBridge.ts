type HandoffWaiter = { resolve: () => void; reject: (error: unknown) => void };
const quiescing = new WeakMap<object, Set<HandoffWaiter>>();
/** Timeout cancels only the subscriber. Neither IPC nor accepted input ownership
 * is cancelled; a never-returning transport retains one operation, not one
 * suspended coroutine/Promise.race reaction per recovery retry. */
export function quiesceAndDrainNativeInput(
  host: { quiesceInput: () => Promise<number> },
  queue: { drainThrough: (target: number, timeoutMs?: number) => Promise<void> },
  timeoutMs = 5000,
): Promise<void> {
  let waiters = quiescing.get(host);
  if (!waiters) {
    waiters = new Set(); quiescing.set(host, waiters);
    const captured = waiters;
    const settle = (error?: unknown) => {
      if (quiescing.get(host) === captured) quiescing.delete(host);
      for (const waiter of captured) { if (error) waiter.reject(error); else waiter.resolve(); }
      captured.clear();
    };
    void Promise.resolve().then(() => host.quiesceInput()).then(target => queue.drainThrough(target, timeoutMs)).then(() => settle(), settle);
  }
  const subscribers = waiters;
  return new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => {
      subscribers.delete(waiter); reject(new Error('native input handoff timeout'));
    }, timeoutMs);
    const waiter = {
      resolve: () => { clearTimeout(timer); resolve(); },
      reject: (error: unknown) => { clearTimeout(timer); reject(error); },
    };
    subscribers.add(waiter);
  });
}

export const NATIVE_INPUT_EVENT = 'aerotab:native-input';
export interface NativeInputEvent { sessionId: string; generation: string; seq: number; data: string }
export function createNativeInputQueue(options: {
  sessionId: string; generation: () => string | null;
  write: (bytes: Uint8Array, valid: () => boolean) => Promise<void>; ack: (seq: number) => Promise<void>;
  fail: () => void; limit?: number;
  /** ACK is idempotent; PTY writes are not. Keep the written head until ACK succeeds. */
  ackRetryMs?: number; ackTimeoutMs?: number; onAckRecovery?: (pending: boolean) => void;
}) {
  let stopped = false;
  let last = 0;
  let acknowledged = 0;
  const waiters = new Set<{ target: number; resolve: () => void; reject: (error: Error) => void }>();
  const check = () => { for (const waiter of waiters) {
    if (acknowledged >= waiter.target) { waiters.delete(waiter); waiter.resolve(); }
  } };
  let running = false;
  let bytes = 0;
  const queue: { event: NativeInputEvent; data: Uint8Array; written: boolean }[] = [];
  let retryTimer: ReturnType<typeof setTimeout> | undefined;
  let wakeRetry: (() => void) | undefined;
  let recovering = false;
  let cancelAckWait: (() => void) | undefined;
  async function waitForAck(seq: number, generation: string): Promise<void> {
    // The IPC transport cannot be cancelled. Keep exactly ONE RPC until it
    // settles; deadline changes UI state, never launches a parallel retry.
    let timer: ReturnType<typeof setTimeout> | undefined;
    let watch: ReturnType<typeof setInterval> | undefined;
    const transport = Promise.resolve().then(() => options.ack(seq));
    try {
      await Promise.race([transport, new Promise<never>((_, reject) => {
        cancelAckWait = () => reject(new Error('native input drain cancelled'));
        timer = setTimeout(() => recovery(true), options.ackTimeoutMs ?? 1500);
        watch = setInterval(() => {
          if (stopped || options.generation() !== generation) { stop(); }
        }, options.ackTimeoutMs ?? 1500);
      })]);
    } finally { clearTimeout(timer); clearInterval(watch); cancelAckWait = undefined; }
  }
  const recovery = (pending: boolean) => {
    if (recovering === pending) return;
    recovering = pending; options.onAckRecovery?.(pending);
  };
  const stop = () => {
    stopped = true; cancelAckWait?.(); clearTimeout(retryTimer); wakeRetry?.(); wakeRetry = undefined;
    queue.length = 0; bytes = 0;
    for (const waiter of waiters) waiter.reject(new Error('native input drain cancelled'));
    waiters.clear();
  };
  const fail = () => { stop(); options.fail(); };
  async function drain() {
    if (running || stopped) return;
    running = true;
    try {
      while (!stopped && queue.length) {
        const item = queue[0];
        if (!item) break;
        if (options.generation() !== item.event.generation) { stop(); break; }
        if (!item.written) {
          // A rejection may mean partially applied bytes. Never retry that write.
          await options.write(item.data, () => !stopped && options.generation() === item.event.generation);
          item.written = true;
        }
        if (stopped || options.generation() !== item.event.generation) { stop(); break; }
        try { await waitForAck(item.event.seq, item.event.generation); }
        catch {
          if (stopped || options.generation() !== item.event.generation) { stop(); break; }
          recovery(true);
          // One bounded-memory retry loop. Drain deadlines reject only their
          // waiter, not the retained reservation or its successful PTY write.
          await new Promise<void>(resolve => {
            wakeRetry = resolve;
            retryTimer = setTimeout(resolve, options.ackRetryMs ?? 250);
          });
          wakeRetry = undefined;
          continue;
        }
        if (stopped || options.generation() !== item.event.generation) { stop(); break; }
        queue.shift(); bytes -= item.data.length;
        acknowledged = item.event.seq;
        recovery(false);
        check();
      }
    } catch { fail(); }
    finally { running = false; }
  }
  return {
    accept(event: NativeInputEvent) {
      if (stopped || !event || event.sessionId !== options.sessionId || event.generation !== options.generation()) return;
      if (!Number.isSafeInteger(event.seq) || event.seq < 1) { fail(); return; }
      if (event.seq <= last) return;
      if (event.seq !== last + 1 || typeof event.data !== 'string' || event.data.length > 87384) { fail(); return; }
      try {
        const decoded = atob(event.data);
        if (btoa(decoded) !== event.data) { fail(); return; }
        const data = Uint8Array.from(decoded, c => c.charCodeAt(0));
        if (queue.length >= (options.limit ?? 256) || bytes + data.length > 262144) { fail(); return; }
        last = event.seq; bytes += data.length;
        queue.push({ event, data, written: false });
        void drain();
      } catch { fail(); }
    },
    drainThrough(target: number, timeoutMs = 5000): Promise<void> {
      if (stopped) return Promise.reject(new Error('native input drain cancelled'));
      if (!Number.isSafeInteger(target) || target < last) return Promise.reject(new Error('invalid native input watermark'));
      if (acknowledged >= target) return Promise.resolve();
      return new Promise<void>((resolve, reject) => {
        const timer = setTimeout(() => {
          waiters.delete(waiter);
          reject(new Error('native input drain timeout'));
        }, timeoutMs);
        const waiter = { target,
          resolve: () => { clearTimeout(timer); resolve(); },
          reject: (error: Error) => { clearTimeout(timer); reject(error); },
        };
        waiters.add(waiter);
        check();
      });
    },
    hasPending: () => last > acknowledged,
    dispose: stop,
  };
}
