import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { compile } from 'svelte/compiler';
import { render } from 'svelte/server';

const read = path => readFileSync(new URL(path, import.meta.url), 'utf8');
const sources = {
  modal: read('../components/hosts/GroupDefaultsModal.svelte'),
  sidebar: read('../components/Sidebar.svelte'),
  editor: read('../components/hosts/ProfileEditorForm.svelte'),
  terminal: read('../components/TerminalPane.svelte'),
  app: read('../App.svelte'),
};
const transpile = source => ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
}).outputText;
const importSource = source => import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
const helpers = await importSource(transpile(read('./profileCatalogHelpers.ts')));
const clone = value => JSON.parse(JSON.stringify(value));

// AST extraction avoids brace counting (templates and nested callbacks contain
// braces too). Execute production functions, not a rewritten version of them.
function script(source) {
  const blocks = [...source.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/g)];
  return blocks.length ? blocks.map(match => match[1]).join('\n') : source;
}
function functions(source, names) {
  const text = script(source);
  const ast = ts.createSourceFile('fixture.ts', text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  return names.map(name => {
    const node = ast.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === name);
    assert.ok(node, `production function ${name} exists`);
    return node.getText(ast).replace(/^export\s+/, '');
  }).join('\n');
}
function factory(source, names, env, extra = '') {
  return new Function(...Object.keys(env), transpile(`${functions(source, names)}\n${extra}\nreturn { ${names.join(', ')} };`))(...Object.values(env));
}
const tree = {
  ...factory(read('./profileTree.ts'), ['parseGroupSegments', 'normalizeGroupPath'], {}),
  ...factory(read('./profileGroupMove.ts'), ['replaceGroupPathPrefix'], {}),
};

