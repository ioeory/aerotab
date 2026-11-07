import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const js = ts.transpileModule(readFileSync(new URL('./sessionAuxiliary.ts', import.meta.url), 'utf8'), { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText;
const { toggleAuxiliaryPanel, pruneAuxiliaryPanels } = await import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);
test('one auxiliary panel per tab; toggling does not affect other tabs', () => {
  const current = { a: 'system', b: 'notes' };
  assert.deepEqual(toggleAuxiliaryPanel(current, 'a', 'notes'), { a: 'notes', b: 'notes' });
  assert.deepEqual(toggleAuxiliaryPanel(current, 'a', 'system'), { b: 'notes' });
  assert.deepEqual(current, { a: 'system', b: 'notes' });
});
test('closed terminal tabs release their auxiliary UI state', () => {
  assert.deepEqual(pruneAuxiliaryPanels({ a: 'system', b: 'notes' }, ['b']), { b: 'notes' });
});
