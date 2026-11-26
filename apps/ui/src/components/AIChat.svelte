<script lang="ts">
 import {onMount,onDestroy,tick,untrack} from 'svelte';
 import AiMarkdown from './ai/AiMarkdown.svelte';
 import HostSelect from './HostSelect.svelte';
 import { profileChoices } from './HostChoiceList.svelte';
 import AiToolActivity from './ai/AiToolActivity.svelte';
 import AiTroubleshoot from './ai/AiTroubleshoot.svelte';
 import {aiPaneLabel,type AiPaneDirectoryEntry} from '../lib/aiPaneDirectory';
 import {aiTroubleshootMessages} from '../lib/aiTroubleshootMessages';
 import type {TroubleshootSeed} from '../lib/aiTroubleshoot';
 import {appendAiEvent,shouldFollowAiScroll,readableAiNotice,readableAiError,type AiPermissionMode,type AiProviderChoice,type AiConfig} from '../lib/ai';
 import type {RpcClient} from '../lib/rpc';
 import {uuidv4} from '../lib/rpc';
 import {i18n} from '../lib/i18n.svelte';
 import { clampMenuToViewport } from '../lib/contextMenuPosition';
 import { isAiCopyShortcut } from '../lib/hotkeys';
 import {boundedTerminalContext,profileContext,contextBytes,pollAi,expandResourceTemplate,type AiTerminalTarget,type AiContext,type AiActivity,type AiMessage,type AiEvent,type McpClientEntry,type McpCatalog} from '../lib/ai';
 import type {SessionMeta,StoredProfile} from '../lib/types';
 import type {NotesSnapshot} from '../lib/notesCore';

 let copyMenu = $state<{x:number; y:number; text:string} | null>(null);
 let copyButton = $state<HTMLButtonElement>();

 function selectedText(root: HTMLElement, target: EventTarget | null): string {
   if (target instanceof HTMLTextAreaElement || target instanceof HTMLInputElement) {
     const { selectionStart, selectionEnd, value } = target;
     return selectionStart !== null && selectionEnd !== null ? value.slice(selectionStart, selectionEnd) : '';
   }
   const selection = window.getSelection();
   return selection && root.contains(selection.anchorNode) && root.contains(selection.focusNode)
     ? selection.toString() : '';
 }

 function copySelection(text: string) {
   void navigator.clipboard.writeText(text).catch(() => {});
 }

 function onCopyShortcut(event: KeyboardEvent) {
   if (!isAiCopyShortcut(event)) return;
   const text = selectedText(event.currentTarget as HTMLElement, event.target);
   if (!text) return;
   event.preventDefault();
   copySelection(text);
 }

 async function openCopyMenu(event: MouseEvent) {
   const text = selectedText(event.currentTarget as HTMLElement, event.target);
   if (!text) return;
   event.preventDefault();
   const menu = { x: event.clientX, y: event.clientY, text };
   copyMenu = menu;
   await tick();
   if (copyMenu?.text !== text || !copyButton) return;
   Object.assign(copyMenu, clampMenuToViewport(menu.x, menu.y, copyButton));
 }

 function dismissCopyMenu(event: PointerEvent) {
   if (copyMenu && !event.composedPath().includes(copyButton!)) copyMenu = null;
 }
 interface Props {
   rpc:RpcClient;
   onSettings:()=>void;
   activeSessionId?:string;
   paneDirectory?:readonly AiPaneDirectoryEntry[];
   troubleshootSeed?:TroubleshootSeed;
   onAddIncidentEvidence?:(entry:import('../lib/operationsWorkbench').IncidentEvidence)=>boolean;
   panelVisible?:boolean;
   onAiEnabled?:(enabled:boolean)=>void;
   settingsRev?:number;
 }
 let {rpc,onSettings,activeSessionId,paneDirectory,troubleshootSeed,panelVisible=true,onAiEnabled,settingsRev,onAddIncidentEvidence}:Props=$props();
 let troubleshootOpen=$state(false),troubleshootMounted=$state(false);
 let acceptedSeed=$state<TroubleshootSeed>();
 let lastSettingsRev:number|undefined;
 let observedSettingsRev=false;
 function openTroubleshoot(){
   if(running||sending)return false;
   troubleshootMounted=true;troubleshootOpen=true;return true;
 }
 function isNewSeed(seed:TroubleshootSeed|undefined):seed is TroubleshootSeed{
   return !!seed&&(seed.nonce!==acceptedSeed?.nonce||seed.id!==acceptedSeed?.id);
 }
 function acceptTroubleshootSeed(seed:TroubleshootSeed|undefined){
   if(!isNewSeed(seed))return;
   // Keep the legacy chat and its stop control visible until the request settles.
   if(!openTroubleshoot())return;
   acceptedSeed=seed;
 }
 const pendingSeedMessage=()=>i18n.resolvedLocale==='zh-CN'
   ?'终端内容已暂存。当前对话结束或停止后将打开排障视图。'
   :'Terminal text is queued. Troubleshooting will open after this chat finishes or is stopped.';
 if(typeof window!=='undefined'){
   $effect(()=>{if(panelVisible)acceptTroubleshootSeed(troubleshootSeed);});
   $effect(()=>{
     const rev=settingsRev;
     if(!observedSettingsRev){observedSettingsRev=true;lastSettingsRev=rev;return;}
     if(rev===lastSettingsRev)return;
     lastSettingsRev=rev;
     untrack(()=>void execute(loadProviders));
   });
 }
 let terminalTarget=$state<AiTerminalTarget|null>(null),readAuthorized=$state(false),generationTarget=$state<AiTerminalTarget|null>(null);
 let targetInitialized=false;
 let mode=$state<AiPermissionMode>('confirm'),autoApproved=$state(false),autoConfirm=$state(false),permissionChanging=$state(false),advanced=$state(false),bindingOpen=$state(false);
 let providers=$state<AiProviderChoice[]>([]),providerId=$state(''),model=$state(''),models=$state<string[]>([]),enabled=$state<boolean|null>(null);let modelEpoch=0;let pendingSendCompletion:Promise<void>|undefined;let finishPendingSend:(()=>void)|undefined;
 let configEpoch=0;
 let providerDefault:string|null|undefined;
 let destinationReady=$state(false);
 function troubleshootDestination(){
   const provider=destinationReady?providers.find(p=>p.id===providerId):undefined;
   return {providerId:provider?.id??'',model,endpoint:provider?.apiBase??'',enabled:enabled===true};
 }
 async function loadProviders(){
   const token=++configEpoch;
   destinationReady=false;
   const configRequest=Promise.resolve(rpc.call<{config:AiConfig}>('ai.config.get')).then(({config})=>{
     if(destroyed||token!==configEpoch)return;
     enabled=config.enabled;
     onAiEnabled?.(enabled);
   });
   const providerRequest=Promise.resolve(rpc.call<{providers:AiProviderChoice[];defaultProviderId:string|null}>('ai.providers.list')).then(list=>{
     if(destroyed||token!==configEpoch)return;
     const previous=providers.find(p=>p.id===providerId);
     const defaultChanged=providerDefault!==list.defaultProviderId;
     providerDefault=list.defaultProviderId;
     providers=list.providers.filter(p=>p.enabled);
     const retained=providers.find(p=>p.id===providerId);
     const provider=(!defaultChanged&&retained)||providers.find(p=>p.id===providerDefault)||providers[0];
     if(!previous||provider?.id!==previous.id||provider.apiBase!==previous.apiBase||provider.model!==previous.model){
       // The saved model/base are usable before optional catalog discovery finishes.
       void loadProviderModels(provider?.id??'');
     }
   });
   await Promise.all([configRequest,providerRequest]);
   if(!destroyed&&token===configEpoch)destinationReady=true;
 }
 async function selectProvider(id:string){if(destroyed||running||sending)return;await loadProviderModels(id);}
 async function loadProviderModels(id:string){providerId=id;const provider=providers.find(p=>p.id===id);model=provider?.model??'';models=model?[model]:[];const token=++modelEpoch;if(!id)return;try{const r=await rpc.call<{models:string[]}>('ai.providers.models',{id});if(!destroyed&&token===modelEpoch&&providerId===id)models=[...new Set([model,...r.models].filter(Boolean))];}catch(e){if(!destroyed&&token===modelEpoch&&providerId===id)error=String(e);}}
 function selectModel(value:string){if(destroyed||running||sending)return;model=value;}
 async function changeMode(next:AiPermissionMode){if(permissionChanging)return;permissionChanging=true;try{if(running||sending)await stop();if(sending)return;autoApproved=false;autoConfirm=false;if(next==='auto'){mode='confirm';autoConfirm=true;}else mode=next;}catch{}finally{permissionChanging=false;}}
 function confirmAuto(){if(running||sending||!terminalTarget||!readAuthorized)return;mode='auto';autoApproved=true;autoConfirm=false;}
 async function revokeTerminal(){if(permissionChanging)return;permissionChanging=true;try{if(running||sending)await stop();if(sending)return;readAuthorized=false;autoApproved=false;autoConfirm=false;mode='confirm';}catch{}finally{permissionChanging=false;}}
 let conversation=$state(uuidv4()),messages=$state<AiMessage[]>([]),message=$state(''),context=$state<AiContext[]>([]),events=$state<AiEvent[]>([]),generation=$state(''),running=$state(false),error=$state(''),status=$state(''),sessions=$state<SessionMeta[]>([]),profiles=$state<StoredProfile[]>([]),notes=$state<NotesSnapshot>({notes:[],folders:[],revision:0}),kind=$state<'terminal'|'notes'|'profile'>('terminal'),selected=$state(''),lines=$state(100),clients=$state<McpClientEntry[]>([]),selectedClients=$state<string[]>([]),catalogs=$state<Record<string,McpCatalog>>({}),templateArgs=$state<Record<string,string>>({}),promptArgs=$state('{}'),resolved=$state<string[]>([]),approving=$state<string[]>([]);
 let feed=$state<HTMLDivElement>();let followLatest=$state(true);
 function trackScroll(){if(feed)followLatest=shouldFollowAiScroll(feed);}
 async function followFeed(){if(!feed)return;await tick();if(feed&&followLatest)feed.scrollTop=feed.scrollHeight;}
 function jumpLatest(){followLatest=true;void followFeed();}
 let timer:ReturnType<typeof setTimeout>|undefined;let destroyed=false;let cursor=0;let epoch=0;let sending=$state(false);
 const t=(k:string)=>i18n.t(`ai.${k}`);
 const paneText=()=>i18n.resolvedLocale==='zh-CN'
   ?{current:'当前窗格',useCurrent:'绑定当前窗格',unassociated:'未关联窗格的会话',unavailable:'会话不可用'}
   :{current:'Current pane',useCurrent:'Bind current pane',unassociated:'Sessions without a pane',unavailable:'Session unavailable'};
 function paneLabel(target:AiTerminalTarget){return aiPaneLabel(target,paneDirectory??[],i18n.resolvedLocale);}
 function paneTitle(target:AiTerminalTarget){return `${paneLabel(target)} (${target.id})`;}
 function displayActivity(activity:AiActivity){
   return activity.target?{...activity,target:{...activity.target,title:paneLabel(activity.target)}}:activity;
 }
 function currentPane(){
   return paneDirectory?paneDirectory.find(p=>p.active):sessions.find(s=>s.id===activeSessionId);
 }
 function paneChoiceLabel(session:AiTerminalTarget){
   const label=paneLabel(session);
   return currentPane()?.id===session.id?`${label} · ${paneText().current}`:label;
 }
 function terminalChoices(){
   const live=new Map(sessions.map(s=>[s.id,s]));
   const associated=(paneDirectory??[]).flatMap(pane=>{
     const session=live.get(pane.id);
     if(!session)return [];
     live.delete(pane.id);
     return [session];
   });
   return {associated,unassociated:[...live.values()]};
 }
 async function selectCurrentPane(){
   const id=currentPane()?.id;
   if(!id||running||sending||permissionChanging)return;
   await refreshSessions();
   if(currentPane()?.id!==id)return;
   if(!sessions.some(s=>s.id===id))throw Error(t('terminalUnavailable'));
   selectTerminal(id);
 }
 async function execute(fn:()=>Promise<unknown>){try{error='';await fn();}catch(e){error=`${t('failed')}: ${readableAiError((e as Error).message,t)}`;}}
 let sessionRefresh:Promise<void>|undefined;
 async function refreshSessions(){
   if(sessionRefresh)return sessionRefresh;
   const initialId=targetInitialized?undefined:activeSessionId;
   sessionRefresh=(async()=>{
     let list:SessionMeta[];
     try{list=await rpc.call<SessionMeta[]>('session.list');}
     catch(e){targetInitialized=true;throw e;}
     if(destroyed)return;
     sessions=list;
     // Only the initial exact ID can be suggested, never another matching title or pane.
     if(!targetInitialized){targetInitialized=true;const s=list.find(s=>s.id===initialId);if(s)terminalTarget={id:s.id,title:s.title};}
     if(terminalTarget&&!list.some(s=>s.id===terminalTarget!.id)){
       if(running||sending)await stop();
       if(!sending){readAuthorized=false;autoApproved=false;autoConfirm=false;mode='confirm';}
     }
   })().finally(()=>{sessionRefresh=undefined;});
   return sessionRefresh;
 }
 async function refresh(){
   await Promise.all([refreshSessions(),(async()=>{
     [profiles,notes,clients]=await Promise.all([rpc.call<StoredProfile[]>('profile.list'),rpc.call<NotesSnapshot>('notes.list'),rpc.call<McpClientEntry[]>('ai.mcp.list')]);
   })()]);
 }
 function openBinding(){bindingOpen=!bindingOpen;if(bindingOpen&&!running&&!sending)void execute(refreshSessions);}
 function selectTerminal(id:string){
   if(running||sending||permissionChanging)return;
   const s=sessions.find(s=>s.id===id);
   if(id&&!s)return;
   readAuthorized=false;autoApproved=false;autoConfirm=false;mode='confirm';targetInitialized=true;terminalTarget=s?{id:s.id,title:s.title}:null;
 }
 function authorizeTerminal(allow:boolean){if(!allow){void revokeTerminal();return;}if(running||sending)return;readAuthorized=allow&&!!terminalTarget&&sessions.some(s=>s.id===terminalTarget!.id);}
 async function attach(){if(!selected)return;if(kind==='terminal'){const id=selected;const s=sessions.find(s=>s.id===id);if(!s)throw Error(t('terminalUnavailable'));const r=await rpc.call<{text:string}>('terminal.exportText',{id});context=[...context,boundedTerminalContext(id,s.title,r.text,lines)];}else if(kind==='notes'){const n=notes.notes.find(n=>n.id===selected);if(n)context=[...context,{kind:'notes',id:n.id,label:n.title,text:n.markdown}];}else{const p=profiles.find(p=>p.id===selected);if(p)context=[...context,profileContext(p)];}}
 function current(token:number,session:string){return !destroyed&&token===epoch&&session===conversation;}
 async function stop(id=generation){const token=++epoch;clearTimeout(timer);if(!id){if(sending&&pendingSendCompletion){await pendingSendCompletion;if(generation&&running)throw Error(t('permissionLocked'));if(!generation)running=false;}else running=false;status=t('canceled');return;}running=true;try{await rpc.call('ai.stop',{id});if(token===epoch){const last=messages.at(-1);if(last?.role==='assistant')appendAiEvent(last,{type:'canceled'});generation='';running=false;status=t('canceled');}}catch(e){if(token===epoch){running=true;error=`${t('failed')}: ${readableAiError((e as Error).message,t)}`;}throw e;}}
 async function newConversation(){try{if(generation)await stop();else void stop();}catch{return;}readAuthorized=false;autoApproved=false;autoConfirm=false;mode='confirm';generationTarget=null;followLatest=true;messages=[];events=[];context=[];conversation=uuidv4();error='';status='';}
 async function poll(id:string,token:number,session:string){try{const r=await pollAi(rpc,id,cursor);if(!current(token,session))return;cursor=r.cursor;for(const event of r.events){const last=messages.at(-1);if(last?.role==='assistant')appendAiEvent(last,event,approvalTarget(event));if(event.type!=='delta'){events.push(event);if(event.type==='error')error=readableAiError(event.message??t('failed'),t);if(event.type==='canceled')status=t('canceled');if(event.type==='done')status=t('done');}}void followFeed();if(r.done){running=false;return;}if(current(token,session))timer=setTimeout(()=>void poll(id,token,session),100);}catch(e){if(current(token,session)){const message=readableAiError((e as Error).message,t);error=message;running=true;try{await stop(id);}catch(stopError){error=`${message}; ${t('failed')}: ${(stopError as Error).message}`;}}}}
 async function send(){
   const text=message.trim();if(!text||running||sending||troubleshootOpen||destroyed||enabled===false)return;
   // Lock synchronously, before the first await. A pending canceled request keeps
   // sending locked until its generation ID arrives and can be stopped.
   sending=true;generation='';pendingSendCompletion=new Promise(resolve=>finishPendingSend=resolve);running=true;events=[];resolved=[];const token=++epoch;const session=conversation;
   generationTarget=readAuthorized&&terminalTarget&&sessions.some(s=>s.id===terminalTarget!.id)?{...terminalTarget}:null;
   const history=messages.map(({role,content})=>({role,content}));const request={conversationId:session,message:text,history,context:JSON.parse(JSON.stringify(context)),clients:[...selectedClients],...(providerId?{providerId,model}:{}),...(generationTarget?{terminal:{sessionId:generationTarget.id,readAuthorized:true,mode,autoApproved:mode==='auto'&&autoApproved}}:{})};
   try{
     const r=await rpc.call<{id:string}>('ai.send',request);
     if(!current(token,session)){generation=r.id;running=true;await stop();return;}
     generation=r.id;cursor=0;events=[];resolved=[];status=t('running');message='';messages.push({role:'user',content:text},{role:'assistant',content:''});void poll(r.id,token,session);
   }catch(e){if(current(token,session)){running=false;throw e;}}
   finally{sending=false;finishPendingSend?.();finishPendingSend=undefined;pendingSendCompletion=undefined;}
 }
 async function approve(event:AiEvent,allow:boolean){if(sending||!generation||!event.call||!event.approvalId||resolved.includes(event.approvalId)||approving.includes(event.approvalId))return;const token=epoch;const session=conversation;approving.push(event.approvalId);try{await rpc.call('ai.approve',{id:generation,conversationId:conversation,approvalId:event.approvalId,call:JSON.parse(JSON.stringify(event.call)),allow});if(current(token,session)){resolved.push(event.approvalId);const activity=messages.at(-1)?.parts?.find(p=>p.type==='activity'&&p.call.id===event.call?.id);if(activity?.type==='activity'&&activity.state==='approval')activity.state=allow?'approved':'denied';}}finally{approving=approving.filter(id=>id!==event.approvalId);}}
 function approvalTarget(event:AiEvent){return ['terminal_read','terminal_wait','terminal_input','terminal_interrupt'].includes(event.call?.name??'')?generationTarget:null;}
 function approvalSource(event:AiEvent){const tool=events.find(e=>e.type==='tool'&&e.call?.id===event.call?.id);return tool?`${tool.client??''} / ${tool.tool??event.call?.name}`:event.call?.name??'';}
 async function connect(id:string){catalogs[id]=await rpc.call('ai.mcp.connect',{id});}
 async function resource(id:string,uri:string){const data=await rpc.call('ai.mcp.readResource',{id,params:{uri}});context.push({kind:'resource',id:`${id}:${uri}`,label:uri,text:JSON.stringify(data,null,2)});}
 async function templateResource(id:string,uriTemplate:string){const params=JSON.parse(templateArgs[`${id}:${uriTemplate}`]??'{}');await resource(id,expandResourceTemplate(uriTemplate,params));}
 async function prompt(id:string,name:string){const data=await rpc.call('ai.mcp.getPrompt',{id,params:{name,arguments:JSON.parse(promptArgs)}});context.push({kind:'prompt',id:`${id}:${name}`,label:name,text:JSON.stringify(data,null,2)});}
 onMount(()=>{void execute(refresh);void execute(loadProviders);});onDestroy(()=>{destroyed=true;void stop().catch(()=>{});});
