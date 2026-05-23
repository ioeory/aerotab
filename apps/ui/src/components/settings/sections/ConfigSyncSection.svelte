<script lang="ts">
  // Config Sync (M11) — UI on top of the existing sync.* IPC.
  //
  // Persists non-secret configuration in settings key `sync` so the next
  // launch can re-show / re-apply the same backend. The master password is
  // NEVER persisted; the user re-enters it each session to (re-)configure
  // the engine.

  import { onMount, onDestroy } from 'svelte';
  import { RefreshCw, Play } from '@lucide/svelte';
  import { b64decode, b64encode, type RpcClient } from '../../../lib/rpc';
  import { settingsCoord } from '../../../lib/settingsStore.svelte';

  interface Props {
    rpc: RpcClient;
    onError: (msg: string) => void;
  }
  let { rpc, onError }: Props = $props();

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
  let gitAuthorName = $state('Tabby');
  let gitAuthorEmail = $state('tabby@localhost');
  let gitRemoteUser = $state('');
  let gitRemotePassword = $state('');
  let gitSshKeyPath = $state('');
  let gitSshPassphrase = $state('');
  let gitAuthMode = $state<'none' | 'https' | 'ssh'>('none');
  // Common
  let masterPassword = $state('');
  let keyringAccount = $state('');
  let masterSecretSaved = $state(false);
  let secretBusy = $state(false);
  let secretInfo = $state('');
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
  let info = $state<string>('');

  let statusTimer: ReturnType<typeof setInterval> | null = null;

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

  function setGroupEnabled(group: SyncGroup, checked: boolean) {
    let nextChecked = checked;
    if (group === 'Credentials' && checked) {
      nextChecked = confirm('Enable credential sync? Passwords and private key material will be encrypted before upload, but they may leave this device.');
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
      if (typeof v.gitRemotePassword === 'string') gitRemotePassword = v.gitRemotePassword;
      if (typeof v.gitSshKeyPath === 'string') gitSshKeyPath = v.gitSshKeyPath;
      if (typeof v.gitSshPassphrase === 'string') gitSshPassphrase = v.gitSshPassphrase;
      if (v.gitAuthMode === 'none' || v.gitAuthMode === 'https' || v.gitAuthMode === 'ssh') {
        gitAuthMode = v.gitAuthMode;
      }
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
    // store credentials in the OS keychain via tabby-core's `secret` module;
    // master password is intentionally excluded.
    await rpc.call('settings.set', {
      key: 'sync',
      value: {
        backend,
        webdavUrl, webdavUser, webdavPassword,
        gitRepoPath, gitRemoteUrl, gitRemoteName, gitRemoteBranch,
        gitAuthorName, gitAuthorEmail,
        gitRemoteUser, gitRemotePassword, gitSshKeyPath, gitSshPassphrase,
        gitAuthMode,
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

  async function clearMasterSecret() {
    if (!confirm('Forget the saved sync master password from the OS credential store?')) return;
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
    if (!passwordForConfigure) { info = 'Master password is required'; return; }
    busy = true; info = 'Configuring…';
    try {
      if (backend === 'webdav') {
        if (!webdavUrl) { info = 'WebDAV URL is required'; return; }
        await rpc.call('sync.configureWebdav', {
          base_url: webdavUrl,
          user: webdavUser || undefined,
          password: webdavPassword || undefined,
          master_password: passwordForConfigure,
          state_dir: stateDir || undefined,
        });
      } else {
        if (!gitRepoPath) { info = 'Git repo path is required'; return; }
        const args: Record<string, unknown> = {
          repo_path: gitRepoPath,
          master_password: passwordForConfigure,
          author_name: gitAuthorName || undefined,
          author_email: gitAuthorEmail || undefined,
          state_dir: stateDir || undefined,
          remote_name: gitRemoteName,
          remote_branch: gitRemoteBranch,
        };
        if (gitRemoteUrl) args.remote_url = gitRemoteUrl;
        if (gitAuthMode === 'https') {
          args.remote_user = gitRemoteUser;
          args.remote_password = gitRemotePassword;
        } else if (gitAuthMode === 'ssh') {
          args.remote_ssh_key = gitSshKeyPath;
          if (gitSshPassphrase) args.remote_ssh_passphrase = gitSshPassphrase;
        }
        await rpc.call('sync.configureGit', args);
      }
      info = 'Engine configured';
      await refreshStatus();
      // Apply auto-sync state per current toggle.
      await applyAutoSync();
    } catch (e) {
      info = '';
      onError(`sync configure: ${(e as Error).message}`);
    } finally {
      busy = false;
    }
  }

  async function syncNow() {
    const groups = selectedGroups();
    if (groups.length === 0) { info = 'Enable at least one sync group'; return; }
    busy = true; info = 'Syncing…';
    try {
      const stats = await rpc.call<Record<string, unknown>>('sync.now', { groups });
      info = `Sync complete: ${Object.keys(stats).length} group(s)`;
      await refreshStatus();
    } catch (e) {
      info = '';
      onError(`sync now: ${(e as Error).message}`);
    } finally {
      busy = false;
    }
  }

  async function applyAutoSync() {
    try {
      if (autoSyncEnabled) {
        const groups = selectedGroups();
        if (groups.length === 0) { info = 'Enable at least one sync group'; return; }
        const interval_ms = Math.max(1, autoSyncMinutes) * 60_000;
        await rpc.call('sync.startAutoSync', { interval_ms, groups });
      } else {
        await rpc.call('sync.stopAutoSync', {});
      }
      await refreshStatus();
    } catch (e) {
      onError(`auto sync: ${(e as Error).message}`);
    }
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
    if (!confirm('Write this local sync record? This is an advanced recovery operation.')) return;
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

  async function deleteSyncRecord(id = inspectorId) {
    if (!id) return;
    if (!confirm(`Delete local sync record ${id}?`)) return;
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
    void loadPersisted();
    void refreshStatus();
    void refreshSecretStatus();
    statusTimer = setInterval(refreshStatus, 5000);
  });
  onDestroy(() => {
    settingsCoord.unregisterSaver('configsync');
    if (statusTimer) clearInterval(statusTimer);
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
            onchange={(e) => setGroupEnabled(group, (e.currentTarget as HTMLInputElement).checked)}
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
        placeholder="https://webdav.example.com/tabby" class="input"
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
        placeholder="/home/me/.tabby-sync" class="input" />
      <label for="cs-remote-url" class="lbl">Remote URL (optional)</label>
      <input id="cs-remote-url" bind:value={gitRemoteUrl} oninput={markDirty}
        placeholder="https://github.com/me/tabby-config.git" class="input" />
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
      </select>
      {#if gitAuthMode === 'https'}
        <label for="cs-r-user" class="lbl">Remote username</label>
        <input id="cs-r-user" bind:value={gitRemoteUser} oninput={markDirty} class="input" />
        <label for="cs-r-pass" class="lbl">Remote password / token</label>
        <input id="cs-r-pass" type="password"
          bind:value={gitRemotePassword} oninput={markDirty} class="input" />
      {:else if gitAuthMode === 'ssh'}
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
      Used to derive the encryption key for every record. Tabby never stores
      this password — re-enter it after each launch.
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
      <input type="checkbox" bind:checked={autoSyncEnabled} onchange={markDirty} />
      Sync automatically every
      <input
        type="number" min="1" max="1440"
        bind:value={autoSyncMinutes} oninput={markDirty}
        class="input" style="width: 70px; display: inline-block; margin: 0 4px;"
      />
      minutes
    </label>
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
      Apply auto-sync setting
    </button>
    {#if info}
      <span class="text-[12px] text-[var(--color-fg-muted)]">{info}</span>
    {/if}
  </div>

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
</style>
