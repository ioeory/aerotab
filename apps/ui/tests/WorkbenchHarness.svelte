<script lang="ts">
 import {onMount,tick,type Snippet} from 'svelte';
 import NativeTerminalView from '../src/components/NativeTerminalView.svelte';
 import PrimaryNav from '../src/components/PrimaryNav.svelte';
 import Workbench from '../src/components/Workbench.svelte';
 import NoteEditor from '../src/components/notes/NoteEditor.svelte';
 import SftpTransferQueue from '../src/components/SftpTransferQueue.svelte';
 import LifetimeProbe from './LifetimeProbe.svelte';
 import {NotesStore} from '../src/lib/notes.svelte';
 import type {Note} from '../src/lib/notesCore';
 import type {RpcClient} from '../src/lib/rpc';
 let nativeReady=$state(false);let nativeFailure=$state('');const nativeWrites:string[]=[];
 const nativeRpc={call:async(method:string)=>{if(method==='terminal.snapshotEnvelope'){const bytes=new Uint8Array(34);const d=new DataView(bytes.buffer);d.setUint8(0,1);d.setUint16(2,2,true);d.setUint16(4,1,true);return{version:1,frameVersion:1,outputSeq:'1',bytes:[...bytes]};}return null;}} as unknown as import('../src/lib/rpc').RpcClient;
 const events:string[]=[];let hidden=$state(false);let short=$state(false);let result=$state<string[]>([]);let failed=$state('');let extra=$state<{id:string;type:'ai';title:string;content:Snippet;visible:boolean}[]>([]);
 const note:Note={id:'mock-note',title:'Mock draft',markdown:'initial',tags:[],profile_ids:[],parent_id:null,version:1,updated_at:0};
 const rpc:RpcClient={async call<T>(method:string):Promise<T>{if(method==='notes.list')return {notes:[note],folders:[],revision:1} as T;if(method==='notes.put')throw Error('Simulated offline save; retain draft');return null as T;}};
 const store=new NotesStore(rpc);let tasks=$state([{id:'task',name:'mock-running.bin',status:'running' as const,percent:37,summary:'37 / 100 B'}]);
 const settle=async()=>{await tick();await new Promise<void>(resolve=>setTimeout(resolve,30));};
 function assert(condition:unknown,message:string){if(!condition)throw Error(message);result.push(message);}
 const panel=(id:string)=>document.querySelector<HTMLElement>(`[data-panel-id="${id}"]`)!;
 const select=async(id:string,side:string)=>{const select=panel(id).querySelector('select')!;select.value=side;select.dispatchEvent(new Event('change',{bubbles:true}));await settle();};
 async function run(){try{
  await settle();const editor=document.querySelector('[data-panel-id="notes"] textarea')!;const queue=panel('sftp').querySelector('[data-probe]')!;const terminal=document.querySelector('[data-terminal-fixture]')!;
  editor.value='unsaved mock draft';editor.dispatchEvent(new Event('input',{bubbles:true}));await settle();const draft=store.drafts[note.id];
  for(const side of ['left','top','float','bottom','right']){await select('notes',side);assert(document.querySelector('[data-panel-id="notes"] textarea')===editor,`NoteEditor same DOM instance after ${side}`);assert(editor.value==='unsaved mock draft',`Draft survives ${side}`);}
  for(const side of ['float','left','bottom','right']){await select('sftp',side);assert(panel('sftp').querySelector('[data-probe]')===queue,`SFTP queue same mounted instance after ${side}`);assert(panel('sftp').textContent?.includes('mock-running.bin'),'Running mock transfer retained');}
  assert(store.drafts[note.id]===draft,'Actual NotesStore draft identity preserved');assert(document.querySelector('[data-terminal-fixture]')===terminal,'Terminal host remains mounted');
  extra=[{id:'ai:tab-one',type:'ai',title:'AI tab one',content:aiContent,visible:true}];await settle();const ai=panel('ai:tab-one').querySelector('input')!;ai.value='AI draft';await select('ai:tab-one','float');await select('ai:tab-one','left');assert(panel('ai:tab-one').querySelector('input')===ai,'Runtime AI registration content not remounted by move');
  extra=[...extra,{id:'ai:tab-two',type:'ai',title:'AI tab two',content:aiContentTwo,visible:true}];await settle();assert(panel('ai:tab-one').querySelector('input')===ai,'Adding second tab AI preserves first instance');
  extra=extra.map(p=>({...p,visible:p.id==='ai:tab-two'}));await settle();extra=extra.map(p=>({...p,visible:true}));await settle();assert(panel('ai:tab-one').querySelector('input')===ai&&ai.value==='AI draft','Tab visibility change preserves AI input');
  extra=extra.filter(p=>p.id!=='ai:tab-two');await settle();assert(!document.querySelector('[data-panel-id="ai:tab-two"]'),'Unregister removes only requested panel');assert(panel('ai:tab-one').querySelector('input')===ai,'Unregister does not destroy surviving content');
  const host=document.querySelector('.workbench')!.getBoundingClientRect();const before=panel('notes').style.cssText;const handle=panel('notes').querySelector('.title-handle')!;const r=handle.getBoundingClientRect();
  handle.dispatchEvent(new PointerEvent('pointerdown',{pointerId:71,button:0,clientX:r.x+10,clientY:r.y+10,bubbles:true}));window.dispatchEvent(new PointerEvent('pointermove',{pointerId:71,clientX:host.right-12,clientY:host.bottom-10}));await settle();assert(!!document.querySelector('[data-drop-index]'),'Drag shows insertion preview');window.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape'}));await settle();assert(panel('notes').style.cssText===before,'Escape restores original dock geometry');assert(!document.querySelector('[data-drop-index]'),'Cancel removes drop preview');
  await select('notes','right');await select('system','right');await select('sftp','right');
  const h=panel('sftp').querySelector('.title-handle')!;const hr=h.getBoundingClientRect();h.dispatchEvent(new PointerEvent('pointerdown',{pointerId:72,button:0,clientX:hr.x+12,clientY:hr.y+10,bubbles:true}));window.dispatchEvent(new PointerEvent('pointermove',{pointerId:72,clientX:host.right-10,clientY:host.bottom-10}));await settle();const index=Number(document.querySelector<HTMLElement>('[data-drop-index]')?.dataset.dropIndex);window.dispatchEvent(new PointerEvent('pointerup',{pointerId:72}));await settle();assert(index>0,'Drag accepts non-first insertion boundary');
  await select('notes','float');const floated=panel('notes').style.cssText;assert(!panel('notes').hasAttribute('data-native-view-blocker'),'Float is not a global modal blocker');assert(panel('notes').hasAttribute('data-workbench-float'),'Float exposes independent native handoff marker');for(let attempt=0;attempt<100&&!nativeReady&&!nativeFailure;attempt++)await settle();assert(nativeReady&&!nativeFailure,'Actual NativeTerminalView WebView controller renders successfully');const vt=document.querySelector<HTMLTextAreaElement>('[data-native-session="fixture-vt"] textarea')!;vt.focus();assert(document.activeElement===vt&&!vt.disabled,'NativeTerminalView WebView input stays focusable beneath nonmodal float');vt.dispatchEvent(new KeyboardEvent('keydown',{bubbles:true,cancelable:true,key:'x',code:'KeyX'}));await settle();assert(nativeWrites.includes('x'),'Actual native engine WebView input writes while float is open');hidden=true;await settle();hidden=false;await settle();assert(panel('notes').style.cssText===floated,'Ancestor display:none preserves floating geometry');
  await select('ai:tab-one','left');const lateStyle=panel('ai:tab-one').style.cssText;extra=[];await settle();await select('notes','bottom');extra=[{id:'ai:tab-one',type:'ai',title:'AI tab one',content:aiContent,visible:true}];await settle();assert(panel('ai:tab-one').offsetLeft===0&&panel('ai:tab-one').offsetWidth===Number.parseFloat(lateStyle.match(/width: ([0-9.]+)px/)?.[1]??'0'),'Late reregistration restores dormant placement');
  await select('notes','right');await select('system','right');await select('sftp','right');short=true;await settle();for(const id of ['notes','system','sftp'])assert(panel(id).getBoundingClientRect().height>=160,`${id} retains useful minimum height in short viewport`);assert(document.querySelector('.workbench')!.scrollHeight>120,'Short workbench scrolls rather than crushing panel content');short=false;await settle();
  assert(events.filter(e=>e.startsWith('destroy:')&&!e.endsWith('ai-one')).length===0,'No terminal/notes/SFTP/AI survivor component destroyed during layout changes');
  assert(events.filter(e=>e==='mount:notes').length===1&&events.filter(e=>e==='mount:sftp').length===1,'Real Notes editor and SFTP queue mounted once');
 }catch(e){failed=String(e);}finally{(window as unknown as {workbenchHarness:unknown}).workbenchHarness={done:true,failed,checks:result,events};}}
 onMount(()=>{void run();});
