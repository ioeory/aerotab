<script lang="ts">
  import { onMount } from 'svelte';
  import { Copy, FileText, Save, RotateCcw, Download, Upload, Trash2 } from '@lucide/svelte';
  import type { RpcClient } from '../../../lib/rpc';
  import type { SettingEntry } from '../../../lib/types';

  interface Props {
    rpc: RpcClient;
    onError: (msg: string) => void;
    onChanged?: () => void;
  }
  let { rpc, onError, onChanged }: Props = $props();

  let yamlText = $state('');
  let originalText = $state('');
  let busy = $state(false);
  let status = $state<string | null>(null);
  let replaceMode = $state(false);
  let settingsEntries = $state<SettingEntry[]>([]);
  let keysBusy = $state(false);
  let keysStatus = $state('');
  let dirty = $derived(yamlText !== originalText);

  function settingSummary(value: unknown): string {
    if (value == null) return String(value);
    if (typeof value === 'string') return value;
    try {
      const text = JSON.stringify(value);
      return text.length > 120 ? `${text.slice(0, 120)}…` : text;
    } catch {
      return String(value);
    }
  }

  async function loadSettingsEntries() {
    keysBusy = true;
    try {
      const entries = await rpc.call<SettingEntry[]>('settings.all', {});
      settingsEntries = entries.sort((a, b) => a.key.localeCompare(b.key));
      keysStatus = `${settingsEntries.length} key${settingsEntries.length === 1 ? '' : 's'}`;
    } catch (e) {
      onError(`settings all: ${(e as Error).message}`);
    } finally {
      keysBusy = false;
    }
  }

  async function reload() {
    busy = true;
    status = null;
    try {
      const r = await rpc.call<{ yaml: string }>('settings.dumpYaml', {});
      yamlText = r.yaml ?? '';
      originalText = yamlText;
      await loadSettingsEntries();
    } catch (e) {
      onError(`dump yaml: ${(e as Error).message}`);
    } finally {
      busy = false;
    }
  }

  async function save() {
    busy = true;
    status = null;
    try {
      const r = await rpc.call<{ written: number }>('settings.loadYaml', {
        yaml: yamlText,
        replace: replaceMode,
      });
      originalText = yamlText;
      status = `Wrote ${r.written} key${r.written === 1 ? '' : 's'}.`;
      onChanged?.();
      await loadSettingsEntries();
    } catch (e) {
      onError(`load yaml: ${(e as Error).message}`);
    } finally {
      busy = false;
    }
  }

  function discard() {
    yamlText = originalText;
    status = 'Discarded changes.';
  }

  function downloadFile() {
    const blob = new Blob([yamlText], { type: 'text/yaml' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'aerotab-config.yaml';
    a.click();
    URL.revokeObjectURL(url);
  }

  function uploadFile(ev: Event) {
    const input = ev.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = () => {
      yamlText = String(reader.result ?? '');
      status = `Loaded ${file.name} into editor — review then Save.`;
    };
    reader.readAsText(file);
    input.value = '';
  }

  async function copySetting(entry: SettingEntry) {
    try {
      await navigator.clipboard.writeText(JSON.stringify(entry.value, null, 2));
      keysStatus = `copied ${entry.key}`;
    } catch (e) {
      onError(`copy setting: ${(e as Error).message}`);
    }
  }

  async function removeSetting(entry: SettingEntry) {
    if (!confirm(`Remove setting key "${entry.key}"?`)) return;
    keysBusy = true;
    try {
      const r = await rpc.call<{ removed: boolean }>('settings.remove', { key: entry.key });
      keysStatus = r.removed ? `removed ${entry.key}` : `${entry.key} was not present`;
      onChanged?.();
      await reload();
    } catch (e) {
      onError(`remove setting: ${(e as Error).message}`);
    } finally {
      keysBusy = false;
    }
  }

  onMount(reload);
</script>

<div class="settings-section">
  <h2 class="flex items-center gap-2"><FileText size={16} /> Config file</h2>
  <p class="text-[12.5px] text-[var(--color-fg-muted)] mb-3">
    The full settings document in YAML. Edit any value and click <strong>Save</strong>
    to apply. When <em>Replace all</em> is enabled, keys missing from the
    document will be removed.
  </p>

  <div class="flex items-center gap-2 mb-2 flex-wrap">
    <button class="btn-primary" onclick={save} disabled={busy || !dirty}>
      <Save size={14} /> Save
    </button>
    <button class="btn-secondary" onclick={discard} disabled={busy || !dirty}>
      <RotateCcw size={14} /> Discard
    </button>
    <button class="btn-secondary" onclick={reload} disabled={busy}>
      <RotateCcw size={14} /> Reload
    </button>
    <button class="btn-secondary" onclick={downloadFile} disabled={busy}>
      <Download size={14} /> Export
    </button>
    <label class="btn-secondary cursor-pointer">
      <Upload size={14} /> Import
      <input type="file" accept=".yaml,.yml,text/yaml" class="hidden" onchange={uploadFile} />
    </label>
    <label class="flex items-center gap-1.5 text-[12px] ml-auto">
      <input type="checkbox" bind:checked={replaceMode} />
      Replace all (remove missing keys)
    </label>
  </div>

  <textarea
    class="config-editor"
    bind:value={yamlText}
    spellcheck="false"
    placeholder="# settings store is empty"
  ></textarea>

  <div class="mt-2 text-[11.5px] text-[var(--color-fg-muted)] flex items-center gap-3">
    {#if dirty}<span class="text-[var(--color-accent)]">Unsaved changes</span>{/if}
    {#if status}<span>{status}</span>{/if}
  </div>

  <div class="section-h mt-5">Settings keys</div>
  <div class="flex items-center gap-2 mb-2">
    <button class="btn-secondary" onclick={loadSettingsEntries} disabled={keysBusy}>
      <RotateCcw size={14} /> Refresh keys
    </button>
    {#if keysStatus}<span class="text-[11.5px] text-[var(--color-fg-muted)]">{keysStatus}</span>{/if}
  </div>
  <div class="settings-key-list">
    {#if settingsEntries.length === 0}
      <div class="settings-key-empty">No settings keys stored.</div>
    {:else}
      {#each settingsEntries as entry (entry.key)}
        <div class="settings-key-row">
          <div class="min-w-0 flex-1">
            <div class="text-[12px] text-[var(--color-fg)] font-mono truncate">{entry.key}</div>
            <div class="text-[10.5px] text-[var(--color-fg-muted)] truncate">{settingSummary(entry.value)}</div>
          </div>
          <button class="btn-secondary !px-2" onclick={() => copySetting(entry)} disabled={keysBusy}
                  title="Copy value" aria-label="Copy value">
            <Copy size={12} />
          </button>
          <button class="btn-secondary danger !px-2" onclick={() => removeSetting(entry)} disabled={keysBusy}
                  title="Remove setting" aria-label="Remove setting">
            <Trash2 size={12} />
          </button>
        </div>
      {/each}
    {/if}
  </div>
</div>

<style>
  .config-editor {
    width: 100%;
    min-height: 460px;
    padding: 10px 12px;
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
    font-size: 12.5px;
    line-height: 1.45;
    background: var(--color-bg-soft);
    color: var(--color-fg);
    border: 1px solid var(--color-border);
    border-radius: 6px;
    resize: vertical;
    tab-size: 2;
  }
  .config-editor:focus {
    outline: none;
    border-color: var(--color-accent);
  }
  .btn-primary,
  .btn-secondary {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    font-size: 12px;
    border-radius: 4px;
    border: 1px solid var(--color-border);
  }
  .btn-primary {
    background: var(--color-accent);
    color: #fff;
    border-color: var(--color-accent);
  }
  .btn-primary:disabled,
  .btn-secondary:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
  .btn-secondary {
    background: var(--color-bg-soft);
    color: var(--color-fg);
  }
  .btn-secondary.danger { color: var(--color-danger); }
  .settings-key-list {
    border: 1px solid var(--color-border-soft);
    border-radius: 6px;
    overflow: hidden;
  }
  .settings-key-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 8px;
    border-top: 1px solid var(--color-border-soft);
    background: var(--color-panel-2);
  }
  .settings-key-row:first-child { border-top: 0; }
  .settings-key-empty {
    padding: 10px;
    color: var(--color-fg-muted);
    font-style: italic;
  }
</style>
