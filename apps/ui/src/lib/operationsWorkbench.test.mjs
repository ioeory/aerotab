import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';

const source = readFileSync(new URL('./operationsWorkbench.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, { compilerOptions: { target: 99, module: 99 } });
const { prepareIncident, incidentHasContent, saveIncident, loadIncident, clearIncident, groupInspection } = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);

function storage() {
  const map = new Map();
  return { map, getItem: key => map.get(key) ?? null, setItem: (key, value) => map.set(key, value), removeItem: key => map.delete(key) };
}

test('an empty record cannot overwrite a saved record', () => {
  assert.equal(incidentHasContent(prepareIncident({ title: ' ', conclusion: '', evidence: [], steps: [] })), false);
  assert.equal(incidentHasContent(prepareIncident({ conclusion: 'Needs manual verification' })), true);
});

test('save preview is detached, excludes evidence by default and only persists allowed fields', () => {
  const current = { title: ' rollout ', conclusion: ' compare flags ', evidence: [
    { label: 'A · Tab 1 / Left', text: 'private output', sessionId: 'private-id' },
    { label: 'B · Tab 1 / Right', text: 'second output' },
  ], steps: [{ label: 'Check listener', expected: 'One listener', actual: 'Two', done: true, autoExecute: true }], token: 'never' };
  const disk = storage();
  const preview = prepareIncident(current);
  current.conclusion = 'unsaved edit';
  assert.deepEqual(preview, { title: 'rollout', conclusion: 'compare flags', evidence: [], steps: [{ label: 'Check listener', expected: 'One listener', actual: 'Two', done: true }] });
  assert.equal(disk.map.size, 0);
  assert.equal(saveIncident(disk, preview), true);
  assert.deepEqual(loadIncident(disk), preview);
  assert.doesNotMatch([...disk.map.values()][0], /private|second output|autoExecute|token|unsaved edit/);
  const selected = prepareIncident(current, [1]);
  assert.deepEqual(selected.evidence, [{ label: 'B · Tab 1 / Right', text: 'second output' }]);
  assert.equal(saveIncident(disk, selected), true);
  assert.deepEqual(loadIncident(disk), selected);
  assert.equal(clearIncident(disk), true);
  assert.equal(loadIncident(disk), null);
});

test('invalid storage is handled without modifying current work', () => {
  assert.equal(loadIncident({ getItem: () => '{invalid' }), null);
  assert.equal(loadIncident({ getItem: () => '{"version":2,"record":{"title":"x"}}' }), null);
  assert.equal(saveIncident({ setItem() { throw new Error('unavailable'); } }, prepareIncident({ title: 'x' })), false);
  assert.equal(prepareIncident({ evidence: [{ text: '日'.repeat(9000) }] }, [0]).evidence[0].text.length, 4000);
});

test('inspection groups duplicate panes without conflating session status or guessing connectivity', () => {
  const panes = [
    { id:'left', hostKey:'profile:1', hostLabel:'prod', label:'Tab 1 / Left', kind:'Ssh' },
    { id:'right', hostKey:'profile:1', hostLabel:'prod', label:'Tab 1 / Right', kind:'Ssh' },
    { id:'other', hostKey:'profile:2', hostLabel:'prod', label:'Tab 2', kind:'Ssh', pending:'failed' },
    { id:'local', hostKey:'local', hostLabel:'local', label:'Local', kind:'LocalShell' },
  ];
  assert.deepEqual(groupInspection(panes, null).map(g => g.panes.map(p => p.status)), [['unknown','unknown'], ['failed']]);
  assert.deepEqual(groupInspection(panes, new Set(['left'])).map(g => g.panes.map(p => p.status)), [['live','absent'], ['failed']]);
});
