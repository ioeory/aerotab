import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';

const load = async (name) => {
  try {
    const src = readFileSync(new URL(`./${name}.ts`, import.meta.url), 'utf8');
    const js = ts.transpileModule(src, { compilerOptions: { target: 99, module: 99 } }).outputText;
    return import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);
  } catch (error) {
    if (error.code !== 'ENOENT') throw error;
    return {};
  }
};
const core = await load('aiTroubleshoot');
const templates = await load('aiTroubleshootTemplates');

test('local resume stores only the confirmed question, template, shell and editable command', () => {
  const storage = new Map();
  const local = { getItem: key => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value), removeItem: key => storage.delete(key) };
  const draft = core.buildTroubleshootResume({ question: 'Check the listener', template: 'Ports', shell: 'bash', command: 'ss -ltn', commandShell: 'sh',
    origin: { id: 'private-pane' }, output: 'private-output', reply: 'private-reply', endpoint: 'private-endpoint' });
  assert.equal(storage.size, 0, 'preview does not persist');
  assert.ok(core.saveTroubleshootResume(local, draft));
  const expected = { question: 'Check the listener', template: 'Ports', shell: 'bash', command: 'ss -ltn', commandShell: 'sh' };
  assert.deepEqual(core.loadTroubleshootResume(local), expected);
  assert.deepEqual([...storage.values()].map(JSON.parse), [{ version: 1, ...expected }]);
  assert.ok(core.removeTroubleshootResume(local));
  assert.equal(core.loadTroubleshootResume(local), null);
});

test('resume masks recognized secrets, bounds text, and fails safely on invalid or blocked storage', () => {
  const draft = core.buildTroubleshootResume({ question: `password=hidden\n${'日'.repeat(20_000)}`, shell: 'invalid', command: 'curl --token hidden', commandShell: 'sh' });
  assert.doesNotMatch(JSON.stringify(draft), /hidden|invalid/);
  assert.ok(Buffer.byteLength(draft.question) <= core.TROUBLESHOOT_MAX_BYTES);
  assert.equal(draft.command, 'curl --token [redacted]');
  assert.equal(core.buildTroubleshootResume({ command: 'with\na newline', commandShell: 'sh' }), null);
  assert.equal(core.loadTroubleshootResume({ getItem: () => '{"version":2,"question":"ignored"}' }), null);
  assert.equal(core.loadTroubleshootResume({ getItem: () => '{invalid' }), null);
  const blocked = () => { throw Error('blocked'); };
  assert.equal(core.loadTroubleshootResume({ getItem: blocked }), null);
  assert.equal(core.saveTroubleshootResume({ setItem: blocked }, draft), false);
  assert.equal(core.removeTroubleshootResume({ removeItem: blocked }), false);
});
const dest = { providerId: 'p', model: 'm', endpoint: 'https://test.invalid/v1' };
const review = () => core.buildTroubleshootReview('terminal_explain', 'sample', dest, { id: 's1', label: 'Terminal A' });
function deferred() {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
}
const settle = () => new Promise(resolve => setImmediate(resolve));

test('comparison requires two sides, bounds each independently, and sends no source metadata or tool fields', () => {
  const a = { text: `password=secret\n${'日'.repeat(20_000)}`, id: 'private-pane', label: 'private-host' };
  assert.equal(core.buildTroubleshootComparison(a, { text: ' ' }, dest), null);
  const comparison = core.buildTroubleshootComparison(a, { text: 'unique evidence B' }, dest);
  const request = core.troubleshootRequest(comparison, 'comparison');
  assert.deepEqual(Object.keys(request).sort(), ['conversationId','expectedEndpoint','message','model','providerId','purpose']);
  assert.equal(request.purpose, 'terminal_explain');
  assert.doesNotMatch(request.message, /secret|private-pane|private-host|\uFFFD/);
  assert.match(request.message, /unique evidence B/);
  assert.ok(Buffer.byteLength(request.message) <= core.TROUBLESHOOT_MAX_BYTES);
});

test('preflight strips terminal escapes and common credentials, with a UTF-8 byte cap', () => {
  assert.equal(typeof core.sanitizeTroubleshootText, 'function');
  const raw = '\x1b[31mfailed\x1b[0m\x1b]8;;https://hidden.invalid\x07\x1bPqhidden-sixel\x1b\\\r\n'
    + 'Authorization: Bearer bearer-leak\npassword="word leak"\n--token cli-leak\nhttps://user:secret@server.invalid/x\n'
    + '-----BEGIN OPENSSH PRIVATE KEY-----\nprivate-leak\n-----END OPENSSH PRIVATE KEY-----\n'
    + `noise ${'日'.repeat(20_000)}`;
  const preview = core.sanitizeTroubleshootText(raw);
  assert.match(preview, /^failed\n/);
  assert.doesNotMatch(preview, /hidden|bearer-leak|word leak|cli-leak|user:secret|private-leak|\uFFFD/);
  assert.ok(Buffer.byteLength(preview) <= core.TROUBLESHOOT_MAX_BYTES);
  assert.equal(core.sanitizeTroubleshootText(preview), preview);
});

