import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';

async function directoryApi() {
  const source = readFileSync(new URL('./aiPaneDirectory.ts', import.meta.url), 'utf8');
  const { outputText } = ts.transpileModule(source, { compilerOptions: { target: 99, module: 99 } });
  return import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);
}

const pane = (id, title = 'shell', connection) => ({ id, title, connection });
const tab = (id, panes, activePaneId, kind = 'terminal') => ({ id, kind, title: 'Lab', panes, activePaneId });
const directory = () => [
  tab('a', [pane('a1'),pane('connecting', 'shell', { status: 'connecting' }), pane('a3')], 'a3'),
  tab('transfer', [pane('transfer-id')], 'transfer-id', 'transfer'),
  tab('b', [pane('b1')], 'b1'),
];

test('pane directory excludes non-session panes and identifies only the visible focused pane', async () => {
  const { buildAiPaneDirectory } = await directoryApi();
  const entries = buildAiPaneDirectory(directory(), 'a');
  assert.deepEqual(entries.map(({id, active}) => [id,active]), [['a1', false], ['a3', true], ['b1', false]]);
  assert.equal(buildAiPaneDirectory(directory(), 'transfer').some(entry => entry.active), false);
  assert.equal(buildAiPaneDirectory(directory(), null).some(entry => entry.active), false);
});

test('pane display labels distinguish duplicate tabs and splits without changing the session ID', async () => {
  const { buildAiPaneDirectory, aiPaneLabel } = await directoryApi();
  const entries = buildAiPaneDirectory(directory(), 'a');
  assert.deepEqual(entries.map(entry => aiPaneLabel(entry, entries)), [
    '1. Lab / #1 shell', '1. Lab / #3 shell', '3. Lab / #1 shell',
  ]);
  assert.equal(aiPaneLabel({ id: 'remote-old', title: 'Previous session' }, entries), 'Previous session');
  assert.equal(aiPaneLabel({ id: 'unnamed', title: '' }, entries), 'Untitled terminal');
  assert.equal(aiPaneLabel({ id: 'unnamed', title: 'unnamed' }, entries, 'zh-CN'), '未命名终端');
});

const leaf = id => ({ type: 'leaf', pane: { id } });
const split = (direction, ...children) => ({ type: 'split', direction, children });

for (const [axis, positions] of [['row', ['Left', 'Right']], ['col', ['Top', 'Bottom']]]) {
  test(`a simple ${axis} split uses visual order even when panes are listed out of order`, async () => {
    const { buildAiPaneDirectory, aiPaneLabel } = await directoryApi();
    const dir = buildAiPaneDirectory([{
      ...tab('a', [pane('a2'), pane('a1')], 'a2'), layout: split(axis, leaf('a1'), leaf('a2')),
    }]);
    assert.deepEqual(['a1', 'a2'].map((id, index) => aiPaneLabel(pane(id), dir).includes(positions[index])), [true, true]);
    assert.match(aiPaneLabel(pane('a2'), dir, 'zh-CN'), axis === 'row' ? /右/ : /下/);
  });
}

test('nested splits fall back to pane numbers', async () => {
  const { buildAiPaneDirectory, aiPaneLabel } = await directoryApi();
  const dir = buildAiPaneDirectory([{
    ...tab('a', [pane('a1'), pane('a2'), pane('a3')], 'a1'),
    layout: split('row', leaf('a1'), split('col', leaf('a2'), leaf('a3'))),
  }]);
  assert.deepEqual(dir.map(entry => aiPaneLabel(entry, dir)), ['1. Lab / #1 shell', '1. Lab / #2 shell', '1. Lab / #3 shell']);
});

