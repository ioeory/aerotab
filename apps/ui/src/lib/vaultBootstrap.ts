/** Vault startup only inspects status; unlocking is explicit and process-local. */
import type { RpcClient } from './rpc';

export interface VaultStatus {
  configured: boolean;
  initialized: boolean;
  unlocked: boolean;
}

export type VaultBootstrapResult =
  | 'not_configured'
  | 'not_initialized'
  | 'already_unlocked'
  | 'needs_password';

/** Keep the startup contract without consulting historical saved vault secrets. */
export async function bootstrapVault(rpc: RpcClient): Promise<VaultBootstrapResult> {
  let st: VaultStatus;
  try {
    st = await rpc.call<VaultStatus>('vault.status', {});
  } catch {
    return 'not_configured';
  }
  if (!st.configured) return 'not_configured';
  if (!st.initialized) return 'not_initialized';
  return st.unlocked ? 'already_unlocked' : 'needs_password';
}

/** Legacy options are accepted but ignored: vault passwords are never persisted. */
export async function unlockVaultWithOptions(
  rpc: RpcClient,
  password: string,
  _options?: { saveToKeyring?: boolean; account?: string },
): Promise<boolean> {
  if (!password) return false;
  const r = await rpc.call<{ unlocked: boolean; initialized: boolean }>(
    'sync.ensureVaultUnlock',
    { password },
  );
  return r.unlocked;
}
