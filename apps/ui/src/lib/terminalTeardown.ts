import type { Terminal } from '@xterm/xterm';

const teardownQueue: Array<() => void> = [];
let teardownRaf: number | null = null;

function drainTeardownQueue(): void {
  teardownRaf = null;
  const run = teardownQueue.shift();
  if (run) {
    try {
      run();
    } catch {
      /* ignore */
    }
  }
  if (teardownQueue.length > 0) {
    teardownRaf = requestAnimationFrame(drainTeardownQueue);
  }
}

function enqueueTeardown(run: () => void): void {
  teardownQueue.push(run);
  if (teardownRaf == null) {
    teardownRaf = requestAnimationFrame(drainTeardownQueue);
  }
}

/** Defer xterm teardown; at most one dispose per frame to avoid UI freezes on tab close. */
export function scheduleTerminalTeardown(parts: {
  term: Terminal | null;
  search: { dispose: () => void } | null;
  rendererAddon?: { dispose: () => void } | null;
  beforeDispose?: () => void;
}): void {
  const { term, search, rendererAddon, beforeDispose } = parts;
  enqueueTeardown(() => {
    try {
      beforeDispose?.();
    } catch {
      /* ignore */
    }
    try {
      rendererAddon?.dispose();
    } catch {
      /* ignore */
    }
    try {
      search?.dispose();
    } catch {
      /* ignore */
    }
    try {
      term?.dispose();
    } catch {
      /* ignore */
    }
  });
}
