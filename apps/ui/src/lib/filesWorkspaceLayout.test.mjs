import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { parse } from 'svelte/compiler';

const read = name => readFileSync(new URL(`../components/${name}.svelte`, import.meta.url), 'utf8');
const css = (name, selector) => {
  const ast = parse(read(name), { modern: true });
  const rule = ast.css.children.find(node => node.type === 'Rule' && node.prelude.children.some(part => part.children.map(child => child.selectors.map(s => s.type === 'ClassSelector' ? `.${s.name}` : s.type === 'TypeSelector' ? s.name : '').join('')).join(' ') === selector));
  assert.ok(rule, `${name}: ${selector} exists`);
  return Object.fromEntries(rule.block.children.filter(node => node.type === 'Declaration').map(node => [node.property, node.value.trim()]));
};

test('Files fills its horizontal flex host independently of local or remote intrinsic sizes', () => {
  const root = css('FilesWorkspace', '.files-workspace');
  assert.equal(root.flex, '1');
  assert.equal(root.width, '100%');
  assert.equal(root['min-width'], '0');
  assert.equal(root.overflow, 'hidden');
});

test('both sides remain bounded while the existing ratio splitter controls their allocation', () => {
  const source = read('FilesWorkspace');
  assert.match(source, /minmax\(0,\$\{sideRatio\}fr\) 5px minmax\(0,\$\{100-sideRatio\}fr\)/);
  assert.match(source, /unitsPerPixel=\{100\/Math\.max\(1,sidesWidth\)\}/);
  for (const selector of ['.file-tab-panel', '.file-tab-body']) {
    assert.equal(css('FileWorkspaceSide', selector)['min-width'], '0');
  }
});

test('remote errors wrap unbroken messages and scroll within a bounded banner', () => {
  const banner = css('SftpBrowser', '.sftp-list-error');
  assert.equal(banner['min-height'], '0');
  assert.equal(banner['max-height'], '30%');
  assert.equal(banner['flex-shrink'], '0');
  assert.equal(banner['overflow-y'], 'auto');
  assert.equal(banner['overflow-wrap'], 'anywhere');
  assert.equal(banner['align-items'], 'flex-start');
});

test('dock remains remote-only and modal retains its local pane, connection identity and shared queue', () => {
  const source = read('SftpBrowser');
  assert.match(source, /mode === 'modal' && showLocalPane && Boolean\(localCwd\)/);
  assert.match(source, /if \(mode !== 'modal' \|\| !showLocalPane\) return/);
  assert.match(source, /profile_id: profile\?\.id \?\? source\?\.profileId/);
  assert.match(source, /sudo: sudoMode/);
  assert.match(source, /fileService\.queue\.enqueue/);
  assert.match(read('FileWorkspaceSide'), /mode="dock" showLocalPane=\{false\}/);
});