async function chatApi(props) {
  const source = readFileSync(new URL('../components/AIChat.svelte', import.meta.url), 'utf8')
    .split('<script lang="ts">')[1].split('</script>')[0].replace(/^\s*import .*;$/gm, '');
  const ai = readFileSync(new URL('./ai.ts', import.meta.url), 'utf8');
  const compile = input => ts.transpileModule(input, { compilerOptions: { target: 99, module: 99 } }).outputText;
  const aiRuntime = await import(`data:text/javascript;base64,${Buffer.from(compile(ai)).toString('base64')}`);
  const labelRuntime = await directoryApi();
  return vm.runInNewContext(compile(`${source}\n({
    acceptTroubleshootSeed, isOpen:()=>troubleshootOpen, close:()=>troubleshootOpen=false,
    refresh, refreshSessions, selectCurrentPane, authorizeTerminal, target:()=>terminalTarget, authorized:()=>readAuthorized,
    terminalChoices, paneChoiceLabel, displayActivity, selectTerminal, setDirectory: entries=>paneDirectory=entries,
    paneLabel, attachTerminal:async id=>{selected=id;await attach();return context.at(-1);},
    loadProviders, destination:()=>troubleshootDestination(),
  });`), {
    ...aiRuntime, ...labelRuntime, $props: () => props, $state: value => value, uuidv4: () => 'chat',
    i18n: { resolvedLocale: props.locale ?? 'en', t: key => key },
    onMount() {}, onDestroy() {},
  });
}

test('a new seed opens troubleshooting; the same source and nonce do not reopen it', async () => {
  const chat = await chatApi({});
  const seed = { text:'example', label:'First', id:'a1', nonce:1 };
  chat.acceptTroubleshootSeed(seed);
  assert.equal(chat.isOpen(), true);
  chat.close();
  chat.acceptTroubleshootSeed({ ...seed, text:'changed' });
  assert.equal(chat.isOpen(), false);
  chat.acceptTroubleshootSeed({ ...seed, nonce:2 });
  assert.equal(chat.isOpen(), true);
});

test('the current pane changes binding only on explicit selection and attachment metadata is display-only', async () => {
  const { buildAiPaneDirectory } = await directoryApi();
  const entries = buildAiPaneDirectory(directory(), 'a');
  const calls = [];
  const sessions = [{id:'a1',title:'Backend title'}, {id:'a3',title:'Second title'}];
  const chat = await chatApi({ activeSessionId:'a1', paneDirectory:entries, rpc:{ async call(method, params) {
    calls.push({method, params});
    if (method === 'session.list') return sessions;
    if (method === 'terminal.exportText') return {text:'example'};
    return [];
  } } });
  await chat.refresh();
  chat.authorizeTerminal(true);
  await chat.refresh();
  assert.equal(chat.target().id, 'a1');
  assert.equal(chat.authorized(), true);
  await chat.selectCurrentPane();
  assert.equal(chat.target().id, 'a3');
  assert.equal(chat.authorized(), false);
  assert.equal(chat.paneLabel(chat.target()), '1. Lab / #3 shell');
  const context = await chat.attachTerminal('a3');
  assert.equal(context.label, 'Second title');
  assert.equal(context.id, 'a3');
  assert.equal(calls.at(-1).params.id, 'a3');
});

test('only live sessions are choices; unassociated sessions and the current pane are distinct', async () => {
  const { buildAiPaneDirectory } = await directoryApi();
  const entries = buildAiPaneDirectory(directory(), 'a');
  const chat = await chatApi({ activeSessionId: 'a1', paneDirectory: entries, locale: 'zh-CN', rpc: { async call() {
    return [pane('remote'), pane('a3'), pane('a1')];
  } } });
  await chat.refreshSessions();
  const { associated, unassociated } = chat.terminalChoices();
  assert.deepEqual(Array.from(associated, session => session.id), ['a1', 'a3']);
  assert.deepEqual(Array.from(unassociated, session => session.id), ['remote']);
  assert.match(chat.paneChoiceLabel(pane('a3')), /当前窗格/);
  assert.doesNotMatch(chat.paneChoiceLabel(pane('a1')), /当前窗格/);
  const activity = chat.displayActivity({ target: pane('a1') });
  assert.equal(activity.target.id, 'a1');
  assert.doesNotMatch(activity.target.title, /a1/);
  chat.setDirectory(buildAiPaneDirectory(directory(), 'b'));
  assert.doesNotMatch(chat.paneChoiceLabel(pane('a3')), /当前窗格/);
  assert.equal(chat.target().id, 'a1');
  assert.equal(chat.displayActivity(activity).target.id, 'a1');
});

