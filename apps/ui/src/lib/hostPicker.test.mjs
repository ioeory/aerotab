import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const component = path => readFileSync(new URL(`../components/${path}.svelte`, import.meta.url), 'utf8');

test('ordinary host selection uses the shared list or its single-select wrapper', () => {
  for (const file of ['HostInspection', 'MonitoringHostPicker', 'notes/NoteEditor', 'EndpointSelector', 'JumpChainEditor', 'ScriptsWorkspace', 'ForwardingWorkspace', 'ProxyWorkspace', 'hosts/ProfileEditorForm', 'hosts/GroupDefaultsModal', 'settings/sections/SshSection', 'AIChat', 'BatchCommandModal']) {
    assert.match(component(file), /<Host(?:ChoiceList|Select)\b/, file);
  }
});

test('inspection uses disjoint saved and live targets without treating session IDs as profile IDs', () => {
  assert.match(component('HostInspection'), /\{ session_id: target\.sessionId \}/);
  assert.match(component('HostInspection'), /\{ profile_id: target\.profileId \}/);
  assert.match(component('OperationsWorkbench'), /id: `session:\$\{pane\.id\}`/);
  assert.match(component('OperationsWorkbench'), /id: `profile:\$\{profile\.id\}`/);
});
