import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import ts from 'typescript';
const source=readFileSync(new URL('./aiProviders.ts',import.meta.url),'utf8');
const js=ts.transpileModule(source,{compilerOptions:{target:99,module:99}}).outputText;
const api=await import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);
test('provider presets use supported protocols, separate identities and local Ollama endpoint',()=>{
 const a=api.newProviderDraft('openai'),b=api.newProviderDraft('openai');
 assert.notEqual(a.id,b.id);assert.equal(a.provider,'openai');assert.equal(a.key,'');
 assert.equal(api.newProviderDraft('ollama').apiBase,'http://localhost:11434/v1');
 assert.deepEqual([...new Set(api.providerPresets.map(p=>p.provider))].sort(),['anthropic','custom','ollama','openai']);
});
test('save payload allowlists fields; key goes only in separate keyring argument and empty preserves it',()=>{
 const draft={...api.newProviderDraft('openai'),hasKey:true,key:'sensitive',extra:'never',makeDefault:true};
 const payload=api.providerSaveParams(draft);
 assert.equal(payload.key,'sensitive');assert.equal(payload.makeDefault,true);
 assert.deepEqual(Object.keys(payload.provider).sort(),['apiBase','enabled','id','model','name','provider']);
 assert.ok(!JSON.stringify(payload.provider).includes('sensitive'));
 assert.ok(!('key' in api.providerSaveParams({...draft,key:''})));
});
test('saved provider drafts never hydrate secrets and saved-config actions detect all draft changes',()=>{
 const p={id:'p',name:'One',provider:'custom',apiBase:'https://example.com/v1',model:'m',enabled:true,hasKey:true,key:'never'};
 const draft=api.providerDraft(p);
 assert.equal(draft.key,'');assert.equal(api.providerDraftChanged(draft,p),false);
 for(const patch of [{name:'Two'},{key:'new'},{makeDefault:true},{enabled:false},{model:'other'}])assert.equal(api.providerDraftChanged({...draft,...patch},p),true);
 assert.equal(api.providerDraftChanged(draft,null),true);
});
test('validation rejects credentials in URL and unsupported remote HTTP while allowing loopback Ollama',()=>{
 const d=api.newProviderDraft('openai');assert.equal(api.validateProviderDraft(d),null);
 for(const apiBase of ['not-url','http://remote.example/v1','https://user:pass@example.com/v1','https://example.com/v1?key=secret','https://example.com/v1#token'])assert.equal(api.validateProviderDraft({...d,apiBase}),'invalidBase');
 assert.equal(api.validateProviderDraft({...d,name:' '}),'required');
 assert.equal(api.validateProviderDraft(api.newProviderDraft('ollama')),null);
});
