<script lang="ts">
  // Plugins: list / reload / invoke WASM plugins.

  import { onMount } from 'svelte';
  import { Puzzle, RefreshCw, Play, FolderOpen, Plus } from '@lucide/svelte';
  import type { RpcClient } from '../../../lib/rpc';
  import type { PluginRow } from '../../../lib/types';

  interface Props {
    rpc: RpcClient;
    onError: (msg: string) => void;
  }
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  let { rpc, onError: _onError }: Props = $props();

  let pluginList = $state<PluginRow[]>([]);
  let pluginBusy = $state(false);
  let pluginStatus = $state('');
  let pluginSelected = $state('');
  let pluginCommand = $state('say-hi');
  let pluginArgs = $state('');
  let pluginResult = $state('');
  let pluginDir = $state('');
  let pluginFile = $state('');

  async function refreshPlugins() {
    pluginBusy = true;
    try {
      pluginList = await rpc.call<PluginRow[]>('plugin.list');
      const first = pluginList[0];
      if (first && !pluginList.find((p) => p.name === pluginSelected)) {
        pluginSelected = first.name;
      }
      pluginStatus = `${pluginList.length} loaded`;
    } catch (e) {
      pluginStatus = `error: ${(e as Error).message ?? e}`;
    } finally {
      pluginBusy = false;
    }
  }

  async function reloadPlugins() {
    pluginBusy = true;
    try {
      const r = await rpc.call<{ loaded: number }>('plugin.reload');
      pluginStatus = `reloaded ${r.loaded}`;
      await refreshPlugins();
    } catch (e) {
      pluginStatus = `error: ${(e as Error).message ?? e}`;
    } finally {
      pluginBusy = false;
    }
  }

  async function invokePlugin() {
    if (!pluginSelected) return;
    pluginBusy = true;
    pluginResult = '';
    try {
      const r = await rpc.call<{ result: string }>('plugin.invoke', {
        name: pluginSelected,
        command: pluginCommand,
        args: pluginArgs,
      });
      pluginResult = r.result;
    } catch (e) {
      pluginResult = `error: ${(e as Error).message ?? e}`;
    } finally {
      pluginBusy = false;
    }
  }

  async function configurePluginDir() {
    if (!pluginDir.trim()) return;
    pluginBusy = true;
    try {
      const r = await rpc.call<{ loaded: number }>('plugin.configure', { path: pluginDir.trim() });
      pluginStatus = `loaded ${r.loaded} from directory`;
      await refreshPlugins();
    } catch (e) {
      pluginStatus = `error: ${(e as Error).message ?? e}`;
    } finally {
      pluginBusy = false;
    }
  }

  async function loadPluginFile() {
    if (!pluginFile.trim()) return;
    pluginBusy = true;
    try {
      const r = await rpc.call<{ name: string }>('plugin.load', { path: pluginFile.trim() });
      pluginStatus = `loaded ${r.name}`;
      pluginSelected = r.name;
      await refreshPlugins();
    } catch (e) {
      pluginStatus = `error: ${(e as Error).message ?? e}`;
    } finally {
      pluginBusy = false;
    }
  }

  onMount(() => { void refreshPlugins(); });
</script>

<div class="settings-section">
  <h2 class="flex items-center gap-1.5"><Puzzle size={14} /> Plugins</h2>

  <div class="row">
    <button type="button" class="btn-secondary flex items-center gap-1.5"
            onclick={reloadPlugins} disabled={pluginBusy}>
      <RefreshCw size={12} /> Reload
    </button>
    {#if pluginStatus}
      <span class="text-[11px] text-[var(--color-fg-muted)]">{pluginStatus}</span>
    {/if}
  </div>

  <div class="grid grid-cols-[1fr_auto] gap-2 items-end">
    <div>
      <label for="pl-dir" class="lbl">Plugin directory</label>
      <input id="pl-dir" bind:value={pluginDir} class="input" placeholder="/path/to/plugins" />
    </div>
    <button type="button" class="btn-secondary flex items-center gap-1.5"
            onclick={configurePluginDir} disabled={pluginBusy || !pluginDir.trim()}>
      <FolderOpen size={12} /> Load directory
    </button>
  </div>

  <div class="grid grid-cols-[1fr_auto] gap-2 items-end">
    <div>
      <label for="pl-file" class="lbl">Single WASM file</label>
      <input id="pl-file" bind:value={pluginFile} class="input" placeholder="/path/to/plugin.wasm" />
    </div>
    <button type="button" class="btn-secondary flex items-center gap-1.5"
            onclick={loadPluginFile} disabled={pluginBusy || !pluginFile.trim()}>
      <Plus size={12} /> Load file
    </button>
  </div>

  {#if pluginList.length === 0}
    <div class="help">
      No plugins found. Drop <code>.wasm</code> files into the app data
      <code>plugins/</code> directory and click Reload.
    </div>
  {:else}
    <ul class="border border-[var(--color-border)] rounded divide-y divide-[var(--color-border-soft)]">
      {#each pluginList as p (p.name)}
        <li class="px-2 py-1.5 flex items-center gap-2">
          <input type="radio" name="plugin-sel" value={p.name}
                 checked={pluginSelected === p.name}
                 onchange={() => (pluginSelected = p.name)} />
          <span class="font-medium text-[var(--color-fg)]">{p.name}</span>
          <span class="ml-auto text-[10.5px] text-[var(--color-fg-muted)] truncate max-w-[280px]"
                title={p.path}>{p.path}</span>
        </li>
      {/each}
    </ul>
    <div class="grid grid-cols-[1fr_2fr_auto] gap-2 items-end">
      <div>
        <label for="pl-cmd" class="lbl">Command</label>
        <input id="pl-cmd" bind:value={pluginCommand} class="input" />
      </div>
      <div>
        <label for="pl-args" class="lbl">Args</label>
        <input id="pl-args" bind:value={pluginArgs} class="input" />
      </div>
      <button type="button" class="btn-primary flex items-center gap-1.5"
              onclick={invokePlugin} disabled={pluginBusy || !pluginSelected}>
        <Play size={12} /> Invoke
      </button>
    </div>
    {#if pluginResult}
      <pre class="text-[11px] whitespace-pre-wrap bg-[var(--color-bg)]
                  border border-[var(--color-border)] rounded p-2 max-h-32 overflow-auto">{pluginResult}</pre>
    {/if}
  {/if}
</div>
