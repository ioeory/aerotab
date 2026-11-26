import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import { spawn } from 'node:child_process';
import { access, mkdtemp, rm } from 'node:fs/promises';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

// Run: node --test src/lib/paneWebviewFocus.browser.test.mjs
// Requires Node 22+ (WebSocket) and CHROMIUM_PATH, or the cached Chromium below.
// No user profile, desktop, live terminal, or installed Tauri app is accessed.
const root = fileURLToPath(new URL('../../', import.meta.url));
const component = join(root, 'tests/__paneFocusBrowser.svelte');
const entry = join(root, 'tests/__paneFocusBrowser.ts');
const source = `<script>
import PaneGrid from '../src/components/PaneGrid.svelte';
import AIChat from '../src/components/AIChat.svelte';
import OperationsWorkbench from '../src/components/OperationsWorkbench.svelte';
import { getProfileCatalog } from '../src/lib/profileCatalog.svelte';
import { shouldAutoFocusTerminal } from '../src/lib/modalFocus';
window.focusGuard = shouldAutoFocusTerminal;
import { tabs } from '../src/lib/tabs.svelte';
import { onMount } from 'svelte';
const pane = id => ({id, kind:'LocalShell', title:id});
const tab = tabs.addSessionsInTab([pane('focus-a'),pane('focus-b')], 'Focus test');
window.focusProbe = {writes:[], calls:[], errors:[], mounted:false, hold:false, frames:[]};
const probe = window.focusProbe, raf = window.requestAnimationFrame.bind(window);
window.requestAnimationFrame = fn => probe.hold ? (probe.frames.push(fn), -probe.frames.length) : raf(fn);
probe.release = () => {probe.hold=false;probe.frames.splice(0).forEach(fn=>raf(fn));};
const rpc = {call:async(method,params)=>{
 probe.calls.push(method);
 if(method==='settings.get')return {value:params.key==='terminal'?{renderer:'native',cursorBlink:false}:null};
 if(method==='terminal.snapshotEnvelope'){
  const bytes=new Uint8Array(34),d=new DataView(bytes.buffer);
  d.setUint8(0,1);d.setUint16(2,2,true);d.setUint16(4,1,true);
  return {version:1,frameVersion:1,outputSeq:'1',bytes:[...bytes]};
 }
 if(method==='session.write')probe.writes.push(params);
 if(method==='session.list')return [{id:'ssh-registered',kind:'Ssh',title:'fixture'}];
 if(method==='ssh.inspectHost'){probe.inspectionRequest=params;return params.checks.map(check=>check.kind==='service'?{status:'failed',detail:'inactive'}:{status:'ok',detail:check.kind==='tcp_listen'?'listening':'fixture'});}
 if(['profile.list','ai.mcp.list'].includes(method))return [];
 if(method==='notes.list')return {notes:[]};
 if(method==='ai.config.get')return {config:{enabled:true}};
 if(method==='ai.providers.list')return {providers:[{id:'test',name:'Fixture',enabled:true,model:'fixture',apiBase:'https://fixture.invalid/v1'}],defaultProviderId:'test'};
 if(method==='ai.providers.models')return {models:['fixture']};
 if(method==='ai.send'){probe.sent=params;return {id:'draft-fixture'};}
 if(method==='ai.poll')return {events:[{type:'delta',text:probe.sent.purpose==='command_draft'?JSON.stringify({kind:'draft',shell:'bash',command:'ss -ltn',expected:'Fixture result, do not store',sideEffects:'None'}):'Comparison result'}],cursor:1,done:true};
 return null;
}};
getProfileCatalog(rpc).profiles = [{id:'fixture-profile',name:'Fixture host',kind:'ssh',group:'Servers',tags:['oncall'],ssh:{user:'operator',host:'fixture.invalid',port:22}}];
let troubleshootSeed = $state();
let incident = $state({title:'',conclusion:'',evidence:[],steps:[]});
let opsVisible = $state(false);
const inspection = [{id:'ssh-registered',hostKey:'profile:fixture',hostLabel:'fixture server',label:'Pane 1',kind:'Ssh'}, {id:'ssh-ended',hostKey:'profile:fixture',hostLabel:'fixture server',label:'Pane 2',kind:'Ssh'}];
probe.showOperations = () => { opsVisible = true; };
probe.captureScene = () => ({title:'Fixture SSH layout',layout:{profileId:'fixture-profile'}});
probe.reopenScene = async scene => {probe.reopened = scene; return true;};
function addEvidence(entry){ incident.evidence.push(entry); opsVisible = true; return true; }
probe.stage = seed => { troubleshootSeed = seed; };
onMount(()=>probe.mounted=true);
</script>
<div style="position:relative;width:1000px;height:600px">{#each tabs.tabs as mountedTab(mountedTab.id)}<PaneGrid {rpc} tab={mountedTab} settingsRev={0} onError={message=>probe.errors.push(message)}/>{/each}</div><div style="position:fixed;top:0;right:0;width:300px;height:500px;z-index:100"><AIChat {rpc} {troubleshootSeed} onSettings={()=>{}} onAddIncidentEvidence={addEvidence}/></div><div hidden={!opsVisible}><OperationsWorkbench {rpc} visible={opsVisible} bind:record={incident} panes={inspection} onFocusPane={id=>probe.focused=id} onAddEvidence={addEvidence} captureScene={probe.captureScene} reopenScene={probe.reopenScene} profileIds={['fixture-profile']}/></div>`;

