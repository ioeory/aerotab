/**
 * Vault bootstrap on app launch: auto-unlock from OS keyring, or prompt once per session.
 */

import type { RpcClient } from './rpc';
import { loadPersistedSyncSettings, type PersistedSyncSettings } from './syncConfig';

export interface VaultStatus {
  configured: boolean;
  initialized: boolean;
  unlocked: boolean;
}

export type VaultBootstrapResult =
  | 'not_configured'
  | 'not_initialized'
  | 'already_unlocked'
  | 'unlocked_keyring'
  | 'needs_password';

const DEFAULT_VAULT_KEYRING = 'sync.vault';

export function vaultKeyringAccountFromSettings(
  settings: PersistedSyncSettings | null,
): string {
  const a = settings?.vaultKeyringAccount?.trim();
  return a || DEFAULT_VAULT_KEYRING;
}

export async function loadVaultKeyringAccount(rpc: RpcClient): Promise<string> {
  const settings = await loadPersistedSyncSettings(rpc);
  return vaultKeyringAccountFromSettings(settings);
}

/**
 * If the vault exists and is locked, try the OS keyring. Returns `needs_password`
 * when the user must enter the master password (show startup modal).
 */
export async function bootstrapVault(rpc: RpcClient): Promise<VaultBootstrapResult> {
  let st: VaultStatus;
  try {
    st = await rpc.call<VaultStatus>('vault.status', {});
  } catch {
    return 'not_configured';
  }
  if (!st.configured) return 'not_configured';
  if (!st.initialized) return 'not_initialized';
  if (st.unlocked) return 'already_unlocked';

  const account = await loadVaultKeyringAccount(rpc);
  try {
    const r = await rpc.call<{ unlocked: boolean; initialized: boolean }>(
      'sync.ensureVaultUnlock',
      { account },
    );
    if (r.unlocked) return 'unlocked_keyring';
  } catch {
    /* fall through to password prompt */
  }
  return 'needs_password';
}

export async function hasVaultKeyringSecret(
  rpc: RpcClient,
  account?: string,
): Promise<boolean> {
  const acct = account ?? (await loadVaultKeyringAccount(rpc));
  try {
    const r = await rpc.call<{ has: boolean }>('secret.hasMaster', { account: acct });
    return r.has;
  } catch {
    return false;
  }
}

/** Unlock vault and optionally persist password to the OS credential store. */
export async function unlockVaultWithOptions(
  rpc: RpcClient,
  password: string,
  options?: { saveToKeyring?: boolean; account?: string },
): Promise<boolean> {
  const pw = password.trim();
  if (!pw) return false;
  const account = options?.account ?? (await loadVaultKeyringAccount(rpc));
  const r = await rpc.call<{ unlocked: boolean; initialized: boolean }>(
    'sync.ensureVaultUnlock',
    { password: pw, account },
  );
  if (!r.unlocked) return false;
  if (options?.saveToKeyring) {
    await rpc.call('secret.setMaster', { account, secret: pw });
  }
  return true;
}
