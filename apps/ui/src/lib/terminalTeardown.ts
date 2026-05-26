import type { Terminal } from '@xterm/xterm';

/** Defer heavy xterm teardown so burst tab closes do not block the UI thread. */
export function scheduleTerminalTeardown(parts: {
  term: Terminal | null;
  search: { dispose: () => void } | null;
  rendererAddon?: { dispose: () => void } | null;
  beforeDispose?: () => void;
}): void {
  const { term, search, rendererAddon, beforeDispose } = parts;
  const run = () => {
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
  };
  if (typeof requestIdleCallback === 'function') {
    requestIdleCallback(() => run(), { timeout: 500 });
  } else {
    setTimeout(run, 0);
  }
}
