<script lang="ts">
  import { untrack } from 'svelte';
  import type { Note } from '../../lib/notesCore';
  import { renderMarkdown } from '../../lib/notesCore';
  import type { NotesStore } from '../../lib/notes.svelte';
  import type { StoredProfile } from '../../lib/types';
  import { nm } from '../../lib/notesMessages';
  import { exportNote } from '../../lib/notesFiles';
  let { store, note, profiles = [], onError }: { store: NotesStore; note: Note; profiles?: StoredProfile[]; onError: (message: string) => void } = $props();
  let draft = $state(untrack(() => store.draft(note)));
  $effect(() => { const current = note; untrack(() => { draft = store.draft(current); }); });
  let preview = $state(false);
  let textarea = $state<HTMLTextAreaElement>();
  let history = $state<string[]>([]); let future = $state<string[]>([]); let historyId = '';
  $effect(() => { if (note.id !== historyId) { history = []; future = []; historyId = note.id; } });
  const tools = [ ['heading', '# ', ''], ['bold', '**', '**'], ['italic', '*', '*'], ['strike', '~~', '~~'], ['quote', '> ', ''], ['code', '`', '`'], ['codeblock', '\n```\n', '\n```\n'], ['ordered', '1. ', ''], ['unordered', '- ', ''], ['task', '- [ ] ', ''], ['separator', '\n---\n', ''], ['table', '\n| A | B |\n| --- | --- |\n|   |   |\n', ''], ['link', '[', '](https://example.com)'], ['image', '![', '](https://example.com/image.png)'] ] as const;
  function changed(): void { store.edit(note.id); }
  function insert(before: string, after: string): void {
    if (!textarea) return;
    const start = textarea.selectionStart, end = textarea.selectionEnd; const body = draft.value.markdown;
    history.push(body); future = [];
    draft.value.markdown = body.slice(0, start) + before + body.slice(start, end) + after + body.slice(end); changed();
    queueMicrotask(() => { textarea?.focus(); textarea?.setSelectionRange(start + before.length, end + before.length); });
  }
  function undo(redo = false): void {
    const from = redo ? future : history, to = redo ? history : future;
    const value = from.pop(); if (value === undefined) return;
    to.push(draft.value.markdown); draft.value.markdown = value; changed();
  }
</script>
<div class="editor">
  <div class="actions">
    <button onclick={() => preview = !preview}>{preview ? nm('edit') : nm('preview')}</button>
    <button onclick={() => store.save(note.id).catch(e => onError(String(e)))}>{nm('save')}</button>
    <button onclick={() => exportNote(draft.value).catch(e => onError(String(e)))}>{nm('export')}</button>
    <span role="status" class:error={draft.status === 'error'}>{nm(draft.status)}</span>
  </div>
  <input aria-label={nm('noteTitle')} class="title" bind:value={draft.value.title} oninput={changed} />
  <input aria-label={nm('tags')} placeholder={nm('tags')} value={draft.value.tags.join(', ')} oninput={event => { draft.value.tags = event.currentTarget.value.split(',').map(v => v.trim()).filter(Boolean); changed(); }} />
  <details><summary>{nm('profiles')} ({draft.value.profile_ids.length})</summary>
    {#each profiles as profile (profile.id)}<label class="host"><input type="checkbox" checked={draft.value.profile_ids.includes(profile.id)} onchange={event => { draft.value.profile_ids = event.currentTarget.checked ? [...draft.value.profile_ids, profile.id] : draft.value.profile_ids.filter(id => id !== profile.id); changed(); }} />{profile.name}</label>{/each}
  </details>
  {#if preview}
    <p class="hint">{nm('images')}</p>
    <article class="preview">{@html renderMarkdown(draft.value.markdown)}</article>
  {:else}
    <div class="format" role="toolbar" aria-label={nm('edit')}>
      <button disabled={!history.length} onclick={() => undo()}>{nm('undo')}</button><button disabled={!future.length} onclick={() => undo(true)}>{nm('redo')}</button>
      {#each tools as [key, before, after]}<button title={nm(key)} onclick={() => insert(before, after)}>{nm(key)}</button>{/each}
    </div>
    <textarea bind:this={textarea} aria-label={nm('body')} value={draft.value.markdown} oninput={event => { history.push(draft.value.markdown); if (history.length > 100) history.shift(); future = []; draft.value.markdown = event.currentTarget.value; changed(); }} onkeydown={event => { if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's') { event.preventDefault(); void store.save(note.id).catch(e => onError(String(e))); } if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'z') { event.preventDefault(); undo(event.shiftKey); } }}></textarea>
  {/if}
</div>
<style>
  .editor { display:flex; flex-direction:column; gap:10px; height:100%; min-height:0; padding:16px; overflow:auto; }
  .actions,.format { display:flex; gap:6px; flex-wrap:wrap; align-items:center; }
  button,input,textarea { color:inherit; background:var(--color-bg-soft); border:1px solid var(--color-border); border-radius:5px; padding:6px 9px; }
  button { cursor:pointer; font-size:12px; } button:disabled { opacity:.45; }
  .title { font-size:20px; font-weight:600; } textarea { resize:vertical; flex:1; min-height:220px; font-family:monospace; line-height:1.65; }
  .hint { opacity:.65; font-size:12px; } .error { color:var(--color-danger); } .host { display:flex; gap:8px; padding:4px; }
  .preview :global(table) { border-collapse:collapse; display:block; overflow:auto; } .preview :global(th), .preview :global(td) { border:1px solid var(--color-border); padding:6px 10px; } .preview :global(ul), .preview :global(ol) { padding-left:24px; } .preview :global(ul) { list-style:disc; } .preview :global(ol) { list-style:decimal; } .preview :global(a) { color:var(--color-accent-text); text-decoration:underline; } .preview :global(h1), .preview :global(h2), .preview :global(h3) { font-weight:600; margin:12px 0; } .preview :global(h1) { font-size:1.6em; } .preview :global(h2) { font-size:1.35em; }
  .preview { flex:1; line-height:1.7; overflow-wrap:anywhere; } .preview :global(pre) { overflow:auto; padding:12px; background:var(--color-panel-2); } .preview :global(blockquote) { border-left:3px solid currentColor; padding-left:12px; opacity:.8; }
</style>
