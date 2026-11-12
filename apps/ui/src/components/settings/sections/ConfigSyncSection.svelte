<script lang="ts">
  // Config Sync (M11) — UI on top of the existing sync.* IPC.
  //
  // Persists non-secret configuration in settings key `sync`. Master password
  // is stored in the OS keyring (not in settings); the app restores the sync
  // engine on launch via bootstrapSyncEngine().

  import { onMount, onDestroy } from 'svelte';
  import { wizardCanEnter, syncSetupComplete, syncTargetFingerprint, hasSyncApproval } from '../../../lib/syncWizard';
  import { RefreshCw, Play } from '@lucide/svelte';
  import { b64decode, b64encode, type RpcClient } from '../../../lib/rpc';
  import { i18n } from '../../../lib/i18n.svelte';
  import { appConfirm } from '../../../lib/confirm.svelte';
  import { settingsCoord } from '../../../lib/settingsStore.svelte';
  import {
    loadPersistedSyncSettings,
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

  type Backend = 'webdav' | 'git' | 's3' | 'drive';
  let s3Endpoint = $state('');
  let s3Region = $state('us-east-1');
  let s3Bucket = $state('');
  let s3Prefix = $state('aerotab');
  let s3PathStyle = $state(true);
  let s3AccessKey = $state('');
  let s3SecretKey = $state('');
  let s3SessionToken = $state('');
  let driveClientId = $state('');
  let drivePrefix = $state('aerotab');
  let firstSyncApproved = $state(false);
  let wizardStep = $state(0);
  let tested = $state(false);
  let configuredTarget = $state('');
  let approvedTarget = $state('');
  let previewAccepted = $state(false);
  let previewToken = $state<string | null>(null);
  const wizardState = $derived.by(() => ({
    storage: backend === 'git' ? Boolean(gitRepoPath.trim()) : backend === 'webdav' ? /^https:\/\//.test(webdavUrl) : backend === 's3' ? Boolean(/^https:\/\//.test(s3Endpoint) && s3Bucket && s3Region) : Boolean(driveClientId),
    tested, password: Boolean(masterPassword || masterSecretSaved), content: selectedGroups().length > 0,
    configured: configured && configuredTarget === syncTargetFingerprint(currentPersistedSettings()), preview: previewAccepted,
  }));
  async function testProvider() {
    if (!wizardState.storage) return;
    if (backend === 's3' || backend === 'drive') { await testCloud(); return; }
    busy = true;
    const target = syncTargetFingerprint(currentPersistedSettings());
    try {
      await saveTransportSecrets();
      await rpc.call('sync.testProvider', { ...currentPersistedSettings() });
      tested = target === syncTargetFingerprint(currentPersistedSettings());
    } catch (e) { onError((e as Error).message); }
    finally { busy = false; }
  }
  function goStep(step: number) { if (!busy && wizardCanEnter(step, wizardState)) wizardStep = step; }
  const wizardLabels = ['sync.stepStorage', 'sync.stepAuthorize', 'sync.stepPassword', 'sync.stepContent', 'sync.stepFirst'] as const;
  let cloudPreview = $state<{ objects: number; bytes: number } | null>(null);

  async function saveTransportSecrets() {
    if (webdavPassword) await rpc.call('secret.setMaster', { account: 'sync.webdav.password', secret: webdavPassword });
    if (gitSshPassphrase) await rpc.call('secret.setMaster', { account: 'sync.git.ssh', secret: gitSshPassphrase });
    if (s3AccessKey && s3SecretKey) await rpc.call('secret.setMaster', { account: 'sync.s3.credentials', secret: JSON.stringify({ access_key_id: s3AccessKey, secret_access_key: s3SecretKey, session_token: s3SessionToken || undefined }) });
    webdavPassword = ''; gitSshPassphrase = ''; s3SecretKey = ''; s3SessionToken = ''; s3AccessKey = '';
  }
  function cloudParams() {
    return { provider: backend, endpoint: s3Endpoint, region: s3Region, bucket: s3Bucket, prefix: backend === 's3' ? s3Prefix : drivePrefix, path_style: s3PathStyle, client_id: driveClientId, master_password: '' };
  }
  async function testCloud() {
    busy = true;
    const target = syncTargetFingerprint(currentPersistedSettings());
    try { await saveTransportSecrets(); const result = await rpc.call<{ objects: number; bytes: number }>('sync.testCloud', cloudParams()); if (target === syncTargetFingerprint(currentPersistedSettings())) { cloudPreview = result; tested = true; } }
    catch (e) { onError((e as Error).message); }
    finally { busy = false; }
  }
  async function authorizeDrive() {
    busy = true;
    try {
      await rpc.call('sync.driveAuthorize', { client_id: driveClientId });
      await rpc.call('sync.driveFinishAuthorization', {});
      configured = false; configuredTarget = ''; approvedTarget = '';
      firstSyncApproved = false; previewAccepted = false; previewToken = null;
      await persist();
      await testCloud();
    } catch (e) { onError((e as Error).message); }
    finally { busy = false; }
  }
  type SyncGroup = import('../../../lib/syncConfig').SyncGroup;
  const syncGroups: SyncGroup[] = ['Connections', 'Appearance', 'Shortcuts', 'PluginCfg', 'Credentials', 'Notes'];

  function defaultEnabledGroups(): Record<SyncGroup, boolean> {
    return {
      Connections: true,
      Appearance: true,
      Shortcuts: true,
      PluginCfg: true,
      Credentials: false,
      Notes: false,
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
  let autoRunning = $state(false);
  let notesPausedDirty = $state(false);
  let kindLive = $state<Backend | null>(null);
  let protocol = $state<number>(1);
  let configuredProtocol = $state(3);
  let maintenancePending = $state<{pending:boolean;checkpointChanged?:boolean;blocked?:boolean;canCancel?:boolean;counts?:Record<string,{candidate:number;checkpoint:number;different:number;tombstones:number}>}>({pending:false});
  async function inspectMaintenance() {try{maintenancePending=await rpc.call('sync.maintenanceStatus',{});}catch(e){setSyncInfo('err',(e as Error).message);}}
  async function cancelMaintenance() {if(!(await appConfirm(i18n.t('sync.pendingCancelConfirm'))))return;busy=true;try{await rpc.call('sync.maintenanceCancel',{confirm:'CANCEL MAINTENANCE KEEP LOCAL'});maintenancePending={pending:false};firstSyncApproved=false;previewAccepted=false;previewToken=null;await refreshStatus();setSyncInfo('info',i18n.t('sync.pendingCancelled'));}catch(e){setSyncInfo('err',(e as Error).message);}finally{busy=false;}}
  let recoveryArchive = $state<{available:boolean;encrypted?:boolean;groups?:string[];records?:number}>({available:false});
  let recoveryError = $state('');
  let recoveryConfirm = $state('');
  async function inspectRecovery() {
    try { const summary=await rpc.call<typeof recoveryArchive>('sync.recoveryInspect',{}); recoveryArchive=summary; recoveryError=''; }
    catch(e) { recoveryError=(e as Error).message; }
  }
  async function exportRecovery() {
    busy=true;
    try {
      const result=await rpc.call<{available:boolean;encrypted:boolean;bundle?:string}>('sync.recoveryExport',{});
      if(!result.available || result.encrypted!==true || !result.bundle) throw new Error(i18n.t('sync.recoveryUnavailable'));
      const bytes=Uint8Array.from(atob(result.bundle),c=>c.charCodeAt(0));
      const url=URL.createObjectURL(new Blob([bytes],{type:'application/octet-stream'}));
      try {const link=document.createElement('a');link.href=url;link.download='aerotab-sync-recovery.bin';link.click();} finally {URL.revokeObjectURL(url);}
      recoveryError='';
    } catch(e) {recoveryError=(e as Error).message;} finally {busy=false;}
  }
  async function restoreRecovery() {
    if(busy || recoveryConfirm!=='RESTORE LOCAL RECOVERY') return;
    if(!(await appConfirm(i18n.t('sync.recoveryRestoreConfirm'),{danger:true})))return;
    busy=true;
    try {
      await rpc.call('sync.recoveryRestore',{confirm:recoveryConfirm});
      firstSyncApproved=false;previewAccepted=false;previewToken=null;recoveryConfirm='';
      await refreshStatus(); await inspectRecovery();setSyncInfo('info',i18n.t('sync.recoveryRestored'));
    } catch(e) {recoveryError=(e as Error).message;} finally {busy=false;}
  }
  let peers = $state<{id:string;retired:boolean;self:boolean}[]>([]);
  async function refreshPeers() {
    if (protocol !== 3) return;
    try { const r=await rpc.call<{devices:typeof peers}>('sync.devices',{}); peers=r.devices; } catch(e) {onError((e as Error).message);}
  }
  async function migrateV3() {
    busy=true;
    try {
      const p=await rpc.call<{previewToken:string;legacyObjects:number;groups:Record<string,number>}>('sync.migrationPreview',{});
      if (!(await appConfirm(i18n.t('sync.v3MigrationConfirm',{count:p.legacyObjects})))) return;
      await rpc.call('sync.migrateV3',{previewToken:p.previewToken,confirm:'MIGRATE TO V3'});
      configuredProtocol=3; firstSyncApproved=false; previewAccepted=false; previewToken=null;
      await persist(); await refreshStatus();
      setSyncInfo('ok',i18n.t('sync.v3MigrationDone'));
    } catch(e) {setSyncInfo('err',(e as Error).message);} finally {busy=false;}
  }
  async function rejoinDevice() {
    if (!(await appConfirm(i18n.t('sync.v3RejoinConfirm'),{danger:true}))) return;
    try {await rpc.call('sync.rejoin',{confirm:'REJOIN AS NEW DEVICE'});firstSyncApproved=false;previewAccepted=false;previewToken=null;await persist();await refreshStatus();await refreshPeers();} catch(e) {setSyncInfo('err',(e as Error).message);}
  }
  async function retirePeer(id:string) {
    if (!(await appConfirm(i18n.t('sync.v3RetireConfirm',{id}),{danger:true}))) return;
    try {await rpc.call('sync.retireDevice',{deviceId:id,confirm:id});await refreshPeers();} catch(e) {setSyncInfo('err',(e as Error).message);}
  }
  let ownSeq = $state<number | null>(null);
  let cursors = $state<Record<string, number> | null>(null);
  let lastOplog = $state<{
    bytesDownloaded?: number;
    bytesUploaded?: number;
    opsApplied?: number;
    snapshotUsed?: boolean;
    compacted?: boolean;
  } | null>(null);
  let capacity = $state<{
    worktreeBytes: number; gitBytes: number | null; totalBytes: number;
    opBytes: number; opFiles: number; snapshotBytes: number; snapshotFiles: number;
    commits: number | null; branch?: string; headOid?: string; historyRewriteAllowed: boolean;
  } | null>(null);
  let rewriteConfirmation = $state('');

  async function refreshCapacity() {
    try { capacity = await rpc.call('sync.capacity', {}); }
    catch (e) { onError(`sync capacity: ${(e as Error).message}`); }
  }

  async function rewriteHistory() {
    if (!capacity?.headOid || rewriteConfirmation !== 'REWRITE SYNC HISTORY') return;
    if (!(await appConfirm(i18n.t('sync.rewriteConfirm'), { danger: true }))) return;
    busy = true;
    try {
      await rpc.call('sync.rewriteHistory', { expectedOid: capacity.headOid, confirmation: rewriteConfirmation });
      rewriteConfirmation = '';
      await refreshCapacity();
      setSyncInfo('ok', i18n.t('sync.rewriteDone'));
    } catch (e) { setSyncInfo('err', (e as Error).message); }
    finally { busy = false; }
  }

  let busy = $state(false);
  let info = $state('');
  let infoTone = $state<'info' | 'ok' | 'err'>('info');

  type SyncStatsPayload = { pushed?: number; pulled?: number; merged?: number; unchanged?: number };
  type OplogStatsPayload = {
    bytesDownloaded?: number;
    bytesUploaded?: number;
    opsApplied?: number;
    snapshotUsed?: boolean;
    compacted?: boolean;
  };

  function setSyncInfo(tone: 'info' | 'ok' | 'err', message: string) {
    infoTone = tone;
    info = message;
  }

  function formatBytes(n: number | undefined): string {
    if (n == null) return '0 B';
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KiB`;
    return `${(n / (1024 * 1024)).toFixed(2)} MiB`;
  }

  function formatSyncStats(stats: Record<string, unknown> | null | undefined): string {
    const bridge = stats?._bridge as {
      credentials_skipped_locked?: boolean;
      credentials_skipped_uninitialized?: boolean;
    } | undefined;
    const oplog = stats?._oplog as OplogStatsPayload | undefined;
    const entries = Object.entries(stats ?? {}).filter(([k]) => k !== '_bridge' && k !== '_oplog');
    if (entries.length === 0 && !oplog) {
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
    if (oplog) {
      notes.push(i18n.t('sync.oplogBytesLine', {
        up: formatBytes(oplog.bytesUploaded),
        down: formatBytes(oplog.bytesDownloaded),
        ops: oplog.opsApplied ?? 0,
      }));
      if (oplog.snapshotUsed) notes.push(i18n.t('sync.oplogSnapshotUsed'));
      if (oplog.compacted) notes.push(i18n.t('sync.oplogCompacted'));
    }
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

  function markDirty() {
    firstSyncApproved = false; approvedTarget = ''; previewAccepted = false; tested = false;
    configuredTarget = ''; cloudPreview = null;
    void rpc.call('sync.stopAutoSync', {}).catch(() => {});
    void rpc.call('settings.get', { key: 'sync' }).then((result) => {
      const value = (result as { value?: PersistedSyncSettings }).value;
      if (value) return rpc.call('settings.set', { key: 'sync', value: { ...value, setupVersion: 3, firstSyncApproved: false, approvedTarget: '' } });
    }).catch(() => {});
    settingsCoord.markDirty('configsync');
  }

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
      case 'Notes': return i18n.t('sync.notesDescription');
    }
  }

  function selectedGroups(): SyncGroup[] {
    return syncGroups.filter((group) => enabledGroups[group]);
  }

  function currentPersistedSettings(): PersistedSyncSettings {
    return {
      backend,
      protocol: configuredProtocol,
      s3Endpoint, s3Region, s3Bucket, s3Prefix, s3PathStyle, driveClientId, drivePrefix, firstSyncApproved, approvedTarget, setupVersion: 3,
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
    firstSyncApproved = false; approvedTarget = ''; configuredTarget = ''; previewAccepted = false;
    void rpc.call('sync.stopAutoSync', {}).catch(() => {});
    settingsCoord.markDirty('configsync');
  }

  async function loadPersisted() {
    try {
      const v = (await loadPersistedSyncSettings(rpc) ?? {}) as Record<string, unknown>;
      configuredProtocol = typeof v.protocol === 'number' ? v.protocol : 3;
      if (v.backend === 'webdav' || v.backend === 'git' || v.backend === 's3' || v.backend === 'drive') backend = v.backend;
      if (typeof v.s3Endpoint === 'string') s3Endpoint = v.s3Endpoint;
      if (typeof v.s3Region === 'string') s3Region = v.s3Region;
      if (typeof v.s3Bucket === 'string') s3Bucket = v.s3Bucket;
      if (typeof v.s3Prefix === 'string') s3Prefix = v.s3Prefix;
      if (typeof v.s3PathStyle === 'boolean') s3PathStyle = v.s3PathStyle;
      if (typeof v.driveClientId === 'string') driveClientId = v.driveClientId;
      if (typeof v.drivePrefix === 'string') drivePrefix = v.drivePrefix;
      firstSyncApproved = v.firstSyncApproved === true;
      approvedTarget = typeof v.approvedTarget === 'string' ? v.approvedTarget : '';
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
    await saveTransportSecrets();
    await rpc.call('settings.set', {
      key: 'sync',
      value: {
        backend,
        s3Endpoint, s3Region, s3Bucket, s3Prefix, s3PathStyle, driveClientId, drivePrefix, firstSyncApproved, approvedTarget, setupVersion: 3,
        webdavUrl, webdavUser,
        gitRepoPath, gitRemoteUrl, gitRemoteName, gitRemoteBranch,
        gitAuthorName, gitAuthorEmail,
        gitRemoteUser,
        gitSshKeyPath, gitSshPort,
        gitAuthMode, githubOAuthClientId, gitlabOAuthClientId, gitlabOAuthBaseUrl,
        keyringAccount, vaultKeyringAccount, gitHttpsKeyringAccount: GIT_HTTPS_KEYRING,
        stateDir, autoSyncEnabled, autoSyncMinutes, enabledGroups,
        protocol: configuredProtocol,
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
        autoRunning?: boolean;
        notesPausedDirty?: boolean;
        protocol?: number;
        ownSeq?: number | null;
        cursors?: Record<string, number> | null;
        lastOplog?: OplogStatsPayload | null;
      }>('sync.status', {});
      configured = s.configured;
      kindLive = s.kind;
      deviceId = s.deviceId;
      lastSyncMs = s.lastSyncMs;
      autoIntervalMs = s.autoIntervalMs;
      autoRunning = s.autoRunning === true;
      notesPausedDirty = s.notesPausedDirty ?? false;
      protocol = s.protocol ?? (s.configured ? 2 : 1);
      ownSeq = s.ownSeq ?? null;
      cursors = s.cursors ?? null;
      lastOplog = s.lastOplog ?? null;
      if(configured) { await inspectMaintenance(); await inspectRecovery(); }
    } catch {
      /* engine not configured yet */
    }
  }

  async function compactNow() {
    if (!configured) return;
    busy = true;
    setSyncInfo('info', i18n.t('sync.compacting'));
    try {
      const result = await rpc.call<{ compacted: boolean }>('sync.compact', { groups: selectedGroups() });
      setSyncInfo(result.compacted ? 'ok' : 'info', i18n.t(result.compacted ? 'sync.compactDone' : 'sync.partialSnapshotRefused'));
      await refreshCapacity();
      await refreshStatus();
    } catch (e) {
      setSyncInfo('err', i18n.t('sync.compactFailed', { message: (e as Error).message }));
      onError(`sync compact: ${(e as Error).message}`);
    } finally {
      busy = false;
    }
  }

  async function purgeV1() {
    if (!configured) return;
    if (!(await appConfirm(i18n.t('sync.purgeV1Confirm'), { danger: true, confirmLabel: i18n.t('common.delete') }))) {
      return;
    }
    busy = true;
    try {
      const r = await rpc.call<{ deleted: number }>('sync.purgeV1', {});
      setSyncInfo('ok', i18n.t('sync.purgeV1Done', { count: r.deleted }));
      await refreshStatus();
    } catch (e) {
      setSyncInfo('err', (e as Error).message);
      onError(`sync purgeV1: ${(e as Error).message}`);
    } finally {
      busy = false;
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

  async function refreshVaultStatus() {
    try {
      vaultStatus = await rpc.call('vault.status', {});
    } catch {
      vaultStatus = { configured: false, initialized: false, unlocked: false };
    }
  }

  async function unlockVaultNow() {
    if (!vaultPassword) {
      vaultInfo = i18n.t('sync.vaultPasswordRequired');
      return;
    }
    vaultBusy = true;
    try {
      const r = await rpc.call<{ unlocked: boolean; initialized: boolean }>('sync.ensureVaultUnlock', {
        password: vaultPassword || undefined,
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
    vaultPassword = '';
    vaultConfirmPassword = '';
    try {
      await rpc.call('vault.lock', {});
      vaultInfo = i18n.t('sync.vaultLocked');
      await refreshVaultStatus();
    } catch (e) {
      onError(`vault lock: ${(e as Error).message}`);
    }
  }

  /** Credential sync only uses a vault already manually unlocked this process. */
  async function ensureVaultForSync(): Promise<boolean> {
    if (!enabledGroups.Credentials) return true;
    await refreshVaultStatus();
    if (vaultStatus.unlocked) return true;
    setSyncInfo('err', i18n.t(vaultStatus.initialized ? 'sync.vaultUnlockRequired' : 'sync.vaultNotInitialized'));
    return false;
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
    if (!wizardCanEnter(4, wizardState)) return;
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
      await saveTransportSecrets();
      const snapshot = currentPersistedSettings();
      if (backend === 'git' && gitAuthMode === 'https') {
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
      configuredTarget = syncTargetFingerprint(currentPersistedSettings());
      previewAccepted = false;
      await persist();
      await rpc.call('sync.stopAutoSync', {});
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
    if (!wizardState.configured || !wizardCanEnter(4, wizardState)) return;
    try {
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
    {
      try {
        const target = syncTargetFingerprint(currentPersistedSettings());
        const preview = await rpc.call<{ added: number; changed: number; deleted: number; publish: number; conflicts: number; previewToken: string }>('sync.preview', { groups });
        if (!(await appConfirm(i18n.t('sync.mergePreview', { added: preview.added, changed: preview.changed, deleted: preview.deleted, publish: preview.publish, conflicts: preview.conflicts })))) return;
        if (target !== syncTargetFingerprint(currentPersistedSettings()) || !wizardState.configured) return;
        previewAccepted = true;
        previewToken = preview.previewToken;
      } catch (e) { onError((e as Error).message); return; }
    }
    if (!syncSetupComplete(wizardState)) return;
    busy = true;
    setSyncInfo('info', i18n.t('sync.syncing'));
    try {
      const stats = await rpc.call<Record<string, unknown>>('sync.now', { groups, preview_token: previewToken });
      firstSyncApproved = true;
      approvedTarget = syncTargetFingerprint(currentPersistedSettings());
      await persist();
      await applyPersistedAutoSync(rpc, currentPersistedSettings());
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
    settingsCoord.markDirty('configsync');
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
    })().catch(() => setSyncInfo('err', i18n.t('sync.restoreFailed')));
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
  <p class="help">{i18n.t('sync.wizardSteps')}</p>
  <nav class="flex gap-2 flex-wrap" aria-label={i18n.t('sync.wizardSteps')}>
    {#each wizardLabels as label, index}
      <button type="button" class="btn-secondary" aria-current={wizardStep === index ? 'step' : undefined} disabled={busy || !wizardCanEnter(index, wizardState)} onclick={() => goStep(index)}>{index + 1}. {i18n.t(label)}</button>
    {/each}
  </nav>
  {#if cloudPreview}<p class="help">{i18n.t('sync.cloudPreview', { count: cloudPreview.objects, bytes: cloudPreview.bytes })}</p>{/if}

  {#if configured}
    <button class="btn" disabled={busy} onclick={() => void inspectMaintenance()}>{i18n.t('sync.pendingInspect')}</button>
    {#if maintenancePending.pending}
      <div role="status" class="help">
        <strong>{i18n.t('sync.pendingMaintenance')}</strong>
        <p>{i18n.t(maintenancePending.checkpointChanged ? 'sync.pendingChanged' : 'sync.pendingCheckpoint')}</p>
        {#each Object.entries(maintenancePending.counts ?? {}) as [group, count]}
          <p>{group}: {i18n.t('sync.pendingCounts',{candidate:count.candidate,checkpoint:count.checkpoint,different:count.different})} / {count.tombstones} {i18n.t('sync.pendingTombstones')}</p>
        {/each}
        <button class="btn" disabled={busy || maintenancePending.blocked || !maintenancePending.canCancel} onclick={() => void cancelMaintenance()}>{i18n.t('sync.pendingCancel')}</button>
      </div>
    {/if}
  {/if}
  <div class="text-[12px] text-[var(--color-fg-muted)]">
    Status:
    {#if configured}
      <span class="text-[var(--color-accent)]">configured</span>
      &middot; protocol <code>v{protocol}</code>
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

  {#if configured && protocol >= 2}
    <div class="info"><p>{i18n.t('sync.v3MigrationHelp')}</p><button class="btn" disabled={busy} onclick={() => void migrateV3()}>{i18n.t('sync.v3Migrate')}</button></div>
  {/if}
  {#if configured && protocol === 3}
    <div class="info"><p>{i18n.t('sync.v3MaintenanceHelp')}</p><button class="btn" onclick={() => void refreshPeers()}>{i18n.t('sync.v3Devices')}</button>
      {#each peers as peer (peer.id)}
        <div><code>{peer.id}</code> {peer.retired ? i18n.t('sync.v3Retired') : ''}
          {#if peer.retired && peer.self}<button class="btn" disabled={busy} onclick={() => void rejoinDevice()}>{i18n.t('sync.v3Rejoin')}</button>{/if}
          {#if !peer.retired && !peer.self}<button class="btn" disabled={busy} onclick={() => void retirePeer(peer.id)}>{i18n.t('sync.v3Retire')}</button>{/if}
        </div>
      {/each}
    </div>
  {/if}
  {#if configured && protocol >= 2}
    <details><summary>{i18n.t('sync.advancedMaintenance')}</summary>
    <div class="mt-2 text-[12px] text-[var(--color-fg-muted)] space-y-1">
      <div class="section-h">{i18n.t('sync.recoveryTitle')}</div>
      <button type="button" class="btn" disabled={busy} onclick={() => void inspectRecovery()}>{i18n.t('sync.recoveryInspect')}</button>
      {#if recoveryError}<p role="alert">{recoveryError}</p>{/if}
      {#if recoveryArchive.available}
        <p>{i18n.t('sync.recoverySummary',{count:recoveryArchive.records ?? 0,groups:(recoveryArchive.groups ?? []).join(', ')})}</p>
        <p class="help">{i18n.t('sync.recoveryCiphertext')}</p>
        <button type="button" class="btn" disabled={busy} onclick={() => void exportRecovery()}>{i18n.t('sync.recoveryExport')}</button>
        <label class="row"><span class="row-label">{i18n.t('sync.recoveryPhrase')}</span><input bind:value={recoveryConfirm} autocomplete="off" spellcheck="false" placeholder="RESTORE LOCAL RECOVERY" /></label>
        <button type="button" class="btn" disabled={busy || recoveryConfirm !== 'RESTORE LOCAL RECOVERY'} onclick={() => void restoreRecovery()}>{i18n.t('sync.recoveryRestore')}</button>
      {:else}<p class="help">{i18n.t('sync.recoveryUnavailable')}</p>{/if}
      <div class="section-h">{i18n.t('sync.oplogSectionTitle')}</div>
      <div>
        {i18n.t('sync.oplogOwnSeq')}: <code>{ownSeq ?? 0}</code>
        {#if lastOplog}
          &middot; ↑{formatBytes(lastOplog.bytesUploaded)}
          &middot; ↓{formatBytes(lastOplog.bytesDownloaded)}
          &middot; ops {lastOplog.opsApplied ?? 0}
          {#if lastOplog.snapshotUsed}&middot; {i18n.t('sync.oplogSnapshotUsed')}{/if}
          {#if lastOplog.compacted}&middot; {i18n.t('sync.oplogCompacted')}{/if}
        {/if}
      </div>
      {#if cursors && Object.keys(cursors).length > 0}
        <div class="font-mono text-[11px] max-h-24 overflow-auto border border-[var(--color-border-soft)] rounded px-2 py-1">
          {#each Object.entries(cursors) as [id, seq] (id)}
            <div>{id.slice(0, 8)}… → {seq}</div>
          {/each}
        </div>
      {/if}
      <div class="flex gap-2 flex-wrap pt-1">
        <button type="button" class="btn-secondary text-[11px] py-1 px-2" disabled={busy}
          onclick={() => void compactNow()}>
          {i18n.t('sync.compactNow')}
        </button>
        <button type="button" class="btn-secondary text-[11px] py-1 px-2" disabled={busy}
          onclick={() => void purgeV1()}>
          {i18n.t('sync.purgeV1')}
        </button>
      </div>
      <div class="help">{i18n.t('sync.oplogHelp')}</div>
      <div class="section-h">{i18n.t('sync.capacityTitle')}</div>
      <button type="button" class="btn-secondary" disabled={busy} onclick={() => void refreshCapacity()}>{i18n.t('sync.capacityRefresh')}</button>
      {#if capacity}
        <div>{i18n.t('sync.capacityWorktree')}: {formatBytes(capacity.worktreeBytes)} · .git: {capacity.gitBytes === null ? '—' : formatBytes(capacity.gitBytes)} · {i18n.t('sync.capacityTotal')}: {formatBytes(capacity.totalBytes)}</div>
        <div>ops: {capacity.opFiles} / {formatBytes(capacity.opBytes)} · snapshots: {capacity.snapshotFiles} / {formatBytes(capacity.snapshotBytes)} · commits: {capacity.commits ?? '—'}</div>
        <div class="help">{i18n.t('sync.capacityScope')}</div>
        {#if kindLive === 'git'}
          <div class="help">{i18n.t('sync.rewriteHelp')} ({capacity.branch ?? '—'})</div>
          <label for="sync-rewrite-confirm" class="lbl">REWRITE SYNC HISTORY</label>
          <input id="sync-rewrite-confirm" class="input" bind:value={rewriteConfirmation} autocomplete="off" />
          <button type="button" class="btn-secondary" disabled={busy || !capacity.historyRewriteAllowed || !capacity.headOid || rewriteConfirmation !== 'REWRITE SYNC HISTORY'} onclick={() => void rewriteHistory()}>{i18n.t('sync.rewriteAction')}</button>
        {/if}
      {/if}
    </div>
    </details>
  {/if}

  <div hidden={wizardStep !== 0}>
    <div class="section-h">Backend</div>
    <label for="cs-backend" class="lbl">Type</label>
    <select id="cs-backend" bind:value={backend} onchange={markDirty} class="select">
      <option value="webdav">WebDAV</option>
      <option value="git">Git</option>
      <option value="s3">S3 compatible</option>
      <option value="drive">Google Drive</option>
    </select>
  </div>

  <div hidden={wizardStep !== 3}>
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

  <div hidden={wizardStep !== 0 && wizardStep !== 1}>
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
  {:else if backend === 's3'}
    <div class="space-y-2">
      <div class="section-h">S3 compatible</div>
      <label class="lbl" for="s3-endpoint">HTTPS endpoint</label><input id="s3-endpoint" class="input" bind:value={s3Endpoint} oninput={markDirty} placeholder="https://s3.example.com" />
      <label class="lbl" for="s3-region">Region</label><input id="s3-region" class="input" bind:value={s3Region} oninput={markDirty} />
      <label class="lbl" for="s3-bucket">Bucket</label><input id="s3-bucket" class="input" bind:value={s3Bucket} oninput={markDirty} />
      <label class="lbl" for="s3-prefix">Prefix</label><input id="s3-prefix" class="input" bind:value={s3Prefix} oninput={markDirty} />
      <label><input type="checkbox" bind:checked={s3PathStyle} onchange={markDirty} /> Path style</label>
      <label class="lbl" for="s3-access">Access key ID</label><input id="s3-access" class="input" bind:value={s3AccessKey} oninput={markDirty} autocomplete="off" />
      <label class="lbl" for="s3-secret">Secret access key</label><input id="s3-secret" class="input" type="password" bind:value={s3SecretKey} oninput={markDirty} autocomplete="new-password" />
      <label class="lbl" for="s3-token">Session token (optional)</label><input id="s3-token" class="input" type="password" bind:value={s3SessionToken} oninput={markDirty} autocomplete="off" />
      <div class="help">{i18n.t('sync.cloudPermissions')}</div>
      <button class="btn-secondary" disabled={busy} onclick={() => void testCloud()}>{i18n.t('sync.cloudTest')}</button>
    </div>
  {:else if backend === 'drive'}
    <div class="space-y-2">
      <div class="section-h">Google Drive</div>
      <label class="lbl" for="drive-client">OAuth client ID</label><input id="drive-client" class="input" bind:value={driveClientId} oninput={markDirty} />
      <label class="lbl" for="drive-prefix">Prefix</label><input id="drive-prefix" class="input" bind:value={drivePrefix} oninput={markDirty} />
      <div class="help">{i18n.t('sync.driveRegistration')}</div>
      <button class="btn-secondary" disabled={busy || !driveClientId} onclick={() => void authorizeDrive()}>{i18n.t('sync.cloudAuthorize')}</button>
      <button class="btn-secondary" disabled={busy} onclick={() => void testCloud()}>{i18n.t('sync.cloudTest')}</button>
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

    {#if backend === 'git' || backend === 'webdav'}
      <button type="button" class="btn-secondary" disabled={busy || !wizardState.storage} onclick={() => void testProvider()}>{i18n.t('sync.cloudTest')}</button>
    {/if}
  </div>
  <div hidden={wizardStep !== 2}>
    <div class="section-h">Encryption</div>
    <label for="cs-mp" class="lbl">
      Master password (required, not persisted)
    </label>
    <input id="cs-mp" type="password" bind:value={masterPassword} oninput={() => { configuredTarget = ''; previewAccepted = false; firstSyncApproved = false; approvedTarget = ''; void rpc.call('sync.stopAutoSync', {}); }} class="input"
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

  {#if enabledGroups.Credentials && wizardStep === 3}
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
        <div class="flex gap-2 items-center pt-2 flex-wrap">
          <button type="button" class="btn-secondary" disabled={vaultBusy}
            onclick={() => void unlockVaultNow()}>
            {i18n.t('sync.vaultUnlock')}
          </button>
          <button type="button" class="btn-secondary" disabled={vaultBusy || !vaultStatus.unlocked}
            onclick={() => void lockVaultForSync()}>
            {i18n.t('sync.vaultLock')}
          </button>
        </div>
        {#if vaultInfo}
          <div class="help mt-1">{vaultInfo}</div>
        {/if}
      {/if}
    </div>
  {/if}

  <details><summary>{i18n.t('sync.advancedMaintenance')}</summary>
  <div>
    <div class="section-h">Local state</div>
    <label for="cs-statedir" class="lbl">State directory (optional)</label>
    <input id="cs-statedir" bind:value={stateDir} oninput={markDirty}
      placeholder="(default: device-local settings storage)" class="input" />
    <div class="help">
      {i18n.t('sync.localStateHelp')}
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
    {#if notesPausedDirty}<p class="help" role="status">{i18n.t('sync.notesPausedDirty')}</p>{/if}
    {#if autoSyncEnabled && !autoRunning}
      <p class="help" role="status">{i18n.t(hasSyncApproval(currentPersistedSettings()) ? 'sync.autoSyncStopped' : 'sync.autoSyncNeedsApproval')}</p>
    {/if}
    {#if configured && autoRunning && autoIntervalMs}
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

  </details>
  {#if wizardStep < 4}
    <button type="button" class="btn-primary" disabled={busy || !wizardCanEnter(wizardStep + 1, wizardState)} onclick={() => goStep(wizardStep + 1)}>{i18n.t('sync.stepNext')}</button>
  {/if}
  <div class="flex gap-2 items-center pt-2" hidden={wizardStep !== 4}>
    <button type="button" class="btn-primary" disabled={busy || !wizardCanEnter(4, wizardState)} onclick={() => void configure()}>
      <Play size={12} /> Configure / re-key
    </button>
    <button type="button" class="btn-secondary" disabled={busy || !wizardState.configured || !wizardCanEnter(4, wizardState)}
      onclick={() => void syncNow()}>
      <RefreshCw size={12} /> Sync now
    </button>
    <button type="button" class="btn-secondary" disabled={busy || !firstSyncApproved || !syncSetupComplete(wizardState)}
      onclick={() => void applyAutoSync()}>
      {i18n.t('sync.autoSyncApply')}
    </button>
  </div>
  {#if info}
    <p class="sync-status" class:sync-status-ok={infoTone === 'ok'} class:sync-status-err={infoTone === 'err'}>
      {info}
    </p>
  {/if}

  <details><summary>{i18n.t('sync.advancedMaintenance')}</summary>
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
  </details>
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
