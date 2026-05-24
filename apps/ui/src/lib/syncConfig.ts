/**
 * Shared sync bootstrap: restore the in-memory SyncEngine on app launch using
 * persisted `settings.sync` + master password from the OS keyring.
 */

import type { RpcClient } from './rpc';

export type SyncBackend = 'webdav' | 'git';
export type SyncGroup =
  | 'Connections'
  | 'Appearance'
  | 'Shortcuts'
  | 'PluginCfg'
  | 'Credentials';

export type GitAuthMode = 'none' | 'https' | 'ssh' | 'oauth_github' | 'oauth_gitlab';

export interface PersistedSyncSettings {
  backend?: SyncBackend;
  webdavUrl?: string;
  webdavUser?: string;
  webdavPassword?: string;
  gitRepoPath?: string;
  gitRemoteUrl?: string;
  gitRemoteName?: string;
  gitRemoteBranch?: string;
  gitAuthorName?: string;
  gitAuthorEmail?: string;
  gitRemoteUser?: string;
  gitRemotePassword?: string;
  gitSshKeyPath?: string;
  gitSshPassphrase?: string;
  gitAuthMode?: GitAuthMode;
  githubOAuthClientId?: string;
  gitlabOAuthClientId?: string;
  gitlabOAuthBaseUrl?: string;
  stateDir?: string;
  autoSyncEnabled?: boolean;
  autoSyncMinutes?: number;
  enabledGroups?: Partial<Record<SyncGroup, boolean>>;
  keyringAccount?: string;
  /** OS keyring account for vault password used during credential sync. */
  vaultKeyringAccount?: string;
}

const SYNC_GROUPS: SyncGroup[] = [
  'Connections',
  'Appearance',
  'Shortcuts',
  'PluginCfg',
  'Credentials',
];

const DEFAULT_ENABLED: Record<SyncGroup, boolean> = {
  Connections: true,
  Appearance: true,
  Shortcuts: true,
  PluginCfg: true,
  Credentials: false,
};

export async function loadPersistedSyncSettings(rpc: RpcClient): Promise<PersistedSyncSettings | null> {
  try {
    const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'sync' });
    if (!r.value || typeof r.value !== 'object') return null;
    return r.value as PersistedSyncSettings;
  } catch {
    return null;
  }
}

export function selectedSyncGroups(settings: PersistedSyncSettings | null): SyncGroup[] {
  const raw = settings?.enabledGroups;
  if (!raw || typeof raw !== 'object') {
    return SYNC_GROUPS.filter((g) => DEFAULT_ENABLED[g]);
  }
  return SYNC_GROUPS.filter((g) => {
    const v = raw[g];
    return typeof v === 'boolean' ? v : DEFAULT_ENABLED[g];
  });
}

function secretParams(settings: PersistedSyncSettings | null): { account?: string } {
  const account = settings?.keyringAccount?.trim();
  return account ? { account } : {};
}

function hasMinimumSyncConfig(settings: PersistedSyncSettings): boolean {
  if (settings.backend === 'webdav') {
    return Boolean(settings.webdavUrl?.trim());
  }
  if (settings.backend === 'git') {
    return Boolean(settings.gitRepoPath?.trim());
  }
  return false;
}

export async function isSyncEngineConfigured(rpc: RpcClient): Promise<boolean> {
  try {
    const s = await rpc.call<{ configured: boolean }>('sync.status', {});
    return Boolean(s.configured);
  } catch {
    return false;
  }
}

/** Configure SyncEngine from saved settings + keyring master password. */
export async function configureSyncEngineFromSettings(
  rpc: RpcClient,
  settings: PersistedSyncSettings,
  masterPassword: string,
): Promise<void> {
  if (settings.backend === 'webdav') {
    await rpc.call('sync.configureWebdav', {
      base_url: settings.webdavUrl,
      user: settings.webdavUser || undefined,
      password: settings.webdavPassword || undefined,
      master_password: masterPassword,
      state_dir: settings.stateDir || undefined,
    });
    return;
  }

  const args: Record<string, unknown> = {
    repo_path: settings.gitRepoPath,
    master_password: masterPassword,
    author_name: settings.gitAuthorName || undefined,
    author_email: settings.gitAuthorEmail || undefined,
    state_dir: settings.stateDir || undefined,
    remote_name: settings.gitRemoteName ?? 'origin',
    remote_branch: settings.gitRemoteBranch ?? 'main',
  };
  if (settings.gitRemoteUrl) args.remote_url = settings.gitRemoteUrl;
  const mode = settings.gitAuthMode ?? 'none';
  if (mode === 'https') {
    args.remote_user = settings.gitRemoteUser;
    args.remote_password = settings.gitRemotePassword;
  } else if (mode === 'ssh') {
    args.remote_ssh_key = settings.gitSshKeyPath;
    if (settings.gitSshPassphrase) args.remote_ssh_passphrase = settings.gitSshPassphrase;
  } else if (mode === 'oauth_github') {
    args.oauth_provider = 'github';
  } else if (mode === 'oauth_gitlab') {
    args.oauth_provider = 'gitlab';
  }
  await rpc.call('sync.configureGit', args);
}

export async function applyPersistedAutoSync(rpc: RpcClient, settings: PersistedSyncSettings): Promise<void> {
  if (!settings.autoSyncEnabled) {
    await rpc.call('sync.stopAutoSync', {});
    return;
  }
  const groups = selectedSyncGroups(settings);
  if (groups.length === 0) return;
  const interval_ms = Math.max(1, settings.autoSyncMinutes ?? 15) * 60_000;
  await rpc.call('sync.startAutoSync', { interval_ms, groups });
}

export type SyncBootstrapResult =
  | 'already_configured'
  | 'configured'
  | 'no_settings'
  | 'no_keyring_secret';

/**
 * On app launch: if sync was set up before, re-key the engine from the OS
 * credential store so Sync now / auto-sync work without opening settings.
 */
export async function bootstrapSyncEngine(rpc: RpcClient): Promise<SyncBootstrapResult> {
  if (await isSyncEngineConfigured(rpc)) {
    return 'already_configured';
  }
  const settings = await loadPersistedSyncSettings(rpc);
  if (!settings || !hasMinimumSyncConfig(settings)) {
    return 'no_settings';
  }
  try {
    const has = await rpc.call<{ has: boolean }>('secret.hasMaster', secretParams(settings));
    if (!has.has) return 'no_keyring_secret';
    const r = await rpc.call<{ secret: string }>('secret.getMaster', secretParams(settings));
    await configureSyncEngineFromSettings(rpc, settings, r.secret);
    await applyPersistedAutoSync(rpc, settings);
    return 'configured';
  } catch {
    return 'no_keyring_secret';
  }
}

/** Ensure engine is live before sync.now (palette or settings). */
export async function ensureSyncEngineConfigured(rpc: RpcClient): Promise<void> {
  const boot = await bootstrapSyncEngine(rpc);
  if (boot === 'configured' || boot === 'already_configured') return;
  if (boot === 'no_settings') {
    throw new Error('Sync is not set up. Open Config sync in settings first.');
  }
  throw new Error(
    'Sync master password is not in the OS credential store. '
      + 'Enter it once in Config sync and click “Save to OS credential store”, then Configure.',
  );
}
