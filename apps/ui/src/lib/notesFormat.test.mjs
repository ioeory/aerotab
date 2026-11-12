import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source = readFileSync(new URL('./notesFormat.ts', import.meta.url), 'utf8');
const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
const api = await import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);
const sel = edit => edit.text.slice(edit.start, edit.end);

test('inline styles wrap the selection and toggle off when already wrapped', () => {
  const bold = api.applyFormat('say hi now', 4, 6, 'bold');
  assert.equal(bold.text, 'say **hi** now'); assert.equal(sel(bold), 'hi');
  const plain = api.applyFormat(bold.text, bold.start, bold.end, 'bold');
  assert.equal(plain.text, 'say hi now'); assert.equal(sel(plain), 'hi');
  assert.equal(api.applyFormat('a', 1, 1, 'code').text, 'a``');
  assert.equal(api.applyFormat('x', 0, 1, 'strike').text, '~~x~~');
});

test('line prefixes apply at line start, never mid-line, and cover every selected line', () => {
  const task = api.applyFormat('### 4 heading', 13, 13, 'task');
  assert.equal(task.text, '- [ ] ### 4 heading');
  const list = api.applyFormat('one\ntwo\nthree', 1, 9, 'unordered');
  assert.equal(list.text, '- one\n- two\n- three');
  assert.equal(api.applyFormat(list.text, 0, list.text.length, 'unordered').text, 'one\ntwo\nthree');
  assert.equal(api.applyFormat('a\nb', 0, 3, 'ordered').text, '1. a\n2. b');
  assert.equal(api.applyFormat('- a', 1, 1, 'ordered').text, '1. a', 'switching list kind replaces the marker');
  assert.equal(api.applyFormat('q', 0, 0, 'quote').text, '> q');
});

test('heading cycles H1 -> H2 -> H3 -> paragraph on the current line', () => {
  let edit = api.applyFormat('intro\ntitle', 8, 8, 'heading');
  assert.equal(edit.text, 'intro\n# title');
  edit = api.applyFormat(edit.text, edit.start, edit.end, 'heading'); assert.equal(edit.text, 'intro\n## title');
  edit = api.applyFormat(edit.text, edit.start, edit.end, 'heading'); assert.equal(edit.text, 'intro\n### title');
  edit = api.applyFormat(edit.text, edit.start, edit.end, 'heading'); assert.equal(edit.text, 'intro\ntitle');
  assert.equal(edit.start, 8, 'caret stays on the same character');
});

test('block inserts sit on their own lines with surrounding blank lines', () => {
  const block = api.applyFormat('before', 6, 6, 'codeblock');
  assert.equal(block.text, 'before\n\n```\n\n```\n'); assert.equal(block.text.slice(0, block.start), 'before\n\n```\n');
  const wrapped = api.applyFormat('a\nls -la\nb', 2, 8, 'codeblock');
  assert.equal(wrapped.text, 'a\n\n```\nls -la\n```\n\nb');
  assert.equal(api.applyFormat('', 0, 0, 'separator').text, '---\n');
  assert.match(api.applyFormat('x', 1, 1, 'table').text, /^x\n\n\| .+ \|\n\| --- \| --- \|\n/);
});

test('links select the URL placeholder, or wrap a selected URL as the target', () => {
  const link = api.applyFormat('see docs', 4, 8, 'link');
  assert.equal(link.text, 'see [docs](https://)'); assert.equal(sel(link), 'https://');
  const url = api.applyFormat('https://a.dev', 0, 13, 'link');
  assert.equal(url.text, '[](https://a.dev)'); assert.equal(url.start, 1); assert.equal(url.end, 1);
  assert.equal(api.applyFormat('', 0, 0, 'image').text, '![](https://)');
});

test('Enter continues lists, increments ordered items and ends an empty item', () => {
  const next = api.continueList('- [x] done', 10, 10);
  assert.equal(next.text, '- [x] done\n- [ ] '); assert.equal(next.start, next.text.length);
  assert.equal(api.continueList('  9. nine', 9, 9).text, '  9. nine\n  10. ');
  assert.equal(api.continueList('> quoted', 8, 8).text, '> quoted\n> ');
  const end = api.continueList('- a\n- ', 6, 6);
  assert.equal(end.text, '- a\n'); assert.equal(end.start, 4);
  assert.equal(api.continueList('plain', 5, 5), null);
  assert.equal(api.continueList('- item', 2, 2), null, 'only continues when the caret is at the end of the line');
  assert.equal(api.continueList('- a', 0, 3), null, 'a selection is left to the browser');
});

test('keyboard shortcuts map to formats only with the primary modifier', () => {
  assert.equal(api.shortcutFormat({ key: 'b', ctrlKey: true }), 'bold');
  assert.equal(api.shortcutFormat({ key: 'I', metaKey: true }), 'italic');
  assert.equal(api.shortcutFormat({ key: 'k', ctrlKey: true }), 'link');
  assert.equal(api.shortcutFormat({ key: 'e', ctrlKey: true }), 'code');
  assert.equal(api.shortcutFormat({ key: 'x', ctrlKey: true, shiftKey: true }), 'strike');
  assert.equal(api.shortcutFormat({ key: 'b' }), null);
  assert.equal(api.shortcutFormat({ key: 'b', ctrlKey: true, altKey: true }), null);
});

test('italic inside bold adds a marker instead of stripping the bold', () => {
  assert.equal(api.applyFormat('say **hi**', 6, 8, 'italic').text, 'say ***hi***');
  assert.equal(api.applyFormat('say ***hi***', 7, 9, 'italic').text, 'say **hi**');
});

test('side panel notes use list -> editor navigation and an icon toolbar without a manual save button', () => {
  const read = path => readFileSync(new URL(path, import.meta.url), 'utf8');
  const workspace = read('../components/notes/NotesWorkspace.svelte'), editor = read('../components/notes/NoteEditor.svelte');
  assert.match(workspace, /\{#if compact\}\s*\{#if selectedNote\}<NoteEditor [^>]*onBack=\{back\}/);
  assert.match(editor, /<textarea bind:this=\{textarea\}/);
  assert.match(editor, /if \(event\.isComposing\) return;/, 'IME composition must not trigger shortcuts or list continuation');
  assert.match(editor, /\{#if draft\.status === 'error'\}<button[^>]*title=\{nm\('retry'\)\}/, 'retry only appears after a failed autosave');
  assert.doesNotMatch(editor, /nm\('save'\)/);
});
