<script lang="ts">
  // Appearance — full Tabby-parity options.
  // UI palette preferences persist separately as `uiAppearance`.
  // Terminal styling retains its two legacy settings keys:
  //   - `font`        : { family, size }                  (legacy compatibility)
  //   - `appearance`  : { ligatures, fontWeight, fontWeightBold, fallbackFont,
  //                       cursorStyle, minContrastRatio, linePadding, customCss }

  import { onMount, onDestroy } from 'svelte';
  import type { RpcClient } from '../../../lib/rpc';
  import { applyTheme, BUILTIN_THEMES, UI_PALETTES } from '../../../lib/theme';
  import { saveSettingsTransaction } from '../../../lib/settingsTransaction';
  import { settingsCoord } from '../../../lib/settingsStore.svelte';
  import { applyCustomCss, applyLigatures } from '../../../lib/customCss';
  import { settingsText as text } from '../settingsMessages';
  import { createAppearanceDraft, normalizeAppearancePreferences, resolveAppearancePalette, isLightPalette, type AppearancePreferences } from '../../../lib/appearancePreferences';

  let preferences = $state<AppearancePreferences>(normalizeAppearancePreferences(null, 'tokyo-night', BUILTIN_THEMES));
  let loaded = $state(false);
  let loading = $state(false);
  let loadError = $state('');
  let busy = $state(false);
  let systemDark = $state(true);
  let appearanceExtras: Record<string, unknown> = {};
  let fontExtras: Record<string, unknown> = {};
  let appliedFields: ReturnType<typeof currentFields> | null = null;
  let renderedPreferences: AppearancePreferences | null = null;
  let draft: ReturnType<typeof createAppearanceDraft> | null = null;
  const paletteIsDark = $derived(preferences.mode === 'system' ? systemDark : preferences.mode === 'dark');
  const visiblePalettes = $derived(BUILTIN_THEMES.filter(p => isLightPalette(p) !== paletteIsDark));

  function renderPreferences(value: AppearancePreferences) {
    renderedPreferences = { ...value };
    const theme = BUILTIN_THEMES.find(p => p.name === resolveAppearancePalette(value, systemDark));
    if (theme) applyTheme(theme, value.accent);
  }

  function currentFields() {
    return { fontFamily, fontSize, ligatures, fontWeight, fontWeightBold, fallbackFont, cursorStyle, minContrastRatio, linePadding, customCss };
  }

  function cancelPreview() {
    if (!draft || !appliedFields) return;
    preferences = draft.cancel();
    ({ fontFamily, fontSize, ligatures, fontWeight, fontWeightBold, fallbackFont, cursorStyle, minContrastRatio, linePadding, customCss } = appliedFields);
    applyCustomCss(customCss);
    applyLigatures(ligatures);
    settingsCoord.markClean('appearance');
  }

  function resetAppearance() {
    preferences = normalizeAppearancePreferences({ mode: 'system' }, '', BUILTIN_THEMES);
    markDirty();
  }

  async function apply() {
    if (!loaded || busy) return;
    busy = true;
    try { await settingsCoord.saveSection('appearance'); }
    catch (error) { onError((error as Error).message); }
    finally { busy = false; }
  }

  interface Props {
    rpc: RpcClient;
    onError: (msg: string) => void;
  }
  let { rpc, onError }: Props = $props();

  type CursorStyle = 'block' | 'bar' | 'underline';

  // Theme / font (legacy keys)
  let themeName = $state('tokyo-night');
  let fontFamily = $state('JetBrains Mono, Menlo, monospace');
  let fontSize = $state(15);

  // Appearance group
  let ligatures = $state(false);
  let fontWeight = $state(400);
  let fontWeightBold = $state(700);
  let fallbackFont = $state('');
  let cursorStyle = $state<CursorStyle>('block');
  let minContrastRatio = $state(1);
  let linePadding = $state(0);
  let customCss = $state('');

  function markDirty() { settingsCoord.markDirty('appearance'); }

  async function load() {
    if (loading) return;
    loading = true;
    loadError = '';
    try {
      const t = await rpc.call<{ value: unknown }>('settings.get', { key: 'theme' });
      if (typeof t.value === 'string') themeName = t.value;

      const ui = await rpc.call<{ value: unknown }>('settings.get', { key: 'uiAppearance' });
      preferences = normalizeAppearancePreferences(ui.value, themeName, BUILTIN_THEMES);

      const f = await rpc.call<{ value: unknown }>('settings.get', { key: 'font' });
      if (f.value && typeof f.value === 'object') {
        const v = f.value as Record<string, unknown>;
        fontExtras = { ...v };
        if (typeof v.family === 'string') fontFamily = v.family;
        if (typeof v.size === 'number') fontSize = v.size;
      }

      const a = await rpc.call<{ value: unknown }>('settings.get', { key: 'appearance' });
      if (a.value && typeof a.value === 'object') {
        const v = a.value as Record<string, unknown>;
        appearanceExtras = { ...v };
        if (typeof v.ligatures === 'boolean') ligatures = v.ligatures;
        if (typeof v.fontWeight === 'number') fontWeight = v.fontWeight;
        if (typeof v.fontWeightBold === 'number') fontWeightBold = v.fontWeightBold;
        if (typeof v.fallbackFont === 'string') fallbackFont = v.fallbackFont;
        if (v.cursorStyle === 'block' || v.cursorStyle === 'bar' || v.cursorStyle === 'underline') {
          cursorStyle = v.cursorStyle;
        }
        if (typeof v.minContrastRatio === 'number') minContrastRatio = v.minContrastRatio;
        if (typeof v.linePadding === 'number') linePadding = v.linePadding;
        if (typeof v.customCss === 'string') customCss = v.customCss;
      }
      appliedFields = currentFields();
      draft = createAppearanceDraft(preferences, {
        render: renderPreferences,
        persist: async value => {
          const fields = currentFields();
          const { fontFamily: family, fontSize: size, ...appearance } = fields;
          await saveSettingsTransaction(rpc, [
            { key: 'font', value: { ...fontExtras, family, size } },
            { key: 'appearance', value: { ...appearanceExtras, ...appearance } },
            { key: 'uiAppearance', value },
          ]);
        },
      });
      loaded = true;
    } catch (e) {
      loadError = (e as Error).message ?? String(e);
      onError(`appearance load: ${loadError}`);
    } finally { loading = false; }
  }

  async function save() {
    try { await persistAppearance(); }
    catch (error) { draft?.cancel(); throw error; }
  }

  async function persistAppearance() {
    if (!loaded || !draft) throw new Error('Appearance settings are not loaded yet');
    await draft.apply(preferences);
    appliedFields = currentFields();
    applyCustomCss(customCss);
    applyLigatures(ligatures);
  }

  function selectPalette(name: string) {
    preferences = paletteIsDark ? { ...preferences, darkPalette: name } : { ...preferences, lightPalette: name };
    markDirty();
  }

  onMount(() => {
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    systemDark = media.matches;
    const changed = () => {
      systemDark = media.matches;
      if (renderedPreferences?.mode === 'system') renderPreferences(renderedPreferences);
    };
    media.addEventListener('change', changed);
    settingsCoord.registerSaver('appearance', save);
    void load();
    return () => media.removeEventListener('change', changed);
  });
  onDestroy(() => {
    draft?.cancel();
    settingsCoord.unregisterSaver('appearance');
  });
