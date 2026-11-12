/**
 * Shared sync bootstrap: restore the in-memory SyncEngine on app launch using
 * persisted `settings.sync` + master password from the OS keyring.
 */

import type { RpcClient } from './rpc';
// @ts-ignore Node's native TS test runner requires an explicit extension.
import { hasSyncApproval } from './syncWizard.ts';

export type SyncBackend = 'webdav' | 'git' | 's3' | 'drive';
export type SyncGroup =
  | 'Connections'
  | 'Appearance'
  | 'Shortcuts'
  | 'PluginCfg'
  | 'Credentials'
  | 'Notes';

export type GitAuthMode = 'none' | 'https' | 'ssh' | 'oauth_github' | 'oauth_gitlab';

export interface PersistedSyncSettings {
  backend?: SyncBackend;
  s3Endpoint?: string;
  s3Region?: string;
  s3Bucket?: string;
  s3Prefix?: string;
  s3PathStyle?: boolean;
  driveClientId?: string;
  drivePrefix?: string;
  firstSyncApproved?: boolean;
  approvedTarget?: string;
  setupVersion?: number;
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
  /** SSH port for Git remote (default 22). */
  gitSshPort?: number;
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
  /** OS keyring account for Git HTTPS PAT (not stored in settings.sled). */
  gitHttpsKeyringAccount?: string;
  /** Sync wire protocol: 2 = oplog/BlobStore (current default after configure). */
  protocol?: number;
  /** Optional oplog compaction thresholds (files / bytes). */
  compact?: { maxOpFiles?: number; maxOpBytes?: number };
}

/** Keyring account for Git HTTPS personal access token. */
export const DEFAULT_GIT_HTTPS_KEYRING_ACCOUNT = 'sync.git.https';

export function gitHttpsKeyringAccount(settings: PersistedSyncSettings | null): string {
  const a = settings?.gitHttpsKeyringAccount?.trim();
  return a || DEFAULT_GIT_HTTPS_KEYRING_ACCOUNT;
}

export async function loadGitHttpsPassword(
  rpc: RpcClient,
  settings: PersistedSyncSettings | null,
): Promise<string | undefined> {
  const inline = settings?.gitRemotePassword?.trim();
  if (inline) return inline;
  try {
    const r = await rpc.call<{ secret: string }>('secret.getMaster', {
      account: gitHttpsKeyringAccount(settings),
    });
    return r.secret?.trim() || undefined;
  } catch {
    return undefined;
  }
}

export async function saveGitHttpsPassword(
  rpc: RpcClient,
  settings: PersistedSyncSettings | null,
  password: string,
): Promise<void> {
  const secret = password.trim();
  if (!secret) return;
  await rpc.call('secret.setMaster', {
    account: gitHttpsKeyringAccount(settings),
    secret,
  });
}

export async function hasGitHttpsPassword(
  rpc: RpcClient,
  settings: PersistedSyncSettings | null,
): Promise<boolean> {
  try {
    const r = await rpc.call<{ has: boolean }>('secret.hasMaster', {
      account: gitHttpsKeyringAccount(settings),
    });
    return r.has;
  } catch {
    return false;
  }
}

export async function clearGitHttpsPassword(
  rpc: RpcClient,
  settings: PersistedSyncSettings | null,
): Promise<void> {
  await rpc.call('secret.clearMaster', {
    account: gitHttpsKeyringAccount(settings),
  });
}

const SYNC_GROUPS: SyncGroup[] = [
  'Connections',
  'Appearance',
  'Shortcuts',
  'PluginCfg',
  'Credentials',
  'Notes',
];

const DEFAULT_ENABLED: Record<SyncGroup, boolean> = {
  Connections: true,
  Appearance: true,
  Shortcuts: true,
  PluginCfg: true,
  Credentials: false,
  Notes: false,
};

export async function loadPersistedSyncSettings(rpc: RpcClient): Promise<PersistedSyncSettings | null> {
  try {
    const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'sync' });
    if (!r.value || typeof r.value !== 'object') return null;
    const settings = { ...r.value } as PersistedSyncSettings;
    // Existing configs without a version are legacy; never silently opt in.
    settings.protocol ??= 2;
    let migrated = false;
    for (const [field, account] of [['webdavPassword', 'sync.webdav.password'], ['gitSshPassphrase', 'sync.git.ssh'], ['gitRemotePassword', gitHttpsKeyringAccount(settings)]] as const) {
      const secret = settings[field];
      if (secret) {
        await rpc.call('secret.setMaster', { account, secret });
        delete settings[field];
        migrated = true;
      }
    }
    if (migrated) await rpc.call('settings.set', { key: 'sync', value: settings });
    return settings;
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
  if (settings.backend === 's3') return Boolean(settings.s3Endpoint && settings.s3Bucket && settings.s3Region);
  if (settings.backend === 'drive') return Boolean(settings.driveClientId);
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
  if (settings.backend === 's3' || settings.backend === 'drive') {
    await rpc.call(settings.backend === 's3' ? 'sync.configureS3' : 'sync.configureDrive', {
      endpoint: settings.s3Endpoint, region: settings.s3Region, bucket: settings.s3Bucket,
      prefix: settings.backend === 's3' ? settings.s3Prefix : settings.drivePrefix,
      path_style: settings.s3PathStyle ?? true, client_id: settings.driveClientId,
      protocol: settings.protocol ?? 3,
      master_password: masterPassword, state_dir: settings.stateDir || undefined,
    });
    return;
  }
  if (settings.backend === 'webdav') {
    const saved = await rpc.call<{ secret: string }>('secret.getMaster', { account: 'sync.webdav.password' }).catch(() => ({ secret: '' }));
    await rpc.call('sync.configureWebdav', {
      protocol: settings.protocol ?? 3,
      base_url: settings.webdavUrl,
      user: settings.webdavUser || undefined,
      password: settings.webdavPassword || saved.secret || undefined,
      master_password: masterPassword,
      state_dir: settings.stateDir || undefined,
    });
    return;
  }

  const args: Record<string, unknown> = {
    protocol: settings.protocol ?? 3,
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
    args.remote_auth = 'https';
    const pw = await loadGitHttpsPassword(rpc, settings);
    if (!pw) {
      throw new Error(
        'Git HTTPS token is missing. Enter your PAT and click “Save Git token” or Configure / re-key.',
      );
    }
    args.remote_user = settings.gitRemoteUser?.trim() || 'oauth2';
    args.remote_password = pw;
  } else if (mode === 'ssh') {
    args.remote_auth = 'ssh';
    args.remote_ssh_key = settings.gitSshKeyPath;
    const saved = await rpc.call<{ secret: string }>('secret.getMaster', { account: 'sync.git.ssh' }).catch(() => ({ secret: '' }));
    if (settings.gitSshPassphrase || saved.secret) args.remote_ssh_passphrase = settings.gitSshPassphrase || saved.secret;
    const port = Number(settings.gitSshPort);
    if (port > 0 && port !== 22) args.remote_ssh_port = port;
  } else if (mode === 'oauth_github') {
    args.oauth_provider = 'github';
  } else if (mode === 'oauth_gitlab') {
    args.oauth_provider = 'gitlab';
  }
  await rpc.call('sync.configureGit', args);
}

