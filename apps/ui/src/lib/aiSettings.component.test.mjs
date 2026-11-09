import test from 'node:test';
import assert from 'node:assert/strict';
import {createServer} from 'vite';
import {svelte} from '@sveltejs/vite-plugin-svelte';
test('settings and provider editor render bilingual navigation and saved-only secret-safe controls without network',async()=>{
 const server=await createServer({configFile:false,cacheDir:'node_modules/.vite-ai-settings-test',plugins:[svelte()],optimizeDeps:{noDiscovery:true,include:[]},ssr:{optimizeDeps:{noDiscovery:true,include:[]}},server:{middlewareMode:true,hmr:false,ws:false}});
 const rpc={call:()=>{throw Error('no network during render');}};
 try {
  const {render}=await server.ssrLoadModule('svelte/server');
  const {i18n}=await server.ssrLoadModule('/src/lib/i18n.svelte.ts');
  const {default:Section}=await server.ssrLoadModule('/src/components/settings/sections/AISection.svelte');
  const {default:Providers}=await server.ssrLoadModule('/src/components/settings/ai/ProviderSettings.svelte');
  const {default:Safety}=await server.ssrLoadModule('/src/components/settings/ai/AiSafety.svelte');
  for(const [locale,tabs,saveFirst] of [['en',['Providers','Assistant','Tools / MCP','Safety'],'Save changes first'],['zh-CN',['提供商','助手','工具 / MCP','安全'],'请先保存更改']]){
   i18n.setLocale(locale);
   const body=render(Section,{props:{rpc,onError:()=>{}}}).body;
   tabs.forEach(t=>assert.ok(body.includes(t)));assert.ok(body.includes('role="tablist"'));
   assert.ok(body.includes('32,768'));assert.ok(body.includes(locale==='en'?'not the context window':'不是上下文窗口'));
   for(const label of locale==='en'?['Maximum model rounds','Maximum tool calls','Consecutive wait limit']:['最大模型轮数','最大工具调用次数','连续等待上限'])assert.ok(body.includes(label),`missing budget setting: ${label}`);
   for(const [min,max,value] of [[2,100,12],[1,200,30],[1,30,3]])assert.ok((body.match(/<input\b[^>]*>/g)??[]).some(input=>[`min="${min}"`,`max="${max}"`,'step="1"','required',`value="${value}"`].every(attribute=>input.includes(attribute))),`missing constrained input for ${value}: ${body.match(/<input\b[^>]*>/g)}`);
   const editor=render(Providers,{props:{rpc,onError:()=>{}}}).body;
   assert.ok(editor.includes('type="password"'));assert.ok(editor.includes(saveFirst));assert.ok(!editor.includes('localStorage'));
   assert.ok(editor.includes(locale==='en'?'Available models':'可选模型'));
   assert.ok(!editor.includes('<datalist'), 'native datalist filters away other models after selection');
   assert.ok(!editor.includes('list="ai-provider-models"'));
   const safety=render(Safety).body;assert.ok(safety.includes('Observer')||safety.includes('观察者'));assert.ok(!safety.includes('type="checkbox"'));
  }
 } finally {await server.close();}
});
