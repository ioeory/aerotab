(async()=>{
if(window.__windowsNativeE2eStarted)return;window.__windowsNativeE2eStarted=true;
const inv=window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);
const evidence=[],receipts=[],identities=new Map();let sessionId,generation,phase='startup';
let ordered=false,receiptPromise=Promise.resolve(),tabScenario;
window.__TAURI_INTERNALS__.invoke=async(cmd,args)=>{if(ordered&&cmd==='native_view_update'&&args?.visible===false)await receiptPromise;return inv(cmd,args);};
Object.defineProperty(window,'__aerotabDebugDrawObserver',{value:{causalTrace:true,pasteTrace:true,identity:(s,c,v)=>{const row=identities.get(s)||{controllers:new Set(),canvases:new Set()};row.controllers.add(c);row.canvases.add(v);identities.set(s,row);},receipt:r=>{receipts.push({...r,at:Date.now()});if(ordered)receiptPromise=probe('receipt',{report:r});}}});
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
const wait=async(fn,label)=>{let last;for(let i=0;i<240;i++){try{last=await fn();if(last)return last;}catch(e){last=String(e);}await sleep(50);}throw Error(label+': '+JSON.stringify(last));};
const rpc=async(method,params={})=>{const r=await inv('rpc',{frame:{jsonrpc:'2.0',id:7722,method,params}});if(r.error)throw Error(JSON.stringify(r.error));return r.result;};
const probe=(action,extra={})=>inv('windows_native_e2e',{action,sessionId,generation,...extra});
const host=()=>inv('native_view_state',{sessionId});
const surface=()=>document.querySelector('[data-terminal-view-backend]');
const toggle=()=>[...document.querySelectorAll('button')].find(b=>b.textContent.includes('⇄'));
const ready=kind=>wait(()=>surface()?.dataset.terminalViewBackend===kind&&toggle()&&!toggle().disabled,'production backend '+kind);
const bytes=s=>btoa(unescape(encodeURIComponent(s)));
try{
await ready('webview');const sessions=await rpc('session.list');if(sessions.length!==1)throw Error('isolated single startup session required');sessionId=sessions[0].id;
await rpc('session.write',{id:sessionId,data:bytes("Write-Output ('WINDOWS_' + 'SHELL_READY')\r")});
await wait(async()=>{const r=await rpc('terminal.text',{id:sessionId});return r.text.includes('WINDOWS_SHELL_READY');},'PowerShell ready');
const baseline=await probe('state');evidence.push({phase:'baseline',baseline});
for(let round=0;round<2;round++){
 phase='forward '+round;toggle().click();await ready('native-surface');let state=await wait(async()=>{const s=await host();return s.drawnRevision>=s.revision&&s.drawnOutputSeq!=null&&BigInt(s.drawnOutputSeq)>=BigInt(s.handoffOutputSeq)&&s;},'native actual draw');generation=state.generation;evidence.push({phase,state});
 if(round===0){ // Aggregate regression: rerun input/paste/modal on current production source.
  phase='synthetic Win32 targeted input ACK PTY';await inv('native_view_focus',{sessionId});const before=await probe('state');
  const accepted=await probe('input',{text:"Write-Output ('WINDOWS_' + 'INPUT_ACK_OK')\r"});
  const ack=await wait(async()=>{const s=await host();return s.inputAccepted>state.inputAccepted&&s.inputAck===s.inputAccepted&&s;},'nonzero host ACK');
  evidence.push({phase:'input reserved and acknowledged',synthetic:true,hardware:false,before,accepted,ack,writer:await probe('state')});
  const after=await wait(async()=>{const s=await probe('state');return s.markers===1&&s;},'PTY marker exactly once');evidence.push({phase,synthetic:true,hardware:false,before,accepted,ack,after});
  phase='paste cancel approve duplicate stale and modal';
  const paste="Write-Output ('PASTE_' + 'WINDOWS_OK')\n#"+' acceptance padding'.repeat(16)+'\n';
  await probe('paste',{requestId:501,text:paste});const dialog=await wait(()=>document.querySelector('dialog.app-confirm-dialog[open]'),'paste confirmation');
  const hidden=await wait(async()=>{const s=await host();return !s.visible&&s;},'modal hides actual HWND');
  const blocked=await probe('input',{text:'SHOULD_NOT_WRITE'});if(blocked.inputAccepted!==ack.inputAccepted)throw Error('modal input accepted');
  dialog.querySelectorAll('button')[0].click();await wait(()=>!document.querySelector('dialog.app-confirm-dialog[open]'),'cancel');await sleep(200);
  const cancelled=await probe('state');if(cancelled.writeCount!==after.writeCount)throw Error('cancel wrote');
  await probe('paste',{requestId:502,generation:'00000000-0000-0000-0000-000000000001',text:paste});await sleep(100);if(document.querySelector('dialog[open]'))throw Error('stale paste dialog');
  await probe('paste',{requestId:503,text:paste});const approve=await wait(()=>document.querySelector('dialog.app-confirm-dialog[open]'),'approve dialog');const buttons=approve.querySelectorAll('button');buttons[buttons.length-1].click();
  await wait(()=>!document.querySelector('dialog.app-confirm-dialog[open]'),'approved dialog close');await wait(async()=>{const s=await host();return s.visible&&s;},'actual show');
  const written=await wait(async()=>{const s=await probe('state');return s.writeCount===after.writeCount+1&&s;},'approved original writer');
  await probe('paste',{requestId:503,text:paste});await sleep(200);if((await probe('state')).writeCount!==written.writeCount||document.querySelector('dialog[open]'))throw Error('duplicate paste accepted');
  await inv('native_view_focus',{sessionId});await probe('input',{text:'\r'});const pasted=await wait(async()=>{const s=await probe('state');return s.pasteMarkers===1&&s;},'paste PTY once');evidence.push({phase,hidden,blocked,cancelled,written,pasted});
 }
 if(round===0){
  phase='reverse target failure rollback';const retained=await probe('os-state');await probe('clear-events');await probe('fault');toggle().click();
  await wait(async()=>!(await probe('events')).faultPending,'fault consumed');await ready('native-surface');
  const restored=await probe('os-state');if(restored.hwnd!==retained.hwnd||restored.host.generation!==generation||!restored.osVisible)throw Error('rollback did not retain source HWND');
  const faultEvents=(await probe('events')).events;if(faultEvents.some(e=>e.phase==='before-detach'))throw Error('failed target detached retained source');
  evidence.push({phase,passed:true,retained,restored,faultEvents});
  phase='tab occlusion and return';tabScenario=async()=>{const tabs=[...document.querySelectorAll('[role=tab]')];const original=tabs.find(t=>t.getAttribute('aria-selected')==='true');
  document.querySelector('.tab-new-button').click();const shell=await wait(()=>[...document.querySelectorAll('[role=option]')].find(e=>/PowerShell|cmd\.exe|Command Prompt/.test(e.textContent)),'production shell picker');shell.click();await wait(()=>document.querySelectorAll('[role=tab]').length===tabs.length+1,'new production tab');
  const hidden=await wait(async()=>{const s=await probe('os-state');return !s.osVisible&&!s.host.visible&&s;},'tab HWND hidden');
  const before=await probe('state');const blocked=await probe('input',{text:'HIDDEN_MUST_NOT_WRITE'});const after=await probe('state');if(blocked.inputAccepted!==hidden.host.inputAccepted||after.writeCount!==before.writeCount)throw Error('hidden tab accepted input');
  document.querySelector('[data-tab-drop="'+original.dataset.tabDrop+'"]').dispatchEvent(new PointerEvent('pointerdown',{button:0,bubbles:true}));await ready('native-surface');const shown=await wait(async()=>{const s=await probe('os-state');return s.osVisible&&s.host.visible&&s;},'tab HWND restored');if(shown.hwnd!==hidden.hwnd)throw Error('tab replaced HWND');
  evidence.push({phase:'tab occlusion and return',passed:true,hidden,blocked,shown,before,after});};
 }
 phase='reverse '+round;await inv('native_view_focus',{sessionId});await probe('input',{text:'# boundary\r'});await wait(async()=>{const s=await host();return s.inputAccepted>0&&s.inputAck===s.inputAccepted;},'source accepted ACK before seal');const receiptStart=receipts.length;const source=await host();await probe('clear-events');ordered=true;toggle().click();await ready('webview');
 const receipt=await wait(()=>receipts.slice(receiptStart).find(r=>r.sessionId===sessionId&&r.drawnOutputSeq!=null&&BigInt(r.drawnOutputSeq)>=BigInt(r.appliedOutputSeq)),'production target draw receipt');
 const events=await wait(async()=>{const e=(await probe('events')).events;return e.some(x=>x.phase==='before-detach')&&e;},'actual detach boundary');ordered=false;
 const draw=events.find(e=>e.phase==='target-receipt');const beforeHide=events.find(e=>e.phase==='before-hide');const hidden=events.find(e=>e.phase==='after-hide');const detach=events.find(e=>e.phase==='before-detach');
 if(!draw||!beforeHide||!hidden||!(draw.order<beforeHide.order&&beforeHide.order<hidden.order&&hidden.order<detach.order)||!beforeHide.host.visible||hidden.host.visible||detach.host.inputAccepted!==hidden.host.inputAccepted||detach.host.inputAck!==detach.host.inputAccepted)throw Error('reverse ordered draw/hide/final drain/detach evidence failed');
 evidence.push({phase:'ordered reverse boundaries '+round,passed:true,events,sealAccepted:hidden.host.inputAccepted,finalAck:detach.host.inputAck});
 await wait(async()=>{try{await host();return false;}catch{return true;}},'source detached');
 const current=await probe('state');const identity=identities.get(sessionId);if(current.coreIdentity!==baseline.coreIdentity||current.channelRegistrations!==1||current.attachCount!==1||current.detachOrCloseCount!==0||identity?.controllers.size!==1||identity?.canvases.size!==1)throw Error('single core/controller/channel continuity');
 evidence.push({phase,source,receipt,current,controllerCount:identity.controllers.size,canvasCount:identity.canvases.size,causalTrace:window.__aerotabViewCausalTrace?.snapshot()});
}
phase='tab occlusion and return';toggle().click();await ready('native-surface');generation=(await host()).generation;await tabScenario();
await probe('report',{report:{ok:true,scope:'Windows production UI staged acceptance',syntheticInput:true,hardwareInput:false,evidence,remaining:['hardware keyboard and Chinese IME acceptance excluded from bounded scope'],publicAvailable:true}});
}catch(error){await probe('report',{report:{ok:false,phase,error:String(error),evidence,receipts,causalTrace:window.__aerotabViewCausalTrace?.snapshot(),pasteTrace:window.__aerotabNativePasteTrace?.snapshot(),publicAvailable:false}});}
})();
