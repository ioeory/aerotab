import test from 'node:test';
import assert from 'node:assert/strict';
import {createServer} from 'vite';
import {svelte} from '@sveltejs/vite-plugin-svelte';
test('real workbench renders stable content hosts with actionable docking controls, not modal windows',async()=>{
 const server=await createServer({cacheDir:'node_modules/.vite-workbench-test',configFile:false,optimizeDeps:{noDiscovery:true,include:[]},ssr:{optimizeDeps:{noDiscovery:true,include:[]}},server:{middlewareMode:true,hmr:false,ws:false},plugins:[svelte()]});
 try{const {default:C}=await server.ssrLoadModule('/src/components/Workbench.svelte');const {render}=await server.ssrLoadModule('svelte/server');const {body}=render(C,{props:{terminal:()=>{},sftp:()=>{},notes:()=>{},system:()=>{},notesVisible:true,systemVisible:true,sftpVisible:true}});assert.equal((body.match(/data-panel-id=/g)||[]).length,3);assert.equal((body.match(/<select/g)||[]).length,3);assert.ok(body.includes('value="float"'));assert.doesNotMatch(body,/aria-modal|<dialog/);}finally{await server.close();}
});