</script>
{#snippet aiContent()}<LifetimeProbe id="ai-one" {events}><input aria-label="Mock AI input" /></LifetimeProbe>{/snippet}
{#snippet aiContentTwo()}<input aria-label="Mock AI two" />{/snippet}
<div class="harness-workbench" style={`height:${short?120:760}px;display:${hidden?'none':'flex'};background:var(--color-bg);color:var(--color-fg)`}>
<Workbench storageKey={`aerotab.harness.${Date.now()}`} sftpVisible notesVisible systemVisible extraPanels={extra}>
 {#snippet terminal()}<LifetimeProbe id="terminal" {events}><div style="position:relative;width:100%;height:100%"><NativeTerminalView rpc={nativeRpc} sessionId="fixture-vt" backend="webview" renderer="canvas2d" cursorBlink={false} onReady={()=>nativeReady=true} onError={msg=>nativeFailure=msg} writeInput={data=>{nativeWrites.push(data);}} /></div><pre data-terminal-fixture>Mock terminal scrollback retained</pre></LifetimeProbe>{/snippet}
 {#snippet sftp()}<LifetimeProbe id="sftp" {events}><SftpTransferQueue {tasks} variant="full" onPause={()=>{}} /></LifetimeProbe>{/snippet}
 {#snippet notes()}<LifetimeProbe id="notes" {events}><NoteEditor {store} {note} onError={()=>{}} /></LifetimeProbe>{/snippet}
 {#snippet system()}<p>Mock system stats</p>{/snippet}
</Workbench>
</div>
<div class="nav-fixture" style="display:flex;height:200px"><PrimaryNav page="sessions" expanded sessionCount={1} onNavigate={()=>{}} onToggle={()=>{}} onSettings={()=>{}} onAppearance={()=>{}} /></div>
<output data-harness-result data-failed={failed}>{failed||`${result.length} browser assertions passed`}</output>
