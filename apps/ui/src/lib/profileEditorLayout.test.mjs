import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { parse } from 'svelte/compiler';

const read = path => readFileSync(new URL(path, import.meta.url), 'utf8');
const modal = parse(read('../components/ProfileModal.svelte'), { modern: true });
const editor = parse(read('../components/hosts/ProfileEditorForm.svelte'), { modern: true });
const rule = (ast, selector) => {
  const node = ast.css.children.find(node => node.type === 'Rule' && node.prelude.children.some(part => part.children.map(child => child.selectors.map(s => s.type === 'TypeSelector' ? s.name : s.type === 'ClassSelector' ? `.${s.name}` : '').join('')).join(' ') === selector));
  assert.ok(node, `CSS rule ${selector} exists`);
  return Object.fromEntries(node.block.children.filter(child => child.type === 'Declaration').map(child => [child.property, child.value.trim()]));
};

test('profile drawer constrains the form instead of scrolling the entire dialog', () => {
  assert.equal(rule(modal, 'dialog.profile-drawer').overflow, 'hidden');
  const form = rule(editor, 'form');
  assert.equal(form.height, '100%');
  assert.equal(form.display, 'flex');
  assert.equal(form['flex-direction'], 'column');
  assert.equal(form['min-height'], '0');
});

test('only profile fields scroll; header and actions stay outside the scroll region', () => {
  const form = editor.fragment.nodes.find(node => node.type === 'RegularElement' && node.name === 'form');
  const children = form.fragment.nodes;
  const body = children.find(node => node.type === 'RegularElement' && node.attributes.some(attr => attr.name === 'class' && attr.value[0]?.data === 'editor-body'));
  assert.ok(body, 'form has a dedicated scroll region');
  assert.ok(body.fragment.nodes.some(node => node.type === 'KeyBlock'), 'all keyed profile fields are inside scroll region');
  assert.ok(children.some(node => node.type === 'RenderTag'), 'header is outside scroll region');
  assert.ok(children.some(node => node.type === 'RegularElement' && node.name === 'footer'), 'actions are outside scroll region');
  const scroll = rule(editor, '.editor-body');
  assert.equal(scroll['min-height'], '0');
  assert.equal(scroll['overflow-y'], 'auto');
  assert.equal(scroll['overscroll-behavior'], 'contain');
  assert.equal(rule(editor, 'footer')['flex-shrink'], '0');
});

test('profile typography uses compact UI tokens rather than oversized fixed text', () => {
  assert.equal(rule(editor, 'form')['font-size'], 'var(--text-compact)');
  assert.equal(rule(editor, '.input')['font-size'], 'inherit');
  assert.equal(rule(editor, 'button')['font-size'], 'inherit');
});
