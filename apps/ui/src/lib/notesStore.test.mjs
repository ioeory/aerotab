import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { compileModule } from 'svelte/compiler';
function moduleUrl(url) {
  let source = ts.transpileModule(readFileSync(url, 'utf8'), { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
  if (url.pathname.endsWith('.svelte.ts')) source = compileModule(source, { filename: url.pathname, generate: 'server' }).js.code;
  source = source.replace(/(from\s*['"])([^'"]+)(['"])/g, (_, prefix, specifier, quote) => `${prefix}${specifier.startsWith('.') ? moduleUrl(new URL(`${specifier}.ts`, url)) : import.meta.resolve(specifier)}${quote}`);
  return `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`;
}
const { getNotesStore } = await import(moduleUrl(new URL('./notes.svelte.ts', import.meta.url)));
test('workspace and session share snapshots, draft edits and failure recovery', async () => {
  let fail = true;
  const rpc = { async call(method, params) {
    if (method === 'notes.list') return { notes: [], folders: [], revision: 0 };
    if (method === 'notes.put') { if (fail) throw Error('conflict'); return { ...params.note, version: params.expectedVersion + 1 }; }
  } };
  const store = getNotesStore(rpc); assert.equal(getNotesStore(rpc), store);
  const note = { id: 'n', title: 'old', markdown: '', tags: [], profile_ids: [], parent_id: null, version: 1 };
  store.snapshot.notes = [note]; const draft = store.draft(note);
  draft.value.markdown = 'retained'; store.edit(note.id);
  await assert.rejects(store.save(note.id)); assert.equal(draft.status, 'error'); assert.equal(store.dirty, true);
  assert.equal(getNotesStore(rpc).draft(note).value.markdown, 'retained');
  fail = false; await store.save(note.id); assert.equal(store.snapshot.notes[0].markdown, 'retained'); assert.equal(store.dirty, false);
  store.profileRemarks.host = { value: 'draft remark', base: '', dirty: true };
  await assert.rejects(store.prepareClose()); assert.equal(store.dirty, true);
});
