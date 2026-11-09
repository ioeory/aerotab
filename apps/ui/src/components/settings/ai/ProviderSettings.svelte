<script lang="ts">
  import {onMount, onDestroy} from 'svelte';
  import type {RpcClient} from '../../../lib/rpc';
  import {i18n} from '../../../lib/i18n.svelte';
  import {settingsCoord} from '../../../lib/settingsStore.svelte';
  import {newProviderDraft, providerDraft, providerDraftChanged, providerSaveParams, providerPresets, validateProviderDraft, type AiProvider, type AiProvidersResult, type ProviderDraft} from '../../../lib/aiProviders';
  let {rpc,onError}:{rpc:RpcClient;onError:(message:string)=>void}=$props();
  const t=(key:string)=>i18n.t(`aiSettings.${key}`);
  let providers=$state<AiProvider[]>([]),defaultId=$state<string|null>(null),draft=$state<ProviderDraft>(newProviderDraft()),models=$state<string[]>([]);
  let busy=$state(false),loaded=$state(false),message=$state(''),removing=$state<string|null>(null),edited=$state(false),pendingSelection=$state<AiProvider|string|null>(null);
  const saved=$derived(providers.find(p=>p.id===draft.id)??null);
  const changed=$derived(providerDraftChanged(draft,saved));
  function dirty(){edited=true;message='';settingsCoord.markDirty('ai-providers');}
  async function refresh(){const result=await rpc.call<AiProvidersResult>('ai.providers.list');providers=result.providers;defaultId=result.defaultProviderId;loaded=true;}
  async function run(action:()=>Promise<unknown>){if(busy)return;busy=true;message='';try{await action();}catch{message=t('failed');onError(message);}finally{busy=false;}}
  function select(p:AiProvider){draft=providerDraft(p);edited=false;models=[];message='';settingsCoord.markClean('ai-providers');}
  function add(preset='openai'){draft=newProviderDraft(preset);edited=false;models=[];message='';settingsCoord.markClean('ai-providers');}
  function requestSelection(next:AiProvider|string){if(edited){pendingSelection=next;return;}if(typeof next==='string')add(next);else select(next);}
  function discard(){const next=pendingSelection;pendingSelection=null;if(typeof next==='string')add(next);else if(next)select(next);}
  async function save(){
    const error=validateProviderDraft(draft);if(error)throw Error(t(error));
    const result=await rpc.call<AiProvider>('ai.providers.save',providerSaveParams(draft));
    draft.key='';draft.makeDefault=false;
    await refresh();select(result);message=t('saved');
    settingsCoord.bumpRev();
  }
  async function saveDraft(){const error=validateProviderDraft(draft);if(error){message=t(error);return;}await run(()=>settingsCoord.saveSection('ai-providers'));}
  onMount(()=>{settingsCoord.registerSaver('ai-providers',save);void run(refresh);});
  onDestroy(()=>{draft.key='';settingsCoord.unregisterSaver('ai-providers');});
