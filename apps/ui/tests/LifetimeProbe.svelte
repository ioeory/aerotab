<script lang="ts">
 import {onMount,onDestroy,type Snippet} from 'svelte';
 let {id,events,children}:{id:string;events:string[];children:Snippet}=$props();
 onMount(()=>{events.push(`mount:${id}`);});onDestroy(()=>events.push(`destroy:${id}`));
</script>
<div data-probe={id} style="height:100%;min-height:0">{@render children()}</div>
