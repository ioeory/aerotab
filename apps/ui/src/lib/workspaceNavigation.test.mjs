import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const app = readFileSync(new URL('../App.svelte', import.meta.url), 'utf8');

test('resource navigation hides but does not conditionally unmount the session workspace', () => {
  assert.match(app, /class:workspace-hidden=\{workspacePage !== 'sessions'\}/);
  assert.match(app, /\{#each tabs\.tabs as tab \(tab\.id\)\}/);
  assert.doesNotMatch(app, /\{#if workspacePage === 'sessions'\}[\s\S]*?<PaneGrid/);
});

test('native view receives explicit workspace visibility and resource blocker', () => {
  assert.match(app, /tabVisible=\{workspacePage === 'sessions' && tabs\.activeId === tab\.id\}/);
  assert.match(app, /data-native-view-blocker=\{workspacePage === 'hosts' \? '' : undefined\}/);
});

test('opening a split in the current tab reveals the session workspace', () => {
  assert.match(app, /const openedSession = ids\.some\(id => !observedSessionIds\.has\(id\)\)/);
  assert.match(app, /\|\| openedSession\) workspacePage = 'sessions'/);
});

test('legacy transfer restoration migrates without mounting the retired transfer executor', () => {
  assert.doesNotMatch(app, /<FileTransferWindow/);
  assert.match(app, /tabs\.remove\(legacyTransfer\.id\);\s*observedActiveTab = tabs\.activeId;\s*workspacePage = 'files';/);
});

test('focus restoration cannot steal focus from the resource workspace', () => {
  assert.match(app, /function focusActivePane\(\)\s*\{\s*if \(workspacePage !== 'sessions'\) return;/);
});
