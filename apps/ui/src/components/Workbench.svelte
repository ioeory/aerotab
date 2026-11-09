<script lang="ts">
  import { onMount, onDestroy, untrack, type Snippet } from 'svelte';
  import { i18n } from '../lib/i18n.svelte';
  import { updateWorkbenchViewport, PANEL_MIN_WIDTH, PANEL_MIN_HEIGHT, spanLimits, reconcilePanels, panelDropTarget, type PanelDropTarget, PanelRegistry, defaultLayout, restoreLayout, measureLayout, placePanel, clampRect, resizeRect, type Side, type Rect } from '../lib/workbench';
  import Splitter from './Splitter.svelte';
  let { terminal, sftp, notes, system, sftpVisible=false, notesVisible=false, systemVisible=false, extraPanels=[], storageKey='aerotab.workbench.layout.v1' }: { terminal:Snippet; sftp:Snippet; notes:Snippet; system:Snippet; sftpVisible?:boolean; notesVisible?:boolean; systemVisible?:boolean; storageKey?:string; extraPanels?: {id:string;type:import('../lib/workbench').PanelType;title:string;content:Snippet;visible:boolean}[] }=$props();
  const registry=new PanelRegistry(); for(const type of ['sftp','notes','system'] as const) registry.register({id:type,type});
  $effect(()=>{
    const entries=extraPanels.map(p=>({id:p.id,type:p.type}));
    const ids=['sftp','notes','system',...entries.map(p=>p.id)];
    if(new Set(ids).size!==ids.length)throw new Error('duplicate panel ID');
    for(const id of registry.ids())if(!ids.includes(id))unregister.get(id)?.();
    for(const entry of entries)if(!registry.get(entry.id))unregister.set(entry.id,registry.register(entry));
    untrack(()=>{layout=reconcilePanels(layout,ids);});
  });
  const unregister=new Map<string,()=>void>();
  let layout=$state(defaultLayout(registry.ids()));let host:HTMLDivElement;let view=$state({width:1000,height:600});let ready=false;let meaningfulViewport=false;let cancelGesture:(()=>void)|undefined;
  const visible=$derived([...(sftpVisible?['sftp']:[]),...(notesVisible?['notes']:[]),...(systemVisible?['system']:[]),...extraPanels.filter(p=>p.visible).map(p=>p.id)]);
  const boxes=$derived(measureLayout(layout,view,visible));
  const storage=()=>storageKey;
  function save(){if(ready&&meaningfulViewport)try{localStorage.setItem(storage(),JSON.stringify(layout));}catch{}}
  function place(id:string,side:Side,index?:number){layout=placePanel(layout,id,side,index);save();}
  function rect(id:string,r:Rect){layout={...layout,panels:layout.panels.map(p=>p.id===id?{...p,rect:r}:p)};}
  function style(r:Rect|undefined){return r?`left:${r.x}px;top:${r.y}px;width:${r.width}px;height:${r.height}px;`:'display:none';}
  function gesture(event:PointerEvent,id:string,edge='') {
    if(event.button!==0)return;cancelGesture?.();event.preventDefault();event.stopPropagation();
    const before=layout;const p=layout.panels.find(p=>p.id===id)!;const r=boxes.panels[id]??p.rect;
    const sx=host.getBoundingClientRect().width/host.offsetWidth||1,sy=host.getBoundingClientRect().height/host.offsetHeight||1;
    let target:PanelDropTarget|null=null;
    const move=(e:PointerEvent)=>{if(e.pointerId!==event.pointerId)return;const dx=(e.clientX-event.clientX)/sx,dy=(e.clientY-event.clientY)/sy;
      if(edge)rect(id,resizeRect(r,edge,dx,dy,view));else{layout=placePanel(layout,id,'float');rect(id,clampRect({...r,x:r.x+dx,y:r.y+dy},view));const h=host.getBoundingClientRect(),x=(e.clientX-h.left)/sx,y=(e.clientY-h.top)/sy;target=panelDropTarget(before,measureLayout(before,view,visible.filter(key=>key!==id)).panels,view,visible,id,x,y);drop=target;}};
    const clear=()=>{window.removeEventListener('pointermove',move);window.removeEventListener('pointerup',up);window.removeEventListener('pointercancel',cancel);window.removeEventListener('keydown',key);window.removeEventListener('blur',cancel);drop=null;cancelGesture=undefined;};
    const cancel=()=>{clear();layout=before;};const up=(e:PointerEvent)=>{if(e.pointerId!==event.pointerId)return;clear();if(target)place(id,target.side,target.index);save();};const key=(e:KeyboardEvent)=>{if(e.key==='Escape'){e.preventDefault();cancel();}};
    cancelGesture=cancel;window.addEventListener('pointermove',move);window.addEventListener('pointerup',up);window.addEventListener('pointercancel',cancel);window.addEventListener('keydown',key);window.addEventListener('blur',cancel);
  }
  let drop=$state<PanelDropTarget|null>(null);
  onMount(()=>{
    try{layout=restoreLayout(JSON.parse(localStorage.getItem(storage())??'null'),registry.ids());}catch{}
    ready=true;
    const obs=new ResizeObserver(entries=>{
      const entry=entries.find(entry=>entry.target===host);
      if(!entry)return;
      // Preserve fractional local CSS pixels. Rounded client sizes can overflow
      // the host at shell zoom and toggle both scrollbars every frame.
      const {width,height}=entry.contentRect;
      if(width<1||height<1||!host.getClientRects().length){meaningfulViewport=false;return;}
      meaningfulViewport=true;view={width,height};layout=updateWorkbenchViewport(layout,view);
    });obs.observe(host);
    return()=>obs.disconnect();
  });
  onDestroy(()=>cancelGesture?.());
