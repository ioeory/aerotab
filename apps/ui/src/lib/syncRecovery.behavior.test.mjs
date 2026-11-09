import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';

const source=readFileSync(new URL('../components/settings/sections/ConfigSyncSection.svelte',import.meta.url),'utf8');
function harness(){
  const start=source.indexOf('  let recoveryArchive =');
  const script=source.slice(start,source.indexOf('  let peers =',start));
  const calls=[],downloads=[],urls=[],notices=[];
  let fail=false,confirm=true;
  const sandbox={Uint8Array,Blob,atob,$state:v=>v,busy:false,firstSyncApproved:true,previewAccepted:true,previewToken:'old',rpc:{async call(method,params){calls.push({method,params});if(fail)throw new Error('fixture error');if(method==='sync.recoveryInspect')return {available:true,encrypted:true,groups:['Credentials'],records:2};if(method==='sync.recoveryExport')return {available:true,encrypted:true,bundle:btoa('ciphertext-only')};return {restored:true};}},appConfirm:async()=>confirm,i18n:{t:k=>k},refreshStatus:async()=>{},setSyncInfo:(...args)=>notices.push(args),URL:{createObjectURL(blob){urls.push(blob);return 'blob:fixture';},revokeObjectURL(url){urls.push(url);}},document:{createElement(){const link={click(){downloads.push({href:link.href,download:link.download});}};return link;}}};
  vm.createContext(sandbox);
  vm.runInContext(ts.transpileModule(script+`\nglobalThis.api={inspectRecovery,exportRecovery,restoreRecovery,setPhrase:v=>recoveryConfirm=v,get:()=>({archive:recoveryArchive,error:recoveryError,approved:firstSyncApproved,accepted:previewAccepted,token:previewToken})};`,{compilerOptions:{target:99,module:99}}).outputText,sandbox);
  return {api:sandbox.api,calls,downloads,urls,setFail:v=>fail=v,setConfirm:v=>confirm=v};
}
test('actual recovery actions inspect safe summary and retain archive on refresh error',async()=>{const h=harness();await h.api.inspectRecovery();assert.equal(h.calls[0].method,'sync.recoveryInspect');assert.equal(h.api.get().archive.records,2);h.setFail(true);await h.api.inspectRecovery();assert.equal(h.api.get().archive.available,true);assert.equal(h.api.get().error,'fixture error');});
test('actual export downloads ciphertext bytes only and revokes URL',async()=>{const h=harness();await h.api.exportRecovery();assert.equal(h.calls[0].method,'sync.recoveryExport');assert.equal(await h.urls[0].text(),'ciphertext-only');assert.equal(h.downloads[0].download,'aerotab-sync-recovery.bin');assert.equal(h.urls[1],'blob:fixture');});
test('actual restore enforces phrase and confirmation then invalidates frontend consent',async()=>{const h=harness();await h.api.restoreRecovery();assert.equal(h.calls.length,0);h.api.setPhrase('RESTORE LOCAL RECOVERY');h.setConfirm(false);await h.api.restoreRecovery();assert.equal(h.calls.length,0);h.setConfirm(true);await h.api.restoreRecovery();assert.equal(h.calls[0].method,'sync.recoveryRestore');assert.equal(h.calls[0].params.confirm,'RESTORE LOCAL RECOVERY');assert.equal(h.api.get().approved,false);assert.equal(h.api.get().accepted,false);assert.equal(h.api.get().token,null);assert.equal(h.calls[1].method,'sync.recoveryInspect');});
test('cancel and status refresh discover archives; restore failure does not hide them',async()=>{assert.match(source,/if\(configured\) \{ await inspectMaintenance\(\); await inspectRecovery\(\); \}/);const cancel=source.slice(source.indexOf('async function cancelMaintenance'),source.indexOf('  let recoveryArchive'));assert.match(cancel,/await refreshStatus\(\)/);const h=harness();await h.api.inspectRecovery();h.api.setPhrase('RESTORE LOCAL RECOVERY');h.setFail(true);await h.api.restoreRecovery();assert.equal(h.api.get().archive.available,true);assert.equal(h.api.get().approved,true);assert.equal(h.api.get().error,'fixture error');});
