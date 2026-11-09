import test from 'node:test';
import assert from 'node:assert/strict';
import { configureSyncEngineFromSettings, loadPersistedSyncSettings, applyPersistedAutoSync, ensureSyncEngineConfigured, bootstrapSyncEngine } from './syncConfig.ts';
import { syncTargetFingerprint } from './syncWizard.ts';
test('incomplete first setup cannot bypass gates through existing engine or startup', async () => {
  const value={backend:'git',gitRepoPath:'/mock/local',setupVersion:3,firstSyncApproved:false,autoSyncEnabled:true};
  const calls=[];
  const rpc={async call(method,args){calls.push(method);if(method==='settings.get')return{value};if(method==='sync.status')return{configured:true};if(method==='secret.hasMaster')return{has:true};if(method==='secret.getMaster')return{secret:'mock'};return{};}};
  await assert.rejects(ensureSyncEngineConfigured(rpc),/Finish Config sync/);
  await applyPersistedAutoSync(rpc,value);
  assert.ok(!calls.includes('sync.startAutoSync'));
  await bootstrapSyncEngine(rpc);
  assert.ok(!calls.includes('sync.startAutoSync'));
});
test('approval is scoped to exact target and groups', async()=>{
  const settings={backend:'s3',setupVersion:3,s3Bucket:'old',firstSyncApproved:true,autoSyncEnabled:true};
  settings.approvedTarget=syncTargetFingerprint(settings);
  settings.s3Bucket='new';
  const calls=[];await applyPersistedAutoSync({async call(m){calls.push(m);return{};}},settings);
  assert.deepEqual(calls,['sync.stopAutoSync']);
});

test('legacy transport secrets migrate to keyring before settings are scrubbed', async () => {
  const calls = [];
  const value = { backend: 'webdav', webdavUrl: 'https://dav.invalid', webdavPassword: 'private', gitSshPassphrase: 'phrase' };
  const rpc = { async call(method, args) { calls.push([method, args]); if (method === 'settings.get') return { value }; return {}; } };
  const settings = await loadPersistedSyncSettings(rpc);
  assert.equal(settings.webdavPassword, undefined);
  assert.equal(settings.gitSshPassphrase, undefined);
  assert.ok(calls.some(([m, p]) => m === 'secret.setMaster' && p.account === 'sync.webdav.password' && p.secret === 'private'));
  assert.ok(calls.some(([m, p]) => m === 'secret.setMaster' && p.account === 'sync.git.ssh' && p.secret === 'phrase'));
  assert.ok(calls.some(([m, p]) => m === 'settings.set' && !('webdavPassword' in p.value)));
});

test('protocol selection defaults new setup to v3 and preserves explicit legacy import mode', async () => {
  for (const protocol of [2,3,undefined]) {
    const calls=[];
    await configureSyncEngineFromSettings({async call(m,p){calls.push([m,p]);return{};}}, {backend:'s3',protocol}, 'pw');
    assert.equal(calls.at(-1)[1].protocol,protocol??3);
  }
});

test('S3 and Drive bootstrap dispatch distinct configure RPCs without inline credentials', async () => {
  for (const [backend, method] of [['s3', 'sync.configureS3'], ['drive', 'sync.configureDrive']]) {
    const calls = [];
    const rpc = { async call(m, p) { calls.push([m, p]); return {}; } };
    await configureSyncEngineFromSettings(rpc, { backend, s3Endpoint: 'https://s3.invalid', s3Region: 'us-east-1', s3Bucket: 'test', driveClientId: 'public-client' }, 'master');
    assert.equal(calls.at(-1)[0], method);
    assert.equal(calls.at(-1)[1].master_password, 'master');
    assert.equal(calls.at(-1)[1].secret_access_key, undefined);
  }
});
