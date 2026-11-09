<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { keyboardResize, pointerResize } from '../lib/splitter';
  let { axis = 'x', value, min = 100, max = 800, sign = 1, label, onChange, onEnd = () => {}, storageKey, unitsPerPixel = 1 }: { axis?: 'x'|'y'; value: number; min?: number; max?: number; sign?: number; label: string; onChange: (value:number)=>void; onEnd?:()=>void; storageKey?:string; unitsPerPixel?:number } = $props();
  let cancel: (()=>void)|undefined;
  function scale(event:PointerEvent) { const el=event.currentTarget as HTMLElement;const rect=el.getBoundingClientRect();return (axis==='x'?rect.width/el.offsetWidth:rect.height/el.offsetHeight)||1; }
  function commit(next = value) { if(storageKey)try{localStorage.setItem(`aerotab.splitter.v1.${storageKey}`,JSON.stringify({version:1,value:next}));}catch{} onEnd(); }
  onMount(()=>{if(storageKey)try{const s=JSON.parse(localStorage.getItem(`aerotab.splitter.v1.${storageKey}`)??'null');if(s?.version===1&&Number.isFinite(s.value))onChange(Math.max(min,Math.min(max,s.value)));}catch{}});
  onDestroy(()=>cancel?.());
</script>
<!-- WAI-ARIA window splitter is intentionally keyboard focusable. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div role="separator" tabindex="0" aria-label={label} aria-orientation={axis==='x'?'vertical':'horizontal'} aria-valuemin={min} aria-valuemax={max} aria-valuenow={Math.round(value)} class="splitter" class:vertical={axis==='y'}
 onpointerdown={event=>{cancel?.();cancel=pointerResize(event,{axis,value,min,max,sign:sign*unitsPerPixel,scale:scale(event),preview:onChange,end:next=>commit(next)});}}
 onkeydown={event=>{const next=keyboardResize(event.key,value,min,max,axis,event.shiftKey);if(next!==null){event.preventDefault();onChange(next);commit(next);}}}></div>
<style>
.splitter{position:relative;flex:0 0 5px;align-self:stretch;cursor:col-resize;touch-action:none;user-select:none;background:var(--color-border-soft);z-index:21;pointer-events:auto}.splitter.vertical{cursor:row-resize}.splitter:hover,.splitter:focus-visible{background:var(--color-accent);outline:2px solid var(--color-accent);outline-offset:-2px}.splitter:before{content:'';position:absolute;inset:0 -3px}.splitter.vertical:before{inset:-3px 0}
</style>
