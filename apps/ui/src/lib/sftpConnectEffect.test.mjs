import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { compileModule } from 'svelte/compiler';
const app = readFileSync(new URL('../components/SftpBrowser.svelte', import.meta.url), 'utf8');
const start = app.lastIndexOf('  $effect(() => {', app.indexOf('    void target?.name;'));
const effect = app.slice(start, app.indexOf('\n  });', start) + '\n  });'.length);
const source = `
import { untrack, flushSync } from 'svelte';
import { effect_root } from 'TEST_RUNTIME';
let target = $state({name:'synthetic',ssh:{host:'invalid',user:'synthetic',port:22}});
let sudoMode = $state(false), sessionId = $state('previous');
let connectSeq = 0, connectInFlight = false, appliedConnectKey = '', attempts = 0;
const sftpConnectKey = () => target ? target.ssh.host + sudoMode : '';
async function connect() {
  attempts++;
  // Actual connect() synchronously calls invalidateSftpSession before its first await.
  const prev = sessionId; sessionId = null;
  await Promise.resolve();
  sessionId = 'new'; appliedConnectKey = sftpConnectKey();
}
const stop = effect_root(() => { ${effect} });
flushSync();
await Promise.resolve(); await Promise.resolve(); flushSync();
if (attempts !== 1) throw Error('connect effect restarted itself: ' + attempts);
sessionId = 'external'; flushSync();
if (attempts !== 1) throw Error('output session tracked by connect effect');
sudoMode = true; flushSync(); await Promise.resolve(); await Promise.resolve(); flushSync();
if (attempts !== 2) throw Error('explicit target setting did not reconnect');
stop();
`;
const code = compileModule(source, { filename: 'sftp-regression.svelte.js', generate: 'client' }).js.code
.replaceAll("'TEST_RUNTIME'", JSON.stringify(import.meta.resolve('svelte/internal/client')))
.replaceAll("'svelte/internal/client'", JSON.stringify(import.meta.resolve('svelte/internal/client')))
.replaceAll("'svelte'", JSON.stringify(new URL('../../node_modules/svelte/src/index-client.js', import.meta.url).href));
test('SFTP auto-connect must not subscribe to session state mutated by invalidation', () => {
  const result = spawnSync(process.execPath, ['--conditions=browser','--input-type=module','-e',code], {encoding:'utf8',timeout:10000});
  assert.equal(result.status, 0, result.stderr || String(result.error));
});
