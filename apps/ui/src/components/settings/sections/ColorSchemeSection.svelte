<script lang="ts">
  // Window-local preview transaction. Only Apply / Save persists the override;
  // cancel and section teardown restore the existing terminal palette.

  import { onMount, onDestroy } from 'svelte';
  import type { RpcClient } from '../../../lib/rpc';
  import { createTerminalPaletteDraft, setTerminalPalettePreview } from '../../../lib/terminalPalettePreview';
  import { COLOR_SCHEMES, type ColorScheme } from '../../../lib/colorSchemes';
  import { i18n } from '../../../lib/i18n.svelte';
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

  let saving = $state(false);
  let loaded = $state(false);
  let draft: ReturnType<typeof createTerminalPaletteDraft> | null = null;
  function preview(name: string) {
    if (!draft || saving) return;
    selected = draft.preview(name);
    settingsCoord.markDirty('colorscheme');
  }
  function pick(s: ColorScheme) { preview(s.name); }
  function clearChoice() { preview(''); }
  function cancel() {
    if (!draft || saving) return;
    selected = draft.cancel();
    settingsCoord.markClean('colorscheme');
  }
  async function apply() {
    try { await settingsCoord.saveSection('colorscheme'); }
    catch (e) { onError(`color scheme save: ${(e as Error).message}`); }
  }

  async function load() {
    try {
      const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'terminalColorScheme' });
      if (typeof r.value === 'string') selected = r.value;
      draft = createTerminalPaletteDraft(selected, { render: setTerminalPalettePreview, persist: value => rpc.call('settings.set', { key: 'terminalColorScheme', value }) });
      loaded = true;
    } catch (e) {
      onError(`color scheme load: ${(e as Error).message}`);
    }
  }
  async function save() {
    if (!draft || !loaded) throw new Error('Terminal palettes have not loaded');
    saving = true;
    try { selected = await draft.apply(); }
    catch (error) { selected = draft.cancel(); throw error; }
    finally { saving = false; }
  }

  onMount(() => {
    settingsCoord.registerSaver('colorscheme', save);
    void load();
  });
  onDestroy(() => { draft?.cancel(); settingsCoord.markClean('colorscheme'); settingsCoord.unregisterSaver('colorscheme'); });
</script>

<div class="settings-section">
  <h2>{i18n.t('settings.terminalPalette.title')}</h2>
  <p role="status">{i18n.t(selected ? 'settings.terminalPalette.independent' : 'settings.terminalPalette.following')}</p>

  <div>
    <input
      type="search" bind:value={query}
      placeholder={i18n.t('settings.terminalPalette.filter')} aria-label={i18n.t('settings.terminalPalette.filter')} class="input"
    />
    <div class="help">
      {i18n.t('settings.terminalPalette.hint')}
    </div>
  </div>

  <div class="flex items-center gap-2">
    <button type="button" class="btn-secondary" onclick={clearChoice} disabled={!loaded || !selected || saving}>
      {i18n.t('settings.terminalPalette.follow')}
    </button>
    {#if selected}
      <span class="text-[12px] text-[var(--color-fg-muted)]">
        Selected: <code>{selected}</code>
      </span>
    {/if}
  </div>

  <div class="flex gap-2">
    <button type="button" class="btn-primary" disabled={!loaded || saving} onclick={apply}>{i18n.t('settings.appearance.apply')}</button>
    <button type="button" class="btn-secondary" disabled={!loaded || saving} onclick={cancel}>{i18n.t('settings.appearance.cancelPreview')}</button>
  </div>
  <div class="grid grid-cols-2 gap-3">
    {#each filtered as scheme (scheme.name)}
      <button
        type="button"
        class="scheme-card"
        class:selected={selected === scheme.name}
        disabled={!loaded || saving} aria-pressed={selected === scheme.name}
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
