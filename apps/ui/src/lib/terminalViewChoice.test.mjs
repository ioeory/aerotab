import test from 'node:test';
import assert from 'node:assert/strict';
async function api() {
  let mod;
  try { mod = await import('./terminalViewChoice.ts'); } catch (e) { if (e.code !== 'ERR_MODULE_NOT_FOUND') throw e; }
  assert.equal(typeof mod?.chooseTerminalView, 'function');
  return mod;
}
test('toggles only requested pane between shared-core views without mutating map', async () => {
  const { chooseTerminalView } = await api();
  const original = { other: 'native-surface' };
  const result = await chooseTerminalView(original, 'focused', 'native', async () => ({ available: true, geometryUnit: 'css-px' }));
  assert.deepEqual(result, { other: 'native-surface', focused: 'native-surface' });
  assert.deepEqual(original, { other: 'native-surface' });
  assert.deepEqual(await chooseTerminalView(result, 'focused', 'native-surface', async () => { throw Error('must not probe returning to compatibility'); }), { other: 'native-surface', focused: 'native' });
});
test('unsupported native rejects explicitly and leaves choice untouched', async () => {
  const { chooseTerminalView } = await api();
  const original = { focused: 'native', other: 'native' };
  await assert.rejects(chooseTerminalView(original, 'focused', 'native', async () => ({ available: false, reason: 'unsupported_platform' })), /unsupported_platform/);
  assert.deepEqual(original, { focused: 'native', other: 'native' });
});
test('legacy xterm footer never switches to another independent parser', async () => {
  const { chooseTerminalView } = await api();
  await assert.rejects(chooseTerminalView({}, 'focused', 'xterm-dom', async () => ({ available: true })), /legacy_renderer/);
});
