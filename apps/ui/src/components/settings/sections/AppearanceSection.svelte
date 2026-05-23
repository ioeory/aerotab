<script lang="ts">
  // Appearance — full Tabby-parity options.
  // Persists to two settings keys:
  //   - `font`        : { family, size }                  (legacy compatibility)
  //   - `appearance`  : { ligatures, fontWeight, fontWeightBold, fallbackFont,
  //                       cursorStyle, minContrastRatio, linePadding, customCss }

  import { onMount, onDestroy } from 'svelte';
  import type { RpcClient } from '../../../lib/rpc';
  import { applyTheme, BUILTIN_THEMES } from '../../../lib/theme';
  import { settingsCoord } from '../../../lib/settingsStore.svelte';
  import { applyCustomCss } from '../../../lib/customCss';

  interface Props {
    rpc: RpcClient;
    onError: (msg: string) => void;
  }
  let { rpc, onError }: Props = $props();

  type CursorStyle = 'block' | 'bar' | 'underline';

  // Theme / font (legacy keys)
  let themeName = $state('tokyo-night');
  let fontFamily = $state('JetBrains Mono, Menlo, monospace');
  let fontSize = $state(13);

  // Appearance group
  let ligatures = $state(false);
  let fontWeight = $state(400);
  let fontWeightBold = $state(700);
  let fallbackFont = $state('');
  let cursorStyle = $state<CursorStyle>('block');
  let minContrastRatio = $state(1);
  let linePadding = $state(0);
  let customCss = $state('');

  function markDirty() { settingsCoord.markDirty(); }

  async function load() {
    try {
      const t = await rpc.call<{ value: unknown }>('settings.get', { key: 'theme' });
      if (typeof t.value === 'string') themeName = t.value;

      const f = await rpc.call<{ value: unknown }>('settings.get', { key: 'font' });
      if (f.value && typeof f.value === 'object') {
        const v = f.value as Record<string, unknown>;
        if (typeof v.family === 'string') fontFamily = v.family;
        if (typeof v.size === 'number') fontSize = v.size;
      }

      const a = await rpc.call<{ value: unknown }>('settings.get', { key: 'appearance' });
      if (a.value && typeof a.value === 'object') {
        const v = a.value as Record<string, unknown>;
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
    } catch (e) {
      onError(`appearance load: ${(e as Error).message}`);
    }
  }

  async function save() {
    await rpc.call('settings.set', { key: 'theme', value: themeName });
    await rpc.call('settings.set', {
      key: 'font',
      value: { family: fontFamily, size: fontSize },
    });
    await rpc.call('settings.set', {
      key: 'appearance',
      value: {
        ligatures,
        fontWeight,
        fontWeightBold,
        fallbackFont,
        cursorStyle,
        minContrastRatio,
        linePadding,
        customCss,
      },
    });
    const th = BUILTIN_THEMES.find((x) => x.name === themeName);
    if (th) applyTheme(th);
    applyCustomCss(customCss);
  }

  function previewTheme(name: string) {
    const t = BUILTIN_THEMES.find((x) => x.name === name);
    if (t) applyTheme(t);
    themeName = name;
    markDirty();
  }

  onMount(() => {
    settingsCoord.registerSaver('appearance', save);
    void load();
  });
  onDestroy(() => settingsCoord.unregisterSaver('appearance'));
</script>

<div class="settings-section">
  <h2>Appearance</h2>

  <div>
    <div class="section-h">Theme</div>
    <div class="grid grid-cols-3 gap-2">
      {#each BUILTIN_THEMES as theme (theme.name)}
        <button
          type="button"
          class="theme-card {themeName === theme.name ? 'selected' : ''}"
          onclick={() => previewTheme(theme.name)}
        >
          <div class="theme-strip" style="background:{theme.bg}">
            <div class="theme-dot" style="background:{theme.accent}"></div>
            <div class="theme-dot" style="background:{theme.fg}"></div>
          </div>
          <div class="text-[11px] mt-1 text-[var(--color-fg)]">{theme.label}</div>
        </button>
      {/each}
    </div>
  </div>

  <div>
    <div class="section-h">Font</div>
    <label for="ap-font-family" class="lbl">Font family</label>
    <input
      id="ap-font-family"
      bind:value={fontFamily}
      oninput={markDirty}
      class="input"
      placeholder="JetBrains Mono"
    />

    <label for="ap-fallback" class="lbl">Fallback font</label>
    <input
      id="ap-fallback"
      bind:value={fallbackFont}
      oninput={markDirty}
      class="input"
      placeholder="Menlo, Consolas, monospace"
    />
    <div class="help">Appended to the family list — used when glyphs are missing.</div>

    <div class="grid grid-cols-3 gap-3 mt-3">
      <div>
        <label for="ap-font-size" class="lbl">Size (px)</label>
        <input
          id="ap-font-size"
          type="number"
          min="8"
          max="32"
          bind:value={fontSize}
          oninput={markDirty}
          class="input"
        />
      </div>
      <div>
        <label for="ap-font-weight" class="lbl">Normal weight</label>
        <input
          id="ap-font-weight"
          type="number"
          min="100"
          max="900"
          step="100"
          bind:value={fontWeight}
          oninput={markDirty}
          class="input"
        />
      </div>
      <div>
        <label for="ap-font-weight-bold" class="lbl">Bold weight</label>
        <input
          id="ap-font-weight-bold"
          type="number"
          min="100"
          max="900"
          step="100"
          bind:value={fontWeightBold}
          oninput={markDirty}
          class="input"
        />
      </div>
    </div>

    <label class="row mt-3">
      <input type="checkbox" bind:checked={ligatures} onchange={markDirty} />
      Enable font ligatures
    </label>
  </div>

  <div>
    <div class="section-h">Cursor</div>
    <label for="ap-cursor-style" class="lbl">Cursor shape</label>
    <select
      id="ap-cursor-style"
      bind:value={cursorStyle}
      onchange={markDirty}
      class="select"
    >
      <option value="block">Block</option>
      <option value="bar">Bar</option>
      <option value="underline">Underline</option>
    </select>
  </div>

  <div>
    <div class="section-h">Layout</div>
    <div class="grid grid-cols-2 gap-3">
      <div>
        <label for="ap-line-padding" class="lbl">Line padding (px)</label>
        <input
          id="ap-line-padding"
          type="number"
          min="0"
          max="20"
          step="1"
          bind:value={linePadding}
          oninput={markDirty}
          class="input"
        />
        <div class="help">Adds vertical spacing between lines.</div>
      </div>
      <div>
        <label for="ap-min-contrast" class="lbl">Minimum contrast ratio</label>
        <input
          id="ap-min-contrast"
          type="number"
          min="1"
          max="21"
          step="0.5"
          bind:value={minContrastRatio}
          oninput={markDirty}
          class="input"
        />
        <div class="help">1 = off, 4.5 = WCAG AA, 7 = WCAG AAA.</div>
      </div>
    </div>
  </div>

  <div>
    <div class="section-h">Custom CSS</div>
    <label for="ap-custom-css" class="lbl">CSS injected into the application document</label>
    <textarea
      id="ap-custom-css"
      bind:value={customCss}
      oninput={markDirty}
      class="input css-area"
      rows="6"
      placeholder={'.xterm-screen { filter: brightness(1.05); }'}
    ></textarea>
    <div class="help">
      Use with care — invalid CSS is ignored, but bad selectors can hide UI.
    </div>
  </div>
</div>

<style>
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
