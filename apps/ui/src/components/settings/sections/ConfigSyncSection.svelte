<script lang="ts">
  // Config Sync (M11) — UI on top of the existing sync.* IPC.
  //
  // Persists non-secret configuration in settings key `sync`. Master password
  // is stored in the OS keyring (not in settings); the app restores the sync
  // engine on launch via bootstrapSyncEngine().

  import { onMount, onDestroy } from 'svelte';
  import { RefreshCw, Play } from '@lucide/svelte';
  import { b64decode, b64encode, type RpcClient } from '../../../lib/rpc';
  import { i18n } from '../../../lib/i18n.svelte';
  import { appConfirm } from '../../../lib/confirm.svelte';
  import { settingsCoord } from '../../../lib/settingsStore.svelte';
  import {
    applyPersistedAutoSync,
    bootstrapSyncEngine,
    configureSyncEngineFromSettings,
    clearGitHttpsPassword,
    ensureSyncEngineConfigured,
    hasGitHttpsPassword,
    isSyncEngineConfigured,
    loadGitHttpsPassword,
    saveGitHttpsPassword,
    type PersistedSyncSettings,
  } from '../../../lib/syncConfig';

  interface Props {
    rpc: RpcClient;
    onError: (msg: string) => void;
    /** Called after sync pulls data into local stores so the app can refresh. */
    onSyncApplied?: () => void | Promise<void>;
  }
  let { rpc, onError, onSyncApplied }: Props = $props();

  type Backend = 'webdav' | 'git';
  type SyncGroup = 'Connections' | 'Appearance' | 'Shortcuts' | 'PluginCfg' | 'Credentials';
  const syncGroups: SyncGroup[] = ['Connections', 'Appearance', 'Shortcuts', 'PluginCfg', 'Credentials'];

  function defaultEnabledGroups(): Record<SyncGroup, boolean> {
    return {
      Connections: true,
      Appearance: true,
      Shortcuts: true,
      PluginCfg: true,
      Credentials: false,
    };
  }

  // ------ UI state ------
  let backend = $state<Backend>('webdav');
  // WebDAV
  let webdavUrl = $state('');
  let webdavUser = $state('');
  let webdavPassword = $state('');
  // Git
  let gitRepoPath = $state('');
  let gitRemoteUrl = $state('');
  let gitRemoteName = $state('origin');
  let gitRemoteBranch = $state('master');
  let gitAuthorName = $state('AeroTab');
  let gitAuthorEmail = $state('aerotab@localhost');
  let gitRemoteUser = $state('');
  let gitRemotePassword = $state('');
  let gitHttpsSaved = $state(false);
  let gitSshKeyPath = $state('');
  let gitSshPassphrase = $state('');
  let gitSshPort = $state(22);
  let gitAuthMode = $state<'none' | 'https' | 'ssh' | 'oauth_github' | 'oauth_gitlab'>('none');
  let githubOAuthClientId = $state('');
  let gitlabOAuthClientId = $state('');
  let gitlabOAuthBaseUrl = $state('https://gitlab.com');
  let oauthDevice = $state<{
    provider: string;
    userCode: string;
    verificationUri: string;
    deviceCode: string;
    interval: number;
  } | null>(null);
  let oauthPollTimer: ReturnType<typeof setInterval> | null = null;
  // Common
  let masterPassword = $state('');
  let keyringAccount = $state('');
  let masterSecretSaved = $state(false);
  let secretBusy = $state(false);
  let secretInfo = $state('');
  // Vault unlock for credential sync (independent of Settings → Vault page)
  let vaultPassword = $state('');
  let vaultConfirmPassword = $state('');
  let vaultKeyringAccount = $state('sync.vault');
  let vaultSecretSaved = $state(false);
  let vaultBusy = $state(false);
  let vaultInfo = $state('');
  let vaultStatus = $state<{ configured: boolean; initialized: boolean; unlocked: boolean }>({
    configured: false,
    initialized: false,
    unlocked: false,
  });
  let stateDir = $state('');
  let autoSyncEnabled = $state(false);
  let autoSyncMinutes = $state(15);
  let enabledGroups = $state<Record<SyncGroup, boolean>>(defaultEnabledGroups());
  let inspectorGroup = $state<SyncGroup>('Connections');
  let inspectorIds = $state<string[]>([]);
  let inspectorId = $state('');
  let inspectorPayload = $state('');
  let inspectorStatus = $state('');
  let inspectorBusy = $state(false);

  // ------ live status ------
  let configured = $state(false);
  let deviceId = $state<string | null>(null);
  let lastSyncMs = $state<number | null>(null);
  let autoIntervalMs = $state<number | null>(null);
  let kindLive = $state<Backend | null>(null);
  let busy = $state(false);
  let info = $state('');
  let infoTone = $state<'info' | 'ok' | 'err'>('info');

  type SyncStatsPayload = { pushed?: number; pulled?: number; merged?: number; unchanged?: number };

  function setSyncInfo(tone: 'info' | 'ok' | 'err', message: string) {
    infoTone = tone;
    info = message;
  }

  function formatSyncStats(stats: Record<string, unknown> | null | undefined): string {
    const bridge = stats?._bridge as {
      credentials_skipped_locked?: boolean;
      credentials_skipped_uninitialized?: boolean;
    } | undefined;
    const entries = Object.entries(stats ?? {}).filter(([k]) => k !== '_bridge');
    if (entries.length === 0) {
      return i18n.t('sync.complete', { count: 0 });
    }
    let pushed = 0;
    let pulled = 0;
    let merged = 0;
    let unchanged = 0;
    for (const [, raw] of entries) {
      const s = raw as SyncStatsPayload;
      pushed += s.pushed ?? 0;
      pulled += s.pulled ?? 0;
      merged += s.merged ?? 0;
      unchanged += s.unchanged ?? 0;
    }
    const line = i18n.t('sync.statsLine', {
      groups: entries.length,
      pushed,
      pulled,
      merged,
      unchanged,
    });
    const notes: string[] = [];
    if (bridge?.credentials_skipped_locked) {
      notes.push(i18n.t('sync.credentialsSkippedLocked'));
    }
    if (bridge?.credentials_skipped_uninitialized) {
      notes.push(i18n.t('sync.credentialsSkippedUninitialized'));
    }
    if (pushed + pulled + merged === 0 && notes.length === 0) {
      return `${line} ${i18n.t('sync.noRecordsExchanged')}`;
    }
    return notes.length ? `${line} ${notes.join(' ')}` : line;
  }

  let statusTimer: ReturnType<typeof setInterval> | null = null;

  interface SyncHistoryEntry {
    id: string;
    at_ms: number;
    trigger: string;
    ok: boolean;
    error?: string;
    groups: string[];
    pushed: number;
    pulled: number;
    merged: number;
    unchanged: number;
  }
  let syncHistory = $state<SyncHistoryEntry[]>([]);

  function markDirty() { settingsCoord.markDirty(); }

  function fmtTime(ms: number | null): string {
    if (ms == null) return 'never';
    const d = new Date(ms);
    return d.toLocaleString();
  }

  function groupDescription(group: SyncGroup): string {
    switch (group) {
      case 'Connections': return 'Profiles, groups, tags, icons, and connection metadata';
      case 'Appearance': return 'Themes, color scheme, window and visual settings';
      case 'Shortcuts': return 'Custom hotkey bindings';
      case 'PluginCfg': return 'Plugin paths and plugin configuration';
      case 'Credentials': return 'Encrypted Vault-backed passwords and private key material';
    }
  }

  function selectedGroups(): SyncGroup[] {
    return syncGroups.filter((group) => enabledGroups[group]);
  }

  function currentPersistedSettings(): PersistedSyncSettings {
    return {
      backend,
      webdavUrl,
      webdavUser,
      webdavPassword,
      gitRepoPath,
      gitRemoteUrl,
      gitRemoteName,
      gitRemoteBranch,
      gitAuthorName,
      gitAuthorEmail,
      gitRemoteUser,
      gitSshKeyPath,
      gitSshPassphrase,
      gitSshPort,
      gitAuthMode,
      githubOAuthClientId,
      gitlabOAuthClientId,
      gitlabOAuthBaseUrl,
      stateDir,
      autoSyncEnabled,
      autoSyncMinutes,
      enabledGroups,
      keyringAccount,
      vaultKeyringAccount,
      gitHttpsKeyringAccount: 'sync.git.https',
    };
  }

  const GIT_HTTPS_KEYRING = 'sync.git.https';

  async function refreshGitHttpsSecretStatus() {
    try {
      gitHttpsSaved = await hasGitHttpsPassword(rpc, currentPersistedSettings());
    } catch {
      gitHttpsSaved = false;
    }
  }

  async function saveGitHttpsSecret() {
    if (!gitRemotePassword.trim()) {
      setSyncInfo('err', i18n.t('sync.gitHttpsRequired'));
      return;
    }
    secretBusy = true;
    try {
      const snapshot = currentPersistedSettings();
      await saveGitHttpsPassword(rpc, snapshot, gitRemotePassword);
      gitRemotePassword = '';
      await refreshGitHttpsSecretStatus();
      setSyncInfo('ok', i18n.t('sync.gitHttpsSavedHint'));
    } catch (e) {
      onError(`git token save: ${(e as Error).message}`);
    } finally {
      secretBusy = false;
    }
  }

  async function clearGitHttpsSecret() {
    if (!(await appConfirm(i18n.t('sync.gitHttpsForgetConfirm')))) return;
    secretBusy = true;
    try {
      await clearGitHttpsPassword(rpc, currentPersistedSettings());
      await refreshGitHttpsSecretStatus();
      setSyncInfo('info', i18n.t('sync.gitHttpsCleared'));
    } catch (e) {
      onError(`git token clear: ${(e as Error).message}`);
    } finally {
      secretBusy = false;
    }
  }

  async function setGroupEnabled(group: SyncGroup, checked: boolean) {
    let nextChecked = checked;
    if (group === 'Credentials' && checked) {
      nextChecked = await appConfirm(i18n.t('sync.credentialSyncEnableConfirm'));
    }
    enabledGroups = { ...enabledGroups, [group]: nextChecked };
    markDirty();
  }

  async function loadPersisted() {
    try {
      const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'sync' });
      const v = (r.value ?? {}) as Record<string, unknown>;
      if (v.backend === 'webdav' || v.backend === 'git') backend = v.backend;
      if (typeof v.webdavUrl === 'string') webdavUrl = v.webdavUrl;
      if (typeof v.webdavUser === 'string') webdavUser = v.webdavUser;
      if (typeof v.webdavPassword === 'string') webdavPassword = v.webdavPassword;
      if (typeof v.gitRepoPath === 'string') gitRepoPath = v.gitRepoPath;
      if (typeof v.gitRemoteUrl === 'string') gitRemoteUrl = v.gitRemoteUrl;
      if (typeof v.gitRemoteName === 'string') gitRemoteName = v.gitRemoteName;
      if (typeof v.gitRemoteBranch === 'string') gitRemoteBranch = v.gitRemoteBranch;
      if (typeof v.gitAuthorName === 'string') gitAuthorName = v.gitAuthorName;
      if (typeof v.gitAuthorEmail === 'string') gitAuthorEmail = v.gitAuthorEmail;
      if (typeof v.gitRemoteUser === 'string') gitRemoteUser = v.gitRemoteUser;
      // PAT is stored in the OS keyring, not settings.sled (see persist()).
      gitRemotePassword = '';
      if (typeof v.gitSshKeyPath === 'string') gitSshKeyPath = v.gitSshKeyPath;
      if (typeof v.gitSshPassphrase === 'string') gitSshPassphrase = v.gitSshPassphrase;
      if (typeof v.gitSshPort === 'number' && v.gitSshPort > 0) gitSshPort = v.gitSshPort;
      if (
        v.gitAuthMode === 'none' || v.gitAuthMode === 'https' || v.gitAuthMode === 'ssh'
        || v.gitAuthMode === 'oauth_github' || v.gitAuthMode === 'oauth_gitlab'
      ) {
        gitAuthMode = v.gitAuthMode;
      }
      if (typeof v.githubOAuthClientId === 'string') githubOAuthClientId = v.githubOAuthClientId;
      if (typeof v.gitlabOAuthClientId === 'string') gitlabOAuthClientId = v.gitlabOAuthClientId;
      if (typeof v.gitlabOAuthBaseUrl === 'string') gitlabOAuthBaseUrl = v.gitlabOAuthBaseUrl;
      if (typeof v.keyringAccount === 'string') keyringAccount = v.keyringAccount;
      if (typeof v.vaultKeyringAccount === 'string') vaultKeyringAccount = v.vaultKeyringAccount;
      if (typeof v.stateDir === 'string') stateDir = v.stateDir;
      if (typeof v.autoSyncEnabled === 'boolean') autoSyncEnabled = v.autoSyncEnabled;
      if (typeof v.autoSyncMinutes === 'number') autoSyncMinutes = v.autoSyncMinutes;
      if (v.enabledGroups && typeof v.enabledGroups === 'object') {
        const raw = v.enabledGroups as Record<string, unknown>;
        const next = defaultEnabledGroups();
        for (const group of syncGroups) {
          if (typeof raw[group] === 'boolean') next[group] = raw[group];
        }
        enabledGroups = next;
      }
    } catch (e) {
      onError(`sync load: ${(e as Error).message}`);
    }
  }

  async function persist() {
    // Note: this is the SettingsLayout "Save" hook; passwords here are stored
    // alongside the rest of settings (sled). Production deployments should
    // store credentials in the OS keychain via aerotab-core's `secret` module;
    // master password is intentionally excluded.
    await rpc.call('settings.set', {
      key: 'sync',
      value: {
        backend,
        webdavUrl, webdavUser, webdavPassword,
        gitRepoPath, gitRemoteUrl, gitRemoteName, gitRemoteBranch,
        gitAuthorName, gitAuthorEmail,
        gitRemoteUser,
        gitSshKeyPath, gitSshPassphrase, gitSshPort,
        gitAuthMode, githubOAuthClientId, gitlabOAuthClientId, gitlabOAuthBaseUrl,
        keyringAccount, vaultKeyringAccount, gitHttpsKeyringAccount: GIT_HTTPS_KEYRING,
        stateDir, autoSyncEnabled, autoSyncMinutes, enabledGroups,
      },
    });
  }

  async function refreshStatus() {
    try {
      const s = await rpc.call<{
        configured: boolean;
        kind: Backend | null;
        deviceId: string | null;
        lastSyncMs: number | null;
        autoIntervalMs: number | null;
      }>('sync.status', {});
      configured = s.configured;
      kindLive = s.kind;
      deviceId = s.deviceId;
      lastSyncMs = s.lastSyncMs;
      autoIntervalMs = s.autoIntervalMs;
    } catch {
      /* engine not configured yet */
    }
  }

  function secretParams(): { account?: string } {
    const account = keyringAccount.trim();
    return account ? { account } : {};
  }

  async function refreshSecretStatus() {
    secretBusy = true;
    try {
      const r = await rpc.call<{ has: boolean }>('secret.hasMaster', secretParams());
      masterSecretSaved = r.has;
      secretInfo = r.has ? 'saved in OS credential store' : 'not saved';
    } catch (e) {
      secretInfo = '';
      onError(`secret status: ${(e as Error).message}`);
    } finally {
      secretBusy = false;
    }
  }

  async function saveMasterSecret() {
    if (!masterPassword) { secretInfo = 'Enter the master password first'; return; }
    secretBusy = true;
    try {
      await rpc.call('secret.setMaster', { ...secretParams(), secret: masterPassword });
      await refreshSecretStatus();
    } catch (e) {
      onError(`secret save: ${(e as Error).message}`);
    } finally {
      secretBusy = false;
    }
  }

  function vaultSecretParams(): { account?: string } {
    const account = vaultKeyringAccount.trim();
    return account ? { account } : { account: 'sync.vault' };
  }

  async function refreshVaultStatus() {
    try {
      vaultStatus = await rpc.call('vault.status', {});
    } catch {
      vaultStatus = { configured: false, initialized: false, unlocked: false };
    }
  }

  async function refreshVaultSecretStatus() {
    vaultBusy = true;
    try {
      const r = await rpc.call<{ has: boolean }>('secret.hasMaster', vaultSecretParams());
      vaultSecretSaved = r.has;
      vaultInfo = r.has ? i18n.t('sync.vaultSecretSaved') : i18n.t('sync.vaultSecretNotSaved');
    } catch (e) {
      vaultInfo = '';
      onError(`vault secret status: ${(e as Error).message}`);
    } finally {
      vaultBusy = false;
    }
  }

  async function saveVaultSecret() {
    if (!vaultPassword) {
      vaultInfo = i18n.t('sync.vaultPasswordRequired');
      return;
    }
    vaultBusy = true;
    try {
      await rpc.call('secret.setMaster', { ...vaultSecretParams(), secret: vaultPassword });
      await refreshVaultSecretStatus();
    } catch (e) {
      onError(`vault secret save: ${(e as Error).message}`);
    } finally {
      vaultBusy = false;
    }
  }

  async function clearVaultSecret() {
    if (!(await appConfirm(i18n.t('sync.vaultForgetConfirm')))) return;
    vaultBusy = true;
    try {
      await rpc.call('secret.clearMaster', vaultSecretParams());
      await refreshVaultSecretStatus();
    } catch (e) {
      onError(`vault secret clear: ${(e as Error).message}`);
    } finally {
      vaultBusy = false;
    }
  }

  async function unlockVaultNow() {
    vaultBusy = true;
    try {
      const r = await rpc.call<{ unlocked: boolean; initialized: boolean }>('sync.ensureVaultUnlock', {
        password: vaultPassword || undefined,
        account: vaultKeyringAccount.trim() || undefined,
      });
      if (!r.initialized) {
        vaultInfo = i18n.t('sync.vaultNotInitialized');
        return;
      }
      if (r.unlocked) {
        vaultPassword = '';
        vaultInfo = i18n.t('sync.vaultUnlocked');
        await refreshVaultStatus();
      } else {
        vaultInfo = i18n.t('sync.vaultUnlockFailed');
      }
    } catch (e) {
      onError(`vault unlock: ${(e as Error).message}`);
    } finally {
      vaultBusy = false;
    }
  }

  async function initializeVaultForSync() {
    if (!vaultPassword) {
      vaultInfo = i18n.t('sync.vaultPasswordRequired');
      return;
    }
    if (vaultPassword !== vaultConfirmPassword) {
      vaultInfo = i18n.t('sync.vaultPasswordMismatch');
      return;
    }
    vaultBusy = true;
    try {
      await rpc.call('vault.initialize', { password: vaultPassword });
      await rpc.call('vault.unlock', { password: vaultPassword });
      vaultPassword = '';
      vaultConfirmPassword = '';
      vaultInfo = i18n.t('sync.vaultUnlocked');
      await refreshVaultStatus();
    } catch (e) {
      onError(`vault initialize: ${(e as Error).message}`);
    } finally {
      vaultBusy = false;
    }
  }

  async function lockVaultForSync() {
    try {
      await rpc.call('vault.lock', {});
      vaultInfo = i18n.t('sync.vaultLocked');
      await refreshVaultStatus();
    } catch (e) {
      onError(`vault lock: ${(e as Error).message}`);
    }
  }

  /** Unlock vault when Credentials group is enabled (keyring or typed password). */
  async function ensureVaultForSync(): Promise<boolean> {
    if (!enabledGroups.Credentials) return true;
    await refreshVaultStatus();
    if (vaultStatus.unlocked) return true;
    try {
      const r = await rpc.call<{ unlocked: boolean; initialized: boolean }>('sync.ensureVaultUnlock', {
        password: vaultPassword || undefined,
        account: vaultKeyringAccount.trim() || undefined,
      });
      if (r.unlocked) {
        vaultPassword = '';
        await refreshVaultStatus();
        return true;
      }
      if (!r.initialized) {
        setSyncInfo('err', i18n.t('sync.vaultNotInitialized'));
      } else {
        setSyncInfo('err', i18n.t('sync.vaultUnlockRequired'));
      }
      return false;
    } catch (e) {
      setSyncInfo('err', (e as Error).message);
      return false;
    }
  }

  async function clearMasterSecret() {
    if (!(await appConfirm(i18n.t('sync.masterPasswordForgetConfirm')))) return;
    secretBusy = true;
    try {
      await rpc.call('secret.clearMaster', secretParams());
      await refreshSecretStatus();
    } catch (e) {
      onError(`secret clear: ${(e as Error).message}`);
    } finally {
      secretBusy = false;
    }
  }

  async function masterPasswordForConfigure(): Promise<string | null> {
    if (masterPassword) return masterPassword;
    if (!masterSecretSaved) return null;
    const r = await rpc.call<{ secret: string }>('secret.getMaster', secretParams());
    return r.secret;
  }

  async function configure() {
    const passwordForConfigure = await masterPasswordForConfigure().catch((e) => {
      onError(`secret read: ${(e as Error).message}`);
      return null;
    });
    if (!passwordForConfigure) {
      setSyncInfo('err', i18n.t('sync.masterRequired'));
      return;
    }
    if (backend === 'webdav' && !webdavUrl) {
      setSyncInfo('err', 'WebDAV URL is required');
      return;
    }
    if (backend === 'git' && !gitRepoPath) {
      setSyncInfo('err', 'Git repo path is required');
      return;
    }
    busy = true;
    setSyncInfo('info', i18n.t('sync.configuring'));
    try {
      const snapshot = currentPersistedSettings();
      if (gitAuthMode === 'https') {
        if (gitRemotePassword.trim()) {
          await saveGitHttpsPassword(rpc, snapshot, gitRemotePassword);
          gitRemotePassword = '';
          await refreshGitHttpsSecretStatus();
        }
        const token = await loadGitHttpsPassword(rpc, snapshot);
        if (!token) {
          setSyncInfo('err', i18n.t('sync.gitHttpsRequired'));
          return;
        }
      }
      await configureSyncEngineFromSettings(rpc, snapshot, passwordForConfigure);
      if (masterPassword.trim()) {
        await rpc.call('secret.setMaster', { ...secretParams(), secret: masterPassword });
        masterPassword = '';
        await refreshSecretStatus();
      }
      await persist();
      await applyPersistedAutoSync(rpc, snapshot, { force: true });
      setSyncInfo('ok', i18n.t('sync.engineConfigured'));
      await refreshStatus();
      await loadSyncHistory();
    } catch (e) {
      const message = (e as Error).message;
      setSyncInfo('err', i18n.t('sync.configureFailed', { message }));
      onError(`sync configure: ${message}`);
    } finally {
      busy = false;
    }
  }

  async function syncNow() {
    try {
      await ensureSyncEngineConfigured(rpc);
      await refreshStatus();
      if (!configured) {
        setSyncInfo('err', i18n.t('sync.notConfiguredHint'));
        return;
      }
    } catch (e) {
      const msg = (e as Error).message ?? '';
      if (msg.includes('HTTPS token')) {
        setSyncInfo('err', i18n.t('sync.gitHttpsRequired'));
        return;
      }
      if (msg.includes('master password') || msg.includes('credential store')) {
        setSyncInfo('err', i18n.t('sync.masterKeyringRequired'));
        return;
      }
      setSyncInfo('err', msg);
      return;
    }
    const groups = selectedGroups();
    if (groups.length === 0) {
      setSyncInfo('err', i18n.t('sync.noGroups'));
      return;
    }
    if (!(await ensureVaultForSync())) return;
    busy = true;
    setSyncInfo('info', i18n.t('sync.syncing'));
    try {
      const stats = await rpc.call<Record<string, unknown>>('sync.now', { groups });
      setSyncInfo('ok', formatSyncStats(stats));
      await refreshStatus();
      await loadSyncHistory();
      await onSyncApplied?.();
    } catch (e) {
      const message = (e as Error).message;
      setSyncInfo('err', i18n.t('sync.failed', { message }));
      onError(`sync now: ${message}`);
    } finally {
      busy = false;
    }
  }

  async function loadSyncHistory() {
    try {
      syncHistory = await rpc.call<SyncHistoryEntry[]>('sync.history', {});
    } catch {
      syncHistory = [];
    }
  }

  async function clearSyncHistory() {
    if (!(await appConfirm(i18n.t('sync.historyClearConfirm'), { danger: true, confirmLabel: i18n.t('common.delete') }))) return;
    try {
      await rpc.call('sync.historyClear', {});
      syncHistory = [];
    } catch (e) {
      onError(`sync history clear: ${(e as Error).message}`);
    }
  }

  async function applyAutoSync() {
    try {
      await persist();
      await applyPersistedAutoSync(rpc, currentPersistedSettings(), { force: true });
      await refreshStatus();
      setSyncInfo('ok', i18n.t('sync.autoSyncApplied'));
    } catch (e) {
      onError(`auto sync: ${(e as Error).message}`);
    }
  }

  function onAutoSyncSettingsChanged() {
    markDirty();
  }

  async function listSyncRecords() {
    inspectorBusy = true;
    inspectorStatus = '';
    try {
      inspectorIds = await rpc.call<string[]>('sync.list', { group: inspectorGroup });
      if (!inspectorIds.includes(inspectorId)) inspectorId = inspectorIds[0] ?? '';
      inspectorStatus = `${inspectorIds.length} local record${inspectorIds.length === 1 ? '' : 's'}`;
    } catch (e) {
      onError(`sync list: ${(e as Error).message}`);
    } finally {
      inspectorBusy = false;
    }
  }

  async function readSyncRecord(id = inspectorId) {
    if (!id) return;
    inspectorBusy = true;
    inspectorPayload = '';
    try {
      const r = await rpc.call<{ data: string } | null>('sync.get', { group: inspectorGroup, id });
      if (!r?.data) {
        inspectorStatus = 'record not found';
        return;
      }
      const bytes = b64decode(r.data);
      inspectorPayload = new TextDecoder().decode(bytes);
      inspectorStatus = `read ${bytes.length} byte${bytes.length === 1 ? '' : 's'}`;
    } catch (e) {
      onError(`sync get: ${(e as Error).message}`);
    } finally {
      inspectorBusy = false;
    }
  }

  async function writeSyncRecord() {
    if (!inspectorId.trim()) { inspectorStatus = 'Record id is required'; return; }
    if (!(await appConfirm(i18n.t('sync.recordWriteConfirm')))) return;
    inspectorBusy = true;
    try {
      await rpc.call('sync.put', {
        group: inspectorGroup,
        id: inspectorId.trim(),
        data: b64encode(new TextEncoder().encode(inspectorPayload)),
      });
      inspectorStatus = 'record written';
      await listSyncRecords();
    } catch (e) {
      onError(`sync put: ${(e as Error).message}`);
    } finally {
      inspectorBusy = false;
    }
  }

  function stopOAuthPoll() {
    if (oauthPollTimer) {
      clearInterval(oauthPollTimer);
      oauthPollTimer = null;
    }
  }

  function oauthClientId(provider: 'github' | 'gitlab'): string {
    return (provider === 'github' ? githubOAuthClientId : gitlabOAuthClientId).trim();
  }

  async function startOAuthDevice(provider: 'github' | 'gitlab') {
    const clientId = oauthClientId(provider);
    if (!clientId) {
      info = provider === 'github' ? 'GitHub client id required' : 'GitLab application id required';
      return;
    }
    stopOAuthPoll();
    busy = true;
    info = 'Starting device flow…';
    try {
      const start = await rpc.call<{
        deviceCode: string;
        userCode: string;
        verificationUri: string;
        interval: number;
      }>('sync.oauthDeviceStart', {
        provider,
        client_id: clientId,
        gitlab_base_url: provider === 'gitlab' ? gitlabOAuthBaseUrl : undefined,
      });
      oauthDevice = {
        provider,
        userCode: start.userCode,
        verificationUri: start.verificationUri,
        deviceCode: start.deviceCode,
        interval: start.interval || 5,
      };
      info = 'Waiting for authorization…';
      const tickMs = Math.max(3, oauthDevice.interval) * 1000;
      oauthPollTimer = setInterval(() => { void pollOAuthDevice(provider); }, tickMs);
      void pollOAuthDevice(provider);
    } catch (e) {
      oauthDevice = null;
      info = '';
      onError(`oauth start: ${(e as Error).message}`);
    } finally {
      busy = false;
    }
  }

  async function pollOAuthDevice(provider: 'github' | 'gitlab') {
    if (!oauthDevice || oauthDevice.provider !== provider) return;
    const clientId = oauthClientId(provider);
    if (!clientId) return;
    try {
      const r = await rpc.call<{ status: string }>('sync.oauthDevicePoll', {
        provider,
        client_id: clientId,
        device_code: oauthDevice.deviceCode,
        gitlab_base_url: provider === 'gitlab' ? gitlabOAuthBaseUrl : undefined,
      });
      if (r.status === 'ok') {
        stopOAuthPoll();
        oauthDevice = null;
        info = `${provider} OAuth connected`;
      } else if (r.status === 'slow_down' && oauthPollTimer) {
        stopOAuthPoll();
        const slower = Math.max(10, (oauthDevice?.interval ?? 5) + 5);
        oauthPollTimer = setInterval(() => { void pollOAuthDevice(provider); }, slower * 1000);
      }
    } catch (e) {
      stopOAuthPoll();
      oauthDevice = null;
      onError(`oauth poll: ${(e as Error).message}`);
    }
  }

  async function clearOAuth(provider: 'github' | 'gitlab') {
    stopOAuthPoll();
    oauthDevice = null;
    try {
      await rpc.call('sync.oauthClear', { provider });
      info = `${provider} token cleared`;
    } catch (e) {
      onError(`oauth clear: ${(e as Error).message}`);
    }
  }

  async function deleteSyncRecord(id = inspectorId) {
    if (!id) return;
    if (!(await appConfirm(i18n.t('sync.recordDeleteConfirm', { id }), { danger: true, confirmLabel: i18n.t('common.delete') }))) return;
    inspectorBusy = true;
    try {
      await rpc.call('sync.delete', { group: inspectorGroup, id });
      inspectorPayload = '';
      inspectorStatus = 'record deleted';
      await listSyncRecords();
    } catch (e) {
      onError(`sync delete: ${(e as Error).message}`);
    } finally {
      inspectorBusy = false;
    }
  }

  onMount(() => {
    settingsCoord.registerSaver('configsync', persist);
    void (async () => {
      await loadPersisted();
      await refreshSecretStatus();
      await refreshVaultStatus();
      await refreshVaultSecretStatus();
      await refreshGitHttpsSecretStatus();
      if (await isSyncEngineConfigured(rpc)) {
        await refreshStatus();
      } else {
        const boot = await bootstrapSyncEngine(rpc);
        if (boot === 'configured') {
          setSyncInfo('ok', i18n.t('sync.engineRestored'));
        }
        await refreshStatus();
      }
      await loadSyncHistory();
    })();
    statusTimer = setInterval(async () => {
      await refreshStatus();
      await loadSyncHistory();
    }, 5000);
  });
  onDestroy(() => {
    settingsCoord.unregisterSaver('configsync');
    if (statusTimer) clearInterval(statusTimer);
    stopOAuthPoll();
  });
