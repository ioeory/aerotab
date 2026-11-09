import test from 'node:test';
import assert from 'node:assert/strict';
import {createServer} from 'vite';
import {svelte} from '@sveltejs/vite-plugin-svelte';
test('one target-bound activity contains readable command, copy and exact once/reject controls; denied stays denied',async()=>{
 const server=await createServer({cacheDir:'node_modules/.vite-ai-terminal-test',configFile:false,optimizeDeps:{noDiscovery:true,include:[]},ssr:{optimizeDeps:{noDiscovery:true,include:[]}},server:{middlewareMode:true,hmr:false,ws:false},plugins:[svelte()]});
 try{const {render}=await server.ssrLoadModule('svelte/server');const {default:Activity}=await server.ssrLoadModule('/src/components/ai/AiToolActivity.svelte');const call={id:'c',name:'terminal_input',arguments:{text:'echo safe\n'}};const activity={type:'activity',call,state:'approval',target:{id:'one',title:'Production'},approval:{type:'approval',call,approvalId:'a'}};
 const body=render(Activity,{props:{activity,onApprove(){}}}).body;for(const text of ['Production','one','echo safe','Copy','Approve once','Deny'])assert.ok(body.includes(text));assert.ok(!body.includes('Always'));assert.equal((body.match(/Approve once/g)??[]).length,1);
 const denied=render(Activity,{props:{activity:{...activity,state:'denied'},onApprove(){}}}).body;assert.ok(denied.includes('Denied'));assert.ok(!denied.includes('Approve once'));assert.ok(!denied.includes('Completed'));
 }finally{await server.close();}
});