test('review holds immutable local origin; the wire request contains only the confirmed text fields', () => {
  assert.equal(typeof core.buildTroubleshootReview, 'function');
  const origin = { id: 'session-secret', label: 'Production' };
  const draft = core.buildTroubleshootReview('command_draft', ' token=secret ', dest, origin, 'bash');
  origin.id = 'other';
  assert.equal(draft.origin.id, 'session-secret');
  assert.ok(Object.isFrozen(draft.origin));
  assert.deepEqual(core.troubleshootRequest(draft, 'isolated'), {
    conversationId: 'isolated', providerId: 'p', model: 'm', expectedEndpoint: dest.endpoint,
    purpose: 'command_draft', shell: 'bash', message: 'token=[redacted]',
  });
  const endpoint = 'https://EXAMPLE.invalid:443/v1//';
  const exact = core.buildTroubleshootReview('terminal_explain', 'text', { ...dest, endpoint });
  assert.equal(core.troubleshootRequest(exact, 's').expectedEndpoint, endpoint);
  const privateEndpoint = 'https://user:secret@example.invalid/v1?token=key#fragment';
  const masked = core.buildTroubleshootReview('terminal_explain', 'text', { ...dest, endpoint: privateEndpoint });
  assert.equal(masked.endpoint, 'https://example.invalid/v1');
  assert.equal(core.troubleshootRequest(masked, 's').expectedEndpoint, privateEndpoint);
  assert.equal(core.buildTroubleshootReview('command_draft', 'text', dest), null);
  assert.equal(core.buildTroubleshootReview('terminal_explain', ' ', dest), null);
  assert.equal(core.buildTroubleshootReview('terminal_explain', 'text', { ...dest, endpoint: '' }), null);
  assert.equal(core.buildTroubleshootReview('terminal_explain', 'text', { ...dest, model: '界'.repeat(86) }), null);
  assert.equal(core.buildTroubleshootReview('terminal_explain', 'text', { ...dest, providerId: '界'.repeat(22) }), null);
  assert.equal(core.buildTroubleshootReview('terminal_explain', 'a\u2028b\u2029c', dest).message, 'a\nb\nc');
});

test('draft parser only accepts complete JSON in the requested shell or a clarification', () => {
  assert.equal(typeof core.parseTroubleshootDraft, 'function');
  const response = { kind: 'draft', command: 'df -h', shell: 'bash', expected: 'Filesystems', sideEffects: 'Read only' };
  assert.deepEqual(core.parseTroubleshootDraft(JSON.stringify(response), 'bash'), response);
  assert.deepEqual(core.parseTroubleshootDraft('{"kind":"clarify","question":"Which path?"}', 'bash'), { kind: 'clarify', question: 'Which path?' });
  assert.equal(core.parseTroubleshootDraft(JSON.stringify({ ...response, shell: 'sh' }), 'bash'), null);
  assert.equal(core.parseTroubleshootDraft(JSON.stringify({ ...response, sideEffects: [] }), 'bash'), null);
  assert.equal(core.parseTroubleshootDraft(JSON.stringify({ ...response, sideEffects: '界'.repeat(1366) }), 'bash'), null);
  assert.equal(core.parseTroubleshootDraft(JSON.stringify({ kind: 'clarify', question: 'A\x1bB' }), 'bash'), null);
  assert.equal(core.parseTroubleshootDraft(JSON.stringify({ ...response, command: 'pwd\u001b' }), 'bash'), null);
  assert.equal(core.parseTroubleshootDraft('```json\n' + JSON.stringify(response) + '\n```', 'bash'), null);
});

test('pending send is locked and its late ID is stopped, never polled', async () => {
  assert.equal(typeof core.createTroubleshootRun, 'function');
  const pending = deferred();
  const calls = [];
  let view;
  const rpc = { call(method, args) {
    calls.push([method, args]);
    return method === 'ai.send' ? pending.promise : Promise.resolve(null);
  } };
  const run = core.createTroubleshootRun(snapshot => { view = snapshot; }, () => 'unique');
  assert.equal(run.start(rpc, review()), true);
  assert.equal(run.start(rpc, review()), false);
  run.stop();
  assert.equal(view.status, 'stopping');
  pending.resolve({ id: 'late' });
  await settle();
  assert.deepEqual(calls.map(([method]) => method), ['ai.send', 'ai.stop']);
  assert.equal(calls.at(-1)[1].id, 'late');
  assert.equal(view.status, 'canceled');
});

