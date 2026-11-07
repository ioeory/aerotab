import test from 'node:test';
import assert from 'node:assert/strict';

async function api() {
  let mod;
  try { mod = await import('./appearancePreferences.ts'); }
  catch (error) { if (error.code !== 'ERR_MODULE_NOT_FOUND') throw error; }
  assert.equal(typeof mod?.normalizeAppearancePreferences, 'function', 'appearance preferences coordinator exists');
  return mod;
}
const palettes = [{ name: 'tokyo-night', bg: '#101010' }, { name: 'one-light', bg: '#fafafa' }, { name: 'termora-dark', bg: '#121212' }];

test('legacy IDs migrate without changing the terminal palette and remember both modes', async () => {
  const { normalizeAppearancePreferences, resolveAppearancePalette } = await api();
  const prefs = normalizeAppearancePreferences(null, 'one-light', palettes);
  assert.equal(prefs.mode, 'light');
  assert.equal(prefs.lightPalette, 'one-light');
  assert.equal(resolveAppearancePalette(prefs, true), 'one-light');
  assert.equal(resolveAppearancePalette({ ...prefs, mode: 'system' }, true), prefs.darkPalette);
  assert.equal(resolveAppearancePalette({ ...prefs, mode: 'system' }, false), prefs.lightPalette);
});

test('invalid accent is discarded, valid legacy palette IDs are preserved', async () => {
  const { normalizeAppearancePreferences } = await api();
  assert.equal(normalizeAppearancePreferences({ mode: 'dark', darkPalette: 'tokyo-night', accent: 'red;display:none' }, 'one-light', palettes).accent, null);
  assert.equal(normalizeAppearancePreferences({ mode: 'dark', darkPalette: 'tokyo-night', accent: '#AABBCC' }, 'one-light', palettes).darkPalette, 'tokyo-night');
});

test('preview never persists; failed apply and cancel return to the last applied baseline', async () => {
  const { createAppearanceDraft, normalizeAppearancePreferences } = await api();
  const baseline = normalizeAppearancePreferences(null, 'tokyo-night', palettes);
  const rendered = []; const persisted = [];
  let fail = false;
  const draft = createAppearanceDraft(baseline, {
    render: value => rendered.push(value),
    persist: async value => { if (fail) throw new Error('offline'); persisted.push(value); },
  });
  const light = { ...baseline, mode: 'light' };
  draft.preview(light);
  assert.equal(persisted.length, 0);
  draft.cancel();
  assert.deepEqual(rendered.at(-1), baseline);
  await draft.apply(light);
  fail = true;
  await assert.rejects(draft.apply({ ...light, accent: '#aabbcc' }), /offline/);
  assert.deepEqual(rendered.at(-1), light);
  draft.cancel();
  assert.deepEqual(rendered.at(-1), light);
  assert.equal(persisted.length, 1);
});

test('save failure identifies the section and does not run subsequent savers', async () => {
  let mod;
  try { mod = await import('./settingsSaveRegistry.ts'); }
  catch (error) { if (error.code !== 'ERR_MODULE_NOT_FOUND') throw error; }
  assert.equal(typeof mod?.SettingsSaveRegistry, 'function', 'save registry exists');
  const registry = new mod.SettingsSaveRegistry(); const calls = [];
  registry.register('appearance', async () => { calls.push('appearance'); });
  registry.register('ssh', async () => { throw new Error('denied'); });
  registry.register('window', async () => { calls.push('window'); });
  await assert.rejects(registry.save(), error => error.sectionId === 'ssh' && error.message.includes('denied'));
  assert.deepEqual(calls, ['appearance']);
});