</script>
{#snippet terminalOptions()}
 {@const choices=terminalChoices()}
 {#each choices.associated as session (session.id)}<option value={session.id} title={paneTitle(session)}>{paneChoiceLabel(session)}</option>{/each}
 {#if choices.unassociated.length}<optgroup label={paneText().unassociated}>
  {#each choices.unassociated as session (session.id)}<option value={session.id} title={paneTitle(session)}>{paneChoiceLabel(session)}</option>{/each}
 </optgroup>{/if}
{/snippet}
<svelte:window onpointerdown={dismissCopyMenu} onblur={() => copyMenu = null}
 onkeydown={event => { if (event.key === 'Escape' && copyMenu) copyMenu = null; }}/>
<!-- svelte-ignore a11y_no_noninteractive_element_interactions (delegate copy shortcuts for selectable text and fields) -->
<div class="chat" role="region" aria-label={t('title')} tabindex="-1"
 data-aerotab-focus-zone="" data-aerotab-ai-panel=""
 onkeydown={onCopyShortcut} oncontextmenu={openCopyMenu}>
 <header>
  <div class="heading"><span class="spark">✦</span><strong>{t('title')}</strong><span class="badge" hidden={panelVisible&&troubleshootOpen}>{t(running?'running':'ready')}</span></div>
  <div class="header-actions">
   {#if panelVisible}<button type="button" class="troubleshoot-toggle" aria-pressed={troubleshootOpen} disabled={!troubleshootOpen&&(running||sending)} onclick={()=>troubleshootOpen?troubleshootOpen=false:openTroubleshoot()}>{aiTroubleshootMessages[i18n.resolvedLocale].title}</button>{/if}
   <button title={t('clear')} aria-label={t('clear')} hidden={panelVisible&&troubleshootOpen} onclick={newConversation}>＋</button><button title={t('settings')} aria-label={t('settings')} onclick={onSettings}>⚙</button>
  </div>
 </header>
 {#if troubleshootMounted}
  <section class="troubleshoot-view" hidden={!panelVisible||!troubleshootOpen}>
   <div class="troubleshoot-destination">
    <label>{t('provider')}<select value={providerId} disabled={running||sending||!destinationReady} onchange={event=>void selectProvider(event.currentTarget.value)}><option value="">{t('noProvider')}</option>{#each providers as provider (provider.id)}<option value={provider.id}>{provider.name}</option>{/each}</select></label>
    <label>{t('model')}<select value={model} disabled={running||sending||!destinationReady||!providerId} onchange={event=>selectModel(event.currentTarget.value)}><option value="">{t(providerId?'modelUnavailable':'noProvider')}</option>{#each models as choice (choice)}<option value={choice}>{choice}</option>{/each}</select></label>
   </div>
   {#if error&&!destinationReady}<p role="alert" class="error">{error}</p>{/if}
   <AiTroubleshoot {rpc} {...troubleshootDestination()} seed={acceptedSeed} active={panelVisible&&troubleshootOpen} {onAddIncidentEvidence}/>
  </section>
 {/if}
 <div class="legacy-chat" hidden={panelVisible&&troubleshootOpen}>
 <div class="feed" bind:this={feed} onscroll={trackScroll} aria-live="polite" role="log" aria-label={t('message')}>
 {#if !messages.length}<div class="empty"><span class="empty-icon">✦</span><h2>{t('welcome')}</h2><p>{t('welcomeHint')}</p>{#if enabled===false}<button onclick={onSettings}>{t('disabledHint')}</button>{/if}</div>{/if}
 {#each messages as m,index(index)}<article class:assistant={m.role==='assistant'} class:user={m.role==='user'}><div class="message-role">{m.role==='assistant'?t('title'):t('message')}</div>{#if m.role==='assistant'}{#if m.parts}{#each m.parts as part,partIndex(partIndex)}{#if part.type==='text'}<AiMarkdown text={part.text}/>{:else if part.type==='notice'}<p class="notice" role="status">{readableAiNotice(part,t)}</p>{:else}<AiToolActivity activity={displayActivity(part)} disabled={!running||sending||permissionChanging||approving.includes(part.approval?.approvalId??'')||resolved.includes(part.approval?.approvalId??'')} onApprove={allow=>{if(part.approval)void execute(()=>approve(part.approval!,allow));}}/>{/if}{/each}{:else}<AiMarkdown text={m.content}/>{/if}{:else}<div class="user-text">{m.content}</div>{/if}</article>{/each}
 </div>
 <div class="controls">
 {#if !followLatest&&messages.length}<button class="jump-latest" onclick={jumpLatest}>{t('latest')} ↓</button>{/if}
 {#if error}<p role="alert" class="error">{error}</p>{/if}
 {#if permissionChanging}<p role="status">{t('permissionLocked')}</p>{/if}
 {#if (running||sending)&&isNewSeed(troubleshootSeed)}<p role="status">{pendingSeedMessage()}</p>{/if}
 {#if bindingOpen}<section class="binding" aria-label={t('terminalInteraction')}><strong>{t('terminalShort')}</strong>
 <label>{t('terminalTarget')}
 <select aria-label={t('terminalTarget')} title={terminalTarget?paneTitle(terminalTarget):t('terminalTarget')} value={terminalTarget?.id??''} disabled={running||sending||permissionChanging} onchange={event=>selectTerminal(event.currentTarget.value)}>
  <option value="">—</option>
  {#if terminalTarget&&!sessions.some(s=>s.id===terminalTarget!.id)}<option value={terminalTarget.id} title={paneTitle(terminalTarget)} disabled>{paneChoiceLabel(terminalTarget)} · {paneText().unavailable}</option>{/if}
  {@render terminalOptions()}
 </select></label>
 <div class="binding-actions"><button disabled={running||sending} onclick={()=>execute(refreshSessions)}>{t('refresh')}</button>
 {#if currentPane()}{@const pane=currentPane()!}<button type="button" title={paneTitle(pane)} disabled={running||sending||permissionChanging||(terminalTarget?.id===pane.id&&sessions.some(s=>s.id===pane.id))} onclick={()=>void execute(selectCurrentPane)}>{paneText().useCurrent}: {paneLabel(pane)}</button>{/if}</div>
 <label><input type="checkbox" checked={readAuthorized} disabled={permissionChanging||(!readAuthorized&&(running||sending))||!terminalTarget||!sessions.some(s=>s.id===terminalTarget!.id)} onchange={event=>authorizeTerminal(event.currentTarget.checked)}/>{t('readShort')}</label>
 <details><summary>{t('terminalInteraction')}</summary>{#if terminalTarget}<p data-terminal-target>{paneTitle(terminalTarget)}</p>{/if}<p>{t('terminalCapabilities')}</p><p>{t('terminalReadScope')}</p><p>{t('terminalToolHelp')}</p></details>
 {#if terminalTarget&&!sessions.some(s=>s.id===terminalTarget!.id)}<p role="status">{t('terminalUnavailable')}</p>{/if}</section>{/if}
 {#if autoConfirm}<section class="auto-confirm" role="alert"><strong data-terminal-target>{terminalTarget?paneLabel(terminalTarget):t('targetChip')} <small>{terminalTarget?.id??''}</small></strong><p>{t('autoWarning')}</p><div class="toolbar"><button onclick={()=>autoConfirm=false}>{t('cancel')}</button><button class="accent" disabled={!terminalTarget||!readAuthorized||running||sending} onclick={confirmAuto}>{t('autoAccept')}</button></div></section>{/if}
 <section data-advanced hidden={!advanced} class="advanced-body" aria-label={t('advanced')}>
 <label>{t('provider')}<select aria-label={t('provider')} value={providerId} disabled={running||sending} onchange={event=>void selectProvider(event.currentTarget.value)}><option value="">{t('noProvider')}</option>{#each providers as provider}<option value={provider.id}>{provider.name}</option>{/each}</select></label>
 <details><summary>{t('context')} ({context.length})</summary>
 <button onclick={()=>execute(refresh)}>{t('refresh')}</button>
 <select aria-label={t('context')} bind:value={kind} onchange={()=>selected=''}><option value="terminal">{t('terminal')}</option><option value="notes">{t('notes')}</option><option value="profile">{t('profile')}</option></select>
 {#if kind==='profile'}
  <HostSelect choices={profileChoices(profiles)} value={selected} label={t('profile')} emptyLabel="—" onChange={id=>selected=id} />
 {:else}
  <select aria-label={t(kind)} bind:value={selected}><option value="">—</option>{#if kind==='terminal'}{@render terminalOptions()}{:else}{#each notes.notes as n (n.id)}<option value={n.id}>{n.title}</option>{/each}{/if}</select>
 {/if}
 {#if kind==='terminal'}<label>{t('lines')}<input type="number" min="1" max="2000" bind:value={lines}/></label>{/if}<button disabled={!selected||running} onclick={()=>execute(attach)}>{t('attach')}</button>
 <p>{t('snapshot')}</p>
 </details>
 <details><summary>{t('clients')}</summary><p>{t('toolsScope')}</p>{#if !clients.length}<p>{t('emptyClients')}</p>{/if}
 {#each clients as entry(entry.config.id)}<section><label><input type="checkbox" value={entry.config.id} bind:group={selectedClients} disabled={!catalogs[entry.config.id]||!entry.trusted||running}/>{entry.config.name}</label><button disabled={!entry.trusted||running} onclick={()=>execute(()=>connect(entry.config.id))}>{t('connect')}</button>
 {#if catalogs[entry.config.id]}<details><summary>{t('discover')}</summary><pre>{JSON.stringify(catalogs[entry.config.id]!.tools,null,2)}</pre></details>{#each catalogs[entry.config.id]!.resources as r}<button onclick={()=>execute(()=>resource(entry.config.id,r.uri))}>{t('resource')}: {r.name}</button>{/each}{#each catalogs[entry.config.id]!.resourceTemplates??[] as template}<section><strong>{template.name}</strong><pre>{template.uriTemplate}</pre><label>{t('templateArgs')}<input bind:value={templateArgs[`${entry.config.id}:${template.uriTemplate}`]} placeholder={'{"path":"value"}'}/></label><button disabled={running} onclick={()=>execute(()=>templateResource(entry.config.id,template.uriTemplate))}>{t('resource')}</button></section>{/each}<label>{t('promptArgs')}<input bind:value={promptArgs}/></label>{#each catalogs[entry.config.id]!.prompts as p}<button onclick={()=>execute(()=>prompt(entry.config.id,p.name))}>{t('prompt')}: {p.name}</button>{/each}{/if}</section>{/each}
 </details>
 <details><summary>{t('preview')} — {contextBytes(context)} {t('bytes')}</summary>{#if readAuthorized&&terminalTarget}<p data-terminal-target>{t('terminalTarget')}: <strong>{paneLabel(terminalTarget)} ({terminalTarget.id})</strong></p><p>{t('terminalBound')}</p>{:else}<p>{t('terminalUnbound')}</p>{/if}<p>{t('history')} ({messages.length})</p>{#if !context.length}<p>{t('noContext')}</p>{/if}{#each context as c,index(index)}<label>{t(c.kind==='resource'?'resource':c.kind==='prompt'?'prompt':c.kind)}: {c.kind==='terminal'?paneLabel({id:c.id,title:c.label}):c.label}<textarea rows="3" bind:value={c.text} disabled={running}></textarea></label><button disabled={running} onclick={()=>context=context.filter((_,i)=>i!==index)}>{t('remove')}</button>{/each}</details>
 <details><summary>{t('privacy')}</summary><p>{t('history')} ({messages.length})</p></details>
 </section>
 <div class="composer">
  <textarea aria-label={t('message')} placeholder={t('placeholder')} rows="3" bind:value={message} disabled={running||sending||enabled===false} onkeydown={event=>{if(event.key==='Enter'&&!event.shiftKey&&!event.isComposing){event.preventDefault();void execute(send);}}}></textarea>
  <div class="composer-options"><div class="model-select"><select aria-label={t('model')} title={providers.find(p=>p.id===providerId)?.name??t('noProvider')} value={model} onchange={event=>selectModel(event.currentTarget.value)} disabled={running||sending||!providerId}><option value="">{t(providerId?'modelUnavailable':'noProvider')}</option>{#each models as choice}<option value={choice}>{providers.find(p=>p.id===providerId)?.name} · {choice}</option>{/each}</select></div></div>
  <div class="composer-bottom"><div class="composer-tools"><button class:active={advanced} aria-label={t('addContext')} title={t('addContext')} onclick={()=>advanced=!advanced}>＋{#if context.length}<small>{context.length}</small>{/if}</button><button class="terminal-chip" class:active={readAuthorized} title={terminalTarget?paneTitle(terminalTarget):t('terminalTarget')} onclick={openBinding}><span>›_</span> {readAuthorized?t('linked'):t('targetChip')}</button></div><select class="mode" aria-label={t('mode')} value={mode} disabled={permissionChanging||sending} onchange={event=>void changeMode(event.currentTarget.value as AiPermissionMode)}><option value="observer">{t('observer')}</option><option value="confirm">{t('confirm')}</option><option value="auto">{t('auto')}</option></select>{#if running||sending}<button class="send stop" aria-label={t('stop')} title={t('stop')} onclick={()=>execute(stop)}>■</button>{:else}<button class="send" aria-label={t('sendShort')} title={t('sendShort')} disabled={sending||!message.trim()||enabled===false} onclick={()=>execute(send)}>↑</button>{/if}</div>
 </div>
 <div class="footer-status" role="status">{status||t(mode==='observer'?'observer':readAuthorized?'terminalShort':'noContext')}</div>
 </div>
 </div>
 {#if copyMenu && panelVisible}
  <button class="selection-copy" data-ai-selection-copy="" bind:this={copyButton}
   style:left={`${copyMenu.x}px`} style:top={`${copyMenu.y}px`}
   onclick={() => { if (copyMenu) copySelection(copyMenu.text); copyMenu = null; }}>{i18n.t('common.copy')}</button>
 {/if}
</div>
<style>
 .chat:focus { outline: none; }
 .selection-copy { position: fixed; z-index: 70; padding: 6px 16px; box-shadow: 0 4px 12px var(--color-overlay); }
 .selection-copy:hover { background: var(--color-accent-soft); }
 .legacy-chat{display:flex;flex-direction:column;flex:1;min-height:0;min-width:0}
 .troubleshoot-view{flex:1;min-height:0;min-width:0;overflow:auto}
 .chat [hidden]{display:none}
 .header-actions .troubleshoot-toggle{font-size:11px;white-space:nowrap;border:1px solid var(--color-border);padding:3px 6px}
 .troubleshoot-toggle[aria-pressed='true']{color:var(--color-accent);border-color:var(--color-accent)}
 .troubleshoot-destination{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:8px;padding:4px 12px 0}
 .troubleshoot-view>.error{padding:0 12px}
 .binding-actions{display:flex;flex-wrap:wrap;gap:5px}
 .notice{font-size:14px;color:var(--color-fg);padding:10px 12px;background:var(--color-bg-soft);border-left:2px solid var(--color-accent);border-radius:3px}.jump-latest{display:block;margin:0 auto 8px}.advanced-body[hidden]{display:none}
 .chat{display:flex;flex-direction:column;height:100%;min-height:0;min-width:0;box-sizing:border-box;color:var(--color-fg);background:var(--color-panel,var(--color-bg));font-size:12px;overflow:hidden}header{display:flex;align-items:center;justify-content:space-between;padding:10px 12px;border-bottom:1px solid var(--color-border);flex:none;gap:8px}.heading,.header-actions{display:flex;align-items:center;gap:7px}.spark{color:var(--color-accent);font-size:17px}.heading strong{font-size:12px;font-weight:650}.badge{font-size:9px;color:var(--color-fg-muted);border:1px solid var(--color-border);border-radius:4px;padding:1px 4px}.header-actions button{border:0;background:transparent;font-size:16px;padding:2px 5px}.feed{flex:1;overflow:auto;min-height:100px;min-width:0;padding:14px 12px;scrollbar-gutter:stable}.empty{display:flex;min-height:220px;height:100%;box-sizing:border-box;flex-direction:column;align-items:center;justify-content:center;text-align:center;padding:20px 8px;gap:8px}.empty-icon{font-size:29px;color:var(--color-accent);opacity:.8}.empty h2{font-size:16px;font-weight:550;margin:4px 0}.empty p{font-size:14px;max-width:250px;line-height:1.7;margin:0}p{font-size:14px;color:var(--color-fg);margin:6px 0;line-height:1.6}article{margin:0 0 22px;min-width:0}.message-role{font-size:10px;font-weight:600;color:var(--color-fg-muted);margin-bottom:6px}.user{margin-left:18px}.user-text{font-size:14px;padding:9px 12px;border:0;border-radius:10px 10px 2px 10px;background:var(--color-bg-soft);white-space:pre-wrap;overflow-wrap:anywhere;line-height:1.65;user-select:text}.assistant .message-role{color:var(--color-accent)}.controls{flex:none;padding:0 10px 5px;min-width:0;max-height:65%;overflow:auto}.composer{border:1px solid var(--color-border);border-radius:10px;background:var(--color-bg-soft);overflow:hidden;box-shadow:0 2px 8px #00000008}.composer:focus-within{border-color:var(--color-accent)}button,input,select,textarea{font:inherit;color:var(--color-fg);background:var(--color-bg-soft);border:1px solid var(--color-border);border-radius:5px;padding:5px 7px;max-width:100%;min-width:0;box-sizing:border-box}button{cursor:pointer}button:disabled{opacity:.4;cursor:default}button:focus-visible,select:focus-visible,input:focus-visible,textarea:focus-visible,summary:focus-visible{outline:2px solid var(--color-accent);outline-offset:2px}.composer textarea{font-size:14px;border:0;border-radius:0;resize:vertical;display:block;width:100%;min-height:72px;max-height:180px;line-height:1.6;padding:12px;background:transparent;outline-offset:-3px}.composer-options{padding:0 8px 4px}.model-select{display:flex;gap:4px}.model-select select{border:0;background:transparent;color:var(--color-fg);font-size:12px;max-width:100%;padding:3px}.composer-bottom{display:flex;gap:5px;align-items:center;padding:5px 7px 7px}.composer-tools{display:flex;align-items:center;gap:3px;min-width:0;flex:1}.composer-tools button{border:0;background:transparent;padding:5px}.composer-tools .terminal-chip{font-size:10px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;max-width:125px;color:var(--color-fg-muted)}.terminal-chip span{font-family:var(--font-mono,monospace)}.composer-tools .active{color:var(--color-accent);background:var(--color-panel)}.mode{font-size:10px;padding:5px 2px;border:0;background:transparent;width:72px}.send{font-size:18px;font-weight:650;background:var(--color-accent);color:var(--color-bg);border:0;width:29px;height:29px;padding:0;flex:none}.stop{font-size:13px}.footer-status{font-size:9px;text-align:center;color:var(--color-fg-muted);min-height:14px;padding-top:4px;overflow:hidden;white-space:nowrap;text-overflow:ellipsis}.binding,.auto-confirm{border:1px solid var(--color-border);border-radius:8px;padding:9px;margin-bottom:8px}.auto-confirm{border-color:var(--color-danger,var(--color-accent));font-size:11px}.auto-confirm small{display:block;color:var(--color-fg-muted);overflow-wrap:anywhere}.toolbar{display:flex;gap:6px;justify-content:flex-end;flex-wrap:wrap}.accent{color:var(--color-accent);border-color:var(--color-accent)}label{display:block;margin:6px 0}label select, label textarea{display:block;width:100%;margin-top:4px}.binding label:has(input){display:flex;align-items:center;gap:5px}details{margin:5px 0}summary{cursor:pointer;color:var(--color-fg-muted);font-size:11px;padding:5px 0}.advanced-body{max-height:230px;overflow:auto;border:0;border-radius:0;padding:8px;margin-bottom:6px}.advanced-body select,.advanced-body input{margin:3px 0}.advanced-body textarea{width:100%;resize:vertical}pre{white-space:pre-wrap;overflow-wrap:anywhere;max-height:200px;overflow:auto;font:11px/1.5 var(--font-mono,monospace);user-select:text}.error{color:var(--color-danger);overflow-wrap:anywhere}input[type=number]{width:75px}@media(max-width:340px){header{padding:8px}.badge{display:none}.feed{padding:12px 9px}.controls{padding-left:7px;padding-right:7px}.composer-tools .terminal-chip{max-width:100px}}@media(prefers-reduced-motion:reduce){*{scroll-behavior:auto!important;transition:none!important;animation:none!important}}
</style>
