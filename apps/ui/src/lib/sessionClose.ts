import type { RpcClient } from './rpc';
import { withRpcTimeout } from './rpcTimeout';

/** Per-session close RPC timeout (tab chrome must not block on slow SSH teardown). */
const SESSION_CLOSE_TIMEOUT_MS = 8_000;

/** Close sessions in parallel without blocking the UI thread. */
export function closeSessionsInBackground(rpc: RpcClient, sessionIds: string[]): void {
  const unique = [...new Set(sessionIds.filter((id) => id.length > 0))];
  if (unique.length === 0) return;
  void Promise.allSettled(
    unique.map((id) =>
      withRpcTimeout(
        rpc.call('session.close', { id }),
        SESSION_CLOSE_TIMEOUT_MS,
        `session.close ${id.slice(0, 8)}`,
      ),
    ),
  ).then((results) => {
    for (const result of results) {
      if (result.status === 'rejected') console.warn('session.close', result.reason);
    }
  });
}
