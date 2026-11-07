import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import ts from 'typescript';
import { SettingsSaveRegistry } from './settingsSaveRegistry.ts';

// Execute the real coordinator class without a DOM. Only Svelte's scalar state
// initializer is substituted; saver registration and async transitions are real.
const source = await readFile(new URL('./settingsStore.svelte.ts', import.meta.url), 'utf8');
const js = ts.transpileModule(source.replace(/^import .*$/gm, '').replace(/^export type .*$/gm, '').replace('export const settingsCoord = new SettingsCoordinator();', 'return SettingsCoordinator;'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 },
}).outputText.replace(/export \{\};?/g, '');
const Coordinator = new Function('SettingsSaveRegistry', '$state', js)(SettingsSaveRegistry, value => value);

test('drafts in multiple visited sections save together, untouched sections do not write', async () => {
  const coordinator = new Coordinator(); const calls = [];
  for (const id of ['appearance', 'ssh', 'window']) coordinator.registerSaver(id, async () => { calls.push(id); });
  coordinator.markDirty('appearance'); coordinator.markDirty('ssh');
  await coordinator.saveAll();
  assert.deepEqual(calls, ['appearance', 'ssh']);
  assert.equal(coordinator.dirty, false);
});

test('single-section apply uses the same save registry and preserves other drafts', async () => {
  const coordinator = new Coordinator(); const calls = [];
  for (const id of ['appearance', 'ssh']) coordinator.registerSaver(id, async () => { calls.push(id); });
  coordinator.markDirty('appearance'); coordinator.markDirty('ssh');
  await coordinator.saveSection('appearance');
  assert.deepEqual(calls, ['appearance']);
  assert.equal(coordinator.dirty, true);
  await coordinator.saveAll();
  assert.deepEqual(calls, ['appearance', 'ssh']);
  assert.equal(coordinator.dirty, false);
});

test('applying appearance alone clears dirty so closing does not warn', async () => {
  const coordinator = new Coordinator();
  coordinator.registerSaver('appearance', async () => {});
  coordinator.markDirty('appearance');
  await coordinator.saveSection('appearance');
  assert.equal(coordinator.dirty, false);
  coordinator.markDirty('appearance');
  coordinator.markClean('appearance');
  assert.equal(coordinator.dirty, false);
});

test('another section edited during appearance apply cannot leave stale appearance dirty', async () => {
  const coordinator = new Coordinator();
  let release;
  coordinator.registerSaver('appearance', () => new Promise(resolve => { release = resolve; }));
  coordinator.markDirty('appearance');
  const pending = coordinator.saveSection('appearance');
  coordinator.markDirty('ssh');
  release(); await pending;
  coordinator.markClean('ssh');
  assert.equal(coordinator.dirty, false, 'the applied appearance draft must already be clean');
});

test('a newer appearance edit during its own apply stays dirty', async () => {
  const coordinator = new Coordinator();
  let release;
  coordinator.registerSaver('appearance', () => new Promise(resolve => { release = resolve; }));
  coordinator.markDirty('appearance');
  const pending = coordinator.saveSection('appearance');
  coordinator.markDirty('appearance');
  release(); await pending;
  assert.equal(coordinator.dirty, true);
});

test('settings shows section load/save errors inside the modal rather than behind it', async () => {
  const layout = await readFile(new URL('../components/settings/SettingsLayout.svelte', import.meta.url), 'utf8');
  assert.match(layout, /role="alert"/);
  assert.match(layout, /function onError\(message: string\)/);
  const appearance = await readFile(new URL('../components/settings/sections/AppearanceSection.svelte', import.meta.url), 'utf8');
  assert.match(appearance, /loadError/);
  assert.match(appearance, /onclick=\{load\}/);
});

test('applying or canceling appearance cannot clear another section draft', () => {
  const coordinator = new Coordinator();
  coordinator.markDirty('appearance'); coordinator.markDirty('ssh');
  coordinator.markClean('appearance');
  assert.equal(coordinator.dirty, true);
  coordinator.markClean('ssh');
  assert.equal(coordinator.dirty, false);
});

test('edits during saving remain dirty and a duplicate save does not run concurrently', async () => {
  const coordinator = new Coordinator();
  let release; let calls = 0;
  coordinator.registerSaver('ssh', async () => { calls++; await new Promise(resolve => { release = resolve; }); });
  coordinator.markDirty('ssh');
  const save = coordinator.saveAll();
  assert.equal(coordinator.saving, true);
  await coordinator.saveAll();
  coordinator.markDirty('ssh'); release(); await save;
  assert.equal(calls, 1);
  assert.equal(coordinator.dirty, true);
  assert.equal(coordinator.saving, false);
});

test('failed saving retains drafts and exposes the failed section for navigation', async () => {
  const coordinator = new Coordinator();
  coordinator.registerSaver('ssh', async () => { throw new Error('offline'); });
  coordinator.markDirty('ssh');
  await assert.rejects(coordinator.saveAll(), /offline/);
  assert.equal(coordinator.failedSection, 'ssh');
  assert.equal(coordinator.dirty, true);
  assert.equal(coordinator.saving, false);
  coordinator.registerSaver('ssh', async () => {});
  await coordinator.saveAll();
  assert.equal(coordinator.failedSection, null);
  assert.equal(coordinator.dirty, false);
});

test('settings keeps visited sections mounted instead of rebuilding active-only branches', async () => {
  const layout = await readFile(new URL('../components/settings/SettingsLayout.svelte', import.meta.url), 'utf8');
  assert.match(layout, /#each visited as section/);
  assert.match(layout, /hidden=\{active !== section\}/);
  assert.doesNotMatch(layout, /#if active === 'application'/);
});

test('appearance persistence does not write the legacy terminal theme key', async () => {
  const appearance = await readFile(new URL('../components/settings/sections/AppearanceSection.svelte', import.meta.url), 'utf8');
  assert.doesNotMatch(appearance, /settings\.set'[^\n]*key: 'theme'/);
  assert.match(appearance, /key: 'uiAppearance'/);
});
