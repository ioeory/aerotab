import assert from 'node:assert/strict';
import { isDeepStrictEqual } from 'node:util';
import { mkdirSync, writeFileSync } from 'node:fs';
import test from 'node:test';
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import headless from '@xterm/headless';
import { fileURLToPath } from 'node:url';
import { snapshot } from './snapshot.mjs';
import { installNativeUnicode } from './unicode-provider.mjs';
import { Utf8Bridge } from './utf8-bridge.mjs';
import { SessionInputRouter } from './input-router.mjs';

// Real xterm 5.5 parser/write queue. No DOM, WebGL, or GPU presentation proof.
const { Terminal } = headless;
class Harness {
  constructor() {
    this.child = spawn(fileURLToPath(new URL('../target/debug/cross-core', import.meta.url)),  [], { stdio: ['pipe', 'pipe', 'inherit'] });
    this.pending = [];
    this.child.on('error', error => { for (const p of this.pending.splice(0)) p.reject(error); });
    createInterface({ input: this.child.stdout }).on('line', line => this.pending.shift()?.resolve(JSON.parse(line)));
    this.term = new Terminal({ cols: 80, rows: 24, scrollback: 20000, allowProposedApi: true });
    this.responses = [];
    this.titles = [];
    this.term.onTitleChange(title => this.titles.push(title));
    this.suppressedResponses = 0;
    this.backpressureRetries = 0;
    this.term.onData(bytes => this.responses.push(bytes));
    this.callbacks = 0;
    // Pin the installed provider explicitly; do not silently follow defaults.
    installNativeUnicode(this.term);
    this.bridge = new Utf8Bridge(this.term);
    this.child.on('exit', (code, signal) => {
      for (const p of this.pending.splice(0)) p.reject(new Error(`runner exited ${code}/${signal}`));
    });
  }
  request(command) {
    return new Promise((resolve, reject) => {
      this.pending.push({ resolve, reject });
      this.child.stdin.write(`${JSON.stringify(command)}\n`);
    });
  }
  async drain() {
    while (!(await this.request({ op: 'next' })).empty) {
      const record = await this.request({ op: 'inflight' });
      if (record.bytes) {
        await this.bridge.write(Uint8Array.from(record.bytes));
        this.callbacks++;
      } else if (record.cols) this.term.resize(record.cols, record.rows);
      else if (record.eof) await this.bridge.eof();
      // Resize is synchronous; EOF is ordered after every prior write callback.
      assert.equal((await this.request({ op: 'ack', seq: record.seq, pendingUtf8: this.bridge.pendingBytes })).ok, true);
      for (const response of this.responses.splice(0)) {
        this.suppressedResponses++;
        assert.equal((await this.request({ op: 'secondary_response', seq: record.seq, bytes: [...Buffer.from(response)] })).rejected, true);
      }
      this.seq = record.seq;
    }
  }
  async append(event, drain = true) {
    let result = await this.request({ op: 'append', ...event });
    if (result.error === 'Backpressure') {
      this.backpressureRetries++;
      await this.drain();
      result = await this.request({ op: 'append', ...event });
    }
    assert.equal(result.error, undefined);
    if (drain) await this.drain();
    return result.seq;
  }
  async text(text, eachByte = false) {
    const bytes = [...Buffer.from(text)];
    if (eachByte) { for (const byte of bytes) await this.append({ bytes: [byte] }); }
    else await this.append({ bytes });
  }
  async compare(label) {
    const xterm = snapshot(this.term);
    const native = await this.request({ op: 'snapshot' });
    // Exact equality remains mandatory; keep failure logs bounded, never project
    // away fields or normalize discrepancies to make a fixture pass.
    if (!isDeepStrictEqual(xterm, native)) {
      const differences = [];
      const visit = (a, b, path) => {
        if (isDeepStrictEqual(a, b) || differences.length >= 12) return;
        if (a && b && typeof a === 'object' && typeof b === 'object') {
          for (const key of new Set([...Object.keys(a), ...Object.keys(b)])) visit(a[key], b[key], `${path}.${key}`);
        } else differences.push({ path, xterm: a, native: b });
      };
      visit(xterm, native, 'snapshot');
      mkdirSync(new URL('../artifacts/', import.meta.url), {recursive:true});
      const name = label.replace(/[^a-zA-Z0-9_-]/g, '_').slice(0, 120);
      writeFileSync(new URL(`../artifacts/diff-${name}.json`, import.meta.url), JSON.stringify({label, differences, xterm, native}, null, 2));
      assert.fail(`${label}: ${JSON.stringify(differences)}`);
    }
  }
  close() { this.term.dispose(); this.child.stdin.end(); }
}

