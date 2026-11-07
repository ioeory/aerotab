import type { RpcClient } from './rpc';

/** Compensating transaction until the backend exposes an atomic multi-key RPC.
 * Read all snapshots before writing; include the rejected write in compensation
 * because an RPC can fail after the backend has already committed it.
 */
export async function saveSettingsTransaction(rpc: Pick<RpcClient, 'call'>, entries: readonly { key: string; value: unknown }[]): Promise<void> {
  const snapshots = new Map<string, unknown>();
  for (const { key } of entries) {
    const result = await rpc.call<{ value: unknown }>('settings.get', { key });
    snapshots.set(key, JSON.parse(JSON.stringify(result.value ?? null)));
  }
  const attempted: string[] = [];
  try {
    for (const { key, value } of entries) {
      attempted.push(key);
      await rpc.call('settings.set', { key, value });
    }
  } catch (cause) {
    const failures: string[] = [];
    for (const key of attempted.reverse()) {
      try { await rpc.call('settings.set', { key, value: snapshots.get(key) }); }
      catch (error) { failures.push(`${key}: ${error instanceof Error ? error.message : String(error)}`); }
    }
    if (failures.length) throw new Error(`${cause instanceof Error ? cause.message : String(cause)}; rollback failed — settings may be partially saved: ${failures.join('; ')}`, { cause });
    throw cause;
  }
}
