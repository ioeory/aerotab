<script lang="ts">
  // Application: updater controls + core version display.

  import { onMount } from 'svelte';
  import { Download, RefreshCw, Trash2 } from '@lucide/svelte';
  import { tauriInvoke, type RpcClient } from '../../../lib/rpc';
  import type { SessionMeta } from '../../../lib/types';

  interface Props {
    rpc: RpcClient;
    onError: (msg: string) => void;
  }
  let { rpc, onError }: Props = $props();

  let coreVersion = $state<string | null>(null);
  let protocolVersion = $state<number | null>(null);
  let sessions = $state<SessionMeta[]>([]);
  let sessionsBusy = $state(false);
  let sessionsStatus = $state('');

  let updateStatus = $state('');
  let updateAvailable = $state<{ version: string; current: string; notes?: string } | null>(null);
  let updateBusy = $state(false);

  async function loadVersion() {
    try {
      const v = await rpc.call<{ version: string }>('core.version');
      coreVersion = v.version;
      protocolVersion = await rpc.call<number>('core.protocolVersion', {});
    } catch {
      coreVersion = null;
      protocolVersion = null;
    }
  }

  async function refreshSessions() {
    sessionsBusy = true;
    sessionsStatus = '';
    try {
      sessions = await rpc.call<SessionMeta[]>('session.list', {});
      sessionsStatus = `${sessions.length} session${sessions.length === 1 ? '' : 's'}`;
    } catch (e) {
      onError(`session list: ${(e as Error).message}`);
    } finally {
      sessionsBusy = false;
    }
  }

  async function closeSession(id: string) {
    if (!confirm('Close this backend session?')) return;
    sessionsBusy = true;
    try {
      await rpc.call('session.close', { id });
      await refreshSessions();
    } catch (e) {
      onError(`session close: ${(e as Error).message}`);
    } finally {
      sessionsBusy = false;
    }
  }

  async function checkForUpdates() {
    updateStatus = 'checking…';
    updateAvailable = null;
    updateBusy = true;
    try {
      const p = tauriInvoke<{ available: boolean; version?: string; current?: string; notes?: string }>(
        'check_update',
      );
      if (!p) { updateStatus = 'updater unavailable in dev build'; return; }
      const r = await p;
      if (r.available && r.version && r.current) {
        updateAvailable = { version: r.version, current: r.current, notes: r.notes };
        updateStatus = `update ${r.version} available (current ${r.current})`;
      } else {
        updateStatus = 'up to date';
      }
    } catch (e) {
      updateStatus = `error: ${(e as Error).message ?? e}`;
    } finally {
      updateBusy = false;
    }
  }

  async function installUpdate() {
    if (!updateAvailable) return;
    updateStatus = 'downloading…';
    updateBusy = true;
    try {
      const p = tauriInvoke<void>('install_update');
      if (!p) { updateStatus = 'updater unavailable in dev build'; return; }
      await p;
      updateStatus = 'installed — restart to apply';
    } catch (e) {
      updateStatus = `error: ${(e as Error).message ?? e}`;
    } finally {
      updateBusy = false;
    }
  }

  onMount(() => { void loadVersion(); });
</script>

<div class="settings-section">
  <h2>Application</h2>

  <div>
    <div class="section-h">Version</div>
    <div class="text-[var(--color-fg)]">
      Tabby v2 — core {coreVersion ?? '…'}
    </div>
    <div class="help">Protocol version {protocolVersion ?? '…'}</div>
  </div>

  <div>
    <div class="section-h">Runtime sessions</div>
    <div class="row">
      <button type="button" class="btn-secondary flex items-center gap-1.5"
              onclick={refreshSessions} disabled={sessionsBusy}>
        <RefreshCw size={12} /> Refresh sessions
      </button>
      {#if sessionsStatus}<span class="help">{sessionsStatus}</span>{/if}
    </div>
    {#if sessions.length > 0}
      <div class="border border-[var(--color-border)] rounded divide-y divide-[var(--color-border-soft)] mt-2">
        {#each sessions as s (s.id)}
          <div class="flex items-center gap-2 px-2 py-1.5 text-[12px]">
            <div class="min-w-0 flex-1">
              <div class="truncate text-[var(--color-fg)]">{s.title}</div>
              <div class="truncate text-[10.5px] text-[var(--color-fg-muted)] font-mono">
                {s.kind} · {s.id}
              </div>
            </div>
            <button type="button" class="btn-secondary !px-2 !py-1"
                    onclick={() => closeSession(s.id)} disabled={sessionsBusy}
                    title="Close backend session" aria-label="Close backend session">
              <Trash2 size={12} />
            </button>
          </div>
        {/each}
      </div>
    {/if}
  </div>

  <div>
    <div class="section-h">Updates</div>
    <div class="row">
      <button type="button" class="btn-secondary" onclick={checkForUpdates} disabled={updateBusy}>
        Check for updates
      </button>
      {#if updateAvailable}
        <button
          type="button"
          class="btn-primary flex items-center gap-1.5"
          onclick={installUpdate}
          disabled={updateBusy}
        >
          <Download size={12} /> Install {updateAvailable.version}
        </button>
      {/if}
    </div>
    {#if updateStatus}
      <div class="help">{updateStatus}</div>
    {/if}
    {#if updateAvailable?.notes}
      <pre class="mt-2 text-[11px] whitespace-pre-wrap bg-[var(--color-bg)]
                  border border-[var(--color-border)] rounded p-2 max-h-32 overflow-auto">{updateAvailable.notes}</pre>
    {/if}
  </div>
</div>
