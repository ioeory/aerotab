<script lang="ts">
  // SSH — M6. Global SSH defaults applied to every SSH session unless a
  // profile overrides them. Persisted under settings key `ssh`.
  import { onMount, onDestroy } from 'svelte';
  import { RefreshCw, Server, Trash2 } from '@lucide/svelte';
  import type { RpcClient } from '../../../lib/rpc';
  import type { KnownHostEntry } from '../../../lib/types';
  import { settingsCoord } from '../../../lib/settingsStore.svelte';

  interface Props { rpc: RpcClient; onError: (msg: string) => void }
  let { rpc, onError }: Props = $props();

  type AgentKind = 'auto' | 'system' | 'pageant' | 'none';

  let warnOnClose = $state(true);
  let agent = $state<AgentKind>('auto');
  let x11Forwarding = $state(false);
  let x11Display = $state('');
  let knownHostsPath = $state('');
  let knownHostsDir = $state('');
  let winscpPath = $state('');
  let keepaliveInterval = $state(30);
  let keepaliveCountMax = $state(3);
  let reuseSession = $state(true);
  let jumpHost = $state('');
  let serverAliveInterval = $state(0);
  let preferredAuth = $state('publickey,password,keyboard-interactive');
  let reconnectOnDrop = $state(false);
  let reconnectDelay = $state(3);
  let hostStatsEnabled = $state(true);
  let hostStatsIntervalSec = $state(30);
  let knownHosts = $state<KnownHostEntry[]>([]);
  let knownHostsBusy = $state(false);
  let knownHostsStatus = $state('');

  function markDirty() { settingsCoord.markDirty(); }

  async function load() {
    try {
      const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'ssh' });
      if (r.value && typeof r.value === 'object') {
        const v = r.value as Record<string, unknown>;
        if (typeof v.warnOnClose === 'boolean') warnOnClose = v.warnOnClose;
        if (v.agent === 'auto' || v.agent === 'system' || v.agent === 'pageant' || v.agent === 'none') agent = v.agent;
        if (typeof v.x11Forwarding === 'boolean') x11Forwarding = v.x11Forwarding;
        if (typeof v.x11Display === 'string') x11Display = v.x11Display;
        if (typeof v.knownHostsPath === 'string') knownHostsPath = v.knownHostsPath;
        if (typeof v.knownHostsDir === 'string') knownHostsDir = v.knownHostsDir;
        if (typeof v.winscpPath === 'string') winscpPath = v.winscpPath;
        if (typeof v.keepaliveInterval === 'number') keepaliveInterval = v.keepaliveInterval;
        if (typeof v.keepaliveCountMax === 'number') keepaliveCountMax = v.keepaliveCountMax;
        if (typeof v.reuseSession === 'boolean') reuseSession = v.reuseSession;
        if (typeof v.jumpHost === 'string') jumpHost = v.jumpHost;
        if (typeof v.serverAliveInterval === 'number') serverAliveInterval = v.serverAliveInterval;
        if (typeof v.preferredAuth === 'string') preferredAuth = v.preferredAuth;
        if (typeof v.reconnectOnDrop === 'boolean') reconnectOnDrop = v.reconnectOnDrop;
        if (typeof v.reconnectDelay === 'number') reconnectDelay = v.reconnectDelay;
        if (typeof v.hostStatsEnabled === 'boolean') hostStatsEnabled = v.hostStatsEnabled;
        if (typeof v.hostStatsIntervalSec === 'number') hostStatsIntervalSec = v.hostStatsIntervalSec;
      }
    } catch (e) { onError(`ssh load: ${(e as Error).message}`); }
  }

  async function save() {
    await rpc.call('settings.set', {
      key: 'ssh',
      value: {
        warnOnClose, agent, x11Forwarding, x11Display, knownHostsPath, winscpPath,
        knownHostsDir,
        keepaliveInterval, keepaliveCountMax, reuseSession, jumpHost,
        serverAliveInterval, preferredAuth, reconnectOnDrop, reconnectDelay,
        hostStatsEnabled, hostStatsIntervalSec,
      },
    });
  }

  async function loadKnownHosts() {
    knownHostsBusy = true;
    try {
      knownHosts = await rpc.call<KnownHostEntry[]>('ssh.knownHosts.list', {});
      knownHostsStatus = `${knownHosts.length} known host${knownHosts.length === 1 ? '' : 's'}`;
    } catch (e) {
      knownHostsStatus = '';
      onError(`known_hosts list: ${(e as Error).message}`);
    } finally {
      knownHostsBusy = false;
    }
  }

  async function configureKnownHostsDir() {
    if (!knownHostsDir.trim()) return;
    knownHostsBusy = true;
    try {
      await rpc.call('ssh.knownHosts.configure', { dir: knownHostsDir.trim() });
      knownHostsStatus = 'known_hosts backend directory applied';
      settingsCoord.markDirty();
      await loadKnownHosts();
    } catch (e) {
      onError(`known_hosts configure: ${(e as Error).message}`);
    } finally {
      knownHostsBusy = false;
    }
  }

  async function removeKnownHost(host: string) {
    if (!confirm(`Remove known host ${host}?`)) return;
    knownHostsBusy = true;
    try {
      const r = await rpc.call<{ removed: boolean }>('ssh.knownHosts.remove', { host });
      knownHostsStatus = r.removed ? `removed ${host}` : `${host} was not present`;
      await loadKnownHosts();
    } catch (e) {
      onError(`known_hosts remove: ${(e as Error).message}`);
    } finally {
      knownHostsBusy = false;
    }
  }

  onMount(() => {
    settingsCoord.registerSaver('ssh', save);
    void (async () => {
      await load();
      await loadKnownHosts();
    })();
  });
  onDestroy(() => settingsCoord.unregisterSaver('ssh'));
