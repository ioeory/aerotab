<script lang="ts">
 import type {AiActivity} from '../../lib/ai';
 import {readableAiCommand,aiToolLabel} from '../../lib/ai';
 import {i18n} from '../../lib/i18n.svelte';
 let {activity,disabled=false,onApprove}:{activity:AiActivity;disabled?:boolean;onApprove:(allow:boolean)=>void}=$props();
 const t=(key:string)=>i18n.t(`ai.${key}`);
 let copied=$state(false);
 async function copy(){try{await navigator.clipboard.writeText(readableAiCommand(activity.call));copied=true;}catch{copied=false;}}
</script>
<div class="activity" class:pending={activity.state==='approval'}>
 <details open={activity.state==='approval'}>
  <summary><span class="dot" class:denied={activity.state==='denied'||activity.state==='error'}></span><span class="name">{aiToolLabel(activity,t)}</span><span class="state">{t(`${activity.state}Tool`)}</span></summary>
  <div class="body">
   {#if activity.target}<p class="target" data-terminal-target>{t('terminalTarget')}: <strong>{activity.target.title}</strong><small>{activity.target.id}</small></p>{:else}<p>{activity.client??t('externalApproval')}</p>{/if}
   <div class="command"><button type="button" onclick={copy}>{t(copied?'copied':'copy')}</button><pre>{readableAiCommand(activity.call)}</pre></div>
   {#if activity.approval}<details class="arguments"><summary>{t('args')}</summary><pre>{JSON.stringify(activity.call.arguments,null,2)}</pre></details>{/if}
   {#if activity.state==='approval'}<p class="warning">{t(activity.target?'approvalTitle':'externalApproval')}</p><div class="actions"><button disabled={disabled} onclick={()=>onApprove(false)}>{t('deny')}</button><button class="accept" disabled={disabled} onclick={()=>onApprove(true)}>{t('once')}</button></div>{/if}
   {#if activity.result!==undefined}<details><summary>{t('result')}</summary><pre>{JSON.stringify(activity.result,null,2)}</pre></details>{/if}
  </div>
 </details>
</div>
<style>
 .activity{border:0;border-left:2px solid transparent;border-radius:0;margin:9px 0;font-size:12px;min-width:0}.pending{border-color:var(--color-accent)}summary{display:flex;align-items:center;gap:7px;cursor:pointer;padding:9px 10px;list-style:none}.dot{width:6px;height:6px;border-radius:50%;background:var(--color-accent);flex:none}.dot.denied{background:var(--color-danger)}.name{font-size:13px;color:var(--color-fg);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.state{color:var(--color-fg-muted);font-size:10px;margin-left:auto;flex-shrink:0}.body{padding:0 10px 10px}.target{margin:3px 0 9px;color:var(--color-fg-muted);overflow-wrap:anywhere}.target strong{color:var(--color-fg)}small{display:block;font-size:10px}.command{position:relative;background:var(--color-bg-soft);border-radius:5px}.command button{float:right;margin:4px}pre{font:12px/1.55 var(--font-mono,monospace);white-space:pre-wrap;overflow-wrap:anywhere;overflow:auto;max-height:240px;padding:9px;margin:0;user-select:text}.command pre{clear:both}.actions{display:flex;justify-content:flex-end;gap:7px}.warning{color:var(--color-fg);font-size:14px}.body summary{padding:7px 0;color:var(--color-fg-muted)}button{background:var(--color-bg-soft);color:var(--color-fg);border:1px solid var(--color-border);border-radius:5px;padding:5px 9px;cursor:pointer}.accept{border-color:var(--color-accent);color:var(--color-accent)}button:disabled{opacity:.45;cursor:default}button:focus-visible,summary:focus-visible{outline:2px solid var(--color-accent);outline-offset:2px}
</style>