test('real headless callback acknowledges Rust broker only after xterm parsing', async () => {
  const h = new Harness();
  try {
    assert.equal((await h.request({ op: 'append', bytes: [...Buffer.from('中\x1b[31mR')] })).seq, 1);
    assert.equal((await h.request({ op: 'next' })).seq, 1);
    const status = await h.request({ op: 'status' });
    assert.equal(status.retained > 0, true);
    const record = await h.request({ op: 'inflight' });
    await new Promise(resolve => h.term.write(Uint8Array.from(record.bytes), () => { h.callbacks++; resolve(); }));
    assert.equal(h.term.buffer.active.getLine(0).getCell(0).getChars(), '中');
    assert.equal((await h.request({ op: 'ack', seq: 1 })).ok, true);
    assert.equal((await h.request({ op: 'status' })).retained, 0);
    assert.equal(h.callbacks, 1);
    await h.compare('wide text and palette red');
  } finally { h.close(); }
});

test('blocked real xterm parser cannot ACK or commit before actual write callback', async () => {
  const h = new Harness();
  let release;
  try {
    const gate = new Promise(resolve => { release = resolve; });
    let handlerEntered;
    const entered = new Promise(resolve => { handlerEntered = resolve; });
    const handler = h.term.parser.registerCsiHandler({ final: 'z' }, () => { handlerEntered(); return gate; });
    const seq = await h.append({ bytes: [...Buffer.from('\x1b[9zheld')] }, false);
    await h.request({ op: 'begin' });
    assert.equal((await h.request({ op: 'next' })).seq, seq);
    const record = await h.request({ op: 'inflight' });
    let callbackFired = false;
    const applied = new Promise(resolve => h.term.write(Uint8Array.from(record.bytes), () => { callbackFired = true; resolve(); }));
    await entered;
    assert.equal(callbackFired, false);
    assert.equal((await h.request({ op: 'status' })).retained > 0, true);
    assert.equal((await h.request({ op: 'host_frame_model', seq })).ok, false);
    assert.equal((await h.request({ op: 'commit' })).error, 'NotReady');
    release(true);
    await applied;
    assert.equal(callbackFired, true);
    assert.equal((await h.request({ op: 'ack', seq })).ok, true);
    assert.equal((await h.request({ op: 'commit' })).error, 'NotReady');
    assert.equal((await h.request({ op: 'host_frame_model', seq })).ok, true);
    assert.equal((await h.request({ op: 'commit' })).ok, true);
    await h.compare('after actual deferred callback');
    handler.dispose();
  } finally { release?.(true); h.close(); }
});

