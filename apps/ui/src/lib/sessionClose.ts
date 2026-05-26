import type { RpcClient } from './rpc';
import { withRpcTimeout } from './rpcTimeout';

/** Per-session close RPC timeout (tab chrome must not block on slow SSH teardown). */
const SESSION_CLOSE_TIMEOUT_MS = 8_000;
/** Avoid flooding the core with dozens of simultaneous SSH teardowns. */
const MAX_CONCURRENT_CLOSES = 4;

const closeQueue: string[] = [];
const queuedIds = new Set<string>();
let closeInFlight = 0;

function drainCloseQueue(rpc: RpcClient): void {
  while (closeInFlight < MAX_CONCURRENT_CLOSES && closeQueue.length > 0) {
    const id = closeQueue.shift()!;
    queuedIds.delete(id);
    closeInFlight += 1;
    void withRpcTimeout(
      rpc.call('session.close', { id }),
      SESSION_CLOSE_TIMEOUT_MS,
      `session.close ${id.slice(0, 8)}`,
    )
      .catch((err) => {
        console.warn('session.close', err);
      })
      .finally(() => {
        closeInFlight -= 1;
        drainCloseQueue(rpc);
      });
  }
}

/** Close sessions in the background without blocking the UI (bounded concurrency). */
export function closeSessionsInBackground(rpc: RpcClient, sessionIds: string[]): void {
  const unique = [...new Set(sessionIds.filter((id) => id.length > 0))];
  if (unique.length === 0) return;
  for (const id of unique) {
    if (queuedIds.has(id)) continue;
    queuedIds.add(id);
    closeQueue.push(id);
  }
  drainCloseQueue(rpc);
}
