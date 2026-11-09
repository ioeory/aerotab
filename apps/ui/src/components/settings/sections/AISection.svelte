<script lang="ts">
  import {onMount,onDestroy} from 'svelte';
  import type {RpcClient} from '../../../lib/rpc';
  import {settingsCoord} from '../../../lib/settingsStore.svelte';
  import {i18n} from '../../../lib/i18n.svelte';
  import {defaultAiConfig,type AiConfig} from '../../../lib/ai';
  import ProviderSettings from '../ai/ProviderSettings.svelte';
  import McpClientSettings from '../ai/McpClientSettings.svelte';
  import AiSafety from '../ai/AiSafety.svelte';
  let {rpc,onError}:{rpc:RpcClient;onError:(msg:string)=>void}=$props();
  const tabs=['providers','assistant','tools','safety'] as const;
  let tab=$state<(typeof tabs)[number]>('providers'),config=$state<AiConfig>({...defaultAiConfig}),loaded=$state(false),busy=$state(false),status=$state('');
  const t=(key:string)=>i18n.t(`ai.${key}`);
  const s=(key:string)=>i18n.t(`aiSettings.${key}`);
  function dirty(){status='';settingsCoord.markDirty('ai');}
  async function execute(action:()=>Promise<unknown>){if(busy)return;busy=true;try{await action();}catch{status=s('failed');onError(status);}finally{busy=false;}}
  async function save(){
    if(!loaded)throw Error('AI configuration not loaded');
    // Preserve the latest legacy provider fields: saving global options must not
    // overwrite a provider switched by ai.providers.default in another panel.
    const current=await rpc.call<{config:AiConfig}>('ai.config.get');
    await rpc.call('ai.config.set',{...current.config,enabled:config.enabled,temperature:config.temperature,maxTokens:config.maxTokens,maxRounds:config.maxRounds,maxCalls:config.maxCalls,maxConsecutiveWaits:config.maxConsecutiveWaits,systemPrompt:config.systemPrompt,toolsEnabled:config.toolsEnabled});
    status=s('assistantSaved');
  }
  function tabKey(event:KeyboardEvent){
    const index=tabs.indexOf(tab);let next=index;
    if(event.key==='ArrowRight')next=(index+1)%tabs.length;
    else if(event.key==='ArrowLeft')next=(index+tabs.length-1)%tabs.length;
    else if(event.key==='Home')next=0;
    else if(event.key==='End')next=tabs.length-1;
    else return;
    event.preventDefault();tab=tabs[next]!;document.getElementById(`ai-settings-tab-${tab}`)?.focus();
  }
  onMount(()=>{settingsCoord.registerSaver('ai',save);void execute(async()=>{const result=await rpc.call<{config:AiConfig}>('ai.config.get');config={...defaultAiConfig,...result.config};loaded=true;});});
  onDestroy(()=>settingsCoord.unregisterSaver('ai'));
