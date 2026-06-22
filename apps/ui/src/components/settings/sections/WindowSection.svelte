<script lang="ts">
  // Window — M6. Window chrome / opacity / tray / docking parity with Tabby.
  // Persists to settings key `window`. Most settings require restart since
  // Tauri does not expose live runtime mutators for every flag.
  import { onMount, onDestroy } from 'svelte';
  import { Monitor } from '@lucide/svelte';
  import type { RpcClient } from '../../../lib/rpc';
  import { i18n } from '../../../lib/i18n.svelte';
  import { settingsCoord } from '../../../lib/settingsStore.svelte';
  import { applyWindowSettings } from '../../../lib/windowSettings';

  interface Props { rpc: RpcClient; onError: (msg: string) => void }
  let { rpc, onError }: Props = $props();

  type FrameStyle = 'native' | 'thin' | 'frameless';
  type TabsLocation = 'top' | 'bottom' | 'left' | 'right';
  type DockSide = 'off' | 'top' | 'bottom' | 'left' | 'right';

  let frameStyle = $state<FrameStyle>('native');
  let opacity = $state(100);
  let acrylic = $state(false);
  let vibrancy = $state(false);
  let spaciness = $state(1.0);
  let tabsLocation = $state<TabsLocation>('top');
  let sidebarVisible = $state(true);
  let dockSide = $state<DockSide>('off');
  let dockAlwaysOnTop = $state(false);
  let dockHideOnBlur = $state(false);
  let dockHotkey = $state('');
  let trayEnabled = $state(false);
  let trayMinimizeToTray = $state(false);
  let focusFollowsMouse = $state(false);
  let confirmCloseWithMultipleTabs = $state(true);
  let disableGpuAcceleration = $state(false);
  let useNativeWindowControls = $state(true);

  function currentValue() {
    return {
      frameStyle, opacity, acrylic, vibrancy, spaciness, tabsLocation,
      sidebarVisible,
      dockSide, dockAlwaysOnTop, dockHideOnBlur, dockHotkey,
      trayEnabled, trayMinimizeToTray,
      focusFollowsMouse, confirmCloseWithMultipleTabs,
      disableGpuAcceleration, useNativeWindowControls,
    };
  }

  function markDirty() {
    settingsCoord.markDirty();
    // Live-preview: apply CSS-affected fields immediately so the user sees
    // opacity/spaciness/tabs-location/etc. update as they tweak the slider.
    applyWindowSettings(currentValue() as unknown as Record<string, unknown>);
  }

  async function load() {
    try {
      const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'window' });
      if (r.value && typeof r.value === 'object') {
        const v = r.value as Record<string, unknown>;
        if (v.frameStyle === 'native' || v.frameStyle === 'thin' || v.frameStyle === 'frameless') frameStyle = v.frameStyle;
        if (typeof v.opacity === 'number') opacity = v.opacity;
        if (typeof v.acrylic === 'boolean') acrylic = v.acrylic;
        if (typeof v.vibrancy === 'boolean') vibrancy = v.vibrancy;
        if (typeof v.spaciness === 'number') spaciness = v.spaciness;
        if (v.tabsLocation === 'top' || v.tabsLocation === 'bottom' || v.tabsLocation === 'left' || v.tabsLocation === 'right') tabsLocation = v.tabsLocation;
        if (typeof v.sidebarVisible === 'boolean') sidebarVisible = v.sidebarVisible;
        if (v.dockSide === 'off' || v.dockSide === 'top' || v.dockSide === 'bottom' || v.dockSide === 'left' || v.dockSide === 'right') dockSide = v.dockSide;
        if (typeof v.dockAlwaysOnTop === 'boolean') dockAlwaysOnTop = v.dockAlwaysOnTop;
        if (typeof v.dockHideOnBlur === 'boolean') dockHideOnBlur = v.dockHideOnBlur;
        if (typeof v.dockHotkey === 'string') dockHotkey = v.dockHotkey;
        if (typeof v.trayEnabled === 'boolean') trayEnabled = v.trayEnabled;
        if (typeof v.trayMinimizeToTray === 'boolean') trayMinimizeToTray = v.trayMinimizeToTray;
        if (typeof v.focusFollowsMouse === 'boolean') focusFollowsMouse = v.focusFollowsMouse;
        if (typeof v.confirmCloseWithMultipleTabs === 'boolean') confirmCloseWithMultipleTabs = v.confirmCloseWithMultipleTabs;
        if (typeof v.disableGpuAcceleration === 'boolean') disableGpuAcceleration = v.disableGpuAcceleration;
        if (typeof v.useNativeWindowControls === 'boolean') useNativeWindowControls = v.useNativeWindowControls;
        applyWindowSettings(v);
      }
    } catch (e) { onError(`window load: ${(e as Error).message}`); }
  }

  async function save() {
    const value = currentValue();
    await rpc.call('settings.set', { key: 'window', value });
    applyWindowSettings(value as unknown as Record<string, unknown>);
    // Notify open consumers (terminal panes etc.).
    settingsCoord.bumpRev();
  }

  onMount(() => { settingsCoord.registerSaver('window', save); void load(); });
  onDestroy(() => settingsCoord.unregisterSaver('window'));
</script>

