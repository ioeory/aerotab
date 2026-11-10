import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';
import { parse } from 'svelte/compiler';

const browser = readFileSync(new URL('../components/SftpBrowser.svelte', import.meta.url), 'utf8');
const app = readFileSync(new URL('../App.svelte', import.meta.url), 'utf8');
const workspace = readFileSync(new URL('../components/FileWorkspaceSide.svelte', import.meta.url), 'utf8');
const script = browser.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
const js = source => ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None } }).outputText;
function functionSource(name) {
  const file = ts.createSourceFile('browser.ts', script, ts.ScriptTarget.Latest, true);
  const fn = file.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === name);
  assert.ok(fn, `missing ${name}`);
  return fn.getText(file);
}
function localPaneVisible(mode, showLocalPane = true) {
  const expression = script.match(/const showLocalPaneEffective = \$derived\((.*)\);/)[1];
  return Boolean(vm.runInNewContext(expression, { mode, showLocalPane, localCwd: '/home/local', localPaneCollapsed: false }));
}
function enclosingConditions(marker) {
  const position = browser.indexOf(marker);
  const conditions = [];
  function visit(node) {
    if (!node || typeof node !== 'object') return;
    if (node.type === 'IfBlock' && node.start <= position && node.end >= position) conditions.push(browser.slice(node.expression.start, node.expression.end));
    for (const value of Object.values(node)) {
      if (Array.isArray(value)) value.forEach(visit);
      else if (value && typeof value === 'object') visit(value);
    }
  }
  visit(parse(browser));
  return conditions;
}

test('dock never renders a local pane, even when wide and explicitly enabled', () => {
  assert.equal(localPaneVisible('dock'), false);
  assert.equal(localPaneVisible('dock', false), false);
  assert.ok(enclosingConditions('<SftpLocalPane').includes('showLocalPaneEffective'));
});
test('modal retains its full local and remote browser', () => {
  assert.equal(localPaneVisible('modal'), true);
  assert.equal(localPaneVisible('modal', false), false);
  assert.ok(browser.includes("i18n.t('sftp.remotePane')"));
});
test('dock offers no local-pane toggle or width-dependent local collapse', () => {
  assert.ok(!browser.includes("i18n.t('sftp.toggleLocalPane')"));
  assert.ok(!script.includes('localPaneCollapsed'));
});

for (const [mode, showLocalPane, expected] of [['dock', true, []], ['dock', false, []], ['modal', false, []], ['modal', true, ['local_home_dir', 'local_list_dir']]]) {
  test(`${mode} showLocalPane=${showLocalPane}: local initialization respects browser mode`, async () => {
    const calls = [];
    const context = vm.createContext({ mode, showLocalPane, defaultDownloadDir: null, localCwd: '',
      localListSeq: 0, localLoading: false, localListError: null, localEntries: [],
      tauriInvoke: async command => { calls.push(command); return command === 'local_home_dir' ? '/home/local' : []; },
    });
    await vm.runInContext(js(`${functionSource('refreshLocal')}\n${functionSource('initLocalPane')}\ninitLocalPane();`), context);
    assert.deepEqual(calls, expected);
  });
}

test('dock does not show the local-browser setup banner', () => {
  const conditions = enclosingConditions("{i18n.t('sftp.downloadDirBanner')}");
  assert.equal(conditions.every(condition => vm.runInNewContext(condition, { mode: 'dock', showLocalPane: true, defaultDownloadDir: null, downloadBannerDismissed: false })), false);
});

test('mount still loads download settings independently of local browsing', () => {
  const mount = script.slice(script.indexOf('  onMount(() => {'));
  assert.match(mount, /await loadSftpSettings\(\);[\s\S]*await initLocalPane\(\)/);
});
test('dock keeps transfer queue and remote OS drop handlers without local pane', () => {
  const conditions = enclosingConditions('<SftpTransferQueue');
  assert.ok(conditions.every(condition => vm.runInNewContext(condition, { mode: 'dock', showLocalPane: true, showLocalPaneEffective: false, sharedTasks: [{}] })));
  assert.match(browser, /ondrop=\{\(e\) => \{ if \(!loading\) void handleRemotePaneDrop\(e\); \}\}/);
  assert.match(functionSource('handleRemotePaneDrop'), /e\.dataTransfer\?\.files/);
  assert.match(workspace, /mode="dock" showLocalPane=\{false\}/);
});

test('dock file download uses picker and configured fallback without localCwd', async () => {
  for (const picked of ['/chosen/file.txt', undefined, null]) {
    const tasks = [];
    const context = vm.createContext({ sessionId: 'remote', defaultDownloadDir: '/downloads', lastDownloadPath: null, cwd: '/remote',
      pickSavePath: async name => { assert.equal(name, 'file.txt'); return picked; },
      nextTransferId: () => 'task', joinPath: (base, name) => `${base}/${name}`, downloadEntries: new Map(),
      enqueueDownloadTransfer: (...args) => tasks.push(args),
    });
    await vm.runInContext(js(`${functionSource('downloadEntry')}\ndownloadEntry({ name: 'file.txt', kind: 'File', size: 1 });`), context);
    assert.equal(tasks.length, picked === null ? 0 : 1);
    if (tasks.length) assert.equal(tasks[0][3].localPath, picked ?? '/downloads/file.txt');
  }
});
test('dock directory download uses picker and configured fallback without local browsing', async () => {
  for (const picked of ['/chosen', undefined, null]) {
    const tasks = [];
    const context = vm.createContext({ sessionId: 'remote', profile: undefined, source: { profileId: 'saved' }, defaultDownloadDir: '/downloads', lastDownloadPath: null, cwd: '/remote', preparingTransfers: false,
      pickDirectoryPath: async () => picked, joinPath: (base, name) => `${base}/${name}`, joinLocalPath: (base, name) => `${base}/${name}`,
      fileService: { queue: { enqueue: task => tasks.push(task) } }, onError: message => assert.fail(message),
    });
    await vm.runInContext(js(`${functionSource('downloadDirectory')}\ndownloadDirectory({ name: 'folder', kind: 'Dir' });`), context);
    assert.equal(tasks.length, picked === null ? 0 : 1);
    if (tasks.length) {
      assert.equal(tasks[0].destination.path, `${picked ?? '/downloads'}/folder`);
      assert.equal(tasks[0].source.profileId, 'saved');
    }
  }
});
test('header pop-out preserves connection identity and sudo and opens modal', () => {
  assert.match(browser, /onclick=\{\(\) => onPopOut\(sudoMode\)\}/);
  assert.match(app, /onPopOut=\{\(sudo\) => openSftpWindow\(\{ \.\.\.dockTarget, sudo \}\)\}/);
  assert.match(app, /source=\{win\.target\}\s+mode="modal"/);
  assert.match(script, /profile_id: profile\?\.id \?\? source\?\.profileId/);
});
