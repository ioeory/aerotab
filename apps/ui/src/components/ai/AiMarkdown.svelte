<script lang="ts">
 import {marked} from 'marked';
 import {i18n} from '../../lib/i18n.svelte';
 type Node={type:string;raw?:string;text?:string;depth?:number;href?:string;lang?:string;ordered?:boolean;start?:number;tokens?:Node[];items?:Node[];header?:{tokens:Node[]}[];rows?:{tokens:Node[]}[][]};
 let {text}:{text:string}=$props();
 const nodes=$derived(marked.lexer(text) as unknown as Node[]);
 let copied=$state('');
 function safeLink(href:string){return /^(https?:\/\/|mailto:)/i.test(href)?href:undefined;}
 async function copy(value:string){try{await navigator.clipboard.writeText(value);copied=value;}catch{copied='';}}
</script>
{#snippet content(items:Node[])}
 {#each items as node,index(index)}
  {#if node.type==='heading'}<svelte:element this={`h${Math.min(6,node.depth??2)}`}>{@render content(node.tokens??[])}</svelte:element>
  {:else if node.type==='paragraph'}<p>{@render content(node.tokens??[])}</p>
  {:else if node.type==='strong'}<strong>{@render content(node.tokens??[])}</strong>
  {:else if node.type==='em'}<em>{@render content(node.tokens??[])}</em>
  {:else if node.type==='del'}<del>{@render content(node.tokens??[])}</del>
  {:else if node.type==='codespan'}<code>{node.text}</code>
  {:else if node.type==='code'}<div class="code"><div class="code-head"><span>{node.lang??''}</span><button type="button" onclick={()=>copy(node.text??'')}>{i18n.t(`ai.${copied===node.text?'copied':'copy'}`)}</button></div><pre><code>{node.text}</code></pre></div>
  {:else if node.type==='blockquote'}<blockquote>{@render content(node.tokens??[])}</blockquote>
  {:else if node.type==='list'}{#if node.ordered}<ol start={node.start??1}>{#each node.items??[] as item}<li>{@render content(item.tokens??[])}</li>{/each}</ol>{:else}<ul>{#each node.items??[] as item}<li>{@render content(item.tokens??[])}</li>{/each}</ul>{/if}
  {:else if node.type==='link'}{#if safeLink(node.href??'')}<a href={safeLink(node.href??'')} target="_blank" rel="noopener noreferrer">{@render content(node.tokens??[])}</a>{:else}{@render content(node.tokens??[])}{/if}
  {:else if node.type==='br'}<br/>
  {:else if node.type==='hr'}<hr/>
  {:else if node.type==='table'}<div class="table"><table><thead><tr>{#each node.header??[] as cell}<th>{@render content(cell.tokens)}</th>{/each}</tr></thead><tbody>{#each node.rows??[] as row}<tr>{#each row as cell}<td>{@render content(cell.tokens)}</td>{/each}</tr>{/each}</tbody></table></div>
  {:else if node.tokens}{@render content(node.tokens)}
  {:else if node.type!=='space'}{node.text??node.raw??''}
  {/if}
 {/each}
{/snippet}
<div class="markdown">{@render content(nodes)}</div>
<style>
 .markdown{font-size:14px;color:var(--color-fg);line-height:1.75;overflow-wrap:anywhere;user-select:text;min-width:0}.markdown :global(p){margin:.45em 0}.markdown :global(h1),.markdown :global(h2),.markdown :global(h3){font-size:1.12em;font-weight:650;margin:1em 0 .4em}.markdown :global(a){color:var(--color-accent);text-decoration:underline}.markdown :global(ul),.markdown :global(ol){padding-left:1.6em;margin:.5em 0}.markdown :global(ul){list-style:disc}.markdown :global(ol){list-style:decimal}.markdown :global(code){font: .95em/1.6 var(--font-mono,monospace);background:var(--color-bg-soft);border-radius:3px;padding:.1em .25em}.code{border:1px solid var(--color-border);border-radius:7px;overflow:hidden;margin:9px 0}.code-head{display:flex;justify-content:space-between;align-items:center;padding:4px 9px;background:var(--color-bg-soft);color:var(--color-fg-muted);font-size:11px}.code pre{margin:0;padding:10px;overflow:auto;white-space:pre;max-width:100%}.code code{background:transparent;padding:0}button{border:0;background:transparent;color:var(--color-fg);cursor:pointer;padding:4px}button:focus-visible,a:focus-visible{outline:2px solid var(--color-accent);outline-offset:2px}.markdown :global(blockquote){margin:8px 0;border-left:2px solid var(--color-border);padding-left:12px;color:var(--color-fg)}.table{overflow:auto}table{border-collapse:collapse;font-size:14px}th,td{border:1px solid var(--color-border);padding:5px 8px}hr{border:0;border-top:1px solid var(--color-border);margin:12px 0}
</style>