</script>
{#if pendingSelection}<div class="draft-confirm" role="alert"><p>{t('discardHint')}</p><button onclick={discard}>{t('discard')}</button><button onclick={()=>pendingSelection=null}>{t('cancel')}</button></div>{/if}
<div class="provider-layout">
  <div class="provider-list">
    <div class="list-heading"><h3>{t('providers')}</h3><button disabled={busy} onclick={()=>requestSelection('openai')}>+ {t('add')}</button></div>
    {#if !loaded}<button disabled={busy} onclick={()=>run(refresh)}>{i18n.t('ai.refresh')}</button>{/if}
    {#if loaded&&!providers.length}<p class="empty">{t('empty')}</p>{/if}
    {#each providers as p(p.id)}
      <article class:active={draft.id===p.id}>
        <button class="provider-select" disabled={busy} onclick={()=>requestSelection(p)}><span class="provider-mark">{p.name.slice(0,2).toUpperCase()}</span><span class="provider-title"><strong>{p.name}</strong><small>{p.model}</small></span></button>
        <div class="badges">{#if defaultId===p.id}<span class="badge">{t('default')}</span>{/if}<span class="muted">{t(p.enabled?'enabled':'disabled')}</span></div>
        <div class="card-actions"><button disabled={busy||(edited&&draft.id===p.id)} onclick={()=>run(async()=>{await rpc.call('ai.providers.save',{provider:{id:p.id,name:p.name,provider:p.provider,apiBase:p.apiBase,model:p.model,enabled:!p.enabled}});await refresh();if(draft.id===p.id)select(providers.find(x=>x.id===p.id)!);settingsCoord.bumpRev();})}>{t(p.enabled?'disabled':'enabled')}</button><button disabled={busy||!p.enabled||defaultId===p.id} onclick={()=>run(async()=>{await rpc.call('ai.providers.default',{id:p.id});await refresh();settingsCoord.bumpRev();})}>{t('setDefault')}</button><button disabled={busy} onclick={()=>removing=p.id}>{t('remove')}</button></div>
        {#if removing===p.id}<div class="confirm" role="alert"><p>{t('removeConfirm')}</p><button disabled={busy} onclick={()=>run(async()=>{await rpc.call('ai.providers.remove',{id:p.id});await refresh();if(draft.id===p.id)add();removing=null;settingsCoord.bumpRev();})}>{t('remove')}</button><button disabled={busy} onclick={()=>removing=null}>{t('cancel')}</button></div>{/if}
      </article>
    {/each}
  </div>
  <form class="editor" onsubmit={(event)=>{event.preventDefault();void saveDraft();}}>
    <fieldset disabled={busy||!loaded}>
      <div class="editor-heading"><h3>{t(saved?'edit':'add')}</h3>{#if saved}<span class="badge">{t(saved.hasKey?'keySaved':'noKey')}</span>{/if}</div>
      {#if !saved}<label>{t('preset')}<select onchange={(event)=>requestSelection(event.currentTarget.value)}><option value="" disabled selected>{t('preset')}</option>{#each providerPresets as p}<option value={p.id}>{p.name}</option>{/each}</select></label>{/if}
      <label>{t('name')}<input required bind:value={draft.name} oninput={dirty}/></label>
      <label>{t('protocol')}<select bind:value={draft.provider} onchange={dirty}><option value="openai">OpenAI</option><option value="anthropic">Anthropic</option><option value="ollama">Ollama · OpenAI /v1</option><option value="custom">{i18n.t('ai.custom')}</option></select></label>
      <label>{i18n.t('ai.base')}<input type="url" required bind:value={draft.apiBase} oninput={dirty} spellcheck="false"/></label>
      <label>{t('key')}<input type="password" bind:value={draft.key} oninput={dirty} autocomplete="new-password" spellcheck="false" placeholder={saved?.hasKey?t('keySaved'):''}/></label><p class="hint">{t('keyHint')}</p>
      <label>{t('availableModels')}<select value={models.includes(draft.model)?draft.model:''} disabled={!models.length} onchange={(event)=>{draft.model=event.currentTarget.value;dirty();}}><option value="" disabled>{t(models.length?'chooseModel':'refreshModelsHint')}</option>{#each models as model}<option value={model}>{model}</option>{/each}</select></label>
      <label>{t('manualModel')}<input required bind:value={draft.model} oninput={dirty} spellcheck="false"/></label>
      <div class="checks"><label><input type="checkbox" bind:checked={draft.enabled} onchange={dirty}/>{t('enabled')}</label><label><input type="checkbox" bind:checked={draft.makeDefault} onchange={dirty} disabled={!draft.enabled}/>{t('setDefault')}</label></div>
      <p class="hint">{t('saveFirst')}</p>
      <div class="actions"><button class="primary" type="submit">{t('save')}</button><button type="button" disabled={changed} onclick={()=>run(async()=>{const result=await rpc.call<{models:string[]}>('ai.providers.models',{id:draft.id});models=result.models;if(!models.length)message=t('noModels');})}>{t('refreshModels')}</button><button type="button" disabled={changed} onclick={()=>run(async()=>{const result=await rpc.call<{ok:boolean;message:string}>('ai.providers.test',{id:draft.id});message=t(result.ok?'testOk':'testFailed');})}>{t('test')}</button></div>
    </fieldset>
    <p class="feedback" role="status" aria-live="polite">{message}</p>
  </form>
</div>
<style>
  .draft-confirm{border:1px solid var(--color-accent);background:var(--color-bg-soft);border-radius:8px;padding:12px;margin-bottom:14px;font-size:12px}.draft-confirm button{margin-right:6px}.provider-layout{display:grid;grid-template-columns:minmax(200px,.85fr) minmax(260px,1.15fr);gap:18px;align-items:start}.list-heading,.editor-heading{display:flex;align-items:center;justify-content:space-between;gap:8px;margin-bottom:14px}h3{margin:0;font-size:13px}button{cursor:pointer}article,.editor{border:1px solid var(--color-border);border-radius:10px;padding:14px;background:var(--color-bg-soft)}article{margin-bottom:10px}article.active{border-color:var(--color-accent);box-shadow:inset 3px 0 var(--color-accent)}.provider-select{display:flex;align-items:center;gap:10px;border:0;background:transparent;padding:0;width:100%;text-align:left}.provider-mark{width:32px;height:32px;display:grid;place-items:center;background:var(--color-bg);border:1px solid var(--color-border);border-radius:8px;font-size:11px;font-weight:700}.provider-title{min-width:0;display:grid;gap:4px}.provider-title strong,.provider-title small{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.provider-title small,.muted,.hint,.empty{color:var(--color-fg-muted);font-size:11px}.badges{display:flex;gap:8px;align-items:center;margin:12px 0}.badge{font-size:10px;border:1px solid var(--color-border);border-radius:5px;padding:2px 6px;color:var(--color-accent)}.card-actions,.actions{display:flex;flex-wrap:wrap;gap:6px}.card-actions button{font-size:10px;padding:4px 7px}fieldset{border:0;padding:0;margin:0;min-width:0}label{display:grid;gap:6px;margin:12px 0;font-size:11px;font-weight:500}input,select{width:100%;box-sizing:border-box}input,select,button{font:inherit;color:var(--color-fg);background:var(--color-bg);border:1px solid var(--color-border);border-radius:5px;padding:7px 9px;font-size:11px}input:focus-visible,select:focus-visible,button:focus-visible{outline:2px solid var(--color-accent);outline-offset:2px}.checks{display:flex;gap:18px;flex-wrap:wrap}.checks label{display:flex;align-items:center;gap:7px}.checks input{width:auto;accent-color:var(--color-accent)}.hint{line-height:1.6;margin:6px 0 12px}.primary{background:var(--color-accent);color:var(--color-accent-fg,var(--color-bg));border-color:var(--color-accent)}button:disabled{opacity:.45;cursor:not-allowed}.feedback{font-size:11px;line-height:1.6;overflow-wrap:anywhere;min-height:16px}.confirm{border-top:1px solid var(--color-border);margin-top:10px;font-size:11px}.empty{padding:24px 8px;line-height:1.8}@media(max-width:720px){.provider-layout{grid-template-columns:1fr}.provider-list{display:grid;grid-template-columns:repeat(auto-fit,minmax(200px,1fr));gap:10px}.list-heading{grid-column:1/-1}article{margin:0}}
  .provider-layout{font-size:14px;gap:24px}h3{font-size:15px}.provider-title strong{font-size:14px}.provider-title small,.muted,.hint,.empty{font-size:13px;color:var(--color-fg)}label,input,select,button,.feedback{font-size:14px}.card-actions button{font-size:12px;min-height:32px}.badge{font-size:12px}.editor{border:0;background:transparent;padding:0 4px}article{border-color:var(--color-border-soft);box-shadow:none}.checks{gap:14px}
</style>
