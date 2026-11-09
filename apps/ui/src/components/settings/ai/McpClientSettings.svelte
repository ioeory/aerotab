<script lang="ts">
  import {onMount,onDestroy} from 'svelte';
  import type {RpcClient} from '../../../lib/rpc';
  import {i18n} from '../../../lib/i18n.svelte';
  import type {McpClientConfig,McpClientEntry,McpCatalog} from '../../../lib/ai';
  let {rpc,onError}:{rpc:RpcClient;onError:(message:string)=>void}=$props();
  const t=(key:string)=>i18n.t(`ai.${key}`);
  const s=(key:string)=>i18n.t(`aiSettings.${key}`);
  let clients=$state<McpClientEntry[]>([]),draft=$state('[]'),tokens=$state<Record<string,string>>({}),catalogs=$state<Record<string,McpCatalog>>({}),busy=$state(false);
  async function execute(action:()=>Promise<unknown>){if(busy)return;busy=true;try{await action();}catch{onError(s('failed'));}finally{busy=false;}}
  async function refresh(){clients=await rpc.call('ai.mcp.list');draft=JSON.stringify(clients.map(c=>c.config),null,2);}
  async function saveClients(){const items=JSON.parse(draft) as McpClientConfig[];await rpc.call('ai.mcp.save',{clients:items});catalogs={};await refresh();}
  onMount(()=>{void execute(refresh);});onDestroy(()=>{tokens={};});
</script>
<div class="mcp-settings">
  <p class="hint">{t('clientSafety')}</p>
  <details class="advanced"><summary>{s('mcpAdvanced')}</summary><label>{t('config')}<textarea rows="8" bind:value={draft} spellcheck="false" disabled={busy}></textarea></label><button disabled={busy} onclick={()=>execute(saveClients)}>{t('import')}</button></details>
  <div class="heading"><h3>{s('integrations')}</h3><button disabled={busy} onclick={()=>execute(refresh)}>{t('refresh')}</button></div>
  {#if !clients.length}<p class="hint">{s('mcpEmpty')}</p>{/if}
  {#each clients as entry(entry.config.id)}
    <article><header><strong>{entry.config.name}</strong><span>{entry.trusted?t('trusted'):t('untrusted')}</span></header>
      <details><summary>{s('configDetails')}</summary><pre>{JSON.stringify(entry.config,null,2)}</pre></details>
      <div class="actions"><button disabled={busy} onclick={()=>execute(async()=>{await rpc.call('ai.mcp.trust',{config:entry.config,allow:!entry.trusted});await refresh();})}>{entry.trusted?t('revoke'):t('trust')}</button>
      <button disabled={busy||!entry.trusted} onclick={()=>execute(async()=>{catalogs[entry.config.id]=await rpc.call('ai.mcp.connect',{id:entry.config.id});})}>{t('connect')}</button>
      <button disabled={busy} onclick={()=>execute(()=>rpc.call('ai.mcp.disconnect',{id:entry.config.id}))}>{t('disconnect')}</button>
      <button disabled={busy||!entry.trusted} onclick={()=>execute(async()=>{catalogs[entry.config.id]=await rpc.call('ai.mcp.discover',{id:entry.config.id});})}>{t('discover')}</button></div>
      <details><summary>{t('auth')}</summary><label>{t('auth')}<input type="password" autocomplete="new-password" bind:value={tokens[entry.config.id]} disabled={busy}/></label><button disabled={busy} onclick={()=>execute(async()=>{await rpc.call('ai.mcp.auth',{id:entry.config.id,key:tokens[entry.config.id]??''});tokens[entry.config.id]='';await refresh();})}>{t('authSave')}</button></details>
      {#if catalogs[entry.config.id]}<pre>{JSON.stringify(catalogs[entry.config.id],null,2)}</pre>{/if}
    </article>
  {/each}
</div>
<style>
  .hint{font-size:12px;line-height:1.7;color:var(--color-fg-muted)}.heading,header{display:flex;justify-content:space-between;align-items:center;gap:12px}.heading{margin:20px 0 12px}h3{font-size:13px;margin:0}article,.advanced{border:1px solid var(--color-border);border-radius:10px;padding:14px;background:var(--color-bg-soft);margin-bottom:12px}header{font-size:12px}header span{font-size:11px;color:var(--color-fg-muted)}details{margin:12px 0}summary{font-size:12px;cursor:pointer}label{display:grid;gap:8px;margin:12px 0;font-size:11px}input,textarea,button{font:inherit;font-size:11px;color:var(--color-fg);background:var(--color-bg);border:1px solid var(--color-border);border-radius:5px;padding:7px 9px}textarea{resize:vertical;font-family:var(--font-mono,monospace)}button{cursor:pointer}button:disabled{opacity:.45;cursor:not-allowed}.actions{display:flex;gap:6px;flex-wrap:wrap;margin:12px 0}pre{white-space:pre-wrap;overflow-wrap:anywhere;max-height:300px;overflow:auto;font-size:11px;line-height:1.6}:is(button,input,textarea,summary):focus-visible{outline:2px solid var(--color-accent);outline-offset:2px}
  .hint,header,header span,summary,label,input,textarea,button{font-size:14px;color:var(--color-fg)}h3{font-size:15px}pre{font-size:13px}article,.advanced{border-color:var(--color-border-soft)}
</style>