// Compile/render the real modal, including its actual form markup. Only imports
// are substituted; no load/save logic is copied, and no desktop RPC is available.
const modalScript = script(sources.modal);
const modalAst = ts.createSourceFile('modal.ts', modalScript, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
const withoutImports = modalAst.statements.filter(node => !ts.isImportDeclaration(node)).map(node => node.getText(modalAst)).join('\n');
const instrumented = sources.modal.replace(modalScript, `
  const { normalizeGroupPath, i18n, onDestroy, capture, fakeDialog } = globalThis[Symbol.for('aerotab.inheritance.test')];
  ${withoutImports}
  dialog = fakeDialog;
  capture({ open, save, parentValue, invalidate,
    edit(values) {
      if (values.overrides) overrides = values.overrides;
      if ('user' in values) user = values.user;
      if ('port' in values) port = values.port;
      if ('authKind' in values) authKind = values.authKind;
      if ('keyPath' in values) keyPath = values.keyPath;
      if ('vaultEntry' in values) vaultEntry = values.vaultEntry;
      if ('passphraseEntry' in values) passphraseEntry = values.passphraseEntry;
      if ('proxyId' in values) proxyId = values.proxyId;
      if ('jumpIds' in values) jumpIds = values.jumpIds;
    },
    snapshot: () => ({ loaded, loading, saving, error, parent, sources, overrides })
  });
`);
const modalCode = compile(instrumented, { filename: 'GroupDefaultsModal.svelte', generate: 'server' }).js.code
  .replace(/from (['"])(svelte(?:\/[^'"]*)?)\1/g, (_, quote, name) => `from ${JSON.stringify(import.meta.resolve(name))}`);
const { default: Modal } = await importSource(modalCode);
function modalHarness(initial = {}) {
  let map = clone(initial), failRead = false, api;
  const calls = [], saved = [], dialog = { open: false, showModal() { this.open = true; }, close() { this.open = false; } };
  const rpc = { async call(method, params) {
    calls.push({ method, params: clone(params ?? {}) });
    if (method === 'settings.get') { if (failRead) throw Error('unavailable'); return { value: clone(map) }; }
    if (method === 'settings.set') { map = clone(params.value); saved.push(clone(params)); return; }
    if (method === 'profile.list' || method === 'proxy.list' || method === 'vault.list') return [];
    if (method === 'vault.status') return { initialized: true, unlocked: true };
    throw Error(`Unexpected RPC: ${method}`);
  } };
  const key = Symbol.for('aerotab.inheritance.test');
  globalThis[key] = { normalizeGroupPath: tree.normalizeGroupPath, i18n: { t: key => key }, onDestroy() {}, capture(value) { api = value; }, fakeDialog: dialog };
  let body;
  try { body = render(Modal, { props: { rpc } }).body; } finally { delete globalThis[key]; }
  return { api, body, calls, saved, dialog, submit: () => api.save({ preventDefault() {} }),
    latest: value => { map = clone(value); }, failRead: value => { failRead = value; }, map: () => clone(map) };
}
const host = (inherit = []) => ({ id: 'host', name: 'Host', kind: 'ssh', group: 'prod/web', ssh: {
  host: 'host.example', port: 2200, user: 'local', auth: { Password: { secret: 'host-only' } },
  proxy_id: 'local-proxy', jump_via: [{ host: 'old-jump', port: 22, user: 'u', auth: 'Agent', jump_via: [] }], inherit,
} });

test('group context menu opens the bound defaults modal and not the host editor', () => {
  assert.match(sources.sidebar, /<GroupDefaultsModal\b[^>]*bind:this=\{groupDefaultsModal\}/);
  assert.match(sources.sidebar, /onclick=\{\(\)\s*=>\s*menuGroupDefaults\(groupPath\)\}/);
  const opened = [];
  let closed = 0;
  factory(sources.sidebar, ['menuGroupDefaults'], { closeMenu() { closed++; }, groupDefaultsModal: { open: path => opened.push(path) } }).menuGroupDefaults('prod/web');
  assert.deepEqual(opened, ['prod/web']);
  assert.equal(closed, 1);
});

test('real modal initially renders a disabled save action and no plaintext credential controls', () => {
  const { body } = modalHarness();
  assert.match(body, /<button\b(?=[^>]*type="submit")(?=[^>]*disabled)[^>]*>/);
  assert.match(body, /<fieldset\b[^>]*disabled/);
  assert.doesNotMatch(body, /<input\b[^>]*type="password"/);
});

test('modal resolves ancestors independently and explicit direct settings override parent routes', async () => {
  const h = modalHarness({ prod: { user: 'parent', port: 2222, auth: 'Agent', proxy_id: 'proxy', jump_profile_ids: ['jump'] }, 'prod/web': { user: 'child', proxy_id: null, jump_profile_ids: [] }, production: { user: 'unrelated' } });
  await h.api.open(' prod / web / eu ');
  const state = h.api.snapshot();
  assert.equal(state.loaded, true);
  assert.deepEqual(state.parent, { user: 'child', port: 2222, auth: 'Agent', proxy_id: null, jump_profile_ids: [] });
  assert.equal(state.sources.user, 'prod/web');
  assert.equal(state.sources.port, 'prod');
  assert.equal(h.api.parentValue('proxy_id'), 'groupDefaults.direct');
  assert.equal(h.api.parentValue('jump_profile_ids'), 'groupDefaults.direct');
});

test('each host field opts in independently, preserves fallback and never mutates input', () => {
  const group = { prod: { user: 'parent', port: 2222, auth: 'Agent', proxy_id: 'parent-proxy', jump_profile_ids: ['jump'] }, 'prod/web': { user: 'child', proxy_id: null, jump_profile_ids: [] } };
  for (const field of ['user', 'port', 'auth', 'proxy_id', 'jump']) {
    const original = host([field]), before = clone(original);
    const result = helpers.previewProfileInheritance(original, group);
    const expected = clone(original);
    if (field === 'jump') { expected.ssh.jump_via = []; expected.ssh.jump_profile_ids = []; }
    else expected.ssh[field] = { user: 'child', port: 2222, auth: 'Agent', proxy_id: null }[field];
    assert.deepEqual(result.profile, expected, field);
    assert.deepEqual(original, before, `${field} does not mutate saved host`);
    assert.deepEqual(helpers.previewProfileInheritance(original, {}).profile, original, `${field} fallback`);
  }
});

test('legacy hosts stay explicit while newly grouped hosts opt in and choices round trip', () => {
  const legacy = host(); delete legacy.ssh.inherit;
  assert.deepEqual(helpers.profileToEditorDraft(legacy).inherit, []);
  assert.deepEqual(helpers.previewProfileInheritance(legacy, { prod: { user: 'wrong', auth: 'Agent' } }).profile, legacy);
  assert.deepEqual(new Set(helpers.profileToEditorDraft(undefined, 'prod').inherit), new Set(['user', 'port', 'auth', 'proxy_id', 'jump']));
  assert.deepEqual(helpers.profileToEditorDraft().inherit, []);
  const draft = helpers.profileToEditorDraft(host(['user', 'auth']));
  assert.deepEqual(helpers.profileFromEditorDraft(draft, 'host').ssh.inherit, ['user', 'auth']);
  const toggle = factory(sources.editor, ['toggleInheritance'], { draft }).toggleInheritance;
  toggle('port', true); toggle('port', true); toggle('auth', false);
  assert.deepEqual(draft.inherit, ['user', 'port']);
  assert.match(sources.editor, /checked=\{draft\.inherit\.includes\(field\)\}/);
});

test('modal saves against the latest settings map and retains unrelated concurrent edits', async () => {
  const h = modalHarness({ prod: { user: 'old' }, other: { port: 22 } });
  await h.api.open('prod');
  h.api.edit({ overrides: { user: true }, user: ' new-user ' });
  h.latest({ prod: { user: 'old' }, other: { port: 2022 }, 'new/group': { auth: 'Agent' } });
  await h.submit();
  assert.deepEqual(h.map(), { prod: { user: 'new-user' }, other: { port: 2022 }, 'new/group': { auth: 'Agent' } });
  assert.equal(h.calls.filter(call => call.method === 'settings.get').length, 2);
});

test('initial load and save-time read failures block writes and keep modal recoverable', async () => {
  const h = modalHarness({ prod: { user: 'old' } });
  h.failRead(true);
  await h.api.open('prod');
  assert.equal(h.api.snapshot().loaded, false);
  assert.ok(h.api.snapshot().error);
  await h.submit();
  assert.equal(h.saved.length, 0);
  h.failRead(false);
  await h.api.open('prod');
  h.api.edit({ overrides: { user: true }, user: 'new' });
  h.failRead(true);
  await h.submit();
  assert.equal(h.saved.length, 0);
  assert.equal(h.dialog.open, true);
  assert.equal(h.api.snapshot().saving, false);
  assert.ok(h.api.snapshot().error);
});

test('explicit direct routing persists null proxy and empty ordered jump IDs', async () => {
  const h = modalHarness({ prod: { proxy_id: 'parent-proxy', jump_profile_ids: ['parent-jump'] } });
  await h.api.open('prod/web');
  h.api.edit({ overrides: { proxy_id: true, jump_profile_ids: true }, proxyId: '', jumpIds: [] });
  await h.submit();
  assert.deepEqual(h.map()['prod/web'], { proxy_id: null, jump_profile_ids: [] });
});

test('group auth persists Vault references or unencrypted key paths, never password/passphrase material', async () => {
  for (const [values, expected] of [
    [{ authKind: 'VaultRef', vaultEntry: 'vault-key', passphraseEntry: 'vault-pass' }, { VaultRef: { entry_id: 'vault-key', passphrase_entry_id: 'vault-pass' } }],
    [{ authKind: 'PublicKey', keyPath: '/keys/id_ed25519' }, { PublicKey: { key_path: '/keys/id_ed25519' } }],
    [{ authKind: 'Agent' }, 'Agent'],
  ]) {
    const h = modalHarness(); await h.api.open('prod');
    h.api.edit({ overrides: { auth: true }, ...values }); await h.submit();
    assert.deepEqual(h.map().prod.auth, expected);
    assert.ok(h.calls.every(call => !/vault\.(?:get|read|reveal)|secret\./.test(call.method)), 'only vault metadata may be read');
  }
  for (const authKind of ['Password', 'password', '']) {
    const h = modalHarness(); await h.api.open('prod');
    h.api.edit({ overrides: { auth: true }, authKind }); await h.submit();
    assert.equal(h.saved.length, 0, `${authKind || 'protected legacy auth'} cannot be persisted`);
  }
});

test('restoring inheritance removes only the current override entry', async () => {
  const h = modalHarness({ prod: { user: 'p' }, 'prod/web': { user: 'c' } });
  await h.api.open('prod/web'); h.api.edit({ overrides: {} }); await h.submit();
  assert.deepEqual(h.map(), { prod: { user: 'p' } });
});

function sidebarHarness(initial, allGroupPaths = []) {
  let map = clone(initial), failed = false;
  const writes = [];
  const rpc = { async call(method, params) {
    if (method === 'settings.get') { if (failed) throw Error('offline'); return { value: clone(map) }; }
    assert.equal(method, 'settings.set'); map = clone(params.value); writes.push(clone(map));
  } };
  const api = factory(sources.sidebar, ['readGroupDefaults', 'underGroup', 'stageGroupDefaultsMove', 'removeGroupDefaults'], {
    rpc, allGroupPaths, normalizeGroupPath: tree.normalizeGroupPath, replaceGroupPathPrefix: tree.replaceGroupPathPrefix, i18n: { t: key => key },
  });
  return { api, writes, map: () => clone(map), latest: next => { map = clone(next); }, fail: () => { failed = true; } };
}

test('rename stages the entire defaults subtree, then removes old paths without prefix collisions', async () => {
  const h = sidebarHarness({ prod: { user: 'root' }, 'prod/web': { port: 2222 }, production: { user: 'unrelated' } });
  const finish = await h.api.stageGroupDefaultsMove('prod', 'live');
  assert.deepEqual(h.map().live, { user: 'root' });
  assert.deepEqual(h.map()['live/web'], { port: 2222 });
  assert.ok(h.map().prod, 'originals survive until host migration completes');
  h.latest({ ...h.map(), concurrent: { auth: 'Agent' } });
  await finish();
  assert.deepEqual(h.map(), { live: { user: 'root' }, 'live/web': { port: 2222 }, production: { user: 'unrelated' }, concurrent: { auth: 'Agent' } });
});

test('rename rejects destination conflicts and detects concurrent edits before deleting originals', async () => {
  for (const [initial, paths] of [[{ prod: {}, 'live/hidden': {} }, []], [{ prod: {} }, ['live/empty']]]) {
    const h = sidebarHarness(initial, paths);
    await assert.rejects(h.api.stageGroupDefaultsMove('prod', 'live'));
    assert.equal(h.writes.length, 0);
  }
  const h = sidebarHarness({ prod: { user: 'before' } });
  const finish = await h.api.stageGroupDefaultsMove('prod', 'live');
  h.latest({ ...h.map(), prod: { user: 'concurrent' } });
  await assert.rejects(finish());
  assert.equal(h.map().prod.user, 'concurrent');
});

test('delete removes descendant defaults only and refuses to overwrite unreadable settings', async () => {
  const h = sidebarHarness({ prod: {}, 'prod/web': { auth: 'Agent' }, production: { user: 'keep' }, other: {} });
  await h.api.removeGroupDefaults('prod');
  assert.deepEqual(h.map(), { production: { user: 'keep' }, other: {} });
  h.fail(); await assert.rejects(h.api.removeGroupDefaults('other'));
  assert.equal(h.writes.length, 1);
  const deletion = functions(sources.sidebar, ['deleteGroup']);
  assert.ok(deletion.indexOf('removeGroupDefaults') > deletion.indexOf('saveExplicitGroups'), 'remove defaults only after hosts and groups have moved');
});

test('modal refuses concurrent same-group changes, creations and deletions without closing', async () => {
  for (const [initial, latest] of [
    [{ prod: { user: 'old' } }, { prod: { user: 'concurrent' } }],
    [{}, { prod: { port: 2222 } }],
    [{ prod: { user: 'old' } }, {}],
  ]) {
    const h = modalHarness(initial);
    await h.api.open('prod');
    h.api.edit({ overrides: { user: true }, user: 'draft' });
    h.latest(latest);
    await h.submit();
    assert.deepEqual(h.map(), latest);
    assert.equal(h.saved.length, 0);
    assert.equal(h.dialog.open, true);
    assert.equal(h.api.snapshot().error, 'groupDefaults.saveConflict');
    assert.equal(h.api.snapshot().saving, false);
  }
});

function deletionHarness({ failAt, change } = {}) {
  let map = { prod: { user: 'root' }, 'prod/web': { port: 2222 }, production: { user: 'keep' } };
  const initial = clone(map), events = [], errors = [];
  const profiles = [host(), { ...host(), id: 'second' }];
  const rpc = { async call(method, params) {
    events.push(method);
    if (method === 'settings.get') { if (failAt === 'read') throw Error('read failed'); return { value: clone(map) }; }
    if (method === 'settings.set') { map = clone(params.value); return; }
    assert.equal(method, 'profile.upsert');
    if (failAt === 'upsert' && params.id === 'second') throw Error('upsert failed');
  } };
  const api = factory(sources.sidebar, ['readGroupDefaults', 'underGroup', 'removeGroupDefaults', 'deleteGroup'], {
    rpc, bulkBusy: false, profiles, explicitGroupPaths: ['prod', 'prod/web'],
    normalizeGroupPath: tree.normalizeGroupPath, profilesUnderGroupPath: list => list,
    i18n: { t: key => key }, closeMenu() {}, appConfirm: async () => true,
    async saveExplicitGroups() { events.push('groups'); if (failAt === 'groups') throw Error('groups failed'); if (change) map = change(clone(map)); },
    catalog: { group: null }, enterDirectory() {}, notifyProfilesChanged() {}, refresh: async () => {}, onError: error => errors.push(error),
  });
  return { api, initial, events, errors, map: () => clone(map) };
}

test('delete preserves defaults if host migration or group persistence fails and reads before moving', async () => {
  for (const failAt of ['read', 'upsert', 'groups']) {
    const h = deletionHarness({ failAt });
    await h.api.deleteGroup('prod');
    assert.deepEqual(h.map(), h.initial, failAt);
    assert.equal(h.events.includes('settings.set'), false, failAt);
    assert.equal(h.errors.length, 1);
    if (failAt === 'read') assert.equal(h.events.includes('profile.upsert'), false);
  }
});

test('delete cleans defaults after successful migration and preserves concurrent subtree edits', async () => {
  const success = deletionHarness();
  await success.api.deleteGroup('prod');
  assert.deepEqual(success.map(), { production: { user: 'keep' } });
  assert.ok(success.events.indexOf('settings.set') > success.events.indexOf('groups'));
  for (const change of [
    map => ({ ...map, prod: { user: 'concurrent' } }),
    map => ({ ...map, 'prod/new': { auth: 'Agent' } }),
    map => { delete map['prod/web']; return map; },
  ]) {
    const h = deletionHarness({ change });
    await h.api.deleteGroup('prod');
    assert.deepEqual(h.map(), change(clone(h.initial)));
    assert.deepEqual(h.errors, ['groupDefaults.concurrentChange']);
    assert.equal(h.events.includes('settings.set'), false);
  }
});

test('delete retains unrelated concurrent edits while cleaning the unchanged subtree', async () => {
  const h = deletionHarness({ change: map => ({ ...map, other: { port: 2022 } }) });
  await h.api.deleteGroup('prod');
  assert.deepEqual(h.map(), { production: { user: 'keep' }, other: { port: 2022 } });
  assert.deepEqual(h.errors, []);
});

test('TerminalPane reconnect resolves saved profile ID instead of stale SSH snapshots', async () => {
  for (const profileId of ['saved-profile', undefined]) {
    const calls = [], replaced = [];
    const session = { id: 'old', title: 'Old', kind: 'Ssh', profileId, sshProfile: host().ssh };
    const api = factory(sources.terminal, ['reconnectSession'], {
      canReconnect: true, reconnecting: false, session,
      tabs: { tabOf: () => ({ id: 'tab' }), replacePaneSession: (...args) => replaced.push(args) },
      rpc: { async call(method, params) { calls.push({ method, params }); return { id: 'new', title: 'New', kind: 'Ssh' }; } },
      document: { dispatchEvent() {} }, CustomEvent: class {}, onError: error => assert.fail(error), reconnectTargetLabel: () => 'target',
    });
    await api.reconnectSession();
    assert.equal(calls[0].method, profileId ? 'session.openSshProfile' : 'session.openSsh');
    assert.deepEqual(calls[0].params, profileId ? { profile_id: profileId } : { title: session.title, profile: session.sshProfile });
    assert.equal(replaced[0][2].id, 'new');
    assert.equal(calls.at(-1).method, 'session.close');
  }
});

test('App host stats sends profile_id for saved hosts and uses inline profiles only for ad hoc panes', async () => {
  for (const profileId of ['saved-profile', undefined]) {
    const calls = []; let fallbacks = 0;
    const api = factory(sources.app, ['refreshHostStats'], {
      hostStatsSeq: 0, hostStatsEnabled: true, hostStats: null, hostStatsStatus: 'idle', hostStatsUpdatedAt: null,
      currentActivePane: () => ({ profileId }), resolveHostStatsTarget: async () => { fallbacks++; return host().ssh; },
      rpc: { async call(method, params) { calls.push({ method, params }); return { cpu_percent: 42 }; } },
    });
    await api.refreshHostStats();
    assert.equal(calls[0].method, 'ssh.hostStats');
    assert.deepEqual(calls[0].params, profileId ? { profile_id: profileId } : { profile: host().ssh });
    assert.equal(fallbacks, profileId ? 0 : 1);
  }
});