async function chromiumPath() {
  const candidates = [process.env.CHROMIUM_PATH, join(homedir(), '.cache/ms-playwright/chromium-1148/chrome-linux/chrome'), '/usr/bin/chromium', '/usr/bin/chromium-browser', '/usr/bin/google-chrome'].filter(Boolean);
  for (const path of candidates) { try { await access(path); return path; } catch {} }
  throw Error('Chromium is required: set CHROMIUM_PATH to a headless Chromium executable');
}
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));

test('terminal first click, AI copy, local draft resume, and evidence comparison', { timeout: 150000 }, async () => {
  const executable = await chromiumPath();
  const profile = await mkdtemp(join(tmpdir(), 'aerotab-pane-focus-'));
  let chrome, ws, server;
  const errors = [];
  try {
    const virtual = {
      name:'pane-focus-browser-fixture', enforce:'pre',
      resolveId(path) { if(path.endsWith('__paneFocusBrowser.svelte'))return component; if(path.endsWith('__paneFocusBrowser.ts'))return entry; },
      load(path) {
        if(path===component)return source;
        if(path===entry)return `import {mount} from 'svelte';import Fixture from './__paneFocusBrowser.svelte';import '../src/app.css';mount(Fixture,{target:document.getElementById('app')});`;
      },
      configureServer(vite) {
        vite.middlewares.use('/pane-focus.html', (_req,res)=>{res.setHeader('Content-Type','text/html');res.end('<div id="app"></div><script type="module" src="/tests/__paneFocusBrowser.ts"></script>');});
      },
    };
    server = await createServer({root, configFile:false, cacheDir:join(profile,'vite-cache'), plugins:[virtual,svelte(),tailwindcss()], server:{host:'127.0.0.1',port:0,hmr:false}});
    await server.listen();
    const url = `http://127.0.0.1:${server.httpServer.address().port}/pane-focus.html`;
    let debuggerUrl = '';
    chrome = spawn(executable, ['--headless','--no-sandbox','--disable-gpu','--password-store=basic','--remote-debugging-port=0',`--user-data-dir=${profile}`,'about:blank'], {stdio:['ignore','ignore','pipe']});
    chrome.stderr.on('data', data => { const text=String(data); errors.push(text); debuggerUrl ||= /DevTools listening on (ws:\/\/\S+)/.exec(text)?.[1] ?? ''; });
    for(let i=0;i<650&&!debuggerUrl&&chrome.exitCode===null;i++)await pause(100);
    assert.ok(debuggerUrl, `Chromium failed to start: ${errors.join('')}`);
    const port = new URL(debuggerUrl).port;
    const pages = await fetch(`http://127.0.0.1:${port}/json`).then(r=>r.json());
    ws = new WebSocket(pages.find(page=>page.type==='page').webSocketDebuggerUrl);
    await new Promise((resolve,reject)=>{ws.addEventListener('open',resolve,{once:true});ws.addEventListener('error',reject,{once:true});});
    let sequence=0; const pending=new Map();
    ws.addEventListener('message', event=>{const message=JSON.parse(event.data);if(message.id){const task=pending.get(message.id);pending.delete(message.id);if(message.error)task.reject(Error(JSON.stringify(message.error)));else task.resolve(message.result);}});
    const call=(method,params={})=>new Promise((resolve,reject)=>{const id=++sequence;pending.set(id,{resolve,reject});ws.send(JSON.stringify({id,method,params}));});
    const evaluate=async expression=>{const result=await call('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});assert.ok(!result.exceptionDetails,JSON.stringify(result.exceptionDetails));return result.result.value;};
    await call('Page.enable');
    await call('Emulation.setDeviceMetricsOverride',{width:1280,height:900,deviceScaleFactor:1,mobile:false});
    await call('Page.navigate',{url});
    let ready=false;
    for(let i=0;i<200;i++){
      ready=await evaluate(`!!window.focusProbe?.mounted && document.activeElement?.closest('[data-native-session]')?.dataset.nativeSession==='focus-a' && !!document.querySelector('[data-pane-drop-pane="focus-b"]')?.getClientRects().length`);
      if(ready)break;await pause(100);
    }
    assert.ok(ready,'production WebView panes did not become ready');
    await pause(200); // Let pre-existing fit retries finish before freezing rAF.
    const mouse=async(selector,button='left')=>{
      // Inactive WebView hosts keep pointer-events so the first click hits the
      // canvas/IME path (not the pane shell). Hit-test must land inside the pane.
      const point=await evaluate(`(()=>{const el=document.querySelector(${JSON.stringify(selector)}),r=el.getBoundingClientRect();const x=r.x+r.width*.6,y=r.y+r.height*.6;const hit=document.elementFromPoint(x,y);return{x,y,inside:!!hit&&el.contains(hit),hitPe:hit&&getComputedStyle(hit.closest('.terminal-host')||hit).pointerEvents};})()`);
      assert.equal(point.inside,true,`click must land inside ${selector}`);
      assert.notEqual(point.hitPe,'none','inactive WebView terminal must accept the first click');
      await call('Input.dispatchMouseEvent',{type:'mousePressed',x:point.x,y:point.y,button,clickCount:1});
      return async()=>call('Input.dispatchMouseEvent',{type:'mouseReleased',x:point.x,y:point.y,button,clickCount:1});
    };
    for(const id of ['focus-b','focus-a']){
      await evaluate('focusProbe.hold=true;focusProbe.frames=[];focusProbe.writes=[]');
      const release=await mouse(`[data-pane-drop-pane="${id}"]`);
      await pause(20); // Drain Svelte microtasks, without releasing any recovery rAF.
      const state=await evaluate(`({tag:document.activeElement.tagName,pane:document.activeElement.closest('[data-native-session]')?.dataset.nativeSession,focused:document.hasFocus(),views:[...document.querySelectorAll('[data-native-session]')].map(el=>({id:el.dataset.nativeSession,active:el.dataset.nativeInputActive}))})`);
      assert.equal(state.tag,'TEXTAREA','first click must focus the target IME without a second click');
      assert.equal(state.pane,id);
      await release();
      await call('Input.dispatchKeyEvent',{type:'keyDown',key:'a',code:'KeyA',text:'a',unmodifiedText:'a',windowsVirtualKeyCode:65});
      await call('Input.dispatchKeyEvent',{type:'keyUp',key:'a',code:'KeyA',windowsVirtualKeyCode:65});
      await pause(20);
      assert.deepEqual(await evaluate('focusProbe.writes'),[{id,data:'YQ=='}]);
      await evaluate('focusProbe.release()');await pause(180);
    }
    // Guardrails exercise the mounted handler, not a copy of its predicate.
    // Direct targets: nonterminal/connecting backgrounds, native-surface,
    // right mouse, interactive descendants and pane drag chrome stay untouched.
    for(const variant of ['button','input','drag','native-surface','missing-terminal','secondary']){
      const state=await evaluate(`(()=>{
        const shell=document.querySelector('[data-pane-drop-pane="focus-b"]');
        const terminal=shell.querySelector('[data-terminal-view-backend]');
        let target=shell, child;
        if(${JSON.stringify(variant)}==='native-surface')terminal.dataset.terminalViewBackend='native-surface';
        if(${JSON.stringify(variant)}==='missing-terminal')terminal.removeAttribute('data-terminal-view-backend');
        if(['button','input','drag'].includes(${JSON.stringify(variant)})){
          child=document.createElement(${JSON.stringify(variant)}==='input'?'input':${JSON.stringify(variant)}==='button'?'button':'div');
          if(${JSON.stringify(variant)}==='drag')child.className='pane-drag-handle';shell.append(child);target=child;
        }
        const event=new MouseEvent('mousedown',{bubbles:true,cancelable:true,button:${variant==='secondary'?2:0}});
        target.dispatchEvent(event);child?.remove();terminal.dataset.terminalViewBackend='webview';
        return {prevented:event.defaultPrevented};
      })()`);
      assert.equal(state.prevented,false,`${variant} mousedown must retain its default`);
    }
    // A real mouse gesture on AI prose must own focus before the selection exists.
    const point = await evaluate(`(() => {
      const p = document.querySelector('.chat .empty p');
      const range = document.createRange(); range.selectNodeContents(p);
      const r = range.getClientRects()[0];
      return {x: r.left + 1, y: r.top + r.height / 2, endX: r.right - 1};
    })()`);
    await call('Input.dispatchMouseEvent',{type:'mousePressed',x:point.x,y:point.y,button:'left',clickCount:1});
    assert.equal(await evaluate(`focusGuard(document.querySelector('[data-native-session]'))`), false,
      'delayed terminal focus must yield as soon as AI text selection begins');
    await call('Input.dispatchMouseEvent',{type:'mouseMoved',x:point.endX,y:point.y,button:'left',buttons:1});
    await call('Input.dispatchMouseEvent',{type:'mouseReleased',x:point.endX,y:point.y,button:'left',clickCount:1});
    const expected = await evaluate(`(() => {
      window.copied = [];
      Object.defineProperty(navigator.clipboard, 'writeText', {value: async text => {copied.push(text)}});
      return getSelection().toString();
    })()`);
    await call('Input.dispatchMouseEvent',{type:'mousePressed',x:(point.x+point.endX)/2,y:point.y,button:'right',clickCount:1});
    await call('Input.dispatchMouseEvent',{type:'mouseReleased',x:(point.x+point.endX)/2,y:point.y,button:'right',clickCount:1});
    assert.ok(expected);
    assert.equal(await evaluate(`document.querySelector('[data-ai-selection-copy]')?.textContent.trim()`), 'Copy');
    await evaluate(`document.querySelector('[data-ai-selection-copy]').click()`);
    await call('Input.dispatchKeyEvent',{type:'keyDown',key:'c',code:'KeyC',modifiers:2,windowsVirtualKeyCode:67});
    await call('Input.dispatchKeyEvent',{type:'keyUp',key:'c',code:'KeyC',modifiers:2,windowsVirtualKeyCode:67});
    assert.deepEqual(await evaluate('copied'), [expected, expected]);
    const openTroubleshoot = () => evaluate(`document.querySelector('.chat .troubleshoot-toggle').click()`);
    const button = label => evaluate(`Array.from(document.querySelectorAll('.chat .troubleshoot button')).find(b => b.textContent.trim() === ${JSON.stringify(label)}).click()`);
    const key = 'aerotab.ai.troubleshoot.resume.v1';
    await openTroubleshoot();
    assert.equal(await evaluate(`(() => {
      const input = document.querySelector('.troubleshoot > label textarea');
      input.value = 'Log that must stay in memory'; input.dispatchEvent(new Event('input', {bubbles:true}));
      return Promise.resolve().then(() => Array.from(document.querySelectorAll('.troubleshoot button'))
        .find(b => b.textContent.trim() === 'Review and save draft').disabled);
    })()`), true);
    await evaluate(`document.querySelector('.troubleshoot details summary').click()`);
    await evaluate(`document.querySelector('[aria-label="Apply: Port already in use"]').click()`);
    await evaluate(`(() => {
      const input = document.querySelector('.troubleshoot > label textarea');
      input.value = 'Inspect listener status'; input.dispatchEvent(new Event('input', {bubbles:true}));
      const shell = document.querySelector('.troubleshoot > label select');
      shell.value = 'bash'; shell.dispatchEvent(new Event('change', {bubbles:true}));
    })()`);
    await button('Confirm and send');
    const commandInput = '.troubleshoot input[maxlength="4096"]';
    for (let i=0;i<75;i++) {
      if (await evaluate(`!!document.querySelector(${JSON.stringify(commandInput)})`)) break;
      await pause(100);
    }
    await evaluate(`(() => {
      const input = document.querySelector(${JSON.stringify(commandInput)});
      input.value = 'ss -ltnp'; input.dispatchEvent(new Event('input', {bubbles:true}));
    })()`);
    await button('Review and save draft');
    assert.equal(await evaluate(`localStorage.getItem(${JSON.stringify(key)})`), null);
    await button('Save locally');
    assert.deepEqual(await evaluate(`JSON.parse(localStorage.getItem(${JSON.stringify(key)}))`), {
      version:1, question:'Inspect listener status', template:'Port already in use', shell:'bash', command:'ss -ltnp', commandShell:'bash',
    });
    assert.deepEqual(await evaluate('focusProbe.errors'),[]);
    await call('Page.reload');
    for (let i=0;i<150;i++) {
      if (await evaluate(`!!window.focusProbe?.mounted && !!document.querySelector('.chat .troubleshoot-toggle')`)) break;
      await pause(100);
    }
    await openTroubleshoot();
    assert.equal(await evaluate(`document.querySelector('.troubleshoot > label textarea').value`), '');
    await button('Review saved draft');
    await button('Restore to editor');
    assert.deepEqual(await evaluate(`[
      document.querySelector('.troubleshoot > label textarea').value,
      document.querySelector('.troubleshoot > label select').value,
      document.querySelector('[aria-label="Local troubleshooting draft"]').textContent.includes('Port already in use'),
      document.querySelector(${JSON.stringify(commandInput)}).value,
      focusProbe.calls.some(method => method.startsWith('ai.send') || method === 'session.write')
    ]`), ['Inspect listener status','bash',true,'ss -ltnp',false]);
    await button('Delete saved draft');
    assert.equal(await evaluate(`localStorage.getItem(${JSON.stringify(key)})`), null);
    await evaluate(`focusProbe.stage({id:'focus-a',label:'Tab 1 / Left',nonce:1,text:'first evidence',evidence:{side:'A',source:'selection'}})`);
    assert.equal(await evaluate(`Array.from(document.querySelectorAll('.troubleshoot button')).find(b => b.textContent.trim() === 'Confirm and send').disabled`), true);
    await evaluate(`focusProbe.stage({id:'focus-b',label:'Tab 1 / Right',nonce:2,text:'previous command output',evidence:{side:'B',source:'lastCommand'}})`);
    const compare = await evaluate(`(() => {
      const a = document.querySelector('[data-evidence-side="A"]');
      const b = document.querySelector('[data-evidence-side="B"]');
      return {a:a.querySelector('textarea').value,b:b.querySelector('textarea').value,labels:a.textContent.includes('Tab 1 / Left') && b.textContent.includes('Tab 1 / Right'),preview:document.querySelector('.troubleshoot pre').textContent};
    })()`);
    assert.equal(compare.a, 'first evidence');
    assert.equal(compare.b, 'previous command output');
    assert.equal(compare.labels, true);
    assert.match(compare.preview, /A:\nfirst evidence\n\nB:\nprevious command output/);
    assert.doesNotMatch(compare.preview, /Tab 1|focus-a|focus-b/);
    assert.equal(await evaluate(`!!document.querySelector('[aria-label="Local troubleshooting draft"]')`), false);
    const opsKey = 'aerotab.operations.incident.v1';
    const ops = (label) => evaluate(`(() => { const panel = document.querySelector('.operations'); const matches = [...panel.querySelectorAll('button')].filter(el => el.textContent.trim() === ${JSON.stringify(label)} && !el.disabled); if (!matches.length) throw Error('Operations button not found: ' + ${JSON.stringify(label)}); matches.at(-1).click(); })()`);
    const passiveCalls = await evaluate('focusProbe.calls.length');
    await button('Add A to incident');
    assert.equal(await evaluate(`document.querySelector('.operations [aria-label="Evidence"] textarea').value`), 'first evidence');
    await ops('Host inspection');
    assert.match(await evaluate(`document.querySelector('.operations').textContent`), /No cached monitoring sample/);
    assert.deepEqual(await evaluate(`focusProbe.calls.slice(${passiveCalls})`), []);
    const inspectionSection = `document.querySelector('[aria-label="Active inspection"]')`;
    const searchHosts = value => evaluate(`(() => {const field=${inspectionSection}.querySelector('input[type=search]');field.value=${JSON.stringify(value)};field.dispatchEvent(new Event('input',{bubbles:true}));})()`);
    const matchedHosts = () => evaluate(`[...${inspectionSection}.querySelectorAll('.host-list label')].map(label=>label.textContent.trim())`);
    const toggleMatchedHost = () => evaluate(`${inspectionSection}.querySelector('.host-list input').click()`);
    await searchHosts('#oncall user:operator');
    assert.deepEqual(await matchedHosts(), ['Fixture hostoperator@fixture.invalid']);
    await toggleMatchedHost();
    await evaluate(`(() => {for(const [field,value] of [...${inspectionSection}.querySelectorAll('fieldset input:not([type="checkbox"])')].map((field,index)=>[field,['nginx','443'][index]])){field.value=value;field.dispatchEvent(new Event('input',{bubbles:true}));}})()`);
    assert.deepEqual(await evaluate(`focusProbe.calls.slice(${passiveCalls})`), []);
    await ops('Start inspection');
    assert.deepEqual(await evaluate('focusProbe.inspectionRequest'), {profile_id:'fixture-profile',checks:[{kind:'resources'},{kind:'inodes'},{kind:'service',unit:'nginx.service'},{kind:'tcp_listen',port:443}]});
    assert.match(await evaluate(`document.querySelector('[aria-label="Active inspection"] article').textContent`), /Failed · nginx\.service.*Service is inactive/);
    await evaluate(`document.querySelector('.session-overview summary').click()`);
    await ops('Refresh session status');
    assert.match(await evaluate(`document.querySelector('.operations').textContent`), /Pane 1 · Session registered/);
    assert.match(await evaluate(`document.querySelector('.operations').textContent`), /Pane 2 · Session no longer registered/);
    await toggleMatchedHost();
    await searchHosts('pane 2');
    assert.deepEqual(await matchedHosts(), []);
    await searchHosts('pane 1 fixture');
    assert.deepEqual(await matchedHosts(), ['Pane 1fixture server']);
    await toggleMatchedHost();
    const writesBeforeInspection = await evaluate('focusProbe.writes.length');
    await ops('Start inspection');
    assert.deepEqual(await evaluate('focusProbe.inspectionRequest'), {session_id:'ssh-registered',checks:[{kind:'resources'},{kind:'inodes'},{kind:'service',unit:'nginx.service'},{kind:'tcp_listen',port:443}]});
    assert.equal(await evaluate('focusProbe.writes.length'), writesBeforeInspection);
    await ops('Show pane');
    assert.equal(await evaluate('focusProbe.focused'), 'ssh-ended');
    await ops('Add to incident');
    await evaluate(`(() => { const select = document.querySelector('.operations select'); select.value = 'port-in-use'; select.dispatchEvent(new Event('change', {bubbles:true})); })()`);
    await ops('Add template steps');
    await evaluate(`document.querySelector('.operations [aria-label="Manual runbook"] input[type="checkbox"]').click()`);
    const fillOps = (label, value) => evaluate(`(() => {const field = [...document.querySelectorAll('.operations label')].find(el => el.textContent.trim() === ${JSON.stringify(label)}).querySelector('input'); field.value = ${JSON.stringify(value)}; field.dispatchEvent(new Event('input', {bubbles:true}));})()`);
    const localEvents = `new Promise((resolve,reject)=>{const r=indexedDB.open('aerotab-operations',1);r.onerror=reject;r.onsuccess=()=>{const db=r.result,q=db.transaction('events').objectStore('events').getAll();q.onsuccess=()=>{db.close();resolve(q.result);};q.onerror=reject;};})`;
    async function saveEventInUi() {
      await ops('Review local save'); await ops('Build save preview'); await ops('Save to this device');
      for(let i=0;i<40;i++){if(await evaluate(`document.querySelector('.operations [role="status"]')?.textContent === 'Saved locally.'`)) return;await pause(50);}
      assert.fail('Local save did not finish');
    }
    await fillOps('Incident title', 'Listener outage');
    await fillOps('Project', 'Payments'); await fillOps('Environment', 'Production');
    await ops('Capture active SSH layout');
    assert.deepEqual(await evaluate(localEvents), []);
    await saveEventInUi();
    assert.equal(await evaluate(`localStorage.getItem(${JSON.stringify(opsKey)})`), null);
    const events = await evaluate(localEvents);
    assert.equal(events.length, 1);
    assert.deepEqual([events[0].record.evidence.length, events[0].record.steps[0].done], [0,true]);
    await ops('New incident'); await ops('Continue');
    await fillOps('Incident title', 'Staging rollback'); await fillOps('Project', 'Payments'); await fillOps('Environment', 'Staging');
    await saveEventInUi();
    await ops('New incident'); await fillOps('Incident title', 'Third incident'); await saveEventInUi();
    assert.equal((await evaluate(localEvents)).length, 3);
    assert.deepEqual(await evaluate(`focusProbe.calls.slice(${passiveCalls})`), ['ssh.inspectHost','session.list','ssh.inspectHost']);
    await button('Remove evidence B');
    assert.equal(await evaluate(`Array.from(document.querySelectorAll('.troubleshoot button')).find(b => b.textContent.trim() === 'Confirm and send').disabled`), true);
    assert.equal(await evaluate(`focusProbe.calls.some(method => method === 'ai.send')`), false);
    const editedPreview = await evaluate(`(() => {
      const b = document.querySelector('[data-evidence-side="B"] textarea');
      b.value = 'manually pasted B'; b.dispatchEvent(new Event('input', {bubbles:true}));
      return Promise.resolve().then(() => document.querySelector('.troubleshoot pre').textContent);
    })()`);
    assert.match(editedPreview, /manually pasted B/);
    await button('Confirm and send');
    assert.deepEqual(await evaluate(`({purpose:focusProbe.sent.purpose,message:focusProbe.sent.message,writes:focusProbe.writes.length})`), {
      purpose:'terminal_explain', message:editedPreview, writes:0,
    });
    await button('Draft a follow-up command from selected differences');
    assert.equal(await evaluate(`document.querySelector('[aria-label="Comparison result"] textarea').value`), 'Comparison result');
    await evaluate(`document.querySelector('[aria-label="Comparison result"] textarea').setSelectionRange(0,10)`);
    await button('Draft a follow-up command from selected differences');
    assert.deepEqual(await evaluate(`[
      document.querySelector('.troubleshoot > label textarea').value,
      document.querySelector('.troubleshoot > label select').value,
      focusProbe.calls.filter(method => method === 'ai.send').length,
      focusProbe.writes.length
    ]`), ['Comparison','',1,0]);
    await evaluate(`(() => {
      const shell = document.querySelector('.troubleshoot > label select');
      shell.value = 'bash'; shell.dispatchEvent(new Event('change', {bubbles:true}));
    })()`);
    await button('Confirm and send');
    assert.deepEqual(await evaluate(`({purpose:focusProbe.sent.purpose,message:focusProbe.sent.message,shell:focusProbe.sent.shell,writes:focusProbe.writes.length})`), {
      purpose:'command_draft', message:'Comparison', shell:'bash', writes:0,
    });
    assert.equal(await evaluate(`!!document.querySelector('.troubleshoot input[maxlength="4096"]')`), true);
    await evaluate(`localStorage.setItem(${JSON.stringify(opsKey)}, JSON.stringify({version:1,record:{title:'Legacy incident', conclusion:'', steps:[], evidence:[]}}))`);
    await call('Page.reload');
    for (let i=0;i<150;i++) {
      if (await evaluate(`!!window.focusProbe?.mounted && [...document.querySelectorAll('.operations button')].some(button => button.textContent.trim() === 'Legacy incident')`)) break;
      await pause(100);
    }
    await evaluate('focusProbe.showOperations()');
    assert.equal((await evaluate(localEvents)).filter(event => event.id === 'legacy-v1').length, 1);
    assert.match(await evaluate(`localStorage.getItem(${JSON.stringify(opsKey)})`), /Legacy incident/);
    const beforeRestore = await evaluate('focusProbe.calls.length');
    await ops('Listener outage · Payments / Production');
    assert.equal(await evaluate('!!focusProbe.reopened'), false);
    await ops('Reopen SSH panes');
    assert.equal(await evaluate('!!focusProbe.reopened'), false);
    await ops('Reopen SSH panes');
    assert.equal(await evaluate('focusProbe.reopened.layout.profileId'), 'fixture-profile');
    assert.equal(await evaluate(`document.querySelector('.operations [aria-label="Manual runbook"] input[type="checkbox"]').checked`), true);
    assert.equal(await evaluate(`document.querySelectorAll('.operations [aria-label="Evidence"] textarea').length`), 0);
    await ops('Preview Markdown handoff');
    assert.match(await evaluate(`document.querySelector('.operations [aria-label="Preview Markdown handoff"] textarea').value`), /Listener outage[\s\S]+Verified/);
    await ops('Close');
    await ops('Staging rollback · Payments / Staging');
    assert.equal(await evaluate(`document.querySelectorAll('.operations [aria-label="Manual runbook"] input[type="checkbox"]').length`), 0);
    assert.deepEqual(await evaluate(`focusProbe.calls.slice(${beforeRestore})`), []);
  } finally {
    ws?.close();
    if(chrome&&chrome.exitCode===null){const exited=new Promise(resolve=>chrome.once('exit',resolve));chrome.kill();await exited;}
    await server?.close();
    await rm(profile,{recursive:true,force:true,maxRetries:5,retryDelay:100});
  }
});