<div class="settings-section">
  <h2 class="flex items-center gap-2"><Monitor size={16} /> {i18n.t('window.title')}</h2>
  <p class="hint">{i18n.t('window.restartHint')}</p>

  <div class="section-h">{i18n.t('window.frameTransparency')}</div>
  <label class="row">
    <span class="row-label">{i18n.t('window.frameStyle')}</span>
    <select bind:value={frameStyle} onchange={markDirty}>
      <option value="native">{i18n.t('window.frame.native')}</option>
      <option value="thin">{i18n.t('window.frame.thin')}</option>
      <option value="frameless">{i18n.t('window.frame.frameless')}</option>
    </select>
  </label>
  <label class="row">
    <span class="row-label">{i18n.t('window.nativeControls')}</span>
    <input type="checkbox" bind:checked={useNativeWindowControls} onchange={markDirty} />
  </label>
  <label class="row">
    <span class="row-label">{i18n.t('window.opacity', { value: opacity })}</span>
    <input type="range" min="40" max="100" step="1" bind:value={opacity} oninput={markDirty} />
  </label>
  <label class="row">
    <span class="row-label">{i18n.t('window.acrylic')}</span>
    <input type="checkbox" bind:checked={acrylic} onchange={markDirty} />
  </label>
  <label class="row">
    <span class="row-label">{i18n.t('window.vibrancy')}</span>
    <input type="checkbox" bind:checked={vibrancy} onchange={markDirty} />
  </label>

  <div class="section-h">{i18n.t('window.layout')}</div>
  <label class="row">
    <span class="row-label">{i18n.t('window.spaciness', { value: spaciness.toFixed(2) })}</span>
    <input type="range" min="0.6" max="1.4" step="0.05" bind:value={spaciness} oninput={markDirty} />
  </label>
  <label class="row">
    <span class="row-label">{i18n.t('window.tabsLocation')}</span>
    <select bind:value={tabsLocation} onchange={markDirty}>
      <option value="top">{i18n.t('window.position.top')}</option>
      <option value="bottom">{i18n.t('window.position.bottom')}</option>
      <option value="left">{i18n.t('window.position.left')}</option>
      <option value="right">{i18n.t('window.position.right')}</option>
    </select>
  </label>
  <label class="row">
    <span class="row-label">{i18n.t('window.sidebar')}</span>
    <input type="checkbox" bind:checked={sidebarVisible} onchange={markDirty} />
  </label>
  <label class="row">
    <span class="row-label">{i18n.t('window.focusFollowsMouse')}</span>
    <input type="checkbox" bind:checked={focusFollowsMouse} onchange={markDirty} />
  </label>
  <label class="row">
    <span class="row-label">{i18n.t('window.confirmCloseMultiplePanes')}</span>
    <input type="checkbox" bind:checked={confirmCloseWithMultipleTabs} onchange={markDirty} />
  </label>

  <div class="section-h">{i18n.t('window.dock')}</div>
  <label class="row">
    <span class="row-label">{i18n.t('window.dockToEdge')}</span>
    <select bind:value={dockSide} onchange={markDirty}>
      <option value="off">{i18n.t('window.off')}</option>
      <option value="top">{i18n.t('window.position.top')}</option>
      <option value="bottom">{i18n.t('window.position.bottom')}</option>
      <option value="left">{i18n.t('window.position.left')}</option>
      <option value="right">{i18n.t('window.position.right')}</option>
    </select>
  </label>
  <label class="row">
    <span class="row-label">{i18n.t('window.alwaysOnTopDocked')}</span>
    <input type="checkbox" bind:checked={dockAlwaysOnTop} onchange={markDirty} disabled={dockSide === 'off'} />
  </label>
  <label class="row">
    <span class="row-label">{i18n.t('window.hideOnBlur')}</span>
    <input type="checkbox" bind:checked={dockHideOnBlur} onchange={markDirty} disabled={dockSide === 'off'} />
  </label>
  <label class="row">
    <span class="row-label">{i18n.t('window.showHideHotkey')}</span>
    <input type="text" bind:value={dockHotkey} oninput={markDirty}
           placeholder="Ctrl+Alt+Space" disabled={dockSide === 'off'} />
  </label>

  <div class="section-h">{i18n.t('window.tray')}</div>
  <label class="row">
    <span class="row-label">{i18n.t('window.showTrayIcon')}</span>
    <input type="checkbox" bind:checked={trayEnabled} onchange={markDirty} />
  </label>
  <label class="row">
    <span class="row-label">{i18n.t('window.minimizeToTray')}</span>
    <input type="checkbox" bind:checked={trayMinimizeToTray} onchange={markDirty} disabled={!trayEnabled} />
  </label>

  <div class="section-h">{i18n.t('window.advanced')}</div>
  <label class="row">
    <span class="row-label">{i18n.t('window.disableGpu')}</span>
    <input type="checkbox" bind:checked={disableGpuAcceleration} onchange={markDirty} />
  </label>
</div>

<style>
  .hint { font-size: 12px; color: var(--color-fg-muted); margin-bottom: 6px; }
  .section-h {
    margin-top: 16px; margin-bottom: 6px; font-size: 11.5px;
    text-transform: uppercase; color: var(--color-fg-muted); letter-spacing: 0.04em;
  }
  .row {
    display: grid; grid-template-columns: 240px 1fr; align-items: center; gap: 10px;
    padding: 4px 0;
  }
  .row-label { font-size: 12.5px; }
  .row input[type='text'],
  .row select {
    padding: 4px 8px; background: var(--color-bg-soft); color: var(--color-fg);
    border: 1px solid var(--color-border); border-radius: 4px; font-size: 12.5px;
    width: 100%; max-width: 380px;
  }
  .row input[type='range'] { max-width: 380px; }
  .row input:focus, .row select:focus { outline: none; border-color: var(--color-accent); }
  .row input:disabled, .row select:disabled { opacity: 0.5; }
</style>
