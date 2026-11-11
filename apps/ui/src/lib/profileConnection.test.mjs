import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const read = name => { try { return readFileSync(new URL(name, import.meta.url), 'utf8'); } catch { return ''; } };
const js = text => ts.transpileModule(text, { compilerOptions: { target: 99, module: 99 } }).outputText;
const source = read('./profileConnection.ts');
const api = new Function(js(source.replace(/^import .*;$/gm, '')).replace(/export /g, '') + ';return typeof openProfileConnection === "function" ? {openProfileConnection,retryProfileConnection} : {};')();
function harness() {
  const code = js(read('./tabs.svelte.ts').replace(/^import .*;$/gm, '')).replace(/export /g, '');
  let n = 0;
  const tabs = new Function('$state', 'uuidv4', code + ';return tabs;')(x => x, () => `id-${++n}`);
  const calls = [], pending = [], errors = [], restored = [];
  let started = 0;
  const deps = { tabs, uuid: () => `id-${++n}`, onStart: () => started++, onError: e => errors.push(e), onConnected: s => restored.push(s),
    rpc: { call(method, params) { calls.push({ method, params }); if (method === 'session.openSshProfile') return new Promise((resolve, reject) => pending.push({ resolve, reject })); return Promise.resolve(); } } };
  return { deps, tabs, calls, pending, errors, restored, get started() { return started; } };
}
const p = { id: 'saved', name: 'Saved', kind: 'ssh', ssh: { host: 'old', auth: { password: 'stale' } } };
const live = { id: 'live', kind: 'Ssh', title: 'Fresh' };
test('saved SSH creates visible connecting pane before RPC resolves, never carries snapshots', async () => {
  assert.equal(typeof api.openProfileConnection, 'function', 'optimistic connection coordinator exists');
  const h = harness(); const result = api.openProfileConnection(p, undefined, h.deps);
  assert.equal(h.started, 1); assert.equal(h.tabs.tabs.length, 1);
  const pane = h.tabs.tabs[0].panes[0]; assert.equal(pane.connection.status, 'connecting'); assert.equal(pane.sshProfile, undefined);
  assert.deepEqual(h.calls[0], { method: 'session.openSshProfile', params: { profile_id: 'saved' } });
  h.pending[0].resolve(live); assert.equal(await result, true); assert.equal(h.tabs.tabs[0].panes[0].id, 'live'); assert.equal(h.restored.length, 1);
});
test('failure retains error pane and retry uses profile ID in the same layout', async () => {
  const h = harness(); const result = api.openProfileConnection(p, undefined, h.deps); h.pending[0].reject(Error('offline')); assert.equal(await result, false);
  const pane = h.tabs.tabs[0].panes[0]; assert.equal(pane.connection.status, 'failed'); assert.match(pane.connection.error, /offline/);
  const retry = api.retryProfileConnection(pane, h.deps); assert.equal(h.tabs.tabs.length, 1); assert.equal(h.tabs.tabs[0].panes[0].connection.status, 'connecting');
  assert.equal(await api.retryProfileConnection(h.tabs.tabs[0].panes[0], h.deps), false);
  h.pending[1].resolve(live); assert.equal(await retry, true); assert.equal(h.tabs.tabs[0].panes[0].id, 'live');
});
test('closing connecting tab or pane closes late session without resurrecting layout', async () => {
  for (const close of ['tab', 'pane']) {
    const h = harness();
    const existing = close === 'pane' ? h.tabs.add({ id: 'anchor', kind: 'Ssh', title: 'A' }) : undefined;
    const target = existing ? { tabId: existing.id, paneId: 'anchor', direction: 'row' } : undefined;
    const result = api.openProfileConnection(p, target, h.deps); const tab = h.tabs.tabs[0], id = tab.panes.at(-1).id;
    if (close === 'tab') h.tabs.remove(tab.id); else h.tabs.removePane(tab.id, id);
    h.pending[0].resolve(live); assert.equal(await result, false); assert.equal(h.tabs.tabs.length, close === 'tab' ? 0 : 1); assert.equal(h.restored.length, 0);
    if (existing) assert.deepEqual(tab.panes.map(p => p.id), ['anchor']);
    assert.deepEqual(h.calls.at(-1), { method: 'session.close', params: { id: 'live' } });
  }
});
test('split reserves captured anchor immediately and a removed anchor never opens a session', async () => {
  const h = harness(); const tab = h.tabs.add({ id: 'anchor', kind: 'Ssh', title: 'A' });
  const result = api.openProfileConnection(p, { tabId: tab.id, paneId: 'anchor', direction: 'col' }, h.deps);
  assert.equal(tab.layout.direction, 'col'); assert.equal(tab.panes.length, 2); h.pending[0].resolve(live); await result;
  assert.equal(await api.openProfileConnection(p, { tabId: tab.id, paneId: 'gone', direction: 'row' }, h.deps), false); assert.equal(h.pending.length, 1);
});
test('closing a retry placeholder also closes the late session and never restores it', async () => {
  const h = harness(); const initial = api.openProfileConnection(p, undefined, h.deps);
  h.pending[0].reject(Error('first failure')); await initial;
  const tab = h.tabs.tabs[0], pane = tab.panes[0]; const retry = api.retryProfileConnection(pane, h.deps);
  h.tabs.remove(tab.id); h.pending[1].resolve(live);
  assert.equal(await retry, false); assert.equal(h.tabs.tabs.length, 0); assert.equal(h.restored.length, 0);
  assert.deepEqual(h.calls.at(-1), { method: 'session.close', params: { id: 'live' } });
});
test('moving a connecting pane to another tab keeps the late session in its current owner', async () => {
  const h = harness(); const result = api.openProfileConnection(p, undefined, h.deps);
  const first = h.tabs.tabs[0], pane = first.panes[0]; const other = h.tabs.add({ id: 'other', title: 'Other', kind: 'Ssh' });
  assert.equal(h.tabs.movePaneBetweenTabs(first.id, pane.id, other.id, 'other', 'right'), true);
  h.pending[0].resolve(live); assert.equal(await result, true);
  assert.deepEqual(other.panes.map(p => p.id), ['other', 'live']); assert.equal(h.calls.length, 1);
});
test('RDP and VNC stay external without connecting panes or workspace switch', async () => {
  for (const kind of ['rdp', 'vnc']) { const h = harness(); assert.equal(await api.openProfileConnection({ ...p, kind }, undefined, h.deps), true); assert.equal(h.tabs.tabs.length, 0); assert.equal(h.started, 0); assert.equal(h.calls[0].method, 'remote.openProfile'); }
});
test('pending panes cannot mount terminal RPC clients or receive broadcasts/closes', () => {
  assert.match(read('../components/PaneGrid.svelte'), /\{#if pane\.connection\}[\s\S]*?\{:else\}[\s\S]*?<TerminalPane/);
  assert.match(read('../App.svelte'), /sessionIds\.filter\(id => !id\.startsWith\('pending-profile:'\)\)/);
  assert.match(read('../App.svelte'), /activeSessionId=\{activePane\?\.connection \? undefined : activePane\?\.id\}/);
  assert.match(read('./broadcast.ts'), /if \(p\.connection\) return false/);
  assert.match(read('./sessionClose.ts'), /!id\.startsWith\('pending-profile:'\)/);
});
