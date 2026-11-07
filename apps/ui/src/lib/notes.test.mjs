import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source = readFileSync(new URL('./notesCore.ts', import.meta.url), 'utf8');
const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
const api = await import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);
test('preview escapes HTML, blocks active links and never loads images', () => {
  const html = api.renderMarkdown('<script>alert(1)</script>\n[x](javascript:alert)\n![private](https://tracker/x)\n**bold**');
  assert.ok(!html.includes('<script>')); assert.ok(!html.includes('href="javascript:')); assert.ok(!html.includes('<img')); assert.ok(html.includes('<strong>bold</strong>'));
});
test('export filenames reject traversal, Windows reserved names and collisions', () => {
  assert.equal(api.safeFilename('../CON'), '_CON');
  assert.equal(api.uniqueFilename('hello', new Set(['hello.md'])), 'hello (2).md');
});
test('serial saver retains newer drafts and keeps failed draft for explicit retry', async () => {
  let release; const calls = [];
  const saver = new api.NoteSaveQueue(async (draft, version) => { calls.push([draft, version]); if (calls.length === 1) await new Promise(r => release = r); return version + 1; }, 1);
  saver.update('a'); const pending = saver.flush(); saver.update('b'); release(); await pending;
  assert.deepEqual(calls, [['a', 1], ['b', 2]]); assert.equal(saver.dirty, false);
  let fail = true; const retry = new api.NoteSaveQueue(async () => { if (fail) throw Error('conflict'); return 2; }, 1);
  retry.update('draft'); await assert.rejects(retry.flush()); assert.equal(retry.dirty, true); fail = false; await retry.flush(); assert.equal(retry.dirty, false);
});
