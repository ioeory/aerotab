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
test('target identity changes invalidate approval while timer changes do not', () => {
  const a={backend:'s3',s3Bucket:'one',enabledGroups:{Connections:true}};
  assert.notEqual(syncTargetFingerprint(a),syncTargetFingerprint({...a,s3Bucket:'two'}));
  assert.equal(syncTargetFingerprint(a),syncTargetFingerprint({...a,autoSyncMinutes:10}));
});
