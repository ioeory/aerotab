import test from 'node:test';
import assert from 'node:assert/strict';
test('preview/cancel preserves original override without persistence; failed apply rolls back', async () => {
  const api = await import('./terminalPalettePreview.ts');
  const writes = [], rendered = [];
  const draft = api.createTerminalPaletteDraft('aero-ember', { render: v => rendered.push(v), persist: async v => { writes.push(v); if (v === 'bad') throw Error('offline'); } });
  draft.preview('aero-cove'); assert.deepEqual(writes, []); assert.equal(rendered.at(-1), 'aero-cove');
  assert.equal(draft.cancel(), 'aero-ember'); assert.equal(rendered.at(-1), null);
  draft.preview(''); await draft.apply(); assert.deepEqual(writes, ['']);
  draft.preview('bad'); await assert.rejects(draft.apply(), /offline/); assert.equal(rendered.at(-1), '');
  assert.equal(draft.cancel(), '');
});
test('terminal preview transaction renders through the same resolver as native/xterm and restores persisted override', async () => {
  const api = await import('./terminalPalettePreview.ts');
  let persisted = 'original';
  const draft = api.createTerminalPaletteDraft(persisted, { render: api.setTerminalPalettePreview, persist: async value => { persisted = value; } });
  draft.preview('candidate'); assert.equal(api.resolveTerminalPaletteName(persisted), 'candidate'); assert.equal(persisted, 'original');
  draft.cancel(); assert.equal(api.resolveTerminalPaletteName(persisted), 'original');
  draft.preview(''); await draft.apply(); assert.equal(persisted, ''); assert.equal(api.resolveTerminalPaletteName(persisted), '');
  draft.preview('candidate'); draft.cancel(); assert.equal(api.resolveTerminalPaletteName(persisted), '');
});

test('window preview overrides terminal settings only until cleared', async () => {
  const api = await import('./terminalPalettePreview.ts');
  api.setTerminalPalettePreview(''); assert.equal(api.resolveTerminalPaletteName('original'), '');
  api.setTerminalPalettePreview('aero-cove'); assert.equal(api.resolveTerminalPaletteName('original'), 'aero-cove');
  api.setTerminalPalettePreview(null); assert.equal(api.resolveTerminalPaletteName('original'), 'original');
});
