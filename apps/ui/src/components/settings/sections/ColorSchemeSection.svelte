<script lang="ts">
  // Color scheme picker (M4). Persists the chosen scheme id under settings
  // key `terminalColorScheme`. TerminalPane consumes it as a palette
  // override layered on top of the app theme.

  import { onMount, onDestroy } from 'svelte';
  import type { RpcClient } from '../../../lib/rpc';
  import { COLOR_SCHEMES, type ColorScheme } from '../../../lib/colorSchemes';
  import { settingsCoord } from '../../../lib/settingsStore.svelte';

  interface Props {
    rpc: RpcClient;
    onError: (msg: string) => void;
  }
  let { rpc, onError }: Props = $props();

  let selected = $state<string>('');
  let query = $state('');

  const filtered = $derived(
    query
      ? COLOR_SCHEMES.filter((s) =>
          s.label.toLowerCase().includes(query.toLowerCase())
          || s.name.toLowerCase().includes(query.toLowerCase()))
      : COLOR_SCHEMES,
  );

  // Color schemes auto-apply on click — no need for a Save step. Persist
  // through the existing settings.set and bump the live-preview revision so
  // every open TerminalPane re-reads the palette immediately.
  async function applyNow(name: string) {
    selected = name;
    try {
      await rpc.call('settings.set', { key: 'terminalColorScheme', value: name });
      settingsCoord.bumpRev();
    } catch (e) {
      onError(`color scheme save: ${(e as Error).message}`);
    }
  }
  function pick(s: ColorScheme) { void applyNow(s.name); }
  function clearChoice() { void applyNow(''); }

  async function load() {
    try {
      const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'terminalColorScheme' });
      if (typeof r.value === 'string') selected = r.value;
    } catch (e) {
      onError(`color scheme load: ${(e as Error).message}`);
    }
  }
  async function save() {
    // Already auto-applied on pick — keep this as a no-op so the Save button
    // still works for other sections.
    await rpc.call('settings.set', { key: 'terminalColorScheme', value: selected });
    settingsCoord.bumpRev();
  }

  onMount(() => {
    settingsCoord.registerSaver('colorscheme', save);
    void load();
  });
  onDestroy(() => settingsCoord.unregisterSaver('colorscheme'));
</script>

<div class="settings-section">
  <h2>Color scheme</h2>

  <div>
    <input
      type="search" bind:value={query}
      placeholder="Filter {COLOR_SCHEMES.length} schemes…" class="input"
    />
    <div class="help">
      Overrides the terminal palette only. The application chrome (panels,
      sidebar) is controlled by the theme in <em>Appearance</em>.
    </div>
  </div>

  <div class="flex items-center gap-2">
    <button type="button" class="btn-secondary" onclick={clearChoice} disabled={!selected}>
      Use Appearance theme (no override)
    </button>
    {#if selected}
      <span class="text-[12px] text-[var(--color-fg-muted)]">
        Selected: <code>{selected}</code>
      </span>
    {/if}
  </div>

  <div class="grid grid-cols-2 gap-3">
    {#each filtered as scheme (scheme.name)}
      <button
        type="button"
        class="scheme-card"
        class:selected={selected === scheme.name}
        onclick={() => pick(scheme)}
        style="background: {scheme.background}; color: {scheme.foreground};
               border-color: {selected === scheme.name ? 'var(--color-accent)' : scheme.selection};"
        title={scheme.name}
      >
        <div class="scheme-label">{scheme.label}</div>
        <div class="scheme-swatches">
          {#each scheme.ansi as c, i (`${scheme.name}-${i}`)}
            <span style="background: {c};"></span>
          {/each}
        </div>
        <div class="scheme-sample" style="color: {scheme.foreground};">
          $ echo "Hello, world"
        </div>
      </button>
    {/each}
  </div>
</div>

<style>
  .scheme-card {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    border-radius: 6px;
    border-width: 2px;
    border-style: solid;
    cursor: pointer;
    text-align: left;
    transition: transform 80ms ease-out;
    font-family: inherit;
  }
  .scheme-card:hover { transform: translateY(-1px); }
  .scheme-card.selected { box-shadow: 0 0 0 1px var(--color-accent); }
  .scheme-label { font-weight: 600; font-size: 12px; }
  .scheme-swatches {
    display: grid;
    grid-template-columns: repeat(16, 1fr);
    height: 10px;
    border-radius: 2px;
    overflow: hidden;
  }
  .scheme-swatches > span { display: block; }
  .scheme-sample {
    font-family: 'JetBrains Mono', monospace;
    font-size: 11px;
    opacity: 0.85;
  }
</style>
