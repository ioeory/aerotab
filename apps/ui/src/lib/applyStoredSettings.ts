/**
 * Re-read persisted settings from the backend and apply them to the live UI.
 * Called after config sync pulls remote changes into `settings.sled`.
 */

import type { RpcClient } from './rpc';
import { applyTheme, BUILTIN_THEMES } from './theme';
import { applyCustomCss, applyLigatures } from './customCss';
import { applyWindowSettings } from './windowSettings';
import { hotkeys } from './hotkeys';
import { i18n } from './i18n.svelte';
import { profileVisualsStore } from './profileVisualsStore.svelte';

export async function applyStoredSettingsToUi(rpc: RpcClient): Promise<void> {
  await i18n.load(rpc);
  await profileVisualsStore.load(rpc);

  try {
    const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'theme' });
    const name = typeof r.value === 'string' ? r.value : 'tokyo-night';
    const theme = BUILTIN_THEMES.find((t) => t.name === name) ?? BUILTIN_THEMES[0];
    if (theme) applyTheme(theme);
  } catch {
    /* keep current theme */
  }

  try {
    const a = await rpc.call<{ value: unknown }>('settings.get', { key: 'appearance' });
    if (a.value && typeof a.value === 'object') {
      const v = a.value as Record<string, unknown>;
      if (typeof v.customCss === 'string') applyCustomCss(v.customCss);
      if (typeof v.ligatures === 'boolean') applyLigatures(v.ligatures);
    }
  } catch {
    /* ignore */
  }

  try {
    const w = await rpc.call<{ value: unknown }>('settings.get', { key: 'window' });
    if (w.value && typeof w.value === 'object') {
      applyWindowSettings(w.value as Record<string, unknown>);
    }
  } catch {
    /* ignore */
  }

  try {
    const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'hotkeys' });
    if (r.value && typeof r.value === 'object') {
      hotkeys.loadFromMap(r.value as Record<string, string[]>);
    }
  } catch {
    /* ignore */
  }

  if (typeof window !== 'undefined') {
    window.dispatchEvent(new CustomEvent('aerotab:settings-synced'));
  }
}
