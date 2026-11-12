<script lang="ts">
  import { tick, untrack } from 'svelte';
  import type { Component } from 'svelte';
  import { Bold, ChevronLeft, Code, Ellipsis, Eye, FileDown, Heading, Image, Italic, Link, List, ListOrdered, ListTodo, Minus, Pencil, Quote, Redo2, RotateCw, Server, SquareCode, Strikethrough, Table, Tag, Undo2 } from '@lucide/svelte';
  import type { Note } from '../../lib/notesCore';
  import { renderMarkdown } from '../../lib/notesCore';
  import { applyFormat, continueList, shortcutFormat, type FormatKind, type TextEdit } from '../../lib/notesFormat';
  import type { NotesStore } from '../../lib/notes.svelte';
  import type { StoredProfile } from '../../lib/types';
  import { nm } from '../../lib/notesMessages';
  import { exportNote } from '../../lib/notesFiles';
  import { hydrateMermaid, mermaidBlocks, mermaidTheme } from '../../lib/notesMermaid';
  let { store, note, profiles = [], onError, onBack }: { store: NotesStore; note: Note; profiles?: StoredProfile[]; onError: (message: string) => void; onBack?: () => void } = $props();
  let draft = $state(untrack(() => store.draft(note)));
  $effect(() => { const current = note; untrack(() => { draft = store.draft(current); }); });
  let preview = $state(false);
  let previewEl = $state<HTMLElement>();
  const previewHtml = $derived(preview ? renderMarkdown(draft.value.markdown) : '');
  const readThemeMode = () => document.documentElement.dataset.themeMode;
  let themeMode = $state(readThemeMode());
  $effect(() => {
    const observer = new MutationObserver(() => { themeMode = readThemeMode(); });
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme-mode'] });
    return () => observer.disconnect();
  });
  $effect(() => {
    const el = previewEl, theme = mermaidTheme(themeMode);
    if (!el || !previewHtml) return;
    let stale = false;
    const [errorLabel, blockedMessage] = untrack(() => [nm('mermaidError'), nm('mermaidBlocked')]);
    void hydrateMermaid(mermaidBlocks(el, errorLabel), { theme, blockedMessage, isStale: () => stale });
    return () => { stale = true; };
  });
  let root = $state<HTMLDivElement>();
  let textarea = $state<HTMLTextAreaElement>();
  let history = $state<string[]>([]); let future = $state<string[]>([]); let historyId = ''; let lastSnapshot = 0;
  $effect(() => { if (note.id !== historyId) { history = []; future = []; historyId = note.id; } });
  type Tool = { kind: FormatKind; icon: Component; label: string; shortcut?: string };
  const primary: Tool[] = $derived([
    { kind: 'heading', icon: Heading, label: nm('headingTip') },
    { kind: 'bold', icon: Bold, label: nm('bold'), shortcut: 'Ctrl+B' },
    { kind: 'italic', icon: Italic, label: nm('italic'), shortcut: 'Ctrl+I' },
    { kind: 'unordered', icon: List, label: nm('unordered') },
    { kind: 'task', icon: ListTodo, label: nm('task') },
    { kind: 'code', icon: Code, label: nm('code'), shortcut: 'Ctrl+E' },
    { kind: 'link', icon: Link, label: nm('link'), shortcut: 'Ctrl+K' },
  ]);
  const secondary: Tool[] = $derived([
    { kind: 'strike', icon: Strikethrough, label: nm('strike'), shortcut: 'Ctrl+Shift+X' },
    { kind: 'quote', icon: Quote, label: nm('quote') },
    { kind: 'ordered', icon: ListOrdered, label: nm('ordered') },
    { kind: 'codeblock', icon: SquareCode, label: nm('codeblock') },
    { kind: 'table', icon: Table, label: nm('table') },
    { kind: 'separator', icon: Minus, label: nm('separator') },
    { kind: 'image', icon: Image, label: nm('image') },
  ]);
  const tip = (tool: Tool) => tool.shortcut ? `${tool.label} (${tool.shortcut})` : tool.label;
  function changed(): void { store.edit(note.id); }
  function save(): void { store.save(note.id).catch(e => onError(String(e))); }
  function snapshot(force = false): void {
    const now = Date.now();
    if (force || now - lastSnapshot > 1000) { history.push(draft.value.markdown); if (history.length > 100) history.shift(); }
    lastSnapshot = force ? 0 : now; future = [];
  }
  function commit(edit: TextEdit): void {
    snapshot(true); draft.value.markdown = edit.text; changed();
    void tick().then(() => { textarea?.focus(); textarea?.setSelectionRange(edit.start, edit.end); });
  }
  function format(kind: FormatKind): void {
    if (!textarea) return;
    commit(applyFormat(draft.value.markdown, textarea.selectionStart, textarea.selectionEnd, kind));
  }
  function undo(redo = false): void {
    const from = redo ? future : history, to = redo ? history : future;
    const value = from.pop(); if (value === undefined) return;
    to.push(draft.value.markdown); draft.value.markdown = value; lastSnapshot = 0; changed();
  }
  function closeMenu(event: Event): void { (event.currentTarget as HTMLElement).closest('details')?.removeAttribute('open'); }
  function dismissMenus(event: PointerEvent): void {
    for (const menu of root?.querySelectorAll<HTMLDetailsElement>('details[open][data-popover]') ?? []) if (!menu.contains(event.target as Node)) menu.open = false;
  }
  function keydown(event: KeyboardEvent): void {
    if (event.isComposing) return;
    const primaryKey = event.ctrlKey || event.metaKey, key = event.key.toLowerCase();
    if (primaryKey && key === 's') { event.preventDefault(); save(); return; }
    if (primaryKey && (key === 'z' || key === 'y')) { event.preventDefault(); undo(key === 'y' || event.shiftKey); return; }
    const kind = shortcutFormat(event);
    if (kind) { event.preventDefault(); format(kind); return; }
    if (event.key === 'Enter' && !primaryKey && !event.shiftKey && !event.altKey && textarea) {
      const edit = continueList(draft.value.markdown, textarea.selectionStart, textarea.selectionEnd);
      if (edit) { event.preventDefault(); commit(edit); }
    }
  }
  function toggleProfile(id: string, checked: boolean): void {
    draft.value.profile_ids = checked ? [...draft.value.profile_ids, id] : draft.value.profile_ids.filter(p => p !== id); changed();
  }
