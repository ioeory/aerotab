import test from 'node:test';
import assert from 'node:assert/strict';
import { wizardCanEnter, syncSetupComplete, syncTargetFingerprint } from './syncWizard.ts';
test('wizard cannot skip authorization, password, content, configuration or preview', () => {
  const state = { storage: true, tested: false, password: false, content: false, configured: false, preview: false };
  assert.equal(wizardCanEnter(2, state), false);
  state.tested = true; assert.equal(wizardCanEnter(2, state), true);
  assert.equal(wizardCanEnter(4, state), false);
  Object.assign(state, { password:true, content:true, configured:true });
  assert.equal(syncSetupComplete(state), false);
  state.preview = true; assert.equal(syncSetupComplete(state), true);
});
test('approval fingerprint survives backend sorting nested group keys', () => {
  const value = { backend: 'git', gitRepoPath: '/mock', enabledGroups: { Connections: true, Appearance: true, Shortcuts: true, PluginCfg: true, Credentials: false, Notes: false } };
  const restored = { ...value, enabledGroups: Object.fromEntries(Object.entries(value.enabledGroups).sort(([a], [b]) => a.localeCompare(b))) };
  assert.equal(syncTargetFingerprint(value), syncTargetFingerprint(restored));
  assert.notEqual(syncTargetFingerprint(value), syncTargetFingerprint({ ...restored, enabledGroups: { ...restored.enabledGroups, Credentials: true } }));
});
test('approval fingerprint survives JSON persistence and omitted optional fields', () => {
  const value = { backend: 'git', gitRepoPath: '/mock', gitRemoteUrl: 'https://git.invalid/repo', optional: undefined, enabledGroups: { Connections: true } };
  assert.equal(syncTargetFingerprint(value), syncTargetFingerprint(JSON.parse(JSON.stringify(value))));
});
test('target identity changes invalidate approval while timer changes do not', () => {
  const a={backend:'s3',s3Bucket:'one',enabledGroups:{Connections:true}};
  assert.notEqual(syncTargetFingerprint(a),syncTargetFingerprint({...a,s3Bucket:'two'}));
  assert.equal(syncTargetFingerprint(a),syncTargetFingerprint({...a,autoSyncMinutes:10}));
});
