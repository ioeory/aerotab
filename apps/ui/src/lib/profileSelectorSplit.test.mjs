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
function harness({ attach = true, fail = false, attachThrows = false } = {}) {
  const calls = [], added = [], restored = [], errors = [];
  const env = { rpc: { async call(method, params) { calls.push({ method, params }); if (fail && method === 'session.openSshProfile') throw Error('offline'); return { id: 'new', title: 'Fresh', kind: 'Ssh' }; } },
    tabs: { add: s => added.push(s), addPane: (...args) => { added.push(args); if (attachThrows) throw Error('attach'); return attach; } },
    recordRestore: (...args) => restored.push(args), requestAnimationFrame: fn => fn(), focusActivePane() {},
    onError: e => errors.push(e), profileEndpointLabel: () => 'old endpoint', i18n: { t: key => key } };
  return { ...functions(app, ['connectProfile'], env), calls, added, restored, errors };
}
const profile = { id: 'saved', kind: 'ssh', name: 'Saved', ssh: { host: 'stale', auth: 'Agent' } };
test('profile split opens by ID, preserves anchor, and records only attached sessions', async () => {
  const h = harness();
  assert.equal(await h.connectProfile(profile, { tabId: 'tab', paneId: 'anchor', direction: 'col' }), true);
  assert.deepEqual(h.calls[0], { method: 'session.openSshProfile', params: { profile_id: 'saved' } });
  assert.deepEqual(h.added[0].slice(0, 1), ['tab']);
  assert.deepEqual(h.added[0].slice(2), ['col', 'after', 'anchor']);
  assert.equal(h.added[0][1].sshProfile, undefined, 'do not retain stale auth or inherited settings');
  assert.equal(h.restored.length, 1);
});
test('vanished target or attachment exception closes new session without recording restore', async () => {
  for (const options of [{ attach: false }, { attachThrows: true }]) {
    const h = harness(options);
    assert.equal(await h.connectProfile(profile, { tabId: 'gone', paneId: 'gone', direction: 'row' }), false);
    assert.deepEqual(h.calls.at(-1), { method: 'session.close', params: { id: 'new' } });
    assert.equal(h.restored.length, 0);
    assert.equal(h.errors.length, 1);
  }
});
test('ordinary profile open remains a new tab and failures can retry', async () => {
  const h = harness(); assert.equal(await h.connectProfile(profile), true); assert.equal(h.added[0].id, 'new');
  const broken = harness({ fail: true }); assert.equal(await broken.connectProfile(profile), false); assert.equal(broken.added.length, 0);
});
test('remote desktops cannot create terminal splits but still open normally', async () => {
  for (const kind of ['rdp', 'vnc']) {
    const h = harness(); assert.equal(await h.connectProfile({ ...profile, kind }, { tabId: 'tab', paneId: 'anchor', direction: 'row' }), false);
    assert.equal(h.calls.length, 0);
    assert.equal(await h.connectProfile({ ...profile, kind }), true);
    assert.equal(h.calls[0].method, 'remote.openProfile');
  }
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