</script>
<svelte:window onpointerdown={dismissMenus} />
<div class="editor" bind:this={root}>
  <div class="bar">
    {#if onBack}<button class="icon" title={nm('back')} aria-label={nm('back')} onclick={onBack}><ChevronLeft size={16} /></button>{/if}
    <input aria-label={nm('noteTitle')} placeholder={nm('noteTitle')} class="title" bind:value={draft.value.title} oninput={changed} />
    <span role="status" class="status" class:error={draft.status === 'error'} class:dirty={draft.status !== 'saved' && draft.status !== 'error'}>{nm(draft.status)}</span>
    {#if draft.status === 'error'}<button class="icon danger" title={nm('retry')} aria-label={nm('retry')} onclick={save}><RotateCw size={15} /></button>{/if}
    <button class="icon" aria-pressed={preview} title={preview ? nm('edit') : nm('preview')} aria-label={preview ? nm('edit') : nm('preview')} onclick={() => preview = !preview}>
      {#if preview}<Pencil size={15} />{:else}<Eye size={15} />{/if}
    </button>
    <details class="menu" data-popover>
      <summary class="icon" title={nm('more')} aria-label={nm('more')}><Ellipsis size={16} /></summary>
      <div class="popover right">
        <button onclick={event => { closeMenu(event); exportNote(draft.value).catch(e => onError(String(e))); }}><FileDown size={14} />{nm('export')}</button>
      </div>
    </details>
  </div>
  <div class="meta">
    <label class="tags" title={nm('tags')}><Tag size={13} /><input aria-label={nm('tags')} placeholder={nm('tags')} value={draft.value.tags.join(', ')} oninput={event => { draft.value.tags = event.currentTarget.value.split(',').map(v => v.trim()).filter(Boolean); changed(); }} /></label>
    <details class="menu" data-popover>
      <summary class="chip" title={nm('profiles')} aria-label={`${nm('profiles')} (${draft.value.profile_ids.length})`}><Server size={13} />{draft.value.profile_ids.length}</summary>
      <div class="popover right hosts">
        {#each profiles as profile (profile.id)}<label class="host"><input type="checkbox" checked={draft.value.profile_ids.includes(profile.id)} onchange={event => toggleProfile(profile.id, event.currentTarget.checked)} />{profile.name}</label>
        {:else}<p class="muted">{nm('noHosts')}</p>{/each}
      </div>
    </details>
  </div>
  {#if preview}
    <article class="preview" bind:this={previewEl}>{@html previewHtml}</article>
    <p class="hint">{nm('images')}</p>
  {:else}
    <div class="format" role="toolbar" aria-label={nm('edit')}>
      <div class="tools">
      <button class="icon" title={`${nm('undo')} (Ctrl+Z)`} aria-label={nm('undo')} disabled={!history.length} onclick={() => undo()}><Undo2 size={15} /></button>
      <button class="icon" title={`${nm('redo')} (Ctrl+Y)`} aria-label={nm('redo')} disabled={!future.length} onclick={() => undo(true)}><Redo2 size={15} /></button>
      <span class="sep"></span>
      {#each primary as tool (tool.kind)}{@const Icon = tool.icon}<button class="icon" title={tip(tool)} aria-label={tool.label} onclick={() => format(tool.kind)}><Icon size={15} /></button>{/each}
      </div>
      <details class="menu" data-popover>
        <summary class="icon" title={nm('moreFormats')} aria-label={nm('moreFormats')}><Ellipsis size={15} /></summary>
        <div class="popover">
          {#each secondary as tool (tool.kind)}{@const Icon = tool.icon}<button title={tip(tool)} onclick={event => { closeMenu(event); format(tool.kind); }}><Icon size={14} />{tool.label}</button>{/each}
        </div>
      </details>
    </div>
    <textarea bind:this={textarea} aria-label={nm('body')} placeholder={nm('bodyPlaceholder')} spellcheck="false" value={draft.value.markdown} oninput={event => { snapshot(); draft.value.markdown = event.currentTarget.value; changed(); }} onkeydown={keydown}></textarea>
  {/if}
</div>
<style>
  .editor { display:flex; flex-direction:column; height:100%; min-height:0; color:var(--color-fg); }
  .bar { display:flex; align-items:center; gap:4px; padding:6px 8px; border-bottom:1px solid var(--color-border-soft); }
  .meta { display:flex; align-items:center; gap:6px; padding:4px 8px; border-bottom:1px solid var(--color-border-soft); }
  .format { display:flex; align-items:center; gap:2px; padding:3px 6px; border-bottom:1px solid var(--color-border-soft); flex-shrink:0; }
  .tools { display:flex; align-items:center; gap:2px; min-width:0; overflow-x:auto; scrollbar-width:none; }
  .bar,.meta { flex-shrink:0; }
  button,input,summary { color:inherit; font:inherit; }
  .icon { display:inline-flex; align-items:center; justify-content:center; width:28px; height:28px; flex-shrink:0; padding:0; border:0; border-radius:5px; background:none; cursor:pointer; color:var(--color-fg-muted); }
  .icon:hover:not(:disabled) { background:var(--color-surface-hover); color:var(--color-fg); }
  .icon[aria-pressed='true'] { background:var(--color-surface-selected); color:var(--color-fg); }
  .icon:disabled { opacity:.35; cursor:default; } .icon.danger { color:var(--color-danger); }
  .sep { width:1px; height:16px; margin:0 4px; background:var(--color-border); flex-shrink:0; }
  .title { flex:1; min-width:0; font-size:15px; font-weight:600; background:none; border:1px solid transparent; border-radius:5px; padding:4px 6px; }
  .title:hover,.title:focus { border-color:var(--color-border); outline:none; }
  .status { font-size:11px; color:var(--color-fg-subtle); white-space:nowrap; flex-shrink:0; } .status.dirty { color:var(--color-fg-muted); } .status.error { color:var(--color-danger); }
  .tags { display:flex; align-items:center; gap:5px; flex:1; min-width:0; color:var(--color-fg-muted); }
  .tags input { flex:1; min-width:0; background:none; border:0; padding:3px 0; font-size:12px; color:var(--color-fg); outline:none; }
  .chip { display:inline-flex; align-items:center; gap:4px; padding:2px 8px; border:1px solid var(--color-border); border-radius:999px; font-size:12px; color:var(--color-fg-muted); }
  .menu { position:relative; flex-shrink:0; } .menu>summary { list-style:none; cursor:pointer; } .menu>summary::-webkit-details-marker { display:none; }
  .popover { position:absolute; top:calc(100% + 4px); left:0; z-index:20; min-width:170px; max-height:260px; overflow:auto; display:flex; flex-direction:column; padding:4px; background:var(--color-surface-raised); border:1px solid var(--color-border); border-radius:7px; box-shadow:0 8px 24px var(--color-overlay); }
  .popover.right { left:auto; right:0; }
  .popover button { display:flex; align-items:center; gap:8px; width:100%; padding:6px 8px; border:0; border-radius:4px; background:none; cursor:pointer; text-align:left; font-size:12px; white-space:nowrap; }
  .popover button:hover { background:var(--color-surface-hover); }
  .host { display:flex; align-items:center; gap:8px; padding:5px 6px; font-size:12px; border-radius:4px; cursor:pointer; } .host:hover { background:var(--color-surface-hover); }
  .muted { margin:6px; font-size:12px; color:var(--color-fg-subtle); }
  textarea { flex:1; min-height:120px; resize:none; border:0; outline:none; padding:10px 12px; background:none; color:inherit; font-family:var(--font-mono, monospace); font-size:13px; line-height:1.65; tab-size:2; }
  .hint { margin:0; padding:6px 12px; font-size:11px; color:var(--color-fg-subtle); border-top:1px solid var(--color-border-soft); flex-shrink:0; }
  .preview { flex:1; min-height:0; overflow:auto; padding:10px 14px; line-height:1.7; overflow-wrap:anywhere; }
  .preview :global(table) { border-collapse:collapse; display:block; overflow:auto; } .preview :global(th), .preview :global(td) { border:1px solid var(--color-border); padding:6px 10px; } .preview :global(ul), .preview :global(ol) { padding-left:24px; } .preview :global(ul) { list-style:disc; } .preview :global(ol) { list-style:decimal; } .preview :global(a) { color:var(--color-accent-text); text-decoration:underline; } .preview :global(h1), .preview :global(h2), .preview :global(h3) { font-weight:600; margin:12px 0; } .preview :global(h1) { font-size:1.6em; } .preview :global(h2) { font-size:1.35em; }
  .preview :global(p) { margin:6px 0; } .preview :global(li:has(> input[type='checkbox'])) { list-style:none; margin-left:-20px; }
  .preview :global(pre) { overflow:auto; padding:12px; background:var(--color-panel-2); border-radius:6px; } .preview :global(code) { font-family:var(--font-mono, monospace); } .preview :global(blockquote) { border-left:3px solid var(--color-border); padding-left:12px; color:var(--color-fg-muted); }
  .preview :global(.mermaid-diagram) { margin:8px 0; } .preview :global(.mermaid-svg) { overflow:auto; text-align:center; } .preview :global(.mermaid-svg svg) { max-width:100%; height:auto; }
  .preview :global(.mermaid-error) { margin:4px 0 0; font-size:11px; color:var(--color-danger); }
</style>
