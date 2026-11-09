import test from 'node:test';
import assert from 'node:assert/strict';
import {createServer} from 'vite';
import {svelte} from '@sveltejs/vite-plugin-svelte';
const server=await createServer({cacheDir:'node_modules/.vite-ai-chat-test',configFile:false,optimizeDeps:{noDiscovery:true,include:[]},ssr:{optimizeDeps:{noDiscovery:true,include:[]}},server:{middlewareMode:true,hmr:false,ws:false},plugins:[svelte()]});
test.after(()=>server.close());
test('activity is grouped per actual call, preserves delta order and denied state',async()=>{
 const {appendAiEvent}=await server.ssrLoadModule('/src/lib/ai.ts');
 const message={role:'assistant',content:''};
 const call={id:'one',name:'terminal_input',arguments:{text:'pwd\n'}};
 for(const event of [{type:'delta',text:'Before'},{type:'tool',call},{type:'approval',approvalId:'a',call},{type:'result',callId:'one',allowed:false,result:{error:'denied'}},{type:'delta',text:'After'},{type:'tool',call:{...call,id:'two'}}])appendAiEvent(message,event);
 assert.deepEqual(message.parts.map(p=>p.type),['text','activity','text','activity']);
 assert.equal(message.parts[1].state,'denied');assert.equal(message.parts[1].approval.approvalId,'a');assert.equal(message.parts[3].state,'running');
 assert.equal(message.content,'BeforeAfter');
});
test('built-in tool summaries are friendly and details stay closed except pending approval',async()=>{const {render}=await server.ssrLoadModule('svelte/server');const {default:Activity}=await server.ssrLoadModule('/src/components/ai/AiToolActivity.svelte');for(const [name,label] of [['terminal_read','Read terminal'],['terminal_wait','Wait for output'],['terminal_input','Run command']]){const {body}=render(Activity,{props:{activity:{type:'activity',call:{id:'c',name,arguments:{text:'pwd'}},state:'completed',result:'output'},onApprove(){}}});assert.ok(body.includes(label));assert.ok(!body.includes(name));assert.ok(!body.includes('<details open'));assert.ok(!body.includes('Approve once'));}const {body}=render(Activity,{props:{activity:{type:'activity',call:{id:'c',name:'terminal_input',arguments:{text:'pwd'}},state:'approval'},onApprove(){}}});assert.match(body,/<details open/);assert.ok(body.includes('Approve once'));assert.ok(body.includes('Deny'));});
test('safe markdown renders formatting, inert raw HTML, safe links and copyable fenced code',async()=>{
 const {render}=await server.ssrLoadModule('svelte/server');
 const {default:Markdown}=await server.ssrLoadModule('/src/components/ai/AiMarkdown.svelte');
 const {body}=render(Markdown,{props:{text:'## Answer\n\n**bold** [bad](javascript:alert(1))\n\n<img src=x onerror=alert(1)>\n\n```sh\npwd\n```'}});
 assert.match(body,/<h2/);assert.match(body,/<strong/);assert.ok(!body.includes('<img'));assert.ok(!body.includes('href="javascript:'));assert.ok(body.includes('Copy'));assert.ok(body.includes('pwd'));
});
test('compact shell has composer selectors, advanced context closed, no default permission wall',async()=>{
 const {render}=await server.ssrLoadModule('svelte/server');const {default:Chat}=await server.ssrLoadModule('/src/components/AIChat.svelte');
 const {body}=render(Chat,{props:{rpc:{call(){throw Error('SSR must be inert');}},onSettings(){}}});
 assert.match(body,/class="composer[ "]/);assert.match(body,/Observer/);assert.match(body,/Confirm/);assert.match(body,/Auto/);assert.match(body,/data-advanced/);assert.ok(!body.includes('<details open'));assert.ok(!body.includes('Always allow'));
});