test('late poll does not render after stop and next run receives a new conversation ID', async () => {
  assert.equal(typeof core.createTroubleshootRun, 'function');
  const poll = deferred();
  const calls = [];
  let view;
  const rpc = { call(method, args) {
    calls.push([method, args]);
    if (method === 'ai.send') return Promise.resolve({ id: args.conversationId });
    if (method === 'ai.poll') return args.id === 'run1' ? poll.promise : Promise.resolve({ events: [{ type: 'delta', text: 'fresh' }, { type: 'done' }], cursor: 2, done: true });
    return Promise.resolve(null);
  } };
  let count = 0;
  const run = core.createTroubleshootRun(snapshot => { view = snapshot; }, () => `run${++count}`);
  run.start(rpc, review());
  await settle();
  run.stop();
  await settle();
  run.start(rpc, review());
  await settle();
  poll.resolve({ events: [{ type: 'delta', text: 'stale' }], cursor: 1, done: true });
  await settle();
  assert.equal(view.status, 'done');
  assert.equal(view.text, 'fresh');
  assert.deepEqual(calls.filter(([name]) => name === 'ai.send').map(([, args]) => args.conversationId), ['run1', 'run2']);
});

test('unexpected tool events fail closed; a failed stop remains locked and can be retried', async () => {
  assert.equal(typeof core.createTroubleshootRun, 'function');
  const calls = [];
  const acks = [Promise.reject(new Error('stop failed')), Promise.resolve()];
  let view;
  const rpc = { call(method) {
    calls.push(method);
    if (method === 'ai.send') return Promise.resolve({ id: 'g' });
    if (method === 'ai.poll') return Promise.resolve({ events: [{ type: 'approval' }], cursor: 1, done: false });
    return acks.shift();
  } };
  const run = core.createTroubleshootRun(snapshot => { view = snapshot; }, () => 'unique');
  run.start(rpc, review());
  await settle();
  assert.equal(view.status, 'stopFailed');
  assert.equal(view.text, '');
  assert.equal(run.start(rpc, review()), false);
  run.stop();
  await settle();
  assert.equal(view.status, 'failed');
  assert.deepEqual(calls, ['ai.send', 'ai.poll', 'ai.stop', 'ai.stop']);
});

test('dispose stops a pending generation without notifying the unmounted view', async () => {
  assert.equal(typeof core.createTroubleshootRun, 'function');
  const pending = deferred();
  const calls = [];
  const views = [];
  const rpc = { call(method, args) {
    calls.push([method, args]);
    return method === 'ai.send' ? pending.promise : Promise.resolve(null);
  } };
  const run = core.createTroubleshootRun(view => views.push(view), () => 'unique');
  run.start(rpc, review());
  run.dispose();
  const viewCount = views.length;
  pending.resolve({ id: 'unmounted' });
  await settle();
  assert.deepEqual(calls.map(([method]) => method), ['ai.send', 'ai.stop']);
  assert.equal(calls.at(-1)[1].id, 'unmounted');
  assert.equal(views.length, viewCount);
  assert.equal(run.start(rpc, review()), false);
});