</script>
<div class="settings-section ai-settings">
  <header class="section-heading"><div><h2>{t('title')}</h2><p>{s('subtitle')}</p></div><label class="enable"><span>{t('enabled')}</span><input type="checkbox" bind:checked={config.enabled} onchange={dirty} disabled={!loaded||busy}/></label></header>
  <div class="tabs" role="tablist" tabindex="-1" aria-label={t('title')} onkeydown={tabKey}>
    {#each tabs as item}<button type="button" id={`ai-settings-tab-${item}`} role="tab" aria-selected={tab===item} aria-controls={`ai-settings-panel-${item}`} tabindex={tab===item?0:-1} onclick={()=>tab=item}>{s(item)}</button>{/each}
  </div>
  <div id="ai-settings-panel-providers" role="tabpanel" aria-labelledby="ai-settings-tab-providers" hidden={tab!=='providers'} tabindex="0"><ProviderSettings {rpc} {onError}/></div>
  <div id="ai-settings-panel-assistant" role="tabpanel" aria-labelledby="ai-settings-tab-assistant" hidden={tab!=='assistant'} tabindex="0">
    <form class="assistant-card" onsubmit={(event)=>{event.preventDefault();void execute(()=>settingsCoord.saveSection('ai'));}}>
      <p>{s('assistantHint')}</p><fieldset disabled={!loaded||busy}>
      <label class="stack">{t('system')}<textarea rows="6" bind:value={config.systemPrompt} oninput={dirty}></textarea></label>
      <details><summary>{s('advanced')}</summary><div class="parameter-grid"><label>{t('temperature')}<input type="number" min="0" max="2" step="0.1" required bind:value={config.temperature} oninput={dirty}/></label><label>{t('maxTokens')}<input type="number" min="1" max="32768" required bind:value={config.maxTokens} oninput={dirty}/></label></div><p class="token-hint">{s('outputLimitHint')}</p>
        <div class="parameter-grid">
          <label>{s('maxRounds')}<input type="number" min="2" max="100" step="1" required bind:value={config.maxRounds} oninput={dirty}/></label>
          <label>{s('maxCalls')}<input type="number" min="1" max="200" step="1" required bind:value={config.maxCalls} oninput={dirty}/></label>
          <label>{s('maxConsecutiveWaits')}<input type="number" min="1" max="30" step="1" required bind:value={config.maxConsecutiveWaits} oninput={dirty}/></label>
        </div><p class="token-hint">{s('toolBudgetHint')}</p></details>
      <button class="primary" type="submit">{s('saveAssistant')}</button></fieldset>
    </form>
  </div>
  <div id="ai-settings-panel-tools" role="tabpanel" aria-labelledby="ai-settings-tab-tools" hidden={tab!=='tools'} tabindex="0">
    <label class="tool-toggle"><input type="checkbox" bind:checked={config.toolsEnabled} onchange={dirty} disabled={!loaded||busy}/><span>{t('tools')}</span></label><McpClientSettings {rpc} {onError}/>
  </div>
  <div id="ai-settings-panel-safety" role="tabpanel" aria-labelledby="ai-settings-tab-safety" hidden={tab!=='safety'} tabindex="0"><AiSafety/></div>
  <p role="status" aria-live="polite">{status}</p>
</div>
<style>
  .ai-settings{max-width:1050px;color:var(--color-fg)}.section-heading{display:flex;align-items:center;justify-content:space-between;gap:16px;margin-bottom:20px}h2{margin:0 0 6px;font-size:20px;letter-spacing:-.4px}.section-heading p,p{font-size:12px;color:var(--color-fg-muted);line-height:1.7;margin:0}.enable{display:flex;align-items:center;gap:10px;font-size:11px;flex-shrink:0}.tabs{display:flex;gap:4px;border-bottom:1px solid var(--color-border);margin-bottom:22px;overflow-x:auto}.tabs button{border:0;border-bottom:2px solid transparent;background:transparent;border-radius:0;padding:11px 16px;color:var(--color-fg-muted);white-space:nowrap}.tabs button[aria-selected=true]{border-bottom-color:var(--color-accent);color:var(--color-accent);font-weight:600}button,input,textarea{font:inherit;color:var(--color-fg);background:var(--color-bg-soft);border:1px solid var(--color-border);border-radius:5px;padding:8px;font-size:12px}button{cursor:pointer}input[type=checkbox]{accent-color:var(--color-accent)}button:disabled{opacity:.45;cursor:not-allowed}:is(button,input,textarea,summary):focus-visible{outline:2px solid var(--color-accent);outline-offset:2px}[role=tabpanel]:focus-visible{outline:1px solid var(--color-accent);outline-offset:4px}fieldset{border:0;padding:0;margin:0;min-width:0}.assistant-card{border:1px solid var(--color-border);border-radius:10px;padding:20px;background:var(--color-bg-soft)}.assistant-card label{display:grid;gap:7px;margin:16px 0;font-size:12px}textarea{resize:vertical;min-width:0;background:var(--color-bg)}details{border-top:1px solid var(--color-border);margin:18px 0;padding-top:14px}summary{font-size:12px;cursor:pointer}.parameter-grid{display:grid;grid-template-columns:1fr 1fr;gap:16px}.parameter-grid input{width:100%;box-sizing:border-box}.primary{background:var(--color-accent);color:var(--color-accent-fg,var(--color-bg));border-color:var(--color-accent)}.tool-toggle{display:flex;align-items:center;gap:9px;margin-bottom:14px;font-size:12px}[role=status]{margin-top:12px}@media(max-width:600px){.section-heading{align-items:flex-start;flex-direction:column}.tabs button{padding:10px}.parameter-grid{grid-template-columns:1fr;gap:0}}
  .ai-settings{font-size:14px}.section-heading p,p{font-size:13px;color:var(--color-fg)}.enable,.assistant-card label,.tool-toggle,summary{font-size:14px}button,input,textarea{font-size:14px}.assistant-card{border:0;border-radius:0;background:transparent;padding:4px 0}.tabs button{font-size:14px;color:var(--color-fg)}.token-hint{max-width:620px;line-height:1.7}
</style>