test('PINNED Unicode mismatch is a real incompatibility, not equivalence', async () => {
  const h = new Harness();
  try {
    const status = await h.request({ op: 'status' });
    assert.deepEqual(status.unicode, [18, 0, 0]);
    // Preserve the original failure as an explicit negative-control test.
    h.term.unicode.activeVersion = '6';
    assert.equal(h.term.unicode.activeVersion, '6');
    // U+1F600 was added in Unicode 6.1. Xterm's installed Unicode 6
    // provider treats it narrow; alacritty's Unicode 18 table treats it wide.
    await h.text('😀X', true);
    const native = await h.request({ op: 'snapshot' });
    const xterm = snapshot(h.term);
    console.log(JSON.stringify({ difference: 'unicode-width', scalar: 'U+1F600', nativeUnicode: status.unicode, xtermUnicode: h.term.unicode.activeVersion, nativeCursor: native.cursor, xtermCursor: xterm.cursor }));
    assert.deepEqual(native.cursor, [3, 0]);
    assert.deepEqual(xterm.cursor, [2, 0]);
    assert.notDeepEqual(native, xterm, 'known real mismatch must remain visible');
  } finally { h.close(); }
});

test('raw upstream Uint8Array decoder loses U+200D at zero-payload continuation boundary: negative control', async () => {
  const h = new Harness();
  try {
    const decoder = h.term._core._inputHandler._utf8Decoder;
    const source = decoder.constructor.toString();
    assert.equal(source.includes('63&this.interim[++'), true, 'installed source stores raw continuation but masks zero payload as loop terminator');
    const raw = [];
    const buf = new Uint32Array(32);
    for (const byte of Buffer.from('👩‍💻X')) {
      const n = decoder.decode(Uint8Array.of(byte), buf);
      raw.push(...buf.subarray(0, n));
    }
    assert.equal(raw.includes(0x200d), false);
    assert.equal(raw.includes(0x1f469), true);
    assert.equal(raw.includes(0x1f4bb), true);
    // e2 80 8d: interim byte 0x80 has payload 0, terminating the reconstruction
    // loop early; state then asks for an extra continuation and discards the scalar.
    assert.equal(new TextDecoder().decode(Buffer.from('👩‍💻X')), '👩‍💻X');
    for (const byte of Buffer.from('👩‍💻X')) await new Promise(resolve => h.term.write(Uint8Array.of(byte), resolve));
    assert.equal(h.term.buffer.active.getLine(0).getCell(0).getChars(), '👩', 'raw route remains a confirmed negative control');
  } finally { h.close(); }
});

for (const eachByte of [false, true]) {
  for (const text of ['中', '😀', '\u{1fae8}', 'e\u0301', '❤\ufe0f', '👩\u200d💻', '🇨🇳']) {
    test(`native Unicode exact cells ${JSON.stringify(text)} eachByte=${eachByte}`, async () => {
      const h = new Harness();
      try {
        await h.text(text + 'X', eachByte);
        await h.compare(`${JSON.stringify(text)} eachByte=${eachByte}`);
      } finally { h.close(); }
    });
  }
}

test('partial UTF8 consumption cannot pass broker barrier until completed', async () => {
  const h = new Harness();
  const packets = [];
  const router = new SessionInputRouter({sessionId:2026, secondary:h.term, send:p=>packets.push(p)});
  try {
    await h.append({ bytes: [0xe4] });
    await h.request({ op: 'begin' });
    const token = router.beginSwitch();
    assert.throws(()=>router.userText('blocked'), /frozen/);
    router.protocol('native', Uint8Array.of(27));
    assert.equal(h.bridge.pendingBytes, 1);
    assert.equal((await h.request({ op: 'status' })).retained, 0);
    assert.equal((await h.request({ op: 'host_frame_model', seq: h.seq })).ok, false);
    assert.equal((await h.request({ op: 'commit' })).error, 'NotReady');
    await h.append({ bytes: [0xb8, 0xad] });
    assert.equal(h.bridge.pendingBytes, 0);
    await h.compare('completed scalar after consumed prefix ACK');
    await h.request({ op: 'host_frame_model', seq: h.seq });
    assert.equal((await h.request({ op: 'commit' })).ok, true);
    router.commitSwitch(token, {parsed:true, presented:true});
    router.userText('resumed');
    assert.deepEqual(packets.map(p=>p.kind), ['protocol','user-text']);
  } finally { h.close(); }
});