test('a stale list can refresh even if other choice RPCs fail and never rebinds to a replacement', async () => {
  let sessions = [pane('a1')];
  const { buildAiPaneDirectory } = await directoryApi();
  const chat = await chatApi({ activeSessionId: 'a1', paneDirectory: buildAiPaneDirectory(directory(), 'a'), rpc: { async call(method) {
    if (method === 'session.list') return sessions;
    throw Error('unavailable');
  } } });
  await assert.rejects(chat.refresh(), /unavailable/);
  assert.equal(chat.target().id, 'a1');
  chat.authorizeTerminal(true);
  sessions = [pane('a3')];
  await chat.refreshSessions();
  assert.equal(chat.target().id, 'a1');
  assert.equal(chat.authorized(), false);
  await assert.rejects(chat.attachTerminal('a1'), /terminalUnavailable/);
  await chat.selectCurrentPane();
  assert.equal(chat.target().id, 'a3');
});

for (const failed of [false, true]) {
  test(`refresh never defaults after an initial ${failed ? 'failed' : 'missing'} session lookup`, async () => {
    let first = true;
    const chat = await chatApi({ activeSessionId: 'a1', rpc: { async call() {
      if (!first) return [pane('a1'), pane('other')];
      first = false;
      if (failed) throw Error('unavailable');
      return [pane('other')];
    } } });
    if (failed) await assert.rejects(chat.refreshSessions(), /unavailable/);
    else await chat.refreshSessions();
    assert.equal(chat.target(), null);
    await chat.refreshSessions();
    assert.equal(chat.target(), null);
  });
}

test('both terminal selectors share a localized option group and show IDs only in detail', () => {
  const chat = readFileSync(new URL('../components/AIChat.svelte', import.meta.url), 'utf8');
  assert.equal(chat.match(/@render terminalOptions\(\)/g)?.length, 2);
  assert.match(chat, /<optgroup label=\{paneText\(\)\.unassociated\}>/);
  assert.match(chat, /<option value=\{session.id\} title=\{paneTitle\(session\)\}>\{paneChoiceLabel\(session\)\}<\/option>/);
  const details = readFileSync(new URL('../components/ai/AiToolActivity.svelte', import.meta.url), 'utf8');
  assert.match(details, /<small>\{activity.target.id\}<\/small>/);
});

test('troubleshooting destination uses the backend model and base for the actual selected provider', async () => {
  let backendEnabled = true;
  const reported = [];
  const chat = await chatApi({ onAiEnabled: value => reported.push(value), rpc:{ async call(method) {
    if (method === 'ai.config.get') return {config:{enabled:backendEnabled,model:'wrong',apiBase:'https://wrong.invalid'}};
    if (method === 'ai.providers.list') return {defaultProviderId:'backend',providers:[
      {id:'backend',enabled:true,model:'configured-model',apiBase:'https://configured.example/v1'},
    ]};
    return {models:[]};
  } } });
  await chat.loadProviders();
  const target = chat.destination();
  assert.equal(target.providerId, 'backend');
  assert.equal(target.model, 'configured-model');
  assert.equal(target.endpoint, 'https://configured.example/v1');
  assert.equal(target.enabled, true);
  backendEnabled = false;
  await chat.loadProviders();
  assert.equal(chat.destination().enabled, false);
  assert.deepEqual(reported, [true, false]);
});
