import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRawBypassGate } from './trzszRawBypass.ts';

test('TerminalPane native raw path defers bypass release to the gate', () => {
  const src = readFileSync(new URL('../components/TerminalPane.svelte', import.meta.url), 'utf8');
  const start = src.indexOf('function onNativeRaw(');
  assert.ok(start >= 0);
  const body = src.slice(start, src.indexOf('\n  }\n', start));
  const afterProcess = body.slice(body.indexOf('processServerOutput'));
  assert.match(afterProcess, /rawBypassGate\.onServerOutput\(\)/);
  assert.doesNotMatch(afterProcess, /setRawBypass\(false\)/);
});

function fakeClock() {
  let now = 0;
  let nextId = 1;
  const timers = new Map();
  return {
    setTimer(fn, ms) {
      const id = nextId++;
      timers.set(id, { at: now + ms, fn });
      return id;
    },
    clearTimer(id) { timers.delete(id); },
    advance(ms) {
      const end = now + ms;
      for (;;) {
        let due = null;
        for (const [id, t] of timers) if (t.at <= end && (!due || t.at < due[1].at)) due = [id, t];
        if (!due) break;
        timers.delete(due[0]);
        now = due[1].at;
        due[1].fn();
      }
      now = end;
    },
  };
}

// trzsz.js detects the magic key in a setTimeout(…, 10) after processServerOutput
// returns, so isTransferringFiles() is still false synchronously.
test('does not release raw bypass before trzsz async magic detection starts the transfer', () => {
  const clock = fakeClock();
  let transferring = false;
  let releases = 0;
  const gate = createRawBypassGate({
    isTransferring: () => transferring,
    release: () => { releases++; },
    setTimer: clock.setTimer,
    clearTimer: clock.clearTimer,
  });
  gate.onServerOutput();
  assert.equal(releases, 0);
  clock.advance(10);
  transferring = true; // detectAndHandleTrzsz created TrzszTransfer
  clock.advance(5_000); // file picker / drag wait / protocol
  assert.equal(releases, 0);
  transferring = false;
  clock.advance(1_000);
  assert.equal(releases, 1);
  clock.advance(5_000);
  assert.equal(releases, 1);
});

test('releases raw bypass after settle when output was not a real transfer', () => {
  const clock = fakeClock();
  let releases = 0;
  const gate = createRawBypassGate({
    isTransferring: () => false,
    release: () => { releases++; },
    setTimer: clock.setTimer,
    clearTimer: clock.clearTimer,
  });
  gate.onServerOutput();
  clock.advance(100);
  gate.onServerOutput(); // more raw output re-arms the settle window
  clock.advance(100);
  assert.equal(releases, 0);
  clock.advance(1_000);
  assert.equal(releases, 1);
});

test('dispose cancels a pending release', () => {
  const clock = fakeClock();
  let releases = 0;
  const gate = createRawBypassGate({
    isTransferring: () => false,
    release: () => { releases++; },
    setTimer: clock.setTimer,
    clearTimer: clock.clearTimer,
  });
  gate.onServerOutput();
  gate.dispose();
  clock.advance(10_000);
  assert.equal(releases, 0);
});