for (const input of [[0xff], [0xc0, 0xaf], [0xed, 0xa0, 0x80], [0xf4, 0x90, 0x80, 0x80], [0xe2, 0x28, 0xa1], [0xe4, 0xb8]]) {
  test(`bridge invalid UTF8 and EOF exact native comparison ${input}`, async () => {
    const h = new Harness();
    try {
      for (const byte of input) await h.append({ bytes: [byte] });
      await h.append({});
      await h.compare('invalid UTF8 plus EOF flush');
      assert.equal(h.bridge.pendingBytes, 0);
    } finally { h.close(); }
  });
}

test('all split boundaries production bridge retains UTF8 CSI OSC and malformed continuation', async () => {
  const fixture = Buffer.from('中👩‍💻\x1b[31mR\x1b]0;split-title\x1b\\\x1b[0m');
  for (let split = 0; split <= fixture.length; split++) {
    const h = new Harness();
    try {
      await h.append({ bytes: [...fixture.subarray(0, split)] });
      await h.append({ bytes: [...fixture.subarray(split)] });
      await h.compare(`exact bytes split=${split}`);
    } finally { h.close(); }
  }
});

test('every-byte UTF8 CSI OSC representative vim/tmux fixture and return to normal buffer', async () => {
  const h = new Harness();
  try {
    // Representative escape fixture only, NOT captured or manually tested vim/tmux.
    await h.text('main\r\n中e\u0301\x1b[38;2;11;22;33mR\x1b[48;5;24mB\x1b[0m\x1b]0;fixture-title\x07', true);
    await h.compare('normal UTF8 truecolor indexed background');
    assert.deepEqual(h.titles, ['fixture-title']);
    assert.deepEqual((await h.request({ op: 'status' })).titles, h.titles);
    await h.text('\x1b[?1049h\x1b[?25l\x1b[?1h\x1b[?2004h\x1b[?1004h\x1b[2J\x1b[Hvim-like\x1b[3;9H中\x1b[6n', true);
    await h.compare('alternate vim/tmux-like state');
    await h.append({ cols: 93, rows: 30 });
    await h.compare('alternate resized');
    await h.text('\x1b[?1049l\x1b[?25h\x1b[?1l\x1b[?2004l\x1b[?1004l', true);
    await h.compare('saved normal buffer after resize');
    await h.text('\x1b[H\x1b[1;2;3;4;7;8;9mstyled\x1b[0m\x1b[?6h\x1b[?7l\x1b[4h\x1b=', true);
    await h.compare('cell attributes origin nowrap insert and application keypad modes');
    assert.equal(h.callbacks > 100, true);
  } finally { h.close(); }
});

test('tail barrier uses real callback ACK; only native DSR reaches mock PTY across switch', async () => {
  const h = new Harness();
  try {
    await h.text('start\x1b[6n');
    await h.request({ op: 'begin' });
    const seq = await h.append({ bytes: [...Buffer.from('\x1b[3;4H\x1b[6n')] }, false);
    assert.equal((await h.request({ op: 'commit' })).error, 'NotReady');
    assert.equal((await h.request({ op: 'host_frame_model', seq })).ok, false);
    await h.drain();
    assert.equal((await h.request({ op: 'commit' })).error, 'NotReady', 'parser callback is not GPU first frame');
    await h.request({ op: 'host_frame_model', seq });
    assert.equal((await h.request({ op: 'commit' })).ok, true);
    // Keep a query's CSI parser state alive across the reverse visibility switch.
    await h.text('\x1b[6');
    await h.request({ op: 'begin', native: true });
    await h.request({ op: 'host_frame_model', seq: h.seq });
    assert.equal((await h.request({ op: 'commit' })).ok, true);
    await h.text('n');
    const status = await h.request({ op: 'status' });
    assert.deepEqual(status.pty, ['\x1b[1;6R', '\x1b[3;4R', '\x1b[3;4R']);
    assert.equal(status.session, 2026);
    assert.equal(status.active, 'Native');
    assert.equal(h.suppressedResponses, 3, 'actual xterm onData replies all rejected');
    await h.compare('DSR across switch');
  } finally { h.close(); }
});