</script>

<div class="settings-section">
  <h2>Config Sync</h2>

  <div class="text-[12px] text-[var(--color-fg-muted)]">
    Status:
    {#if configured}
      <span class="text-[var(--color-accent)]">configured</span>
      &middot; backend <code>{kindLive ?? '?'}</code>
      &middot; device <code class="text-[10px]">{deviceId?.slice(0, 8) ?? '?'}</code>
      &middot; last sync <strong>{fmtTime(lastSyncMs)}</strong>
      {#if autoIntervalMs}
        &middot; auto every {Math.round(autoIntervalMs / 60_000)} min
      {/if}
    {:else}
      <span class="text-[var(--color-fg-muted)]">not configured</span>
    {/if}
  </div>

  <div>
    <div class="section-h">Backend</div>
    <label for="cs-backend" class="lbl">Type</label>
    <select id="cs-backend" bind:value={backend} onchange={markDirty} class="select">
      <option value="webdav">WebDAV</option>
      <option value="git">Git</option>
    </select>
  </div>

  <div>
    <div class="section-h">Groups</div>
    <div class="group-grid">
      {#each syncGroups as group (group)}
        <label class="group-card" class:enabled={enabledGroups[group]} class:credentials={group === 'Credentials'}>
          <input
            type="checkbox"
            checked={enabledGroups[group]}
            onchange={(e) => { void setGroupEnabled(group, (e.currentTarget as HTMLInputElement).checked); }}
          />
          <span class="group-main">
            <span class="group-title">{group}</span>
            <span class="group-desc">{groupDescription(group)}</span>
          </span>
        </label>
      {/each}
    </div>
    <div class="help">
      Credentials are disabled by default. Enable them only when you want encrypted Vault-backed secrets to sync across devices.
    </div>
  </div>

  {#if backend === 'webdav'}
    <div>
      <div class="section-h">WebDAV</div>
      <label for="cs-url" class="lbl">Base URL</label>
      <input
        id="cs-url" bind:value={webdavUrl} oninput={markDirty}
        placeholder="https://webdav.example.com/aerotab" class="input"
      />
      <label for="cs-user" class="lbl">Username (optional)</label>
      <input id="cs-user" bind:value={webdavUser} oninput={markDirty} class="input" />
      <label for="cs-pw" class="lbl">Password (optional)</label>
      <input id="cs-pw" type="password"
        bind:value={webdavPassword} oninput={markDirty} class="input" />
    </div>
  {:else}
    <div>
      <div class="section-h">Git</div>
      <label for="cs-repo" class="lbl">Local repo path</label>
      <input id="cs-repo" bind:value={gitRepoPath} oninput={markDirty}
        placeholder="/home/me/.aerotab-sync" class="input" />
      <label for="cs-remote-url" class="lbl">Remote URL (optional)</label>
      <input id="cs-remote-url" bind:value={gitRemoteUrl} oninput={markDirty}
        placeholder="https://github.com/me/aerotab-config.git" class="input" />
      <div class="grid grid-cols-2 gap-3">
        <div>
          <label for="cs-remote-name" class="lbl">Remote name</label>
          <input id="cs-remote-name" bind:value={gitRemoteName} oninput={markDirty} class="input" />
        </div>
        <div>
          <label for="cs-remote-branch" class="lbl">Branch</label>
          <input id="cs-remote-branch" bind:value={gitRemoteBranch} oninput={markDirty} class="input" />
        </div>
      </div>
      <div class="grid grid-cols-2 gap-3">
        <div>
          <label for="cs-author-name" class="lbl">Author name</label>
          <input id="cs-author-name" bind:value={gitAuthorName} oninput={markDirty} class="input" />
        </div>
        <div>
          <label for="cs-author-email" class="lbl">Author email</label>
          <input id="cs-author-email" bind:value={gitAuthorEmail} oninput={markDirty} class="input" />
        </div>
      </div>
      <label for="cs-auth-mode" class="lbl">Remote auth</label>
      <select id="cs-auth-mode" bind:value={gitAuthMode} onchange={markDirty} class="select">
        <option value="none">None / system credential helper</option>
        <option value="https">HTTPS user/password</option>
        <option value="ssh">SSH key</option>
        <option value="oauth_github">GitHub OAuth (device)</option>
        <option value="oauth_gitlab">GitLab OAuth (device)</option>
      </select>
      {#if gitAuthMode === 'oauth_github' || gitAuthMode === 'oauth_gitlab'}
        {#if gitAuthMode === 'oauth_github'}
          <label class="lbl" for="cs-github-client">GitHub OAuth client id</label>
          <input id="cs-github-client" bind:value={githubOAuthClientId} oninput={markDirty} class="input"
                 placeholder="GitHub OAuth App client id" />
        {:else}
          <label class="lbl" for="cs-gitlab-client">GitLab OAuth application id</label>
          <input id="cs-gitlab-client" bind:value={gitlabOAuthClientId} oninput={markDirty} class="input" />
          <label class="lbl" for="cs-gitlab-base">GitLab base URL</label>
          <input id="cs-gitlab-base" bind:value={gitlabOAuthBaseUrl} oninput={markDirty} class="input" />
        {/if}
        <div class="oauth-row">
          <button type="button" class="btn-secondary" disabled={busy}
                  onclick={() => { void startOAuthDevice(gitAuthMode === 'oauth_github' ? 'github' : 'gitlab'); }}>
            Sign in (device flow)
          </button>
          <button type="button" class="btn-secondary" disabled={busy}
                  onclick={() => { void clearOAuth(gitAuthMode === 'oauth_github' ? 'github' : 'gitlab'); }}>
            Clear token
          </button>
        </div>
        {#if oauthDevice}
          <div class="oauth-hint">
            Open <a href={oauthDevice.verificationUri} target="_blank" rel="noreferrer">{oauthDevice.verificationUri}</a>
            and enter code <strong>{oauthDevice.userCode}</strong>
          </div>
        {/if}
      {:else if gitAuthMode === 'https'}
        <label for="cs-r-user" class="lbl">Remote username</label>
        <input id="cs-r-user" bind:value={gitRemoteUser} oninput={markDirty} class="input"
          placeholder="GitLab: oauth2 · GitHub: x-access-token" />
        <label for="cs-r-pass" class="lbl">Remote password / token</label>
        <input id="cs-r-pass" type="password"
          bind:value={gitRemotePassword} oninput={markDirty} class="input" />
        <div class="help">{i18n.t('sync.gitHttpsAuthHelp')}</div>
        {#if gitHttpsSaved && !gitRemotePassword}
          <div class="help text-[var(--color-success)]">{i18n.t('sync.gitHttpsSavedHint')}</div>
        {/if}
        <div class="flex gap-2 flex-wrap pt-2">
          <button type="button" class="btn-secondary" disabled={secretBusy}
                  onclick={() => { void saveGitHttpsSecret(); }}>
            {i18n.t('sync.gitHttpsSave')}
          </button>
          <button type="button" class="btn-secondary" disabled={secretBusy || !gitHttpsSaved}
                  onclick={() => { void clearGitHttpsSecret(); }}>
            {i18n.t('sync.gitHttpsClear')}
          </button>
        </div>
      {:else if gitAuthMode === 'ssh'}
        <label for="cs-ssh-port" class="lbl">{i18n.t('sync.gitSshPort')}</label>
        <input id="cs-ssh-port" type="number" min="1" max="65535" bind:value={gitSshPort} oninput={markDirty}
          class="input" />
        <div class="help">{i18n.t('sync.gitSshPortHelp')}</div>
        <label for="cs-ssh-key" class="lbl">SSH private-key path</label>
        <input id="cs-ssh-key" bind:value={gitSshKeyPath} oninput={markDirty}
          placeholder="/home/me/.ssh/id_ed25519" class="input" />
        <label for="cs-ssh-pp" class="lbl">SSH passphrase (optional)</label>
        <input id="cs-ssh-pp" type="password"
          bind:value={gitSshPassphrase} oninput={markDirty} class="input" />
      {/if}
    </div>
  {/if}

  <div>
    <div class="section-h">Encryption</div>
    <label for="cs-mp" class="lbl">
      Master password (required, not persisted)
    </label>
    <input id="cs-mp" type="password" bind:value={masterPassword} class="input"
      placeholder="Enter to (re-)configure the engine" />
    <div class="help">
      {i18n.t('sync.masterPasswordHelp')}
    </div>
    <label for="cs-keyring-account" class="lbl">Credential account (optional)</label>
    <input id="cs-keyring-account" bind:value={keyringAccount} class="input"
      placeholder="default: sync.master" />
    <div class="flex gap-2 items-center pt-2 flex-wrap">
      <button type="button" class="btn-secondary" disabled={secretBusy || !masterPassword}
        onclick={() => void saveMasterSecret()}>
        Save to OS credential store
      </button>
      <button type="button" class="btn-secondary" disabled={secretBusy}
        onclick={() => void refreshSecretStatus()}>
        Check saved credential
      </button>
      <button type="button" class="btn-secondary" disabled={secretBusy || !masterSecretSaved}
        onclick={() => void clearMasterSecret()}>
        Forget saved credential
      </button>
      {#if secretInfo}
        <span class="text-[12px] text-[var(--color-fg-muted)]">{secretInfo}</span>
      {/if}
    </div>
    {#if masterSecretSaved && !masterPassword}
      <div class="help">Configure / re-key will use the saved credential for this run.</div>
    {/if}
  </div>

  {#if enabledGroups.Credentials}
    <div>
      <div class="section-h">{i18n.t('sync.vaultSectionTitle')}</div>
      <div class="text-[12px] text-[var(--color-fg-muted)] mb-2">
        {i18n.t('sync.vaultSectionHelp')}
        {#if vaultStatus.unlocked}
          <span class="text-[var(--color-accent)]"> · {i18n.t('sync.vaultStatusUnlocked')}</span>
        {:else if vaultStatus.initialized}
          <span> · {i18n.t('sync.vaultStatusLocked')}</span>
        {:else}
          <span> · {i18n.t('sync.vaultStatusNotInitialized')}</span>
        {/if}
      </div>

      {#if !vaultStatus.initialized}
        <label for="cs-vault-pw-init" class="lbl">{i18n.t('sync.vaultPassword')}</label>
        <input id="cs-vault-pw-init" type="password" bind:value={vaultPassword} class="input" />
        <label for="cs-vault-pw2" class="lbl">{i18n.t('sync.vaultPasswordConfirm')}</label>
        <input id="cs-vault-pw2" type="password" bind:value={vaultConfirmPassword} class="input" />
        <button type="button" class="btn-secondary mt-2" disabled={vaultBusy}
          onclick={() => void initializeVaultForSync()}>
          {i18n.t('sync.vaultInitialize')}
        </button>
      {:else}
        <label for="cs-vault-pw" class="lbl">{i18n.t('sync.vaultPassword')}</label>
        <input id="cs-vault-pw" type="password" bind:value={vaultPassword} class="input"
          placeholder={i18n.t('sync.vaultPasswordPlaceholder')} />
        <label for="cs-vault-keyring" class="lbl">{i18n.t('sync.vaultKeyringAccount')}</label>
        <input id="cs-vault-keyring" bind:value={vaultKeyringAccount} oninput={markDirty} class="input"
          placeholder="sync.vault" />
        <div class="flex gap-2 items-center pt-2 flex-wrap">
          <button type="button" class="btn-secondary" disabled={vaultBusy}
            onclick={() => void unlockVaultNow()}>
            {i18n.t('sync.vaultUnlock')}
          </button>
          <button type="button" class="btn-secondary" disabled={vaultBusy || !vaultStatus.unlocked}
            onclick={() => void lockVaultForSync()}>
            {i18n.t('sync.vaultLock')}
          </button>
          <button type="button" class="btn-secondary" disabled={vaultBusy || !vaultPassword}
            onclick={() => void saveVaultSecret()}>
            {i18n.t('sync.vaultSaveToKeyring')}
          </button>
          <button type="button" class="btn-secondary" disabled={vaultBusy}
            onclick={() => void refreshVaultSecretStatus()}>
            {i18n.t('sync.vaultCheckKeyring')}
          </button>
          <button type="button" class="btn-secondary" disabled={vaultBusy || !vaultSecretSaved}
            onclick={() => void clearVaultSecret()}>
            {i18n.t('sync.vaultForgetKeyring')}
          </button>
        </div>
        {#if vaultInfo}
          <div class="help mt-1">{vaultInfo}</div>
        {/if}
        {#if vaultSecretSaved && !vaultPassword}
          <div class="help">{i18n.t('sync.vaultKeyringAutoHint')}</div>
        {/if}
      {/if}
    </div>
  {/if}

  <div>
    <div class="section-h">Local state</div>
    <label for="cs-statedir" class="lbl">State directory (optional)</label>
    <input id="cs-statedir" bind:value={stateDir} oninput={markDirty}
      placeholder="(default: in-memory only)" class="input" />
    <div class="help">
      When set, the local sync state survives process restart so reconciliation
      can resume incrementally.
    </div>
  </div>

  <div>
    <div class="section-h">Auto sync</div>
    <label class="row">
      <input type="checkbox" bind:checked={autoSyncEnabled}
        onchange={onAutoSyncSettingsChanged} />
      {i18n.t('sync.autoSyncEvery')}
      <input
        type="number" min="1" max="1440"
        bind:value={autoSyncMinutes}
        onchange={onAutoSyncSettingsChanged}
        class="input" style="width: 70px; display: inline-block; margin: 0 4px;"
      />
      {i18n.t('sync.autoSyncMinutes')}
    </label>
    <div class="help">{i18n.t('sync.autoSyncHelp')}</div>
    {#if configured && autoIntervalMs}
      <div class="help text-[var(--color-accent)]">
        {i18n.t('sync.autoSyncActive', { minutes: Math.round(autoIntervalMs / 60_000) })}
      </div>
    {/if}
  </div>

  <div>
    <div class="section-h flex items-center justify-between gap-2">
      <span>{i18n.t('sync.historyTitle')}</span>
      <button type="button" class="btn-secondary text-[11px] py-1 px-2"
              disabled={syncHistory.length === 0}
              onclick={() => { void clearSyncHistory(); }}>
        {i18n.t('sync.historyClear')}
      </button>
    </div>
    {#if syncHistory.length === 0}
      <div class="help">{i18n.t('sync.historyEmpty')}</div>
    {:else}
      <div class="sync-history-table">
        <table>
          <thead>
            <tr>
              <th>{i18n.t('sync.historyColTime')}</th>
              <th>{i18n.t('sync.historyColTrigger')}</th>
              <th>{i18n.t('sync.historyColResult')}</th>
              <th>{i18n.t('sync.historyColStats')}</th>
            </tr>
          </thead>
          <tbody>
            {#each syncHistory as row (row.id)}
              <tr class:sync-history-err={!row.ok}>
                <td>{new Date(row.at_ms).toLocaleString()}</td>
                <td>{row.trigger === 'auto' ? i18n.t('sync.historyTriggerAuto') : i18n.t('sync.historyTriggerManual')}</td>
                <td>
                  {#if row.ok}
                    <span class="text-[var(--color-success)]">{i18n.t('sync.historyOk')}</span>
                  {:else}
                    <span class="text-[var(--color-danger)]" title={row.error ?? ''}>{i18n.t('sync.historyFailed')}</span>
                  {/if}
                </td>
                <td class="text-[11px]">
                  {#if row.ok}
                    +{row.pushed} / ↓{row.pulled} / ⇄{row.merged} / ={row.unchanged}
                  {:else}
                    <span class="truncate max-w-[200px] inline-block align-bottom" title={row.error}>{row.error}</span>
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </div>

  <div class="flex gap-2 items-center pt-2">
    <button type="button" class="btn-primary" disabled={busy} onclick={() => void configure()}>
      <Play size={12} /> Configure / re-key
    </button>
    <button type="button" class="btn-secondary" disabled={busy || !configured}
      onclick={() => void syncNow()}>
      <RefreshCw size={12} /> Sync now
    </button>
    <button type="button" class="btn-secondary" disabled={busy || !configured}
      onclick={() => void applyAutoSync()}>
      {i18n.t('sync.autoSyncApply')}
    </button>
  </div>
  {#if info}
    <p class="sync-status" class:sync-status-ok={infoTone === 'ok'} class:sync-status-err={infoTone === 'err'}>
      {info}
    </p>
  {/if}

  <div>
    <div class="section-h">Advanced local records</div>
    <div class="help">
      Inspect or repair local sync records for the configured engine. These controls are intended for recovery.
    </div>
    <div class="grid grid-cols-[160px_1fr_auto] gap-2 items-end">
      <div>
        <label for="cs-inspector-group" class="lbl">Group</label>
        <select id="cs-inspector-group" bind:value={inspectorGroup} class="select">
          {#each syncGroups as group (group)}
            <option value={group}>{group}</option>
          {/each}
        </select>
      </div>
      <div>
        <label for="cs-inspector-id" class="lbl">Record id</label>
        <input id="cs-inspector-id" bind:value={inspectorId} class="input"
          placeholder="uuid" list="cs-sync-records" />
        <datalist id="cs-sync-records">
          {#each inspectorIds as id (id)}
            <option value={id}></option>
          {/each}
        </datalist>
      </div>
      <button type="button" class="btn-secondary" disabled={inspectorBusy || !configured}
        onclick={() => void listSyncRecords()}>
        <RefreshCw size={12} /> List
      </button>
    </div>
    {#if inspectorIds.length > 0}
      <div class="mt-2 border border-[var(--color-border-soft)] rounded divide-y divide-[var(--color-border-soft)] max-h-28 overflow-auto">
        {#each inspectorIds as id (id)}
          <button type="button" class="w-full text-left px-2 py-1 text-[11px] font-mono hover:bg-[var(--color-panel-2)]"
            onclick={() => { inspectorId = id; void readSyncRecord(id); }}>
            {id}
          </button>
        {/each}
      </div>
    {/if}
    <label for="cs-inspector-payload" class="lbl">Payload text</label>
    <textarea id="cs-inspector-payload" bind:value={inspectorPayload} rows="5" class="input font-mono text-[11px]"
      placeholder="Read a record, or enter text to write as a local sync payload"></textarea>
    <div class="flex gap-2 items-center pt-2 flex-wrap">
      <button type="button" class="btn-secondary" disabled={inspectorBusy || !configured || !inspectorId.trim()}
        onclick={() => void readSyncRecord()}>
        Read
      </button>
      <button type="button" class="btn-secondary" disabled={inspectorBusy || !configured || !inspectorId.trim()}
        onclick={() => void writeSyncRecord()}>
        Write
      </button>
      <button type="button" class="btn-secondary" disabled={inspectorBusy || !configured || !inspectorId.trim()}
        onclick={() => void deleteSyncRecord()}>
        Delete
      </button>
      {#if inspectorStatus}
        <span class="text-[12px] text-[var(--color-fg-muted)]">{inspectorStatus}</span>
      {/if}
    </div>
  </div>
</div>

<style>
  .group-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 8px;
  }
  .group-card {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 8px;
    border: 1px solid var(--color-border-soft);
    border-radius: 6px;
    background: var(--color-panel-2);
    color: var(--color-fg);
  }
  .group-card.enabled {
    border-color: color-mix(in srgb, var(--color-accent) 55%, var(--color-border-soft));
  }
  .group-card.credentials.enabled {
    border-color: color-mix(in srgb, var(--color-danger) 55%, var(--color-border-soft));
  }
  .group-main {
    min-width: 0;
    display: grid;
    gap: 2px;
  }
  .group-title {
    font-size: 12.5px;
    font-weight: 600;
  }
  .group-desc {
    font-size: 11px;
    color: var(--color-fg-muted);
    line-height: 1.25;
  }
  .oauth-row {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    margin-top: 8px;
  }
  .oauth-hint {
    margin-top: 8px;
    font-size: 12px;
    color: var(--color-fg-muted);
    line-height: 1.35;
  }
  .sync-status {
    margin: 8px 0 0;
    font-size: 12px;
    line-height: 1.45;
    color: var(--color-fg-muted);
  }
  .sync-status-ok {
    color: var(--color-accent);
  }
  .sync-status-err {
    color: var(--color-danger, #f85149);
  }
  .sync-history-table {
    max-height: 220px;
    overflow: auto;
    border: 1px solid var(--color-border-soft);
    border-radius: 6px;
  }
  .sync-history-table table {
    width: 100%;
    border-collapse: collapse;
    font-size: 11px;
  }
  .sync-history-table th,
  .sync-history-table td {
    padding: 6px 8px;
    text-align: left;
    border-bottom: 1px solid var(--color-border-soft);
  }
  .sync-history-table th {
    position: sticky;
    top: 0;
    background: var(--color-panel-2);
    color: var(--color-fg-muted);
    font-weight: 600;
  }
  .sync-history-table tr.sync-history-err td {
    background: color-mix(in srgb, var(--color-danger) 8%, transparent);
  }
</style>
