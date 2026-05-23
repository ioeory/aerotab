<script lang="ts">
  // Shell — auto-discovered built-in shells (CMD / PowerShell / pwsh / Git Bash
  // / WSL distros on Windows; bash / zsh / fish from /etc/shells on
  // Linux/macOS). The backend `profile.discover` RPC returns the canonical
  // list; this section lets the user pick one as the default for new local
  // tabs and quick-open any of them in a new tab.

  import { onMount, onDestroy } from 'svelte';
  import { Terminal as TerminalIcon, Plus } from '@lucide/svelte';
  import type { RpcClient } from '../../../lib/rpc';
  import { settingsCoord } from '../../../lib/settingsStore.svelte';
  import { tabs } from '../../../lib/tabs.svelte';

  interface Props {
    rpc: RpcClient;
    onError: (msg: string) => void;
  }
  let { rpc, onError }: Props = $props();

  interface ShellEntry {
    id: string;
    label: string;
    command: string;
    args: string[];
    icon: string;
  }

  let shells = $state<ShellEntry[]>([]);
  let defaultId = $state<string>('');
  let loading = $state(true);

  async function load() {
    loading = true;
    try {
      const r = await rpc.call<{ shells: ShellEntry[] }>('profile.discover');
      shells = Array.isArray(r.shells) ? r.shells : [];
    } catch (e) {
      onError(`shell discover: ${(e as Error).message}`);
    }
    try {
      const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'defaultShell' });
      if (typeof r.value === 'string') defaultId = r.value;
    } catch { /* not configured yet */ }
    loading = false;
  }

  function markDefault(s: ShellEntry) {
    defaultId = s.id;
    settingsCoord.markDirty();
  }

  async function openInTab(s: ShellEntry) {
    try {
      const meta = await rpc.call<{ id: string; kind: string; title: string }>(
        'session.openLocal',
        { title: s.label, shell: s.command, shell_args: s.args },
      );
      tabs.add({ id: meta.id, kind: meta.kind, title: meta.title });
    } catch (e) {
      onError(`open shell: ${(e as Error).message}`);
    }
  }

  async function save() {
    await rpc.call('settings.set', { key: 'defaultShell', value: defaultId });
  }

  onMount(() => {
    settingsCoord.registerSaver('shell', save);
    void load();
  });
  onDestroy(() => settingsCoord.unregisterSaver('shell'));
</script>

<div class="settings-section">
  <h2>Shell</h2>

  <div class="help">
    Built-in shells detected on this host. Click <em>Set default</em> to mark
    one for use by future local tabs, or <em>Open</em> to launch it
    immediately in a new tab.
  </div>

  {#if loading}
    <div class="placeholder">Scanning host…</div>
  {:else if shells.length === 0}
    <div class="placeholder">No shells detected.</div>
  {:else}
    <div class="flex flex-col gap-2">
      {#each shells as s (s.id)}
        <div class="shell-row" class:selected={defaultId === s.id}>
          <div class="flex items-center gap-2 min-w-0 flex-1">
            <TerminalIcon size={14} />
            <div class="flex flex-col min-w-0">
              <div class="text-[12.5px] font-medium truncate">{s.label}</div>
              <div class="text-[11px] text-[var(--color-fg-muted)] truncate font-mono">
                {s.command}{s.args.length ? ' ' + s.args.join(' ') : ''}
              </div>
            </div>
          </div>
          <div class="flex items-center gap-1.5">
            {#if defaultId === s.id}
              <span class="text-[10.5px] text-[var(--color-accent)] uppercase tracking-wider">Default</span>
            {:else}
              <button type="button" class="btn-secondary text-[11px]" onclick={() => markDefault(s)}>
                Set default
              </button>
            {/if}
            <button type="button" class="btn-secondary text-[11px] flex items-center gap-1"
                    onclick={() => openInTab(s)}>
              <Plus size={11} /> Open
            </button>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .shell-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border-radius: 6px;
    border: 1px solid var(--color-border-soft);
    background: var(--color-panel-2);
  }
  .shell-row.selected {
    border-color: var(--color-accent);
    box-shadow: 0 0 0 1px var(--color-accent);
  }
</style>