function seeded(seed) {
  let state = seed >>> 0;
  return () => { state ^= state << 13; state ^= state >>> 17; state ^= state << 5; return state >>> 0; };
}

test('minimal C1 counterexamples remain strict pending native-policy adapter', async () => {
  for (const input of [[0x84], [0x9b, 0x32, 0x4a], [0x9d, 0x41], [0xc2, 0x84]]) {
    const h = new Harness();
    try {
      await h.append({bytes:input});
      await h.compare(`C1 policy hex=${Buffer.from(input).toString('hex')}`);
    } finally { h.close(); }
  }
});

for (const seed of [0x51a7, 0xc0ffee, 0x200d]) {
  test(`bounded arbitrary-byte differential seed=${seed}`, async () => {
    const random = seeded(seed);
    const h = new Harness();
    const trace = [];
    try {
      for (let step = 0; step < 80; step++) {
        const bytes = Array.from({ length: 1 + random() % 9 }, () => random() & 255);
        trace.push(bytes);
        await h.append({ bytes });
        try { await h.compare(`seed=${seed} step=${step} hex=${Buffer.from(bytes).toString('hex')}`); }
        catch (error) { error.message += ` trace=${JSON.stringify(trace)}`; throw error; }
        assert.equal(h.bridge.pendingBytes <= 3, true);
      }
      await h.append({});
      await h.compare(`seed=${seed} EOF`);
    } finally { h.close(); }
  });
}

for (const seed of [0xabc, 0x1234]) {
  test(`bounded valid scalars CSI OSC resize switch differential seed=${seed}`, async () => {
    const random = seeded(seed);
    const h = new Harness();
    const events = [];
    const tokens = ['中', '😀', '👩‍💻', 'e\u0301', '❤\ufe0f', '\u{1fae8}', 'plain', '\r\n', '\x1b[31m', '\x1b[0m', '\x1b[2;3H', '\x1b]0;fuzz\x07', '\x1b[?25l', '\x1b[?25h'];
    try {
      for (let step = 0; step < 60; step++) {
        const bytes = Buffer.from(tokens[random() % tokens.length]);
        for (let start = 0; start < bytes.length;) {
          const end = Math.min(bytes.length, start + 1 + random() % 4);
          const event = {bytes:[...bytes.subarray(start,end)]};
          events.push(event);
          await h.append(event);
          start = end;
          assert.equal(h.bridge.pendingBytes <= 3, true);
        }
        if (step % 15 === 0) {
          const resize = {cols:80+random()%8,rows:24+random()%3};
          events.push(resize);
          await h.append(resize);
          await h.request({ op: 'begin', native: step % 30 === 0 });
          assert.equal((await h.request({ op: 'host_frame_model', seq: h.seq })).ok, true);
          assert.equal((await h.request({ op: 'commit' })).ok, true);
        }
        try { await h.compare(`mixed seed=${seed} step=${step}`); }
        catch (error) {
          writeFileSync(new URL(`../artifacts/mixed-seed-${seed}-events.json`,import.meta.url),JSON.stringify(events,null,2));
          error.message += ` events=${JSON.stringify(events)}`;
          throw error;
        }
      }
      await h.append({});
      await h.compare(`mixed seed=${seed} EOF`);
    } finally { h.close(); }
  });
}

