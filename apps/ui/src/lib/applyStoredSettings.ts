/**
 * Re-read persisted settings from the backend and apply them to the live UI.
 * Called after config sync pulls remote changes into `settings.sled`.
 */

import type { RpcClient } from './rpc';
import { loadAppliedAppearance } from './appliedAppearance';
import { applyCustomCss, applyLigatures } from './customCss';
import { applyWindowSettings } from './windowSettings';
import { hotkeys } from './hotkeys';
import { i18n } from './i18n.svelte';
import { profileVisualsStore } from './profileVisualsStore.svelte';

export async function applyStoredSettingsToUi(rpc: RpcClient): Promise<void> {
  await i18n.load(rpc);
  await profileVisualsStore.load(rpc);

  try {
    await loadAppliedAppearance(rpc);
  } catch {
    /* keep current theme */
  }

  try {
    const a = await rpc.call<{ value: unknown }>('settings.get', { key: 'appearance' });
    const v = a.value && typeof a.value === 'object' ? a.value as Record<string, unknown> : {};
    applyCustomCss(typeof v.customCss === 'string' ? v.customCss : '');
    applyLigatures(v.ligatures === true);
  } catch {
    /* ignore */
  }

  try {
    const w = await rpc.call<{ value: unknown }>('settings.get', { key: 'window' });
    applyWindowSettings(w.value && typeof w.value === 'object' ? w.value as Record<string, unknown> : {}, { replace: true });
  } catch {
    /* ignore */
  }

  try {
    const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'hotkeys' });
    if (r.value && typeof r.value === 'object') {
      hotkeys.loadFromMap(r.value as Record<string, string[]>);
    } else { hotkeys.resetToDefaults(); }
  } catch {
    /* ignore */
  }

  if (typeof window !== 'undefined') {
    window.dispatchEvent(new CustomEvent('aerotab:settings-synced'));
  }
}
