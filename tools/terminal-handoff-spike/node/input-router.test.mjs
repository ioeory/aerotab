import test from 'node:test';
import assert from 'node:assert/strict';
import headless from '@xterm/headless';
import { SessionInputRouter } from './input-router.mjs';

test('fixed input authority freezes user input during switch, never UTF8 completion ingress', async () => {
  const writes = [];
  const term = new headless.Terminal({allowProposedApi:true});
  const router = new SessionInputRouter({sessionId:42, secondary:term, send: packet => writes.push(packet)});
  try {
    const data = [];
    term.onData(x => data.push(x));
    term.input('SHOULD_NOT_SEND', true);
    await new Promise(resolve => term.write('\x1b[6n', resolve));
    assert.deepEqual(data, [], 'real disableStdin suppresses both input and automatic responses');
    router.userText('hello');
    const token = router.beginSwitch();
    assert.throws(() => router.userText('blocked'), /frozen/);
    assert.throws(() => router.commitSwitch(token, {parsed:false, presented:false}), /not ready/);
    router.cancelSwitch(token);
    router.userBinary('\xff');
    router.protocol('native', new Uint8Array([27,91,48,110]));
    assert.throws(() => router.protocol('xterm', new Uint8Array([1])), /authority/);
    assert.deepEqual(writes.map(p=>p.kind), ['user-text','user-binary','protocol']);
    assert.equal(writes.every(p=>p.sessionId===42), true);
  } finally { term.dispose(); }
});