test('templates have three built-ins; custom create/read/update/delete is versioned and local', () => {
  assert.equal(typeof templates.loadTroubleshootTemplates, 'function');
  const entries = new Map();
  const storage = { getItem: key => entries.get(key) ?? null, setItem: (key, value) => entries.set(key, value) };
  assert.deepEqual(templates.troubleshootBuiltinIds, ['ssh-failure', 'port-in-use', 'disk-full']);
  assert.deepEqual(templates.loadTroubleshootTemplates(storage), []);
  const legacy = { id: 'legacy', name: 'Legacy title', prompt: 'Old prompt' };
  const tracked = { id: 'custom-1', name: 'Test', description: 'Check a service', steps: '1. Identify the service\n2. Check status', prompt: 'Check status' };
  const custom = { ...tracked, description: ` ${tracked.description} `, steps: ` ${tracked.steps}\n`,
    result: 'do not keep', context: { id: 'secret' }, terminalOutput: 'private output', origin: { id: 'pane-id' } };
  assert.ok(templates.saveTroubleshootTemplates(storage, [legacy, custom]));
  assert.deepEqual(JSON.parse(entries.get(templates.TROUBLESHOOT_TEMPLATE_KEY)), {
    version: 1, templates: [legacy, tracked],
  });
  assert.deepEqual(templates.loadTroubleshootTemplates(storage), [legacy, tracked]);
  const updated = { ...tracked, description: ' ', steps: '', prompt: 'Check latency' };
  assert.ok(templates.saveTroubleshootTemplates(storage, [updated]));
  assert.deepEqual(templates.loadTroubleshootTemplates(storage), [{ id: tracked.id, name: tracked.name, prompt: updated.prompt }]);
  assert.equal(templates.saveTroubleshootTemplates(storage, [{ ...custom, description: 123 }]), false);
  assert.equal(templates.saveTroubleshootTemplates(storage, [{ ...custom, steps: ['not text'] }]), false);
  assert.equal(templates.saveTroubleshootTemplates(storage, [{ ...custom, steps: 'x'.repeat(20_000) }]), false);
  assert.ok(templates.saveTroubleshootTemplates(storage, []));
  assert.deepEqual(templates.loadTroubleshootTemplates(storage), []);
  entries.set(templates.TROUBLESHOOT_TEMPLATE_KEY, '{"version":2,"templates":[]}');
  assert.deepEqual(templates.loadTroubleshootTemplates(storage), []);
  assert.equal(templates.saveTroubleshootTemplates(storage, [custom]), false);
});

const templateSource = readFileSync(new URL('../components/ai/AiTroubleshootTemplates.svelte', import.meta.url), 'utf8');
const templateScript = templateSource.split('<script lang="ts">')[1].split('</script>')[0]
  .replace(/^\s*import\s[^;]+;/gm, '').replace(/\bexport\s+(?=function)/g, '');
const templateEditorScript = ts.transpileModule(templateScript + `
  globalThis.api = {
    duplicate, openBlankEditor, save, edit,
    view: () => ({ editing, name, description, steps, prompt }),
    enter: fields => ({ name, description, steps, prompt } = fields),
  };
`, { compilerOptions: { target: 99, module: 99 } }).outputText;

test('template duplicate copies only the selected fields; opening from analysis clears the editor', async () => {
  let persisted = null;
  const storage = { getItem: () => persisted, setItem: (_key, value) => { persisted = value; } };
  const rpc = await load('rpc');
  const applied = [];
  const picked = { id: 'picked', name: 'Picked', description: 'A reusable description', steps: 'Pick a path', prompt: 'Check permissions' };
  const unpicked = { id: 'other', name: 'Other', prompt: 'Other prompt' };
  assert.ok(templates.saveTroubleshootTemplates(storage, [picked, unpicked]));
  const sandbox = {
    ...templates, $state: value => value, $props: () => ({ t: key => key, onApply: prompt => applied.push(prompt) }),
    onMount: mount => mount(), tick: () => Promise.resolve(), window: { localStorage: storage },
    uuidv4: () => vm.runInNewContext(`${rpc.uuidv4.toString()}; uuidv4()`, {
      crypto: { getRandomValues: bytes => bytes.fill(17) },
    }),
  };
  vm.runInNewContext(templateEditorScript, sandbox);
  const { duplicate, openBlankEditor, save, edit, view, enter } = sandbox.api;
  edit(unpicked);
  enter({ name: 'unsaved', description: 'unsaved', steps: 'unsaved', prompt: 'unsaved' });
  duplicate(picked);
  assert.deepEqual({ ...view() }, { editing: '', name: picked.name, description: picked.description, steps: picked.steps, prompt: picked.prompt });
  assert.deepEqual(templates.loadTroubleshootTemplates(storage), [picked, unpicked]);
  assert.deepEqual(applied, []);
  save();
  const saved = templates.loadTroubleshootTemplates(storage);
  assert.deepEqual(saved.slice(0, 2), [picked, unpicked]);
  assert.deepEqual(saved[2], { ...picked, id: '11111111-1111-4111-9111-111111111111' });
  duplicate(picked);
  openBlankEditor();
  assert.deepEqual({ ...view() }, { editing: '', name: '', description: '', steps: '', prompt: '' });
  assert.deepEqual(templates.loadTroubleshootTemplates(storage), saved);
  assert.deepEqual(applied, []);
});

test('analysis save opens the blank editor without passing context and template apply is separate', () => {
  const analysisSource = readFileSync(new URL('../components/ai/AiTroubleshoot.svelte', import.meta.url), 'utf8');
  assert.match(analysisSource, /onclick=\{\(\) => templateEditor\?\.openBlankEditor\(\)\}/);
  assert.match(analysisSource, /onApply=\{applyTemplate\}/);
  assert.match(templateSource, /onclick=\{\(\) => duplicate\(entry\)\}/);
  assert.match(templateSource, /onclick=\{\(\) => onApply\(entry\.prompt, entry\.name\)\}/);
});