</script>

<div class="settings-section">
  <h2 class="flex items-center gap-2"><Server size={16} /> SSH</h2>

  <div class="section-h">Connection</div>
  <label class="row">
    <span class="row-label">SSH agent</span>
    <select bind:value={agent} onchange={markDirty}>
      <option value="auto">Auto (system default)</option>
      <option value="system">System agent (ssh-agent / gpg-agent)</option>
      <option value="pageant">Pageant (Windows)</option>
      <option value="none">Disabled</option>
    </select>
  </label>
  <label class="row">
    <span class="row-label">Preferred auth methods</span>
    <input type="text" bind:value={preferredAuth} oninput={markDirty}
           placeholder="publickey,password,keyboard-interactive" />
  </label>
  <label class="row">
    <span class="row-label">Jump host (ProxyJump)</span>
    <input type="text" bind:value={jumpHost} oninput={markDirty} placeholder="user@bastion:22" />
  </label>
  <label class="row">
    <span class="row-label">Reuse multiplexed session</span>
    <input type="checkbox" bind:checked={reuseSession} onchange={markDirty} />
  </label>

  <div class="section-h">Keep-alive</div>
  <label class="row">
    <span class="row-label">Client interval (s)</span>
    <input type="number" min="0" max="3600" bind:value={keepaliveInterval} oninput={markDirty} />
  </label>
  <label class="row">
    <span class="row-label">Max missed responses</span>
    <input type="number" min="0" max="20" bind:value={keepaliveCountMax} oninput={markDirty} />
  </label>
  <label class="row">
    <span class="row-label">ServerAliveInterval (s)</span>
    <input type="number" min="0" max="3600" bind:value={serverAliveInterval} oninput={markDirty} />
  </label>

  <div class="section-h">X11 forwarding</div>
  <label class="row">
    <span class="row-label">Enable X11 forwarding</span>
    <input type="checkbox" bind:checked={x11Forwarding} onchange={markDirty} />
  </label>
  <label class="row">
    <span class="row-label">X11 DISPLAY</span>
    <input type="text" bind:value={x11Display} oninput={markDirty} placeholder=":0.0" disabled={!x11Forwarding} />
  </label>

  <div class="section-h">Files</div>
  <label class="row">
    <span class="row-label">known_hosts path</span>
    <input type="text" bind:value={knownHostsPath} oninput={markDirty} placeholder="~/.ssh/known_hosts" />
  </label>
  <div class="row">
    <span class="row-label">Backend known_hosts directory</span>
    <div class="inline-row">
      <input type="text" bind:value={knownHostsDir} oninput={markDirty} placeholder="(default: app data directory)" />
      <button type="button" class="btn-secondary" onclick={configureKnownHostsDir}
              disabled={knownHostsBusy || !knownHostsDir.trim()}>
        Apply
      </button>
    </div>
  </div>
  <label class="row">
    <span class="row-label">WinSCP path (Windows)</span>
    <input type="text" bind:value={winscpPath} oninput={markDirty} placeholder="C:\Program Files (x86)\WinSCP\WinSCP.exe" />
  </label>

  <div class="section-h">Known hosts</div>
  <div class="known-hosts-toolbar">
    <button type="button" class="btn-secondary" onclick={loadKnownHosts} disabled={knownHostsBusy}>
      <RefreshCw size={12} /> Refresh
    </button>
    {#if knownHostsStatus}<span>{knownHostsStatus}</span>{/if}
  </div>
  <div class="known-hosts-list">
    {#if knownHosts.length === 0}
      <div class="known-host-empty">No backend known hosts recorded.</div>
    {:else}
      {#each knownHosts as item (`${item.host}:${item.key_type}`)}
        <div class="known-host-row">
          <div class="min-w-0 flex-1">
            <div class="truncate text-[var(--color-fg)]">{item.host}</div>
            <div class="truncate text-[10.5px] text-[var(--color-fg-muted)] font-mono">
              {item.key_type} · {item.key_b64.slice(0, 36)}{item.key_b64.length > 36 ? '…' : ''}
            </div>
          </div>
          <button type="button" class="btn-danger" onclick={() => removeKnownHost(item.host)}
                  disabled={knownHostsBusy} title="Remove host key" aria-label="Remove host key">
            <Trash2 size={12} />
          </button>
        </div>
      {/each}
    {/if}
  </div>

  <div class="section-h">Reconnect</div>
  <label class="row">
    <span class="row-label">Auto-reconnect on drop</span>
    <input type="checkbox" bind:checked={reconnectOnDrop} onchange={markDirty} />
  </label>
  <label class="row">
    <span class="row-label">Reconnect delay (s)</span>
    <input type="number" min="0" max="300" bind:value={reconnectDelay} oninput={markDirty} disabled={!reconnectOnDrop} />
  </label>

  <div class="section-h">UI</div>
  <label class="row">
    <span class="row-label">Show active host stats</span>
    <input type="checkbox" bind:checked={hostStatsEnabled} onchange={markDirty} />
  </label>
  <label class="row">
    <span class="row-label">Host stats interval (s)</span>
    <input type="number" min="10" max="3600" bind:value={hostStatsIntervalSec} oninput={markDirty} disabled={!hostStatsEnabled} />
  </label>
  <label class="row">
    <span class="row-label">Warn before closing active SSH tabs</span>
    <input type="checkbox" bind:checked={warnOnClose} onchange={markDirty} />
  </label>
</div>

<style>
  .section-h {
    margin-top: 16px;
    margin-bottom: 6px;
    font-size: 11.5px;
    text-transform: uppercase;
    color: var(--color-fg-muted);
    letter-spacing: 0.04em;
  }
  .row {
    display: grid;
    grid-template-columns: 220px 1fr;
    align-items: center;
    gap: 10px;
    padding: 4px 0;
  }
  .row-label { font-size: 12.5px; }
  .row input[type='text'],
  .row input[type='number'],
  .row select {
    padding: 4px 8px;
    background: var(--color-bg-soft);
    color: var(--color-fg);
    border: 1px solid var(--color-border);
    border-radius: 4px;
    font-size: 12.5px;
    width: 100%;
    max-width: 380px;
  }
  .row input:focus,
  .row select:focus { outline: none; border-color: var(--color-accent); }
  .row input:disabled { opacity: 0.5; }
  .inline-row {
    display: flex;
    gap: 8px;
    align-items: center;
    max-width: 520px;
  }
  .btn-secondary,
  .btn-danger {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 9px;
    border-radius: 4px;
    border: 1px solid var(--color-border);
    background: var(--color-bg-soft);
    color: var(--color-fg);
    font-size: 12px;
  }
  .btn-danger { color: var(--color-danger); }
  .btn-secondary:disabled,
  .btn-danger:disabled { opacity: 0.45; cursor: not-allowed; }
  .known-hosts-toolbar {
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--color-fg-muted);
    font-size: 11.5px;
    margin-bottom: 6px;
  }
  .known-hosts-list {
    border: 1px solid var(--color-border-soft);
    border-radius: 4px;
    overflow: hidden;
  }
  .known-host-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border-top: 1px solid var(--color-border-soft);
    background: var(--color-panel-2);
  }
  .known-host-row:first-child { border-top: 0; }
  .known-host-empty {
    padding: 8px;
    color: var(--color-fg-muted);
    font-style: italic;
  }
</style>
