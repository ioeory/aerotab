import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { compile } from 'svelte/compiler';
const read = p => readFileSync(new URL(p, import.meta.url), 'utf8');
const app = read('../App.svelte'), picker = read('../components/ProfileSelector.svelte');
const transpile = s => ts.transpileModule(s, { compilerOptions: { target: 99, module: 99 } }).outputText;
function functions(source, names, env, extra = '') {
  const text = source.match(/<script[^>]*>([\s\S]*?)<\/script>/)?.[1] ?? source;
  const ast = ts.createSourceFile('test.ts', text, 99, true, ts.ScriptKind.TS);
  const code = names.map(name => {
    const fn = ast.statements.find(n => ts.isFunctionDeclaration(n) && n.name?.text === name);
    assert.ok(fn, `production function ${name} exists`); return fn.getText(ast);
  }).join('\n');
  return new Function(...Object.keys(env), transpile(`${code}\n${extra}\nreturn {${names}}`))(...Object.values(env));
}
const profile = { id: 'saved', kind: 'ssh', name: 'Saved', ssh: { host: 'stale', auth: 'Agent' } };
test('App delegates profile opens and captured picker targets to the shared optimistic coordinator', async () => {
  const calls = [], deps = {};
  const api = functions(app, ['connectProfile'], { profileConnectionDeps: () => deps,
    openProfileConnection: (...args) => { calls.push(args); return Promise.resolve(true); } });
  const target = { tabId: 'tab', paneId: 'anchor', direction: 'col' };
  assert.equal(await api.connectProfile(profile, target), true);
  assert.deepEqual(calls, [[profile, target, deps]]);
});
test('Sidebar actual host entry delegates immediately, capturing split target without opening a second RPC', async () => {
  const sidebar = read('../components/Sidebar.svelte');
  const calls = [], recent = [], tab = { id: 'tab', kind: 'terminal' };
  const api = functions(sidebar, ['openProfile'], { catalog: { recordRecent: id => recent.push(id) },
    tabs: { activeId: 'tab', tabs: [tab], activePane: () => ({ id: 'anchor' }) },
    connectProfile: (...args) => { calls.push(args); return Promise.resolve(true); } });
  await api.openProfile(profile, 'split-down');
  assert.deepEqual(calls[0], [profile, { tabId: 'tab', paneId: 'anchor', direction: 'col' }]);
  await api.openProfile(profile); assert.equal(calls[1][1], undefined); assert.deepEqual(recent, ['saved', 'saved']);
  assert.match(app, /<ResourceWorkspace[\s\S]*?\{connectProfile\}/);
  assert.match(read('../components/hosts/ResourceWorkspace.svelte'), /<Sidebar[^>]*\{connectProfile\}/);
});
test('pane insertion honors explicit anchor after focus changes with no fallback or tab creation', () => {
  const source = read('./tabs.svelte.ts').replace(/import[^;]+;/g, '').replace('export const tabs', 'const tabs');
  const store = new Function('$state', 'uuidv4', transpile(source).replace(/export \{\};?/g, '') + ';return tabs;')(x => x, () => crypto.randomUUID());
  const tab = store.add({ id: 'a', title: 'A' });
  store.addPane(tab.id, { id: 'b' }, 'row');
  assert.equal(store.addPane(tab.id, { id: 'c' }, 'col', 'after', 'a'), true);
  assert.equal(tab.layout.children[0].direction, 'col');
  assert.deepEqual(tab.layout.children[0].children.map(n => n.pane.id), ['a', 'c']);
  assert.equal(tab.activePaneId, 'c');
  const before = JSON.stringify(tab.layout);
  assert.equal(store.addPane(tab.id, { id: 'leak' }, 'row', 'after', 'missing'), false);
  assert.equal(store.addPane('missing-tab', { id: 'leak' }, 'row', 'after', 'a'), false);
  assert.equal(JSON.stringify(tab.layout), before); assert.equal(store.tabs.length, 1);
});
test('selector has top-layer menu, keyboard invocation, busy guard and profile-only binding', () => {
  assert.doesNotThrow(() => compile(picker, { generate: 'server' }));
  assert.match(picker, /<dialog[\s\S]*?role="menu"/);
  assert.match(picker, /showModal\(/);
  assert.match(picker, /clampMenuToViewport/);
  assert.match(picker, /ContextMenu/);
  assert.equal((picker.match(/oncontextmenu=/g) ?? []).length, 1);
  assert.match(picker, /oncontextmenu=\{[^}]*showProfileMenu\(p, ev\)/);
  assert.match(picker, /if \(busy\) return/);
});
test('split picker captures target before await, prevents duplicate opens, and is retryable', async () => {
  let finish; const opened = [], tab = { id: 'tab', kind: 'terminal', activePaneId: 'anchor' };
  const api = functions(app, ['onPickerSplit'], { pickerConnecting: false, pickerOpen: true,
    tabs: { activeId: 'tab', tabs: [tab], activePane: t => ({ id: t.activePaneId }) },
    connectProfile: (...args) => { opened.push(args); return new Promise(r => { finish = r; }); },
    requestAnimationFrame: fn => fn(), focusActivePane() {} });
  const first = api.onPickerSplit(profile, 'row');
  tab.activePaneId = 'changed'; await api.onPickerSplit(profile, 'col');
  assert.equal(opened.length, 1); assert.equal(opened[0][1].paneId, 'anchor');
  finish(false); await first;
  const retry = api.onPickerSplit(profile, 'col'); assert.equal(opened.length, 2); finish(true); await retry;
});
test('no active terminal prevents any split connection', async () => {
  for (const tab of [undefined, { id: 'files', kind: 'transfer' }, { id: 'empty', kind: 'terminal' }]) {
    const api = functions(app, ['onPickerSplit'], { pickerConnecting: false,
      tabs: { activeId: tab?.id, tabs: tab ? [tab] : [], activePane: () => undefined },
      connectProfile: () => assert.fail('must not connect') });
    assert.equal(await api.onPickerSplit(profile, 'row'), false);
  }
});
test('menu arrows wrap, Home/End focus endpoints and Escape/Tab close only the menu', () => {
  let focus = 0, closed = 0, stopped = 0;
  const buttons = [0, 1].map(index => ({ focus() { focus = index; } }));
  const api = functions(picker, ['onMenuKey'], { menuEl: { querySelectorAll: () => buttons }, document: { get activeElement() { return buttons[focus]; } }, closeMenu: () => { closed++; } });
  const key = key => api.onMenuKey({ key, preventDefault() {}, stopPropagation() { stopped++; } });
  key('ArrowUp'); assert.equal(focus, 1); key('ArrowDown'); assert.equal(focus, 0);
  key('End'); assert.equal(focus, 1); key('Home'); assert.equal(focus, 0);
  key('Escape'); key('Tab'); assert.equal(closed, 2); assert.equal(stopped, 6);
});
test('menu selection is disabled without terminal or during connection; recent updated only on success', async () => {
  for (const [busy, canSplit, success] of [[true, true, true], [false, false, true], [false, true, false], [false, true, true]]) {
    const events = [];
    const api = functions(picker, ['splitProfile'], { busy, canSplit, menuProfile: profile, closeMenu: () => events.push('close'),
      onSplit: async (p, dir) => { assert.equal(p.id, 'saved'); assert.equal(dir, 'row'); events.push('split'); return success; }, pushRecent: () => events.push('recent') });
    await api.splitProfile('row');
    assert.deepEqual(events, busy || !canSplit ? [] : success ? ['close', 'split', 'recent'] : ['close', 'split']);
  }
});
