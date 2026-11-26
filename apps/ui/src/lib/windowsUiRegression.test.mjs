import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const read = path => readFileSync(new URL(path, import.meta.url), 'utf8');
test('endpoint menu has its own opaque surface outside transparency utility overrides', () => {
  assert.match(read('../components/EndpointSelector.svelte'), /<HostSelect\b/);
  const source = read('../components/HostSelect.svelte');
  assert.match(source, /class="menu"/);
  assert.match(source, /\.menu\s*\{[^}]*background:\s*var\(--color-panel\)/);
  assert.match(source, /\.host-select\s*\{[^}]*flex-shrink:\s*0/);
});
test('native select popup options explicitly pair foreground and opaque background', () => {
  const css = read('../app.css');
  assert.match(css, /select option, select optgroup\s*\{[^}]*color:\s*var\(--color-fg\)[^}]*background(?:-color)?:\s*var\(--color-panel\)/);
});
test('profile clicks only select after checkbox selection mode is entered', () => {
  const sidebar = read('../components/Sidebar.svelte');
  const handler = sidebar.slice(sidebar.indexOf('function onProfileRowClick'), sidebar.indexOf('function toggleProfileCheckbox'));
  assert.match(handler, /if \(!hasProfileSelection\)/);
  assert.match(handler, /toggleProfileInSelection\(selectedProfileIds, p.id\)/);
  assert.doesNotMatch(handler, /new Set\(\[p.id\]\)/);
});
