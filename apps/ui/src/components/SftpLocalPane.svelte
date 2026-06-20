<script lang="ts">
  import {
    Folder,
    FileText,
    RefreshCw,
    ChevronRight,
    Home,
    ArrowUp,
    FolderPlus,
    Pencil,
    Trash2,
  } from '@lucide/svelte';
  import type { LocalEntry } from '../lib/types';
  import { i18n } from '../lib/i18n.svelte';
  import { tauriInvoke } from '../lib/rpc';
  import { appConfirm, appPrompt } from '../lib/confirm.svelte';
  import {
    SFTP_DRAG_LOCAL,
    joinLocalPath,
    localBreadcrumbs,
    setSftpDragData,
    type LocalDragPayload,
  } from '../lib/sftpLocal';

  interface Props {
    cwd: string;
    entries: LocalEntry[];
    loading: boolean;
    listError: string | null;
    onRefresh: () => void;
    onNavigate: (path: string) => void;
    onGoUp: () => void;
    onGoHome: () => void;
    onDropRemote: (e: DragEvent) => void;
    onDropFiles: (e: DragEvent) => void;
    onDragOverPane: (e: DragEvent) => void;
    onError?: (msg: string) => void;
  }
  let {
    cwd,
    entries,
    loading,
    listError,
    onRefresh,
    onNavigate,
    onGoUp,
    onGoHome,
    onDropRemote,
    onDropFiles,
    onDragOverPane,
    onError,
  }: Props = $props();

  let paneEl = $state<HTMLElement | null>(null);
  let selectedNames = $state<Set<string>>(new Set());
  let focusedName = $state<string | null>(null);
  let lastSelectedName = $state<string | null>(null);
  let menuEntry = $state<LocalEntry | null>(null);
  let menuOpen = $state(false);
  let menuX = $state(0);
  let menuY = $state(0);
  let lastActionPosition = $state<{ x: number; y: number } | null>(null);

  const crumbs = $derived(localBreadcrumbs(cwd));
  const selectedEntries = $derived(entries.filter((entry) => selectedNames.has(entry.name)));

  $effect(() => {
    const valid = new Set(entries.map((entry) => entry.name));
    const next = new Set([...selectedNames].filter((name) => valid.has(name)));
    if (next.size !== selectedNames.size) selectedNames = next;
    if (focusedName && !valid.has(focusedName)) focusedName = entries[0]?.name ?? null;
  });

  function formatSize(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
    return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
  }

  function fullPath(entry: LocalEntry): string {
    return joinLocalPath(cwd, entry.name);
  }

  function dragPayload(entry: LocalEntry, path: string): LocalDragPayload {
    return { path, name: entry.name, kind: entry.kind, size: entry.size };
  }

  function onDragStartLocal(e: DragEvent, entry: LocalEntry, path: string) {
    if (!selectedNames.has(entry.name)) selectEntry(entry);
    setSftpDragData(e.dataTransfer, SFTP_DRAG_LOCAL, JSON.stringify(dragPayload(entry, path)));
  }

  function closeMenu() {
    menuEntry = null;
    menuOpen = false;
  }

  function reportError(prefix: string, err: unknown) {
    onError?.(`${prefix}: ${(err as Error).message}`);
  }

  function focusPane() {
    paneEl?.focus({ preventScroll: true });
  }

  function rememberActionPosition(ev?: MouseEvent) {
    if (ev) lastActionPosition = { x: ev.clientX, y: ev.clientY };
  }

  function selectEntry(entry: LocalEntry, ev?: MouseEvent) {
    focusPane();
    rememberActionPosition(ev);
    focusedName = entry.name;
    if (ev?.shiftKey && lastSelectedName) {
      const start = entries.findIndex((candidate) => candidate.name === lastSelectedName);
      const end = entries.findIndex((candidate) => candidate.name === entry.name);
      if (start >= 0 && end >= 0) {
        const [from, to] = start < end ? [start, end] : [end, start];
        selectedNames = new Set(entries.slice(from, to + 1).map((candidate) => candidate.name));
        return;
      }
    }
    if (ev?.ctrlKey || ev?.metaKey) {
      const next = new Set(selectedNames);
      if (next.has(entry.name)) next.delete(entry.name);
      else next.add(entry.name);
      selectedNames = next;
      lastSelectedName = entry.name;
      return;
    }
    selectedNames = new Set([entry.name]);
    lastSelectedName = entry.name;
  }

  function selectAll() {
    focusPane();
    selectedNames = new Set(entries.map((entry) => entry.name));
    focusedName = entries[0]?.name ?? null;
    lastSelectedName = focusedName;
  }

  function focusRelative(delta: number) {
    focusPane();
    if (entries.length === 0) return;
    const current = Math.max(0, entries.findIndex((entry) => entry.name === focusedName));
    const next = Math.max(0, Math.min(entries.length - 1, current + delta));
    const entry = entries[next];
    if (!entry) return;
    focusedName = entry.name;
    selectedNames = new Set([entry.name]);
    lastSelectedName = entry.name;
  }

  function focusedDialogPosition(): { x: number; y: number } | undefined {
    if (menuOpen) return { x: menuX, y: menuY };
    if (lastActionPosition) return lastActionPosition;
    const row = paneEl?.querySelector<HTMLElement>('.local-row.focused');
    if (row) {
      const rect = row.getBoundingClientRect();
      return { x: Math.round(rect.left + rect.width / 2), y: Math.round(rect.top + rect.height / 2) };
    }
    const rect = paneEl?.getBoundingClientRect();
    if (rect === undefined) return undefined;
    return { x: Math.round(rect.left + rect.width / 2), y: Math.round(rect.top + Math.min(rect.height / 2, 280)) };
  }

  function openMenu(entry: LocalEntry, ev: MouseEvent) {
    ev.preventDefault();
    ev.stopPropagation();
    focusPane();
    rememberActionPosition(ev);
    selectedNames = new Set([entry.name]);
    focusedName = entry.name;
    lastSelectedName = entry.name;
    menuEntry = entry;
    menuOpen = true;
    menuX = Math.min(ev.clientX, window.innerWidth - 190);
    menuY = Math.min(ev.clientY, window.innerHeight - 170);
    lastActionPosition = { x: menuX, y: menuY };
  }

  async function renameEntry(entry = entries.find((candidate) => candidate.name === focusedName)) {
    focusPane();
    closeMenu();
    if (!entry || loading) return;
    const nextName = await appPrompt(i18n.t('sftp.renamePrompt'), { defaultValue: entry.name, position: focusedDialogPosition() });
    const trimmed = nextName?.trim();
    if (!trimmed || trimmed === entry.name) return;
    if (trimmed.includes('/') || trimmed.includes('\\')) {
      onError?.('rename: name must not contain path separators');
      return;
    }
    try {
      await tauriInvoke<void>('local_rename', { from: fullPath(entry), to: joinLocalPath(cwd, trimmed) });
      onRefresh();
    } catch (err) {
      reportError('rename', err);
    }
  }

  async function deleteEntries(items = selectedEntries) {
    focusPane();
    closeMenu();
    if (items.length === 0 || loading) return;
    const label = items.length === 1 ? items[0]?.name ?? '' : i18n.t('sftp.deleteManyLabel', { count: items.length });
    const confirmed = await appConfirm(i18n.t('sftp.deleteConfirm', { name: label }), {
      danger: true,
      confirmLabel: i18n.t('common.delete'),
      position: focusedDialogPosition(),
    });
    if (!confirmed) return;
    try {
      for (const entry of items) {
        await tauriInvoke<boolean>('local_remove', { path: fullPath(entry), recursive: entry.kind === 'dir' });
      }
      selectedNames = new Set();
      focusedName = null;
      onRefresh();
    } catch (err) {
      reportError('delete', err);
    }
  }

  async function mkdirHere() {
    focusPane();
    closeMenu();
    if (loading) return;
    const name = await appPrompt(i18n.t('sftp.mkdirPrompt'), { position: focusedDialogPosition() });
    const trimmed = name?.trim();
    if (!trimmed) return;
    if (trimmed.includes('/') || trimmed.includes('\\')) {
      onError?.('mkdir: name must not contain path separators');
      return;
    }
    try {
      await tauriInvoke<void>('local_mkdir', { path: joinLocalPath(cwd, trimmed) });
      onRefresh();
    } catch (err) {
      reportError('mkdir', err);
    }
  }

  function openFocused() {
    const entry = entries.find((candidate) => candidate.name === focusedName);
    if (entry?.kind === 'dir') onNavigate(fullPath(entry));
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'F5') {
      e.preventDefault();
      onRefresh();
      return;
    }
    if (loading) return;
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'a') {
      e.preventDefault();
      selectAll();
      return;
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      focusRelative(1);
      return;
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault();
      focusRelative(-1);
      return;
    }
    if (e.key === 'Enter') {
      e.preventDefault();
      openFocused();
      return;
    }
    if (e.key === 'F2') {
      e.preventDefault();
      void renameEntry();
      return;
    }
    if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault();
      void deleteEntries();
    }
  }