</script>
<div class="workbench" bind:this={host}>
  <div class="layout-extent" style={`width:${boxes.extent.width}px;height:${boxes.extent.height}px;`}>
  <div class="terminal-host" style={style(boxes.main)}>{@render terminal()}</div>
  {#each layout.panels.filter(p=>['sftp','notes','system',...extraPanels.map(p=>p.id)].includes(p.id)) as panel (panel.id)}
    {@const box=boxes.panels[panel.id]}
    {@const peers=layout.panels.filter(p=>p.side===panel.side&&visible.includes(p.id))}
    {@const next=peers[peers.findIndex(p=>p.id===panel.id)+1]}
    <section class="panel" class:floating={panel.side==='float'} style={style(box)} hidden={!visible.includes(panel.id)} data-panel-id={panel.id} data-workbench-float={panel.side==='float'&&visible.includes(panel.id)?'':undefined} data-aerotab-focus-zone="">
      <header>
        <button class="title-handle" aria-label={i18n.t('workbench.move')} onpointerdown={e=>gesture(e,panel.id)}>{extraPanels.find(p=>p.id===panel.id)?.title??i18n.t(`shell.${panel.id==='sftp'?'files':panel.id}`)}</button>
        <select aria-label={i18n.t('workbench.position')} value={panel.side} onchange={e=>place(panel.id,e.currentTarget.value as Side)}>{#each ['left','right','top','bottom','float'] as side}<option value={side}>{i18n.t(`workbench.${side}`)}</option>{/each}</select>
        <button aria-label={i18n.t('workbench.reorder')} onclick={()=>place(panel.id,panel.side,0)}>↑</button>
      </header>
      <div class="content">{#if panel.id==='sftp'}{@render sftp()}{:else if panel.id==='notes'}{@render notes()}{:else if panel.id==='system'}{@render system()}{:else}{@const extra=extraPanels.find(p=>p.id===panel.id)}{#if extra}{@render extra.content()}{/if}{/if}</div>
      {#if panel.side==='float'}
        {#each ['n','s','e','w','ne','nw','se','sw'] as edge}<button class="resize {edge}" aria-label={`${i18n.t('workbench.resize')} ${edge}`} onpointerdown={e=>gesture(e,panel.id,edge)} onkeydown={e=>{if(!e.key.startsWith('Arrow'))return;e.preventDefault();rect(panel.id,resizeRect(panel.rect,edge,e.key==='ArrowLeft'?-10:e.key==='ArrowRight'?10:0,e.key==='ArrowUp'?-10:e.key==='ArrowDown'?10:0,view));save();}}></button>{/each}
      {:else}
        <div class="dock-splitter {panel.side}"><Splitter axis={panel.side==='left'||panel.side==='right'?'x':'y'} value={box?(panel.side==='left'||panel.side==='right'?box.width:box.height):panel.size} min={panel.side==='left'||panel.side==='right'?PANEL_MIN_WIDTH:PANEL_MIN_HEIGHT} max={Math.max(panel.side==='left'||panel.side==='right'?PANEL_MIN_WIDTH:PANEL_MIN_HEIGHT,panel.side==='left'||panel.side==='right'?view.width*.42:view.height*.42)} sign={panel.side==='left'||panel.side==='top'?1:-1} label={i18n.t('workbench.resize')} onChange={size=>layout={...layout,panels:layout.panels.map(p=>p.side===panel.side?{...p,size}:p)}} onEnd={save}/></div>
      {/if}
      {#if panel.side!=='float' && next}
        {@const pair=(panel.span??1)+(next.span??1)}
        <div class="span-splitter" class:horizontal={panel.side==='top'||panel.side==='bottom'}><Splitter axis={panel.side==='top'||panel.side==='bottom'?'x':'y'} value={(panel.span??1)*100} min={spanLimits(pair,panel.side==='top'||panel.side==='bottom'?(box?.width??0)+(boxes.panels[next.id]?.width??0):(box?.height??0)+(boxes.panels[next.id]?.height??0),panel.side==='top'||panel.side==='bottom'?PANEL_MIN_WIDTH:PANEL_MIN_HEIGHT).min} max={spanLimits(pair,panel.side==='top'||panel.side==='bottom'?(box?.width??0)+(boxes.panels[next.id]?.width??0):(box?.height??0)+(boxes.panels[next.id]?.height??0),panel.side==='top'||panel.side==='bottom'?PANEL_MIN_WIDTH:PANEL_MIN_HEIGHT).max} unitsPerPixel={pair*100/Math.max(1,((panel.side==='top'||panel.side==='bottom')?((box?.width??0)+(boxes.panels[next.id]?.width??0)):((box?.height??0)+(boxes.panels[next.id]?.height??0))))} label={i18n.t('workbench.resize')} onChange={v=>layout={...layout,panels:layout.panels.map(p=>p.id===panel.id?{...p,span:v/100}:p.id===next.id?{...p,span:pair-v/100}:p)}} onEnd={save}/></div>
      {/if}
    </section>
  {/each}
  </div>
  {#if drop}<div class="drop-preview" data-drop-index={drop.index} style={style(drop.preview)}>{i18n.t(`workbench.${drop.side}`)}</div>{/if}
</div>
<style>
.span-splitter{position:absolute;bottom:0;left:0;right:0;display:flex;flex-direction:column}.span-splitter.horizontal{bottom:0;left:auto;top:32px;right:0;flex-direction:row}.workbench{position:relative;flex:1;min-width:0;min-height:0;overflow:auto}.layout-extent{position:relative;min-width:100%;min-height:100%}.terminal-host,.panel{position:absolute;min-width:0;min-height:0}.panel{display:flex;flex-direction:column;box-sizing:border-box;border:1px solid var(--color-border-soft);background:var(--color-panel);z-index:22}.panel[hidden]{display:none}.floating{z-index:45;box-shadow:0 12px 36px var(--color-overlay);border-radius:6px}header{height:32px;flex-shrink:0;display:flex;gap:4px;align-items:center;padding:2px 8px;border-bottom:1px solid var(--color-border-soft)}button,select{color:var(--color-fg);background:var(--color-panel);border:0;font-size:11px}button{cursor:pointer}.title-handle{flex:1;text-align:left;cursor:grab;touch-action:none}.content{flex:1;min-height:0;min-width:0;position:relative;overflow:auto}.dock-splitter{position:absolute;display:flex}.dock-splitter.left{right:0;top:32px;bottom:0}.dock-splitter.right{left:0;top:32px;bottom:0}.dock-splitter.top{bottom:0;left:0;right:0;flex-direction:column}.dock-splitter.bottom{top:0;left:0;right:0;flex-direction:column}.resize{position:absolute;padding:0;touch-action:none;background:transparent}.n,.s{height:6px;left:8px;right:8px;cursor:ns-resize}.n{top:0}.s{bottom:0}.e,.w{width:6px;top:8px;bottom:8px;cursor:ew-resize}.e{right:0}.w{left:0}.ne,.nw,.se,.sw{width:10px;height:10px}.ne{right:0;top:0;cursor:nesw-resize}.nw{left:0;top:0;cursor:nwse-resize}.se{right:0;bottom:0;cursor:nwse-resize}.sw{left:0;bottom:0;cursor:nesw-resize}.drop-preview{position:absolute;z-index:60;pointer-events:none;background:var(--color-accent-soft);border:2px solid var(--color-accent);color:var(--color-fg);display:grid;place-items:center}.drop-preview.left{inset:0 70% 0 0}.drop-preview.right{inset:0 0 0 70%}.drop-preview.top{inset:0 0 70%}.drop-preview.bottom{inset:70% 0 0}</style>
