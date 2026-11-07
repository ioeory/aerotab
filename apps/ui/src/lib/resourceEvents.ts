import type { RpcClient } from './rpc';

/** Wrap the application's shared RPC client to invalidate resources across all
 * existing entry points (unlock modal, sync and settings). No params are emitted. */
export function instrumentResourceClient(client: RpcClient): RpcClient {
  return { async call<T = unknown>(method: string, params?: unknown): Promise<T> {
    const result = await client.call<T>(method, params);
    if (['vault.initialize', 'vault.unlock', 'vault.lock', 'vault.put', 'vault.remove', 'vault.changePassword', 'sync.ensureVaultUnlock', 'sync.now'].includes(method)) notifyResource('vault');
    if (['ssh.knownHosts.put', 'ssh.knownHosts.remove', 'ssh.knownHosts.configure', 'session.openSsh', 'session.openSshProfile'].includes(method)) notifyResource('knownHosts');
    return result;
  } };
}

export type ResourceName = 'vault' | 'knownHosts';
const listeners = new Map<ResourceName, Set<() => void>>();
/** Metadata invalidation only: never transport secrets through notifications. */
export function subscribeResource(resource: ResourceName, listener: () => void): () => void {
  const group = listeners.get(resource) ?? new Set();
  listeners.set(resource, group); group.add(listener);
  return () => { group.delete(listener); };
}
export function notifyResource(resource: ResourceName): void {
  for (const listener of listeners.get(resource) ?? []) {
    try { listener(); } catch { /* A stale view must not prevent other views refreshing. */ }
  }
}
export async function hostKeyFingerprint(keyB64: string): Promise<string> {
  if (!/^[A-Za-z0-9+/]+={0,2}$/.test(keyB64)) throw new Error('Invalid base64 host key');
  const bytes = Uint8Array.from(atob(keyB64), char => char.charCodeAt(0));
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
  return `SHA256:${btoa(String.fromCharCode(...digest)).replace(/=+$/, '')}`;
}