</script>

<svelte:window onclick={closeMenu} onkeydown={(e) => e.key === 'Escape' && closeMenu()} />

<div
  bind:this={paneEl}
  class="local-pane-shell flex flex-col min-w-0 min-h-0 h-full border-r border-[var(--color-border-soft)] outline-none"
  role="listbox"
  tabindex="0"
  aria-label={i18n.t('sftp.localPane')}
  onkeydown={onKeydown}
  ondragover={onDragOverPane}
  ondrop={(e) => {
    if (e.dataTransfer?.types.includes(SFTP_DRAG_LOCAL)) return;
    onDropRemote(e);
    onDropFiles(e);
  }}
  oncontextmenu={(e) => {
    e.preventDefault();
    focusPane();
    rememberActionPosition(e);
    menuEntry = null;
    focusedName = null;
    lastSelectedName = null;
    selectedNames = new Set();
    menuOpen = true;
    menuX = Math.min(e.clientX, window.innerWidth - 190);
    menuY = Math.min(e.clientY, window.innerHeight - 170);
  }}
>
  <div class="local-pane-title px-2 py-1 shell-section-title border-b border-[var(--color-border-soft)]">
    {i18n.t('sftp.localPane')}
  </div>
  <div class="local-pane-toolbar flex items-center gap-1 px-2 py-1 border-b border-[var(--color-border-soft)] text-[12px]">
    <button type="button" class="toolbtn" onclick={onGoHome} title={i18n.t('common.home')}><Home size={13} /></button>
    <button type="button" class="toolbtn" onclick={onGoUp} title={i18n.t('common.up')}><ArrowUp size={13} /></button>
    <button type="button" class="toolbtn" onclick={onRefresh} title={i18n.t('common.refresh')}><RefreshCw size={13} class={loading ? 'animate-spin' : ''} /></button>
    <button type="button" class="toolbtn" onclick={() => { void mkdirHere(); }} title={i18n.t('sftp.newFolder')}><FolderPlus size={13} /></button>
    <div class="mx-1 flex items-center gap-0.5 flex-wrap text-[11px] text-[var(--color-fg-muted)] min-w-0">
      {#each crumbs as bc, i (bc.path)}
        {#if i > 0}<ChevronRight size={11} class="text-[var(--color-border)]" />{/if}
        <button type="button" class="hover:text-[var(--color-accent)] px-0.5 truncate max-w-[80px]" onclick={() => onNavigate(bc.path)}>
          {bc.label}
        </button>
      {/each}
    </div>
  </div>

  {#if listError}
    <div class="mx-2 mt-1 px-2 py-1 text-[11px] text-[var(--color-danger)] truncate">{listError}</div>
  {/if}

  <div class="relative flex-1 min-h-0 overflow-y-auto {loading && entries.length > 0 ? 'pointer-events-none opacity-70' : ''}">
    {#if loading && entries.length === 0 && !listError}
      <div class="px-3 py-4 text-[12px] text-[var(--color-fg-muted)]">{i18n.t('common.loading')}</div>
    {:else if entries.length === 0 && !listError}
      <div class="px-3 py-4 text-[12px] text-[var(--color-fg-muted)] italic">{i18n.t('sftp.emptyDirectory')}</div>
    {:else}
      <table class="w-full text-[12px]">
        <thead class="sticky top-0 local-table-head text-[10px] uppercase tracking-[0.12em] text-[var(--color-fg-muted)]">
          <tr>
            <th class="text-left px-2 py-1 font-normal">{i18n.t('sftp.name')}</th>
            <th class="text-right px-2 py-1 font-normal w-[72px]">{i18n.t('sftp.size')}</th>
          </tr>
        </thead>
        <tbody
          ondragover={onDragOverPane}
          ondrop={(e) => {
            e.stopPropagation();
            if (e.dataTransfer?.types.includes(SFTP_DRAG_LOCAL)) return;
            onDropRemote(e);
            onDropFiles(e);
          }}
        >
          {#each entries as e (e.name)}
            {@const path = fullPath(e)}
            {@const selected = selectedNames.has(e.name)}
            {@const focused = focusedName === e.name}
            <tr
              role="option"
              aria-selected={selected}
              class="local-row {selected ? 'selected' : ''} {focused ? 'focused' : ''}"
              draggable={e.kind === 'file' || e.kind === 'dir'}
              ondragstart={(ev) => onDragStartLocal(ev, e, path)}
              oncontextmenu={(ev) => openMenu(e, ev)}
              onclick={(ev) => selectEntry(e, ev)}
              ondblclick={(ev) => { focusPane(); rememberActionPosition(ev); if (e.kind === 'dir') onNavigate(path); }}
            >
              <td class="px-2 py-0.5 truncate">
                <div class="flex items-center gap-1.5 w-full text-left min-w-0">
                  {#if e.kind === 'dir'}
                    <Folder size={12} class="text-[var(--color-accent)] shrink-0" />
                  {:else}
                    <FileText size={12} class="text-[var(--color-fg-muted)] shrink-0" />
                  {/if}
                  <span class="truncate">{e.name}</span>
                </div>
              </td>
              <td class="px-2 py-0.5 text-right text-[var(--color-fg-muted)]">
                {e.kind === 'file' ? formatSize(e.size) : ''}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </div>
</div>

{#if menuOpen}
  <div
    class="panel local-context-menu fixed z-[80] min-w-[176px] py-1 text-[12px] text-[var(--color-fg)]"
    style="left: {menuX}px; top: {menuY}px;"
    role="menu"
    tabindex="-1"
    onkeydown={(e) => e.stopPropagation()}
    onclick={(e) => e.stopPropagation()}
  >
    {#if menuEntry}
      <button type="button" class="menu-item" onclick={() => { const entry = menuEntry; closeMenu(); if (entry?.kind === 'dir') onNavigate(fullPath(entry)); }}>
        <Folder size={13} />
        {i18n.t('sftp.contextOpen')}
      </button>
      <button type="button" class="menu-item" onclick={() => { void renameEntry(menuEntry ?? undefined); }}>
        <Pencil size={13} />
        {i18n.t('sftp.contextRename')}
      </button>
      <button type="button" class="menu-item danger" onclick={() => { void deleteEntries(selectedNames.has(menuEntry?.name ?? '') ? selectedEntries : menuEntry ? [menuEntry] : []); }}>
        <Trash2 size={13} />
        {selectedNames.size > 1 ? i18n.t('sftp.contextDeleteSelected', { count: selectedNames.size }) : i18n.t('sftp.contextDelete')}
      </button>
      <div class="my-1 border-t border-[var(--color-border-soft)]"></div>
    {/if}
    <button type="button" class="menu-item" onclick={() => { void mkdirHere(); }}>
      <FolderPlus size={13} />
      {i18n.t('sftp.newFolder')}
    </button>
    <button type="button" class="menu-item" onclick={() => { closeMenu(); onRefresh(); }}>
      <RefreshCw size={13} />
      {i18n.t('common.refresh')}
    </button>
  </div>
{/if}

<style>
  .local-pane-shell {
    background: var(--color-bg-soft);
  }

  .local-pane-title,
  .local-pane-toolbar,
  .local-table-head {
    background: var(--color-surface-raised);
  }

  .local-row {
    cursor: default;
    color: var(--color-fg);
  }

  .local-row:hover {
    background: var(--color-panel-2);
  }

  .local-row.selected {
    background: color-mix(in srgb, var(--color-accent) 24%, var(--color-panel-2));
  }

  .local-row.focused {
    box-shadow: inset 2px 0 0 var(--color-accent);
  }

  .local-context-menu {
    max-height: calc(100vh - 16px);
    overflow-y: auto;
    background: var(--color-panel);
    border-color: var(--color-border);
    box-shadow: var(--shadow-lg);
  }

  .menu-item {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    text-align: left;
    color: var(--color-fg);
  }

  .menu-item:hover {
    background: color-mix(in srgb, var(--color-accent) 12%, var(--color-panel-2));
  }

  .menu-item.danger:hover {
    color: var(--color-danger);
  }
</style>
