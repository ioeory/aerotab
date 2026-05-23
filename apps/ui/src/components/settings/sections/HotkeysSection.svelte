<script lang="ts">
  // M5 — Hotkeys editor. Lists every registered action grouped by category;
  // per-row inline recorder appends a key combo to that action.
  import { onMount, onDestroy } from 'svelte';
  import { Trash2, Plus, RotateCcw } from '@lucide/svelte';
  import type { RpcClient } from '../../../lib/rpc';
  import { settingsCoord } from '../../../lib/settingsStore.svelte';
  import { ACTIONS, hotkeys, formatEvent } from '../../../lib/hotkeys';

  interface Props { rpc: RpcClient; onError: (msg: string) => void }
  let { rpc, onError }: Props = $props();

  let query = $state('');
  // Per-action displayed bindings. Edits live here until save() writes the
  // canonical state back to HotkeyManager + sled.
  let bindings = $state<Record<string, string[]>>({});
  let recordingFor = $state<string | null>(null);

  const grouped = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const out = new Map<string, typeof ACTIONS>();
    for (const a of ACTIONS) {
      if (q && !a.label.toLowerCase().includes(q) && !a.id.includes(q)) continue;
      if (!out.has(a.category)) out.set(a.category, []);
      out.get(a.category)!.push(a);
    }
    return Array.from(out.entries());
  });

  function snapshot() {
    const m: Record<string, string[]> = {};
    for (const a of ACTIONS) m[a.id] = hotkeys.getBindings(a.id);
    bindings = m;
  }

  function startRecord(actionId: string) {
    recordingFor = actionId;
  }
  function onRecordKey(ev: KeyboardEvent) {
    if (!recordingFor) return;
    ev.preventDefault();
    ev.stopPropagation();
    if (ev.key === 'Escape') { recordingFor = null; return; }
    const formatted = formatEvent(ev);
    if (!formatted) return; // pure modifier press
    const cur = bindings[recordingFor] ?? [];
    if (!cur.includes(formatted)) bindings[recordingFor] = [...cur, formatted];
    recordingFor = null;
    settingsCoord.markDirty();
  }
  function removeBinding(actionId: string, idx: number) {
    const cur = bindings[actionId] ?? [];
    bindings[actionId] = cur.filter((_, i) => i !== idx);
    settingsCoord.markDirty();
  }
  function resetAction(actionId: string) {
    const def = ACTIONS.find((a) => a.id === actionId)?.defaultBindings ?? [];
    bindings[actionId] = [...def];
    settingsCoord.markDirty();
  }
  function resetAll() {
    hotkeys.resetToDefaults();
    snapshot();
    settingsCoord.markDirty();
  }

  async function load() {
    try {
      const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'hotkeys' });
      if (r.value && typeof r.value === 'object') {
        hotkeys.loadFromMap(r.value as Record<string, string[]>);
      } else {
        hotkeys.resetToDefaults();
      }
      snapshot();
    } catch (e) {
      onError(`hotkeys load: ${(e as Error).message}`);
    }
  }
  async function save() {
    for (const [id, list] of Object.entries(bindings)) {
      hotkeys.setBindings(id, list);
    }
    const map = hotkeys.toMap();
    await rpc.call('settings.set', { key: 'hotkeys', value: map });
  }

  onMount(() => {
    settingsCoord.registerSaver('hotkeys', save);
    void load();
    window.addEventListener('keydown', onRecordKey, true);
  });
  onDestroy(() => {
    settingsCoord.unregisterSaver('hotkeys');
    window.removeEventListener('keydown', onRecordKey, true);
  });
</script>

<div class="settings-section">
  <h2>Hotkeys</h2>

  <div class="flex items-center gap-2">
    <input type="search" class="input flex-1" bind:value={query}
      placeholder="Filter actions…" />
    <button type="button" class="btn-secondary" onclick={resetAll}>
      <RotateCcw size={12} /> Reset all
    </button>
  </div>

  {#each grouped as [cat, list] (cat)}
    <div class="hotkey-group">
      <div class="hotkey-cat">{cat}</div>
      {#each list as a (a.id)}
        <div class="hotkey-row">
          <div class="hotkey-label">
            <div class="font-medium">{a.label}</div>
            <div class="text-[11px] text-[var(--color-fg-muted)]"><code>{a.id}</code></div>
          </div>
          <div class="hotkey-keys">
            {#each (bindings[a.id] ?? []) as combo, i (combo + i)}
              <span class="kbd-chip">
                <kbd>{combo}</kbd>
                <button type="button" class="kbd-x" aria-label="Remove binding"
                  onclick={() => removeBinding(a.id, i)}>
                  <Trash2 size={11} />
                </button>
              </span>
            {/each}
            {#if recordingFor === a.id}
              <span class="kbd-chip recording">press keys… (Esc to cancel)</span>
            {:else}
              <button type="button" class="btn-secondary" onclick={() => startRecord(a.id)}>
                <Plus size={11} /> add
              </button>
            {/if}
            <button type="button" class="btn-secondary" onclick={() => resetAction(a.id)}
              title="Reset to default">
              <RotateCcw size={11} />
            </button>
          </div>
        </div>
      {/each}
    </div>
  {/each}
</div>

<style>
  .hotkey-group { margin-top: 12px; }
  .hotkey-cat {
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.14em;
    color: var(--color-fg-muted);
    padding: 6px 0;
    border-bottom: 1px solid var(--color-border-soft);
    margin-bottom: 4px;
  }
  .hotkey-row {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 16px;
    padding: 6px 0;
    align-items: center;
  }
  .hotkey-keys {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    align-items: center;
  }
  .kbd-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: var(--color-panel-2);
    border: 1px solid var(--color-border-soft);
    border-radius: 4px;
    padding: 2px 4px 2px 6px;
    font-size: 11px;
  }
  .kbd-chip.recording {
    color: var(--color-accent);
    border-color: var(--color-accent);
    padding: 2px 6px;
  }
  .kbd-chip kbd {
    font-family: 'JetBrains Mono', monospace;
    font-size: 11px;
  }
  .kbd-x {
    background: transparent;
    border: none;
    color: var(--color-fg-muted);
    cursor: pointer;
    padding: 0 2px;
  }
  .kbd-x:hover { color: var(--color-accent); }
</style>
