<script lang="ts">
 import AIChat from '../src/components/AIChat.svelte';
 import AISection from '../src/components/settings/sections/AISection.svelte';
 import {i18n} from '../src/lib/i18n.svelte';
 import type {RpcClient} from '../src/lib/rpc';
 let settings=$state(false),error=$state('');
 const provider={id:'fixture',name:'Local test provider',provider:'custom',apiBase:'https://example.invalid/v1',model:'test-model',enabled:true,hasKey:true};
 let polled=false;
 const budgetFixture=typeof window!=='undefined'&&new URLSearchParams(window.location.search).has('budget');
 i18n.setLocale('zh-CN');
 const requests:{method:string;params:unknown}[]=[];
 if(typeof window!=='undefined')Object.assign(window,{aiHarness:{requests}});
 const rpc={async call(method:string,params?:any):Promise<any>{
  requests.push({method,params});
  if(method==='ai.providers.list')return {providers:[provider],defaultProviderId:'fixture'};
  if(method==='ai.providers.models')return {models:['test-model','other-model']};
  if(method==='ai.providers.test')return {ok:true,message:'Fixture connection verified'};
  if(method==='ai.providers.save')return {...params.provider,hasKey:true};
  if(method==='ai.providers.default'||method==='ai.providers.remove')return null;
  if(method==='ai.config.get')return {config:{enabled:true,provider:'custom',apiBase:provider.apiBase,model:provider.model,temperature:.3,maxTokens:4096,systemPrompt:'',toolsEnabled:false},hasKey:true};
  if(method==='session.list')return [{id:'terminal-one',title:'root@router',kind:'Ssh'}];
  if(method==='profile.list'||method==='ai.mcp.list')return [];
  if(method==='notes.list')return {notes:[],folders:[],revision:0};
  if(method==='ai.send'){polled=false;return {id:'fixture-generation'};}
  if(method==='ai.poll'){if(budgetFixture&&!polled){polled=true;return {cursor:3,done:true,events:[{type:'notice',code:'terminal_wait_limit',message:'Repeated terminal waits; summarizing observations.'},{type:'delta',text:'### 已完成\n\n已读取最近输出，**尚未确认命令结束**。\n\n连续等待已达到本轮上限，暂不继续调用工具。终端中的命令不会因此停止。\n\n你可以稍后继续询问，或自行检查终端。'},{type:'done'}]};}if(polled)return {cursor:1,done:true,events:[]};polled=true;return {cursor:1,done:true,events:[{type:'delta',text:'### 检查结果\n\n当前终端为 **OpenWrt / ash**。\n\n```sh\npwd; df -h\n```\n\n- 目录：`/root`\n- 磁盘：尚未执行检查，等待真实工具结果。'},{type:'done'}]};}
  if(method==='terminal.exportText')return {text:'root@router:~# ',alive:true};
  if(method==='settings.get')return {value:null};
  return null;
 }} as unknown as RpcClient;
</script>
<div class="fixture"><header><strong>AeroTab · AI 工作区验证</strong><button onclick={()=>settings=!settings}>{settings?'查看对话':'查看设置'}</button><button onclick={()=>i18n.setLocale(i18n.locale==='zh-CN'?'en':'zh-CN')}>中 / EN</button></header><main class:settings>{#if settings}<AISection {rpc} onError={message=>error=message}/>{:else}<AIChat {rpc} activeSessionId="terminal-one" onSettings={()=>settings=true}/>{/if}</main>{#if error}<p>{error}</p>{/if}</div>
<style>:global(body){margin:0;background:var(--color-bg,#171b1f);color:var(--color-fg,#e0e4e8)}.fixture{height:100vh;display:flex;flex-direction:column;max-width:1080px;margin:auto;border-inline:1px solid var(--color-border,#353c43)}header{padding:10px 16px;display:flex;align-items:center;gap:16px;border-bottom:1px solid var(--color-border,#353c43);font-size:13px}header strong{flex:1}button{background:var(--color-bg-soft,#252b31);color:inherit;border:1px solid var(--color-border,#353c43);border-radius:5px;padding:6px 10px}main{height:calc(100vh - 54px);min-height:0;width:min(390px,100%);margin-left:auto;border-left:1px solid var(--color-border,#353c43)}main.settings{width:100%;overflow:auto}</style>
