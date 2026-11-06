// @ts-ignore Node strip-types tests require an explicit extension; Vite resolves this source directly.
import { traceView } from './viewCausalTrace.ts';

/** One settings reader/renderer transition per pane. Notifications during an await
 * are coalesced, and a superseded settings read never starts a transition. */
export function createTerminalSettingsReload<T>(options: {
  load: () => Promise<T>;
  apply: (settings: T) => Promise<void>;
}) {
  let revision = 0;
  let disposed = false;
  let running: Promise<void> | null = null;

  async function drain() {
    let applied = -1;
    while (!disposed && applied !== revision) {
      const current = revision;
      const settings = await options.load();
      if (disposed) return;
      if (current !== revision) continue;
      traceView({ stage: 'reload.apply', request: current });
      await options.apply(settings);
      traceView({ stage: 'reload.applied', request: current });
      applied = current;
    }
  }

  return {
    request(): Promise<void> {
      if (disposed) return Promise.resolve();
      revision++;
      traceView({ stage: 'reload.request', request: revision });
      if (!running) {
        running = drain().finally(() => { running = null; });
      }
      return running;
    },
    dispose() { disposed = true; },
  };
}
