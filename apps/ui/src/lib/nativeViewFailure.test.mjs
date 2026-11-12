import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import * as transactions from './nativeViewTransaction.ts';

const reasons = ['disposed', 'target-error', 'controller', 'config', 'geometry', 'visibility', 'epoch', 'host', 'generation'];
test('failure payload copies only enum allowlists, never arbitrary diagnostic data', () => {
  assert.equal(typeof transactions.nativeViewFailureDiagnostic, 'function');
  for (const reason of reasons) {
    assert.deepEqual(transactions.nativeViewFailureDiagnostic('webview-to-native', 'prepare', reason), { direction: 'webview-to-native', stage: 'prepare', reason });
  }
  const secret = { sessionId: 'SECRET', host: 'SECRET', text: 'SECRET', token: 'SECRET', toString() { throw Error('must not stringify'); } };
  assert.deepEqual(transactions.nativeViewFailureDiagnostic(secret, secret, secret), { direction: 'unknown', stage: 'unknown', reason: 'unknown' });
});
for (const stage of ['prepare', 'drain', 'commit']) test(`reports ${stage} rejection once without exposing or replacing original exception`, async () => {
  const original = Error('PRIVATE terminal text host token');
  const records = [];
  const options = { current: () => true, prepare: async () => 1, validate: () => true, drain: async () => {}, commit: async () => {}, rollback: async () => {}, diagnostic: { direction: 'native-to-webview', reason: () => 'current', report: event => records.push(event) } };
  options[stage] = async () => { throw original; };
  await assert.rejects(transactions.runNativeViewTransaction(options), e => e === original);
  assert.deepEqual(records, [{ direction: 'native-to-webview', stage, reason: 'operation-error' }]);
});
for (const reason of reasons) test(`current failure ${reason} retains rollback and exception`, async () => {
  const events = [], records = [];
  await assert.rejects(transactions.runNativeViewTransaction({ current: () => false, prepare: async () => 1, validate: () => true, drain: async () => {}, commit: async () => {}, rollback: async () => events.push('rollback'), diagnostic: { direction: 'webview-to-native', reason: () => reason, report: event => records.push(event) } }), /view transaction superseded or session closed/);
  assert.deepEqual(events, ['rollback']);
  assert.deepEqual(records, [{ direction: 'webview-to-native', stage: 'prepare', reason }]);
});
for (const boundary of ['prepare', 'drain', 'commit']) test(`current invalidation at ${boundary} records the last evaluated enum once`, async () => {
  let live = true, status = 'current'; const records = [];
  const current = () => { status = live ? 'current' : 'visibility'; return live; };
  const options = { current, prepare: async () => 1, validate: () => true, drain: async () => {}, commit: async () => {}, rollback: async () => {}, diagnostic: { direction: 'native-to-webview', reason: () => status, report: e => records.push(e) } };
  if (boundary === 'prepare') options.prepare = async () => { live = false; return 1; };
  if (boundary === 'drain') options.drain = async () => { live = false; };
  if (boundary === 'commit') options.commit = async () => { live = false; if (!current()) throw Error('WebView staging superseded'); };
  await assert.rejects(transactions.runNativeViewTransaction(options), /superseded/);
  assert.deepEqual(records, [{ direction: 'native-to-webview', stage: boundary, reason: 'visibility' }]);
});
test('invalid receipt records its enum without serializing the receipt', async () => {
  const records = [];
  await assert.rejects(transactions.runNativeViewTransaction({ current: () => true, prepare: async () => ({ token: 'SECRET' }), validate: () => false, drain: async () => {}, commit: async () => {}, rollback: async () => {}, diagnostic: { direction: 'webview-to-native', reason: () => 'current', report: e => records.push(e) } }), /stale target draw receipt/);
  assert.deepEqual(records, [{ direction: 'webview-to-native', stage: 'prepare', reason: 'stale-receipt' }]);
});
test('diagnostic sink failure cannot prevent rollback or replace original exception', async () => {
  const original = Error('original'); let rolledBack = false;
  await assert.rejects(transactions.runNativeViewTransaction({ current: () => true, prepare: async () => { throw original; }, validate: () => true, drain: async () => {}, commit: async () => {}, rollback: async () => { rolledBack = true; }, diagnostic: { direction: 'webview-to-native', reason: () => 'current', report: () => { throw Error('storage'); } } }), e => e === original);
  assert.equal(rolledBack, true);
});
test('successful transaction produces no failure event', async () => {
  const records = [];
  await transactions.runNativeViewTransaction({ current: () => true, prepare: async () => 1, validate: () => true, drain: async () => {}, commit: async () => {}, rollback: async () => {}, diagnostic: { direction: 'webview-to-native', reason: () => 'current', report: e => records.push(e) } });
  assert.deepEqual(records, []);
});
test('rollback rejection keeps its existing precedence while recording only the transaction failure', async () => {
  const records = [], rollbackError = Error('rollback');
  await assert.rejects(transactions.runNativeViewTransaction({ current: () => false, prepare: async () => 1, validate: () => true, drain: async () => {}, commit: async () => {}, rollback: async () => { throw rollbackError; }, diagnostic: { direction: 'webview-to-native', reason: () => 'geometry', report: e => records.push(e) } }), e => e === rollbackError);
  assert.equal(records.length, 1);
});
const component = readFileSync(new URL('../components/NativeTerminalView.svelte', import.meta.url), 'utf8');
const { nativeViewGeometryEquals } = await import('./nativeViewHost.ts');
for (const direction of ['webview-to-native', 'native-to-webview']) {
  const forward = direction === 'webview-to-native';
  for (const reason of forward ? ['disposed', 'target-error', 'controller', 'config', 'geometry', 'visibility'] : ['disposed', 'epoch', 'controller', 'host', 'generation', 'config', 'geometry', 'visibility']) {
    test(`actual ${direction} component identifies ${reason} without exporting state`, async () => {
      const records = []; let scope;
      const mutate = () => {
        if (reason === 'disposed') scope.disposed = true;
        if (reason === 'target-error') scope.stagingError = Error('SECRET');
        if (reason === 'controller') scope.controller = {};
        if (reason === 'config') scope.viewConfig = () => ({ fontSize: 20, secret: 'SECRET' });
        if (reason === 'geometry') scope.surface.getBoundingClientRect = () => ({ x: 0, y: 0, width: 120, height: 10 });
        if (reason === 'visibility') scope.visible = false;
        if (reason === 'epoch') scope.reverseEpoch++;
        if (reason === 'host') scope.hostAdapter = {};
        if (reason === 'generation') host.generation = () => 'SECRET-new-generation';
      };
      const source = { prepareWebviewSurface: async () => {}, waitForDraw: async () => mutate(), rollbackWebviewSurface: async () => {}, resumeWebviewInput() {}, focus() {} };
      const host = { generation: () => 'SECRET-generation', waitForDraw: async () => true, dispose: async () => {}, setFocusAllowed() {} };
      scope = { backend: 'webview', controller: source, hostAdapter: host, inputQueue: { dispose() {} }, disposed: false, transitionClosing: false,
        committedWebview: false, committedNative: !forward, stagingNative: false, stagingWebview: false, webviewHandoff: false, reverseEpoch: 0,
        surface: { getBoundingClientRect: () => ({ x: 0, y: 0, width: 100, height: 10 }) }, settlePaneLayout: async () => {}, viewConfig: () => ({ fontSize: 15 }),
        traceLayout() {}, traceView() {}, visible: true, occluded: false, globalOccluded: false, document: { hidden: false }, stagingError: null,
        runNativeViewTransaction: transactions.runNativeViewTransaction, nativeViewGeometryEquals, mountSurface: async () => mutate(), quiesceAndDrainNativeInput: async () => {},
        cleanupSurface: null, syncSurface: null, rollbackPending: false, unlistenInput: null, unlistenPaste: null, unlistenFontDelta: null,
        canFocusSurface: () => false, onRecovered() {}, diagnostics: { record: (...args) => records.push(args) } };
      const name = forward ? 'stageNativeSurface' : 'stageWebviewSurface';
      const end = forward ? '  /** Native -> WebView' : '  export function cancelViewTransition';
      const body = component.slice(component.indexOf(`  export async function ${name}(`), component.indexOf(end)).replace('export async', 'async');
      const js = ts.transpileModule(body + `\nreturn ${name};`, { compilerOptions: { target: 99, module: 0 } }).outputText;
      const run = new Function('scope', `with(scope){${js}}`)(scope);
      await assert.rejects(run());
      assert.equal(records.length, 1);
      assert.equal(records[0][0], 'app'); assert.equal(records[0][1], 'terminal.view-switch');
      assert.deepEqual(JSON.parse(records[0][2]), { direction, stage: 'prepare', reason });
      assert.ok(!records[0][2].includes('SECRET'));
    });
  }
}
test('production diagnostics store persists and exports the bounded enum-only event', async () => {
  const { compileModule } = await import('svelte/compiler');
  const source = readFileSync(new URL('./diagnostics.svelte.ts', import.meta.url), 'utf8').replace(/^import .*;\n/gm, '');
  const js = ts.transpileModule(source, { compilerOptions: { target: 99, module: 99 } }).outputText;
  const code = compileModule(js, { filename: 'diagnostics.svelte.js', generate: 'server' }).js.code.replaceAll("'svelte/internal/server'", JSON.stringify(import.meta.resolve('svelte/internal/server')));
  let saved;
  globalThis.localStorage = { getItem: () => null, setItem: (_, value) => { saved = value; } };
  const api = await import(`data:text/javascript;base64,${Buffer.from(`const terminalGeometrySnapshot=()=>[];\nconst terminalPipelineSnapshot=()=>[];\n${code}`).toString('base64')}`);
  for (let i = 0; i < 260; i++) api.diagnostics.record('app', 'terminal.view-switch', JSON.stringify(transactions.nativeViewFailureDiagnostic('webview-to-native', 'drain', 'geometry')));
  assert.equal(api.diagnostics.events.length, 250);
  assert.equal(JSON.parse(saved).length, 250);
  assert.deepEqual(api.diagnostics.pack('test', null).events, api.diagnostics.events);
  assert.deepEqual(JSON.parse(api.diagnostics.events[0].message), { direction: 'webview-to-native', stage: 'drain', reason: 'geometry' });
});
