import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';

const server = await createServer({
  configFile: false,
  optimizeDeps: { noDiscovery: true, include: [] },
  server: { middlewareMode: true },
});
const { encodePaste, encodeKeyEvent } = await server.ssrLoadModule('/src/lib/termgrid/keyEncoder.ts');
const { MODE_BRACKETED_PASTE } = await server.ssrLoadModule('/src/lib/termgrid/frame.ts');
await server.close();

test('encodePaste collapses CRLF/CR/LF to a single CR so Windows clipboard paste is not double-spaced', () => {
  assert.equal(encodePaste('a\r\nb\nc\rd', 0), 'a\rb\rc\rd');
  assert.equal(
    encodePaste('a\r\nb\n', MODE_BRACKETED_PASTE),
    '\x1b[200~a\rb\r\x1b[201~',
  );
});

test('Shift+PageUp/Down are local scroll chords and do not encode to the PTY', () => {
  const base = { mode: 0, altIsMeta: false };
  assert.equal(encodeKeyEvent({ key: 'PageUp', shiftKey: true, ctrlKey: false, altKey: false, metaKey: false }, base), null);
  assert.equal(encodeKeyEvent({ key: 'PageDown', shiftKey: true, ctrlKey: false, altKey: false, metaKey: false }, base), null);
  assert.equal(encodeKeyEvent({ key: 'PageUp', shiftKey: false, ctrlKey: false, altKey: false, metaKey: false }, base), '\x1b[5~');
});