</script>

<div class="settings-section">
  <h2>{text('settings.nav.appearance')}</h2>
  {#if loadError}
    <div class="appearance-load-error" role="alert">
      <p>{loadError}</p>
      <button type="button" class="btn-secondary" disabled={loading} onclick={load}>{text('common.refresh')}</button>
    </div>
  {:else if loading}
    <p role="status">{text('common.loading')}</p>
  {/if}

  <fieldset disabled={!loaded || busy || settingsCoord.saving} class="appearance-palette">
    <legend class="section-h">{text('settings.appearance.mode')}</legend>
    <div class="mode-switch" role="group" aria-label={text('settings.appearance.mode')}>
      {#each ['light', 'system', 'dark'] as mode}
        <button type="button" class:active={preferences.mode === mode} aria-pressed={preferences.mode === mode}
          onclick={() => { preferences = { ...preferences, mode: mode as AppearancePreferences['mode'] }; markDirty(); }}>
          {text(`settings.appearance.${mode}`)}
        </button>
      {/each}
    </div>
    <div class="section-h">{text('settings.appearance.palette')}</div>
    <p class="help">{text('settings.appearance.independent')}</p>
    <div class="palette-grid">
      {#each visiblePalettes as theme (theme.name)}
        <button type="button" class="theme-card" class:selected={resolveAppearancePalette(preferences, systemDark) === theme.name}
          aria-pressed={resolveAppearancePalette(preferences, systemDark) === theme.name} onclick={() => selectPalette(theme.name)}>
          <div class="theme-strip" style="background:{UI_PALETTES[theme.name]?.bg ?? theme.bg}" aria-hidden="true">
            <div class="theme-dot" style="background:{UI_PALETTES[theme.name]?.accent ?? theme.accent}"></div>
            <div class="theme-dot" style="background:{UI_PALETTES[theme.name]?.fg ?? theme.fg}"></div>
          </div>
          <span>{theme.label}</span>
        </button>
      {/each}
    </div>
    <label class="row accent-row">
      <input type="checkbox" checked={preferences.accent !== null} onchange={(event) => {
        preferences = { ...preferences, accent: event.currentTarget.checked ? '#5b8def' : null }; markDirty();
      }} />
      <span>{text('settings.appearance.accent')}</span>
      {#if preferences.accent}
        <input type="color" aria-label={text('settings.appearance.accent')} value={preferences.accent}
          oninput={(event) => { preferences = { ...preferences, accent: event.currentTarget.value }; markDirty(); }} />
      {/if}
    </label>
    <p class="help">{text('settings.appearance.accentHint')}</p>
    <div class="preview-actions">
      <button class="btn-secondary" type="button" onclick={() => draft?.preview(preferences)}>{text('settings.appearance.preview')}</button>
      <button class="btn-primary" type="button" onclick={apply}>{text('settings.appearance.apply')}</button>
      <button class="btn-secondary" type="button" onclick={cancelPreview}>{text('settings.appearance.cancelPreview')}</button>
      <button class="btn-secondary" type="button" onclick={resetAppearance}>{text('settings.appearance.reset')}</button>
    </div>
    <p class="help">{text('settings.appearance.previewHint')}</p>
  </fieldset>

  <fieldset disabled={!loaded || busy || settingsCoord.saving}>
    <legend class="section-h">{text('settings.appearance.font')}</legend>
    <label for="ap-font-family" class="lbl">{text('settings.appearance.family')}</label>
    <input id="ap-font-family" bind:value={fontFamily} oninput={markDirty} class="input" placeholder="JetBrains Mono" />
    <label for="ap-fallback" class="lbl">{text('settings.appearance.fallback')}</label>
    <input id="ap-fallback" bind:value={fallbackFont} oninput={markDirty} class="input" placeholder="Menlo, Consolas, monospace" />
    <div class="help">{text('settings.appearance.fallbackHint')}</div>
    <div class="font-grid">
      <label class="lbl">{text('settings.appearance.size')}
        <input type="number" min="8" max="32" bind:value={fontSize} oninput={markDirty} class="input" />
      </label>
      <label class="lbl">{text('settings.appearance.weight')}
        <input type="number" min="100" max="900" step="100" bind:value={fontWeight} oninput={markDirty} class="input" />
      </label>
      <label class="lbl">{text('settings.appearance.boldWeight')}
        <input type="number" min="100" max="900" step="100" bind:value={fontWeightBold} oninput={markDirty} class="input" />
      </label>
    </div>
    <label class="row mt-3"><input type="checkbox" bind:checked={ligatures} onchange={markDirty} />{text('settings.appearance.ligatures')}</label>
  </fieldset>
  <fieldset disabled={!loaded || busy || settingsCoord.saving}>
    <legend class="section-h">{text('settings.appearance.cursor')}</legend>
    <label for="ap-cursor-style" class="lbl">{text('settings.appearance.cursorShape')}</label>
    <select id="ap-cursor-style" bind:value={cursorStyle} onchange={markDirty} class="select">
      <option value="block">{text('settings.appearance.block')}</option>
      <option value="bar">{text('settings.appearance.bar')}</option>
      <option value="underline">{text('settings.appearance.underline')}</option>
    </select>
  </fieldset>
  <fieldset disabled={!loaded || busy || settingsCoord.saving}>
    <legend class="section-h">{text('settings.appearance.layout')}</legend>
    <label for="ap-line-padding" class="lbl">{text('settings.appearance.linePadding')}</label>
    <input id="ap-line-padding" type="number" min="0" max="20" step="1" bind:value={linePadding} oninput={markDirty} class="input" />
    <div class="help">{text('settings.appearance.linePaddingHint')}</div>
    <label for="ap-min-contrast" class="lbl">{text('settings.appearance.contrast')}</label>
    <input id="ap-min-contrast" type="number" min="1" max="21" step="0.5" bind:value={minContrastRatio} oninput={markDirty} class="input" />
    <div class="help">{text('settings.appearance.contrastHint')}</div>
  </fieldset>
  <fieldset disabled={!loaded || busy || settingsCoord.saving}>
    <legend class="section-h">{text('settings.appearance.customCss')}</legend>
    <label for="ap-custom-css" class="lbl">{text('settings.appearance.cssLabel')}</label>
    <textarea id="ap-custom-css" bind:value={customCss} oninput={markDirty} class="input css-area" rows="6"
      placeholder={'.xterm-screen { filter: brightness(1.05); }'}></textarea>
    <div class="help">{text('settings.appearance.cssHint')}</div>
  </fieldset>
</div>

<style>
  .appearance-load-error { padding: 16px; border: 1px solid var(--color-danger); border-radius: 8px; overflow-wrap: anywhere; }
  .appearance-load-error p { margin: 0 0 12px; }
  fieldset { border: 0; padding: 0; min-width: 0; }
  .mode-switch { display: flex; gap: 4px; padding: 4px; background: var(--color-panel-2); border: 1px solid var(--color-border); border-radius: 10px; }
  .mode-switch button { flex: 1; min-height: 44px; border-radius: 7px; }
  .mode-switch button.active { background: var(--color-panel); color: var(--color-accent); box-shadow: inset 0 0 0 1px var(--color-accent); }
  .palette-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(145px, 1fr)); gap: 12px; margin: 16px 0; }
  .font-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); gap: 12px; }
  .preview-actions { display: flex; flex-wrap: wrap; gap: 8px; margin: 16px 0 8px; }
  .accent-row { margin-top: 20px; }
  .accent-row input[type=color] { margin-left: auto; width: 64px; padding: 4px; }
  .theme-card span { display: block; margin-top: 8px; font-size: 16px; }
  .theme-card {
    background: var(--color-panel-2);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    padding: 8px;
    cursor: pointer;
    text-align: center;
  }
  .theme-card.selected {
    border-color: var(--color-accent);
    box-shadow: 0 0 0 1px var(--color-accent);
  }
  .theme-strip {
    width: 100%;
    height: 36px;
    border-radius: var(--radius-sm);
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
  }
  .theme-dot {
    width: 12px;
    height: 12px;
    border-radius: 50%;
  }
  .css-area {
    font-family: var(--font-mono);
    resize: vertical;
    min-height: 96px;
  }
</style>
