import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { defaultMcpConfig } from './mcp.ts';

test('phase six grants are independent and opt in', () => {
  const config = defaultMcpConfig();
  for (const permission of ['readNotes', 'writeNotes', 'readWorkspaces', 'batchOperations']) {
    assert.equal(config.permissions[permission], false);
  }
  config.permissions.readNotes = true;
  assert.equal(config.permissions.writeNotes, false);
});
test('visible approval displays full command and exact targets without automatic execution', () => {
  const component = readFileSync(new URL('../components/McpBatchApproval.svelte', import.meta.url), 'utf8');
  for (const value of ['plan.command', 'plan.scriptDigest', 'plan.targets', 'mcp.batchApprove', 'mcp.batchPending', 'digest:plan.digest']) assert.ok(component.includes(value), value);
  assert.ok(!component.includes('batch_execute'));
  assert.ok(!component.includes('{@html'));
  const app = readFileSync(new URL('../App.svelte', import.meta.url), 'utf8');
  assert.ok(app.includes('<McpBatchApproval {rpc} />'));
});
