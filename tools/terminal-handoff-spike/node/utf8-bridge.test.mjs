import assert from 'node:assert/strict';
import test from 'node:test';
import headless from '@xterm/headless';
import { Utf8Bridge } from './utf8-bridge.mjs';
import { installNativeUnicode } from './unicode-provider.mjs';

test('persistent stream decoder retains bounded pending prefix and delivers ZWJ without raw decoder', async () => {
  const term = new headless.Terminal({ allowProposedApi: true });
  installNativeUnicode(term);
  const bridge = new Utf8Bridge(term);
  try {
    for (const byte of Buffer.from('👩‍💻X')) await bridge.write(Uint8Array.of(byte));
    assert.equal(bridge.pendingBytes, 0);
    assert.equal(term.buffer.active.getLine(0).getCell(0).getChars(), '👩‍');
    await bridge.write(Uint8Array.of(0xe4));
    assert.equal(bridge.pendingBytes, 1);
    await bridge.write(Uint8Array.of(0xb8));
    assert.equal(bridge.pendingBytes, 2);
    await bridge.write(Uint8Array.of(0xad));
    assert.equal(bridge.pendingBytes, 0);
    assert.equal(bridge.maxPendingBytes <= 3, true);
  } finally { term.dispose(); }
});
