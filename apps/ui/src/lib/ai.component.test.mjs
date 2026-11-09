import test from 'node:test';
import assert from 'node:assert/strict';
import {createServer} from 'vite';
import {svelte} from '@sveltejs/vite-plugin-svelte';
test('assistant shell is compact, translated, inert during SSR and keeps advanced context closed',async()=>{
 const server=await createServer({cacheDir:'node_modules/.vite-ai-component-test',configFile:false,optimizeDeps:{noDiscovery:true,include:[]},ssr:{optimizeDeps:{noDiscovery:true,include:[]}},server:{middlewareMode:true,hmr:false,ws:false},plugins:[svelte()]});
 const calls=[];const rpc={call:async(method)=>{calls.push(method);throw Error('render must not execute RPC');}};
 try{const {render}=await server.ssrLoadModule('svelte/server');const {i18n}=await server.ssrLoadModule('/src/lib/i18n.svelte.ts');const {default:Chat}=await server.ssrLoadModule('/src/components/AIChat.svelte');
 for(const [locale,title,confirm] of [['en','AI assistant','Confirm'],['zh-CN','AI 助手','确认']]){i18n.setLocale(locale);const body=render(Chat,{props:{rpc,onSettings(){}}}).body;assert.ok(body.includes(title));assert.ok(body.includes(confirm));assert.match(body,/role="log"/);assert.match(body,/data-advanced/);assert.ok(!body.includes('<details open'));assert.ok(!body.includes('Every input and interrupt'));assert.ok(body.includes('disabled'));}
 i18n.setLocale('system');assert.equal(calls.length,0);
 }finally{await server.close();}
});
