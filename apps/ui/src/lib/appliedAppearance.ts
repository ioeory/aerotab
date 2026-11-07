import type { RpcClient } from './rpc';
import { applyAppearancePreferences, normalizeAppearancePreferences, type AppearancePreferences } from './appearancePreferences';

let current: AppearancePreferences | null = null;
let media: MediaQueryList | null = null;

function paint() {
  if (current) applyAppearancePreferences(current, media?.matches ?? false);
}

/** One system-theme listener per WebView; UI palettes never write terminal settings. */
export function setAppliedAppearance(value: unknown, legacyTheme?: unknown) {
  current = normalizeAppearancePreferences(value, legacyTheme);
  if (!media && typeof window.matchMedia === 'function') {
    media = window.matchMedia('(prefers-color-scheme: dark)');
    media.addEventListener('change', () => {
      // Settings owns its preview transaction while open, including system changes.
      if (current?.mode === 'system' && !document.querySelector('[data-settings-layout]')) paint();
    });
  }
  paint();
}

export async function loadAppliedAppearance(rpc: RpcClient) {
  const legacy = await rpc.call<{ value: unknown }>('settings.get', { key: 'theme' });
  const ui = await rpc.call<{ value: unknown }>('settings.get', { key: 'uiAppearance' })
    .catch(() => ({ value: null }));
  setAppliedAppearance(ui.value, legacy.value);
}
