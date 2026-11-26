import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const compile = name => ts.transpileModule(readFileSync(new URL(`./${name}.ts`, import.meta.url), 'utf8'), { compilerOptions: { target: 99, module: 99 } }).outputText;
const url = text => `data:text/javascript;base64,${Buffer.from(text).toString('base64')}`;
const lib = await import(url(compile('opsProject').replaceAll("'./operationsWorkbench'", `'${url(compile('operationsWorkbench'))}'`)));
const env = { project: 'Payments', environment: 'Production' };

test('events remain isolated, searchable and archived with an auditable transition', () => {
  const first = lib.saveEvent(null, 'a', env, { title: 'Listener down' }, [], 100);
  const second = lib.saveEvent(null, 'b', { ...env, environment: 'Staging' }, { title: 'Listener down' }, [], 200);
  const archived = lib.archiveEvent(first, true, 300);
  assert.equal(archived.timeline.at(-1).kind, 'archived');
  assert.equal(lib.searchEvents([archived, second], 'payments staging', false)[0].id, 'b');
  assert.equal(lib.searchEvents([archived, second], '', true)[0].id, 'a');
  assert.equal(first.archived, false);
});

test('snapshots keep runbook implementation and evidence metadata; export is opt-in', () => {
  const steps = lib.instantiateRunbook('storage', 'Disk check', ['Verify space']);
  const entry = { title: '# Header', evidence: [{ label:'A', text:'private excerpt', source:'selection', at:10 }], steps };
  const event = lib.saveEvent(null, 'a', env, entry, [0], 100);
  assert.equal(event.record.evidence[0].source, 'selection');
  assert.equal(event.record.steps[0].template.id, 'storage');
  assert.notEqual(lib.instantiateRunbook('storage', 'Disk check', ['Updated'])[0].template.version, steps[0].template.version);
  assert.match(lib.handoffMarkdown(event, [], 'en'), /Verify space/);
  assert.doesNotMatch(lib.handoffMarkdown(event, [], 'en'), /private excerpt/);
  assert.match(lib.handoffMarkdown(event, [0], 'en'), /private excerpt/);
});

test('migration writes once and never erases the old data', async () => {
  const source = { title:'Legacy', evidence:[], steps:[], conclusion:'' };
  const values = new Map();
  const putIfAbsent = async e => { if (!values.has(e.id)) values.set(e.id, e); };
  await assert.rejects(lib.migrateLegacy(source, async () => { throw Error('storage failed'); }, 1));
  assert.equal(source.title, 'Legacy');
  await lib.migrateLegacy(source, putIfAbsent, 1);
  await lib.migrateLegacy(source, putIfAbsent, 2);
  assert.equal(values.size, 1);
  assert.equal([...values.values()][0].updatedAt, 1);
});
