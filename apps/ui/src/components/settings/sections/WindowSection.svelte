<script lang="ts">
  // Window — M6. Window chrome / opacity / tray / docking parity with Tabby.
  // Persists to settings key `window`. Most settings require restart since
  // Tauri does not expose live runtime mutators for every flag.
  import { onMount, onDestroy } from 'svelte';
  import { Monitor } from '@lucide/svelte';
  import type { RpcClient } from '../../../lib/rpc';
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
  <h2 class="flex items-center gap-2"><Monitor size={16} /> Window</h2>
  <p class="hint">Most window-chrome options require an app restart to take effect.</p>

  <div class="section-h">Frame &amp; transparency</div>
  <label class="row">
    <span class="row-label">Frame style</span>
    <select bind:value={frameStyle} onchange={markDirty}>
      <option value="native">Native</option>
      <option value="thin">Thin (custom titlebar)</option>
      <option value="frameless">Frameless</option>
    </select>
  </label>
  <label class="row">
    <span class="row-label">Use native window controls</span>
    <input type="checkbox" bind:checked={useNativeWindowControls} onchange={markDirty} />
  </label>
  <label class="row">
    <span class="row-label">Background opacity ({opacity}%)</span>
    <input type="range" min="40" max="100" step="1" bind:value={opacity} oninput={markDirty} />
  </label>
  <label class="row">
    <span class="row-label">Acrylic background (Windows)</span>
    <input type="checkbox" bind:checked={acrylic} onchange={markDirty} />
  </label>
  <label class="row">
    <span class="row-label">Vibrancy (macOS)</span>
    <input type="checkbox" bind:checked={vibrancy} onchange={markDirty} />
  </label>

  <div class="section-h">Layout</div>
  <label class="row">
    <span class="row-label">UI spaciness ({spaciness.toFixed(2)})</span>
    <input type="range" min="0.6" max="1.4" step="0.05" bind:value={spaciness} oninput={markDirty} />
  </label>
  <label class="row">
    <span class="row-label">Tabs location</span>
    <select bind:value={tabsLocation} onchange={markDirty}>
      <option value="top">Top</option>
      <option value="bottom">Bottom</option>
      <option value="left">Left</option>
      <option value="right">Right</option>
    </select>
  </label>
  <label class="row">
    <span class="row-label">Show left sidebar</span>
    <input type="checkbox" bind:checked={sidebarVisible} onchange={markDirty} />
  </label>
  <label class="row">
    <span class="row-label">Focus follows mouse</span>
    <input type="checkbox" bind:checked={focusFollowsMouse} onchange={markDirty} />
  </label>
  <label class="row">
    <span class="row-label">Confirm when closing with multiple tabs</span>
    <input type="checkbox" bind:checked={confirmCloseWithMultipleTabs} onchange={markDirty} />
  </label>

  <div class="section-h">Dock</div>
  <label class="row">
    <span class="row-label">Dock to screen edge</span>
    <select bind:value={dockSide} onchange={markDirty}>
      <option value="off">Off</option>
      <option value="top">Top</option>
      <option value="bottom">Bottom</option>
      <option value="left">Left</option>
      <option value="right">Right</option>
    </select>
  </label>
  <label class="row">
    <span class="row-label">Always on top while docked</span>
    <input type="checkbox" bind:checked={dockAlwaysOnTop} onchange={markDirty} disabled={dockSide === 'off'} />
  </label>
  <label class="row">
    <span class="row-label">Hide on blur</span>
    <input type="checkbox" bind:checked={dockHideOnBlur} onchange={markDirty} disabled={dockSide === 'off'} />
  </label>
  <label class="row">
    <span class="row-label">Show/hide hotkey</span>
    <input type="text" bind:value={dockHotkey} oninput={markDirty}
           placeholder="Ctrl+Alt+Space" disabled={dockSide === 'off'} />
  </label>

  <div class="section-h">Tray</div>
  <label class="row">
    <span class="row-label">Show tray icon</span>
    <input type="checkbox" bind:checked={trayEnabled} onchange={markDirty} />
  </label>
  <label class="row">
    <span class="row-label">Minimize to tray instead of taskbar</span>
    <input type="checkbox" bind:checked={trayMinimizeToTray} onchange={markDirty} disabled={!trayEnabled} />
  </label>

  <div class="section-h">Advanced</div>
  <label class="row">
    <span class="row-label">Disable GPU acceleration</span>
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
