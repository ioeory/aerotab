import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
// Wire-level provider regressions live in Rust; these guard the actual UI entrypoints.
test('cloud setup exposes real provider entries and OS keyring persistence', () => {
  const source = fs.readFileSync(new URL('../components/settings/sections/ConfigSyncSection.svelte', import.meta.url), 'utf8');
  assert.match(source, /value="s3"/);
  assert.match(source, /value="drive"/);
  assert.match(source, /sync\.driveAuthorize/);
  assert.match(source, /sync\.testCloud/);
  const persist = source.slice(source.indexOf('async function persist()'), source.indexOf('async function refreshStatus()'));
  assert.doesNotMatch(persist, /webdavUser, webdavPassword|gitSshKeyPath, gitSshPassphrase/);
});