export async function applyPersistedAutoSync(
  rpc: RpcClient,
  settings: PersistedSyncSettings,
  opts?: { force?: boolean },
): Promise<void> {
  if (settings.setupVersion === 3 && !hasSyncApproval(settings)) {
    await rpc.call('sync.stopAutoSync', {});
    return;
  }
  if (!settings.autoSyncEnabled) {
    await rpc.call('sync.stopAutoSync', {});
    return;
  }
  const groups = selectedSyncGroups(settings);
  if (groups.length === 0) {
    await rpc.call('sync.stopAutoSync', {});
    return;
  }
  const configured = await isSyncEngineConfigured(rpc);
  if (!configured) {
    return;
  }
  const interval_ms = Math.max(1, settings.autoSyncMinutes ?? 15) * 60_000;
  if (!opts?.force) {
    try {
      const s = await rpc.call<{
        configured: boolean;
        autoIntervalMs: number | null;
        autoRunning?: boolean;
      }>('sync.status', {});
      if (s.configured && s.autoRunning && s.autoIntervalMs === interval_ms) {
        return;
      }
    } catch {
      /* start below */
    }
  }
  await rpc.call('sync.startAutoSync', { interval_ms, groups });
}

export type SyncBootstrapResult =
  | 'already_configured'
  | 'configured'
  | 'no_settings'
  | 'no_keyring_secret'
  | 'no_git_https_secret';

/**
 * On app launch: if sync was set up before, re-key the engine from the OS
 * credential store so Sync now / auto-sync work without opening settings.
 */
export async function bootstrapSyncEngine(rpc: RpcClient): Promise<SyncBootstrapResult> {
  const settings = await loadPersistedSyncSettings(rpc);
  if (!settings || !hasMinimumSyncConfig(settings)) {
    return 'no_settings';
  }
  const wasConfigured = await isSyncEngineConfigured(rpc);
  try {
    const has = await rpc.call<{ has: boolean }>('secret.hasMaster', secretParams(settings));
    if (!has.has) return 'no_keyring_secret';
    if (settings.backend === 'git' && settings.gitAuthMode === 'https') {
      const token = await loadGitHttpsPassword(rpc, settings);
      if (!token) return 'no_git_https_secret';
    }
    const r = await rpc.call<{ secret: string }>('secret.getMaster', secretParams(settings));
    // Keep keyring failures separate from configuration/scheduler failures.
    if (!r.secret) return 'no_keyring_secret';
    await configureSyncEngineFromSettings(rpc, settings, r.secret);
  } catch (e) {
    const msg = (e as Error).message ?? '';
    if (msg.includes('HTTPS token is missing')) return 'no_git_https_secret';
    throw e;
  }
  await applyPersistedAutoSync(rpc, settings);
  return wasConfigured ? 'already_configured' : 'configured';
}

/** Ensure engine is live before sync.now (palette or settings). */
export async function ensureSyncEngineConfigured(rpc: RpcClient): Promise<void> {
  const settings = await loadPersistedSyncSettings(rpc);
  if (settings?.setupVersion === 3 && !hasSyncApproval(settings)) throw new Error('Finish Config sync setup and approve the first merge preview before syncing.');
  if (await isSyncEngineConfigured(rpc)) return;
  const boot = await bootstrapSyncEngine(rpc);
  if (boot === 'configured' || boot === 'already_configured') return;
  if (boot === 'no_settings') {
    throw new Error('Sync is not set up. Open Config sync in settings first.');
  }
  if (boot === 'no_git_https_secret') {
    throw new Error(
      'Git HTTPS token is not in the OS credential store. '
        + 'Enter your PAT in Config sync, click “Save Git token”, then Configure / re-key.',
    );
  }
  throw new Error(
    'Sync master password is not in the OS credential store. '
      + 'Enter it once in Config sync and click “Save to OS credential store”, then Configure.',
  );
}
