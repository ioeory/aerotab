import test from 'node:test';
import assert from 'node:assert/strict';
import { createPasteRequestGate, sanitizeNativePaste, preparePasteText } from './nativePasteBridge.ts';

const request = { sessionId: 's', generation: 'g', requestId: 1, text: 'one' };
test('matching active request is consumed once; stale/wrong/hidden requests fail closed', () => {
  const gate = createPasteRequestGate();
  const context = { sessionId: 's', generation: 'g', eligible: true };
  assert.equal(gate.accept({ ...request, generation: 'old' }, context), null);
  const token = gate.accept(request, context);
  assert.ok(token);
  assert.equal(gate.accept(request, context), null);
  assert.equal(gate.accept({ ...request, requestId: 0 }, context), null);
  assert.equal(gate.accept({ ...request, requestId: 2 }, { ...context, eligible: false }), null);
  token.finish();
  assert.equal(gate.accept({ ...request, requestId: 2 }, context), null);
  assert.equal(token.valid('g'), true); // modal hide does not invalidate accepted paste
  assert.equal(token.valid('replacement'), false);
  gate.dispose();
  assert.equal(token.valid('g'), false);
  assert.equal(gate.accept({ ...request, requestId: 2 }, context), null);
});
test('rejection and disposal during confirmation never send; approved text uses same callback', async () => {
  for (const scenario of ['reject', 'dispose', 'replace', 'single', 'approve']) {
    const gate = createPasteRequestGate();
    let generation = 'g';
    const text = scenario === 'single' ? 'echo safe' : 'line\n'.repeat(50);
    const token = gate.accept({ ...request, text }, { sessionId: 's', generation, eligible: true });
    const sent = [];
    const prepared = await preparePasteText(text, { trim: false, flatten: false, warn: true,
      confirm: async () => {
        if (scenario === 'dispose') gate.dispose();
        if (scenario === 'replace') generation = 'new';
        return scenario !== 'reject';
      },
    }, async () => token.valid(generation));
    if (prepared !== null) sent.push(prepared);
    assert.deepEqual(sent, ['single', 'approve'].includes(scenario) ? [text.replace(/\n/g, '\r')] : []);
  }
});
test('sanitize prevents escape sequence breakout and NUL; newline conversion happens after confirmation', () => {
  assert.equal(sanitizeNativePaste('a\0\x1b[201~b\u009bc\r\nd\ne\rf'), 'a[201~bc\rd\re\rf');
});
test('native CR input still confirms and trim/flatten use the same policy', async () => {
  let confirmations = 0;
  const policy = { trim: true, flatten: false, warn: true, confirm: async () => { confirmations++; return false; } };
  assert.equal(await preparePasteText('line\r'.repeat(50), policy, async () => true), null);
  assert.equal(confirmations, 1);
  assert.equal(await preparePasteText('a  \r\nb\t\n', { ...policy, flatten: true }, async () => true), 'a b ');
});