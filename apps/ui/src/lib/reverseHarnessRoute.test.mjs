import test from 'node:test';
import assert from 'node:assert/strict';
import {createServer} from 'vite';
const server=await createServer({configFile:false,optimizeDeps:{noDiscovery:true,include:[]},server:{middlewareMode:true}});
const {selectClient}=await server.ssrLoadModule('/src/lib/rpc.ts');
await server.close();
test('actual production client bound before harness internal replacement bypasses late snapshot injection',async()=>{
 const calls=[];let injected=false;
 const original=async(cmd,args)=>{calls.push(args.frame.method);return {result:{version:1,outputSeq:'818'}};};
 globalThis.window={__TAURI_INTERNALS__:{invoke:original}};
 const client=selectClient();
 window.__TAURI_INTERNALS__.invoke=async()=>{injected=true;throw Error('injected reverse failure');};
 const envelope=await client.call('terminal.snapshotEnvelope',{id:'s'});
 assert.equal(envelope.outputSeq,'818');assert.equal(injected,false);assert.deepEqual(calls,['terminal.snapshotEnvelope']);
 delete globalThis.window;
});