const chatSource = readFileSync(new URL('../components/AIChat.svelte', import.meta.url), 'utf8');
const chatScript = chatSource.split('<script lang="ts">')[1].split('</script>')[0]
  .replace(/^\s*import .*;$/gm, '');
const chatTestScript = ts.transpileModule(chatScript + `
  globalThis.api = {
    send, stop, openTroubleshoot,
    setSeed: seed => troubleshootSeed = seed,
    setDraft: draft => message = draft,
    view: () => ({ troubleshootOpen, acceptedSeed, messages, running, sending }),
  };
`, { compilerOptions: { target: 99, module: 99 } }).outputText;

for (const finish of ['stop', 'complete']) test(`late seed waits for the legacy chat to ${finish}`, async () => {
  const sending = deferred();
  const polling = deferred();
  const stopping = deferred();
  const calls = [];
  const effects = [];
  const rpc = { call(method, params) {
    calls.push([method, params]);
    return ({ 'ai.send': sending, 'ai.poll': polling, 'ai.stop': stopping })[method].promise;
  } };
  const sandbox = {
    window: {}, $state: value => value, $props: () => ({ rpc }), $effect: effect => effects.push(effect),
    onMount() {}, onDestroy() {}, uuidv4: () => 'legacy', i18n: { t: key => key },
    pollAi: (rpc, id, cursor) => rpc.call('ai.poll', { id, cursor }),
    appendAiEvent(message, event) { if (event.type === 'delta') message.content += event.text; },
    clearTimeout, setTimeout,
  };
  vm.runInNewContext(chatTestScript, sandbox);
  const { send, stop, openTroubleshoot, setSeed, setDraft, view } = sandbox.api;
  const flushEffects = () => effects.forEach(effect => effect());
  setDraft('legacy question');
  const pending = send();
  const seed = { id: 'pane', nonce: 1, text: 'new terminal text' };
  setSeed(seed);
  flushEffects();
  assert.equal(view().sending, true);
  assert.equal(openTroubleshoot(), false);
  assert.equal(view().troubleshootOpen, false);
  assert.equal(view().acceptedSeed, undefined);

  sending.resolve({ id: 'legacy-generation' });
  await pending;
  flushEffects();
  assert.equal(view().sending, false);
  assert.equal(view().running, true);
  assert.equal(view().troubleshootOpen, false);
  assert.equal(view().acceptedSeed, undefined);

  if (finish === 'stop') {
    const pendingStop = stop();
    flushEffects();
    assert.equal(view().troubleshootOpen, false);
    assert.equal(calls.at(-1)[1].id, 'legacy-generation');
    stopping.resolve();
    await pendingStop;
  }
  polling.resolve({ events: [{ type: 'delta', text: 'legacy answer' }], cursor: 1, done: true });
  await settle();
  flushEffects();
  assert.equal(view().running, false);
  assert.equal(view().troubleshootOpen, true);
  assert.equal(view().acceptedSeed, seed);
  assert.equal(view().messages[0].content, 'legacy question');
  if (finish === 'complete') assert.equal(view().messages[1].content, 'legacy answer');
  assert.equal(calls.filter(([method]) => method === 'ai.send').length, 1);
});

test('chat shows the queued-seed notice and retains the stop control while legacy work is active', () => {
  assert.match(chatSource, /seed=\{acceptedSeed\}/);
  assert.match(chatSource, /disabled=\{!troubleshootOpen&&\(running\|\|sending\)\}/);
  assert.match(chatSource, /role="status">\{pendingSeedMessage\(\)\}/);
  assert.match(chatSource, /\{#if running\|\|sending\}<button class="send stop"/);
});

test('isolated component uses editable preflight and no execution, tool, attach or HTML rendering path', () => {
  const path = new URL('../components/ai/AiTroubleshoot.svelte', import.meta.url);
  let source = '';
  try { source = readFileSync(path, 'utf8'); } catch (error) { if (error.code !== 'ENOENT') throw error; }
  assert.match(source, /buildTroubleshootReview/);
  assert.match(source, /bind:value=\{input\}/);
  assert.match(source, /displayed = busy \? view\.review : review/);
  assert.match(source, /<pre>\{displayed\.message\}/);
  assert.match(source, /selectionStart/);
  assert.match(source, /clipboard\.writeText/);
  assert.doesNotMatch(source, /terminal\.|ai\.approve|ai\.mcp|\{@html/);
});
