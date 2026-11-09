import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source = readFileSync(new URL('./sftpDockTarget.ts', import.meta.url), 'utf8');
const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
const api = await import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);
const ssh = { host: 'host', port: 22, user: 'local', auth: 'Agent', jump_via: [] };
test('saved-profile dock retains identity even when the pane carries an SSH snapshot', () => {
  const result = api.resolveDockTargetState(true, null, { id: 'session', kind: 'Ssh', title: 'target', profileId: 'saved', sshProfile: ssh }, undefined, false);
  assert.equal(result.target.profileId, 'saved');
});
test('inline and temporary jump dock never infer saved profile identity', () => {
  const result = api.resolveDockTargetState(true, null, { id: 'session', kind: 'Ssh', title: 'temporary', sshProfile: ssh }, undefined, false);
  assert.equal(result.target.profileId, undefined);
});
test('profile-backed dock fetch preserves ID for backend inheritance on every connection', async () => {
  const target = await api.fetchProfileDockTarget({ call: async (method, params) => {
    assert.equal(method, 'profile.get');
    assert.deepEqual(params, { id: 'saved' });
    return { id: 'saved', name: 'target', kind: 'ssh', ssh };
  } }, 'saved');
  assert.equal(target.profileId, 'saved');
});
