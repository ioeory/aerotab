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
 return null;
}};
onMount(()=>probe.mounted=true);
</script>
<div style="position:relative;width:1000px;height:600px">{#each tabs.tabs as mountedTab(mountedTab.id)}<PaneGrid {rpc} tab={mountedTab} settingsRev={0} onError={message=>probe.errors.push(message)}/>{/each}</div>`;

async function chromiumPath() {
  const candidates = [process.env.CHROMIUM_PATH, join(homedir(), '.cache/ms-playwright/chromium-1148/chrome-linux/chrome'), '/usr/bin/chromium', '/usr/bin/chromium-browser', '/usr/bin/google-chrome'].filter(Boolean);
  for (const path of candidates) { try { await access(path); return path; } catch {} }
  throw Error('Chromium is required: set CHROMIUM_PATH to a headless Chromium executable');
}
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));

test('real inactive pane mousedown preserves IME focus before any recovery rAF and ASCII writes only to that pane', { timeout: 90000 }, async () => {
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
    for(let i=0;i<300&&!debuggerUrl&&chrome.exitCode===null;i++)await pause(100);
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
    assert.deepEqual(await evaluate('focusProbe.errors'),[]);
  } finally {
    ws?.close();
    if(chrome&&chrome.exitCode===null){const exited=new Promise(resolve=>chrome.once('exit',resolve));chrome.kill();await exited;}
    await server?.close();
    await rm(profile,{recursive:true,force:true,maxRetries:5,retryDelay:100});
  }
});
