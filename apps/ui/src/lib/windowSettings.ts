// Window-chrome live applier. Translates the persisted `window` settings
// object into CSS variables and `data-*` attributes that app.css consumes.
//
// Some flags (frameStyle, useNativeWindowControls, tray, dockSide) require
// rebuilding the Tauri window and so still need a restart; they are stored
// for the backend to read on next launch.

export interface WindowSettings {
  frameStyle?: 'native' | 'thin' | 'frameless';
  opacity?: number; // 40-100
  acrylic?: boolean;
  vibrancy?: boolean;
  spaciness?: number; // 0.6-1.4
  tabsLocation?: 'top' | 'bottom' | 'left' | 'right';
  sidebarVisible?: boolean;
  dockSide?: 'off' | 'top' | 'bottom' | 'left' | 'right';
  trayEnabled?: boolean;
  trayMinimizeToTray?: boolean;
  focusFollowsMouse?: boolean;
  confirmCloseWithMultipleTabs?: boolean;
  disableGpuAcceleration?: boolean;
  useNativeWindowControls?: boolean;
}

let cached: WindowSettings = {};

export function getWindowSettings(): Readonly<WindowSettings> {
  return cached;
}

export function applyWindowSettings(v: Record<string, unknown>): void {
  const out: WindowSettings = {};
  const root = document.documentElement;
  const body = document.body;
  if (typeof v.opacity === 'number') {
    out.opacity = v.opacity;
    const clamped = Math.max(40, Math.min(100, v.opacity)) / 100;
    root.style.setProperty('--ui-opacity', String(clamped));
    root.style.setProperty('--ui-bg-opacity-percent', `${clamped * 100}%`);
    body.dataset.translucent = clamped < 1 ? 'true' : 'false';
  }
  if (typeof v.spaciness === 'number') {
    out.spaciness = v.spaciness;
    const s = Math.max(0.6, Math.min(1.4, v.spaciness));
    root.style.setProperty('--ui-spaciness', String(s));
  }
  if (v.tabsLocation === 'top' || v.tabsLocation === 'bottom'
      || v.tabsLocation === 'left' || v.tabsLocation === 'right') {
    out.tabsLocation = v.tabsLocation;
    body.dataset.tabsLocation = v.tabsLocation;
  }
  if (typeof v.sidebarVisible === 'boolean') {
    out.sidebarVisible = v.sidebarVisible;
    body.dataset.sidebarVisible = String(v.sidebarVisible);
  }
  if (v.frameStyle === 'native' || v.frameStyle === 'thin' || v.frameStyle === 'frameless') {
    out.frameStyle = v.frameStyle;
    body.dataset.frameStyle = v.frameStyle;
  }
  if (typeof v.focusFollowsMouse === 'boolean') {
    out.focusFollowsMouse = v.focusFollowsMouse;
    body.dataset.focusFollowsMouse = String(v.focusFollowsMouse);
  }
  if (typeof v.confirmCloseWithMultipleTabs === 'boolean') {
    out.confirmCloseWithMultipleTabs = v.confirmCloseWithMultipleTabs;
  }
  for (const k of ['acrylic', 'vibrancy', 'trayEnabled', 'trayMinimizeToTray', 'disableGpuAcceleration', 'useNativeWindowControls'] as const) {
    if (typeof v[k] === 'boolean') (out as Record<string, unknown>)[k] = v[k];
  }
  cached = { ...cached, ...out };
  if (typeof document !== 'undefined') {
    document.dispatchEvent(new CustomEvent('aerotab:settings-changed', { detail: 'window' }));
  }
}