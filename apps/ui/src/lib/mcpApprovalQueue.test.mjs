import test from 'node:test';
import assert from 'node:assert/strict';
import { selectApproval } from './mcpApprovalQueue.ts';
test('current review remains pinned under reordered polls and new arrivals', () => {
  const a={id:'a',digest:'a'}, b={id:'b',digest:'b'}, c={id:'c',digest:'c'};
  assert.equal(selectApproval(a,[c,b,a]),a);
  assert.equal(selectApproval(a,[b,a]),a);
  assert.equal(selectApproval(a,[b]),b);
  assert.equal(selectApproval(null,[a,b]),a);
  assert.equal(selectApproval(a,[]),null);
});