for (const kind of ['vim', 'tmux']) {
  test(`real isolated ${kind} PTY input/output survives switch and subsequent input`, { timeout: 30000 }, async () => {
    const h = new Harness();
    const child = spawn('python3', ['-u', fileURLToPath(new URL('./pty-session.py', import.meta.url)), kind], { stdio: ['pipe', 'pipe', 'inherit'] });
    const pending = [];
    createInterface({ input: child.stdout }).on('line', line => pending.shift()?.resolve(JSON.parse(line)));
    child.on('exit', code => { for (const p of pending.splice(0)) p.reject(new Error(`PTY helper exited ${code}`)); });
    const command = value => new Promise((resolve, reject) => { pending.push({ resolve, reject }); child.stdin.write(JSON.stringify(value) + '\n'); });
    const trace = [];
    const feed = async (input, quit = false) => {
      const response = await command({ input: input === null ? null : Buffer.from(input).toString('base64'), quit });
      trace.push({ input, outputBase64: response.chunks, outputBytes: response.chunks.reduce((sum, chunk) => sum + Buffer.from(chunk, 'base64').length, 0) });
      for (const chunk of response.chunks) {
        const bytes = Buffer.from(chunk, 'base64');
        for (let offset = 0; offset < bytes.length; offset += 1024) await h.append({ bytes: [...bytes.subarray(offset, offset + 1024)] });
      }
    };
    try {
      await feed(null);
      await h.compare(`${kind} startup PTY`);
      await feed(kind === 'vim' ? 'iBEFORE中😀\x1b' : "printf 'BEFORE中😀\\n'\n");
      await h.compare(`${kind} before switch PTY`);
      await h.request({ op: 'begin' });
      // Input goes to the REAL running process while the visibility switch is pending.
      await feed(kind === 'vim' ? 'aAFTER\x1b' : "printf 'AFTER\\n'\n");
      assert.equal((await h.request({ op: 'commit' })).error, 'NotReady');
      assert.equal((await h.request({ op: 'host_frame_model', seq: h.seq })).ok, true);
      assert.equal((await h.request({ op: 'commit' })).ok, true);
      await h.compare(`${kind} after switch PTY`);
      assert.equal(snapshot(h.term).lines.some(line => line.includes('AFTER')), true);
      await feed(kind === 'vim' ? ':q!\r' : 'exit\n', true);
      await h.append({});
      await h.compare(`${kind} exit PTY`);
      assert.equal((await h.request({ op: 'status' })).session, 2026);
      const artifacts = new URL('../artifacts/', import.meta.url);
      mkdirSync(artifacts, { recursive: true });
      writeFileSync(new URL(`${kind}-pty-trace.json`, artifacts), JSON.stringify({ kind, trace, visualSmoke: false, gpuFirstFrame: false }, null, 2) + '\n');
      console.log(JSON.stringify({ realPty: kind, trace: trace.map(({ outputBase64, ...entry }) => entry), visualSmoke: false, gpuFirstFrame: false }));
    } finally {
      child.stdin.end();
      h.close();
    }
  });
}

test('real cross-core history exceeds 256KiB with bounded backlog resize and EOF', async () => {
  const h = new Harness();
  try {
    let bytes = 0;
    for (let n = 0; n < 4000; n++) {
      const chunk = [...Buffer.from(`line-${String(n).padStart(5, '0')}: ${'0123456789'.repeat(8)}\r\n`)];
      bytes += chunk.length;
      await h.append({ bytes: chunk }, false);
      if (n === 1500) { await h.drain(); await h.compare('history before resize'); await h.append({ cols: 93, rows: 30 }); }
    }
    await h.append({});
    await h.compare('complete history after EOF');
    assert.equal(bytes > 256 * 1024, true);
    assert.equal(h.backpressureRetries > 0, true);
    assert.equal(h.callbacks, 4000, 'every actual xterm write callback completed');
    assert.equal((await h.request({ op: 'status' })).retained, 0);
    console.log(JSON.stringify({ bytes, callbacks: h.callbacks, backpressureRetries: h.backpressureRetries, history: h.term.buffer.active.baseY, logBudget: 4096, realXterm: '5.5.0' }));
  } finally { h.close(); }
});
