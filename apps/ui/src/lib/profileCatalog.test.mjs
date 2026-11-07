import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source = readFileSync(new URL('./profileCatalogHelpers.ts', import.meta.url), 'utf8');
const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
const api = await import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);
const jumpSource = readFileSync(new URL('./jumpProfiles.ts', import.meta.url), 'utf8');
const jumpJs = ts.transpileModule(jumpSource, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
const jumpApi = await import(`data:text/javascript;base64,${Buffer.from(jumpJs).toString('base64')}`);
test('edited profile and literal jump references save correct credentials through full draft serializer', async () => {
  const target = api.profileToEditorDraft();
  target.name = 'target'; target.host = 'target'; target.user = 'target-user'; target.auth = api.authToDraft('Agent');
  const bastion = { id: 'jump-id', name: 'Bastion Profile', kind: 'ssh', ssh: { host: 'bastion', port: 2222, user: 'jump-user', auth: { PublicKey: { key_path: '/jump-key', passphrase: 'jump-pass' } }, jump_via: [] } };
  target.jumps = await api.resolveProfileEditorJumps('@jump-id\nraw@literal:2200', '', [], async () => jumpApi.parseJumpLines('@jump-id\nraw@literal:2200', api.authFromDraft(target.auth), [bastion]));
  const saved = api.profileFromEditorDraft(target, 'target-id');
  assert.deepEqual(saved.ssh.jump_via[0], bastion.ssh);
  assert.deepEqual(saved.ssh.jump_via[1], { host: 'literal', port: 2200, user: 'raw', auth: 'Agent', jump_via: [] });
  const reloaded = api.profileToEditorDraft(saved);
  assert.deepEqual(api.profileFromEditorDraft(reloaded, saved.id), saved);
  assert.throws(() => jumpApi.parseJumpLines('@missing', 'Agent', [bastion]), /not found/);
});
test('saving an unchanged profile preserves stable jump IDs without snapshot migration', () => {
  const original = { id: 'target-id', name: 'target', kind: 'ssh', ssh: { host: 'target', port: 22, user: 'u', auth: 'Agent', jump_via: [], jump_profile_ids: ['jump-id'] } };
  const saved = api.profileFromEditorDraft(api.profileToEditorDraft(original), original.id);
  assert.deepEqual(saved.ssh.jump_profile_ids, ['jump-id']);
  assert.deepEqual(saved.ssh.jump_via, []);
});
test('group breadcrumbs retain each ancestor path', () => {
  assert.deepEqual(api.profileGroupBreadcrumbs(' prod / web / eu '), [
    { name: 'prod', path: 'prod' }, { name: 'web', path: 'prod/web' }, { name: 'eu', path: 'prod/web/eu' },
  ]);
  assert.deepEqual(api.profileGroupBreadcrumbs(null), []);
  assert.deepEqual(api.profileGroupBreadcrumbs('prod\\\\web>eu').map(crumb => crumb.path), ['prod', 'prod/web', 'prod/web/eu']);
});
test('full SSH, remote and nested jump drafts preserve original data on identity edits', () => {
  const auths = ['Agent', { Password: { secret: 'secret' } }, { PublicKey: { key_path: '/key', passphrase: 'pass' } }, { VaultRef: { entry_id: 'entry', passphrase_entry_id: 'pass-entry' } }];
  const hop = { host: 'jump', port: 2222, user: 'jump-user', auth: { PublicKey: { key_path: '/jump-key', passphrase: 'jump-pass' } }, jump_via: [{ host: 'outer', port: 22, user: 'outer-user', auth: 'Agent', jump_via: [] }] };
  const base = { schemaVersion: 7, id: 'id', name: 'name', group: 'prod/web', tags: ['one'], note: 'note', favorite: true, icon: { kind: 'remote', value: 'https://example/icon.svg' } };
  const cases = auths.map(auth => ({ ...base, kind: 'ssh', ssh: { host: 'host', port: 2222, user: 'user', auth, jump_via: [hop] } }));
  for (const kind of ['rdp', 'vnc']) cases.push({ ...base, kind, [kind]: { host: 'remote', port: kind === 'rdp' ? 3389 : 5901, ssh_profile_id: 'tunnel', local_bind_port: 4444 } });
  for (const original of cases) {
    const draft = api.profileToEditorDraft(original);
    draft.name = 'edited';
    const saved = api.profileFromEditorDraft(draft, original.id);
    assert.deepEqual(saved, { ...original, name: 'edited' });
    if (saved.kind === 'ssh') saved.ssh.jump_via[0].host = 'changed-copy';
    assert.equal(original.kind === 'ssh' ? original.ssh.jump_via[0].host : 'jump', 'jump');
  }
});
test('unchanged jump lines keep per-hop authentication and never require catalog access', async () => {
  const original = [{ host: 'bastion', port: 22, user: 'u', auth: { Password: { secret: 'per-hop' } }, jump_via: [] }];
  const next = await api.resolveProfileEditorJumps('u@bastion', 'u@bastion', original, async () => { throw Error('must not load'); });
  assert.deepEqual(next, original);
  next[0].auth.Password.secret = 'copy';
  assert.equal(original[0].auth.Password.secret, 'per-hop');
});
test('edited jump lines use resolved credentials and failures propagate without mutating saved chain', async () => {
  const original = [{ host: 'old', port: 22, user: 'u', auth: 'Agent', jump_via: [] }];
  const resolved = [{ host: 'new', port: 2222, user: 'n', auth: { VaultRef: { entry_id: 'vault' } }, jump_via: [] }];
  assert.deepEqual(await api.resolveProfileEditorJumps('n@new:2222', 'u@old', original, async () => resolved), resolved);
  await assert.rejects(api.resolveProfileEditorJumps('invalid', 'u@old', original, async () => { throw Error('invalid jump'); }), /invalid jump/);
  assert.equal(original[0].host, 'old');
});
test('remote drafts apply tunnel edits and keep protocol defaults without SSH auth leaking', () => {
  for (const kind of ['rdp', 'vnc']) {
    const draft = api.profileToEditorDraft({ id: 'id', name: 'remote', kind, [kind]: { host: 'h', port: 1234, ssh_profile_id: null } });
    draft.remoteSshProfileId = 'new-tunnel'; draft.localBindPort = 5555; draft.port = 0;
    const saved = api.profileFromEditorDraft(draft, 'id');
    assert.deepEqual(saved[kind], { host: 'h', port: kind === 'rdp' ? 3389 : 5900, ssh_profile_id: 'new-tunnel', local_bind_port: 5555 });
    assert.equal('ssh' in saved, false);
    draft.remoteSshProfileId = ''; draft.localBindPort = '';
    assert.equal(api.profileFromEditorDraft(draft, 'id')[kind].ssh_profile_id, null);
  }
});
test('new and duplicated drafts retain defaults and remote direct configuration', () => {
  const draft = api.profileToEditorDraft(undefined, 'prod');
  assert.equal(draft.group, 'prod');
  draft.name = 'remote'; draft.kind = 'vnc'; draft.host = 'localhost'; draft.port = 5900;
  const saved = api.profileFromEditorDraft(draft, 'new-id');
  assert.equal(saved.id, 'new-id');
  assert.deepEqual(saved.vnc, { host: 'localhost', port: 5900, ssh_profile_id: null, local_bind_port: undefined });
});
const profiles = [{ id: 'a', group: 'prod', favorite: true }, { id: 'b', group: 'prod/web' }, { id: 'c', group: 'production' }, { id: 'd', group: null }];
test('group scope includes descendants but not similarly prefixed paths', () => {
  assert.deepEqual(profiles.filter(p => api.profileInScope(p, 'prod', false)).map(p => p.id), ['a', 'b']);
  assert.deepEqual(profiles.filter(p => api.profileInScope(p, '', false)).map(p => p.id), ['d']);
  assert.equal(profiles.filter(p => api.profileInScope(p, null, false)).length, 4);
  assert.deepEqual(profiles.filter(p => api.profileInScope(p, null, true)).map(p => p.id), ['a']);
});
test('selection reconciliation removes hidden/deleted ids without changing input', () => {
  const selected = new Set(['a', 'b', 'gone']);
  assert.deepEqual([...api.reconcileCatalogSelection(selected, [profiles[0]])], ['a']);
  assert.equal(selected.size, 3);
});
test('unknown view preferences fall back safely', () => {
  assert.equal(api.normalizeResourceView('grid'), 'grid');
  assert.equal(api.normalizeResourceView('list'), 'list');
  assert.equal(api.normalizeResourceView('tree'), 'tree');
  assert.equal(api.normalizeResourceView({ view: 'list' }), 'grid');
});
test('all supported authentication modes round trip without losing secrets', () => {
  for (const auth of ['Agent', { Password: { secret: 'secret' } }, { PublicKey: { key_path: '/key', passphrase: 'pass' } }, { VaultRef: { entry_id: 'entry', passphrase_entry_id: 'pass-entry' } }]) {
    assert.deepEqual(api.authFromDraft(api.authToDraft(auth)), auth);
  }
});
