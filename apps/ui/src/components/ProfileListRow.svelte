<script lang="ts">
  import {
    FolderOpen, Image, Pencil, Plug, ShieldAlert, ShieldCheck, ShieldX, Star, StickyNote, Tags, Trash2,
  } from '@lucide/svelte';
  import type { ProfileHealthResult, StoredProfile } from '../lib/types';
  import { profileEndpointLabel } from '../lib/profileMeta';
  import { i18n } from '../lib/i18n.svelte';
  import ProfileIcon from './ProfileIcon.svelte';
  import ProfileKindBadge from './ProfileKindBadge.svelte';
  import ProfileTag from './ProfileTag.svelte';
  import { portal } from '../lib/portal';

  export type ProfileQuickAction = 'note' | 'tags' | 'icon' | 'rename' | 'sftp';

  interface Props {
    profile: StoredProfile;
    variant?: 'sidebar' | 'settings';
    depth?: number;
    health?: ProfileHealthResult;
    healthIssues?: ProfileHealthResult['checks'];
    focused?: boolean;
    selected?: boolean;
    showSelection?: boolean;
    draggable?: boolean;
    onOpen?: () => void;
    onClick?: (ev: MouseEvent) => void;
    onCheckboxToggle?: () => void;
    onContextMenu?: (ev: MouseEvent) => void;
    onDragStart?: (ev: DragEvent) => void;
    onKeydown?: (ev: KeyboardEvent) => void;
    onQuickAction?: (action: ProfileQuickAction, ev: MouseEvent) => void;
    onConnect?: () => void;
    onEdit?: () => void;
    onRemove?: () => void;
    renaming?: boolean;
    renameDraft?: string;
    onRenameDraftChange?: (value: string) => void;
    onRenameCommit?: () => void;
    onRenameCancel?: () => void;
  }

  let {
    profile: p,
    variant = 'sidebar',
    depth = 0,
    health: h,
    healthIssues = [],
    focused = false,
    selected = false,
    showSelection = false,
    draggable = false,
    onOpen,
    onClick,
    onCheckboxToggle,
    onContextMenu,
    onDragStart,
    onKeydown,
    onQuickAction,
    onConnect,
    onEdit,
    onRemove,
    renaming = false,
    renameDraft = '',
    onRenameDraftChange,
    onRenameCommit,
    onRenameCancel,
  }: Props = $props();

  const tagLimit = $derived(variant === 'sidebar' ? 3 : 6);
  const allTags = $derived(p.tags ?? []);
  const visibleTags = $derived(allTags.slice(0, tagLimit));
  const extraTagCount = $derived(Math.max(0, allTags.length - tagLimit));
  const hiddenTagNames = $derived(allTags.slice(tagLimit).join(', '));

  let tagPopoverOpen = $state(false);
  let tagPopoverPos = $state({ left: 0, top: 0 });
  let tagPopoverCloseTimer: ReturnType<typeof setTimeout> | undefined;

  function updateTagPopoverPos(el: HTMLElement) {
    const rect = el.getBoundingClientRect();
    tagPopoverPos = { left: rect.left, top: rect.bottom + 4 };
  }

  function openTagPopover(el: HTMLElement) {
    updateTagPopoverPos(el);
    tagPopoverOpen = true;
  }

  function cancelTagPopoverClose() {
    if (tagPopoverCloseTimer !== undefined) {
      clearTimeout(tagPopoverCloseTimer);
      tagPopoverCloseTimer = undefined;
    }
  }

  function scheduleTagPopoverClose() {
    cancelTagPopoverClose();
    tagPopoverCloseTimer = setTimeout(() => {
      tagPopoverOpen = false;
      tagPopoverCloseTimer = undefined;
    }, 120);
  }

  function onTagMoreMouseEnter(ev: MouseEvent) {
    cancelTagPopoverClose();
    openTagPopover(ev.currentTarget as HTMLElement);
  }

  function onTagMoreClick(ev: MouseEvent) {
    ev.stopPropagation();
    cancelTagPopoverClose();
    const el = ev.currentTarget as HTMLElement;
    if (tagPopoverOpen) {
      tagPopoverOpen = false;
    } else {
      openTagPopover(el);
    }
  }

  $effect(() => {
    if (!tagPopoverOpen) return;
    const onScroll = () => { tagPopoverOpen = false; };
    const onKey = (ev: KeyboardEvent) => {
      if (ev.key === 'Escape') tagPopoverOpen = false;
    };
    window.addEventListener('scroll', onScroll, true);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('scroll', onScroll, true);
      window.removeEventListener('keydown', onKey);
    };
  });

  function healthLabel(status: ProfileHealthResult['status']): string {
    if (status === 'ok') return i18n.t('profiles.healthOk');
    if (status === 'warning') return i18n.t('profiles.healthWarning');
    return i18n.t('profiles.healthError');
  }

  function healthTitle(result: ProfileHealthResult): string {
    const issues = result.checks.filter((c) => c.status !== 'ok');
    const visible = issues.length > 0 ? issues : result.checks.slice(0, 1);
    return visible.map((c) => `${c.name}: ${c.message}`).join('\n') || i18n.t('profiles.healthNoIssues');
  }

  function quickAction(ev: MouseEvent, action: ProfileQuickAction) {
    ev.stopPropagation();
    onQuickAction?.(action, ev);
  }

  function onRowKeydown(ev: KeyboardEvent) {
    if (renaming) return;
    onKeydown?.(ev);
    if (!ev.defaultPrevented && ev.key === 'Enter' && onOpen) {
      onOpen();
      ev.preventDefault();
    }
  }

  function onRenameKeydown(ev: KeyboardEvent) {
    if (ev.key === 'Enter') {
      ev.preventDefault();
      ev.stopPropagation();
      onRenameCommit?.();
    } else if (ev.key === 'Escape') {
      ev.preventDefault();
      ev.stopPropagation();
      onRenameCancel?.();
    }
  }
</script>

<div
  class="profile-list-row profile-list-row--{variant} group
         {focused ? 'profile-list-row--focused' : ''}
         {selected ? 'profile-list-row--selected' : ''}"
  style={variant === 'sidebar' ? `--depth: ${depth}` : undefined}
  role="group"
  aria-label={p.name}
  title={variant === 'sidebar'
    ? `${profileEndpointLabel(p)} — ${i18n.t('sidebar.profileRowHint')}${p.note ? `\n${p.note}` : ''}`
    : undefined}
  draggable={draggable}
  ondragstart={onDragStart}
  oncontextmenu={onContextMenu}
>
  {#if variant === 'sidebar'}
    <div class="profile-list-row-indent shrink-0" aria-hidden="true"></div>
  {/if}
  <input
    type="checkbox"
    class="profile-list-row-check shrink-0"
    class:always-visible={selected || showSelection}
    checked={selected}
    onclick={(ev) => {
      ev.stopPropagation();
      onCheckboxToggle?.();
    }}
    aria-label={p.name}
  />
  <div class="profile-list-row-content min-w-0 flex-1">
    <button
      type="button"
      class="profile-list-row-main"
      onclick={(ev) => { if (!renaming) onClick?.(ev); }}
      ondblclick={() => { if (!renaming) onOpen?.(); }}
      onkeydown={onRowKeydown}
    >
      <ProfileIcon icon={p.icon} name={p.name} kind={p.kind} size={variant === 'sidebar' ? 12 : 14} />
      <div class="profile-list-row-body min-w-0 flex-1 text-left">
        <div class="profile-list-row-title flex items-center gap-1 truncate">
          {#if renaming}
            <!-- svelte-ignore a11y_autofocus -->
            <input
              type="text"
              class="profile-inline-rename input min-w-0 flex-1"
              value={renameDraft}
              autofocus
              onclick={(ev) => ev.stopPropagation()}
              oninput={(ev) => onRenameDraftChange?.((ev.currentTarget as HTMLInputElement).value)}
              onkeydown={onRenameKeydown}
              onblur={() => onRenameCommit?.()}
              aria-label={i18n.t('sidebar.renameProfile')}
            />
          {:else}
            <span class="truncate">{p.name}</span>
          {/if}
          <ProfileKindBadge kind={p.kind} compact />
          {#if h}
            <span class="health-chip {h.status}" title={healthTitle(h)} aria-label={healthLabel(h.status)}>
              {#if h.status === 'ok'}
                <ShieldCheck size={10} />
              {:else if h.status === 'warning'}
                <ShieldAlert size={10} />
              {:else}
                <ShieldX size={10} />
              {/if}
            </span>
          {/if}
          {#if p.favorite}
            <Star size={10} class="shrink-0 text-[var(--color-accent)]" fill="currentColor" />
          {/if}
        </div>
        <div class="profile-list-row-endpoint truncate">{profileEndpointLabel(p)}</div>
        {#if variant === 'sidebar' && focused}
          <div class="profile-list-row-connect-hint truncate">{i18n.t('sidebar.profileConnectHint')}</div>
        {/if}
        {#if variant === 'sidebar' && p.note}
          <div class="profile-list-row-note truncate">{p.note}</div>
        {/if}
        {#if variant === 'settings' && h && h.status !== 'ok' && healthIssues.length > 0}
          <div class="profile-list-row-health-details">
            {#each healthIssues as check (`${p.id}-${check.name}`)}
              <span>{check.name}: {check.message}</span>
            {/each}
          </div>
        {/if}
      </div>
    </button>
    {#if allTags.length > 0}
      <div class="profile-list-row-tags">
        {#each visibleTags as tag (tag)}
          <ProfileTag {tag} compact={variant === 'sidebar'} />
        {/each}
        {#if extraTagCount > 0}
          <span class="profile-tag-more-wrap">
            <button
              type="button"
              class="profile-tag-more"
              aria-expanded={tagPopoverOpen}
              aria-label={i18n.t('profileTags.moreHidden', { count: extraTagCount, names: hiddenTagNames })}
              onclick={onTagMoreClick}
              onmouseenter={onTagMoreMouseEnter}
              onmouseleave={scheduleTagPopoverClose}
            >
              +{extraTagCount}
            </button>
          </span>
        {/if}
      </div>
    {/if}
  </div>
  {#if variant === 'sidebar'}
    <div class="profile-action-chips shrink-0">
      <button type="button" class="action-chip {p.note ? 'action-chip--active' : ''}" title={p.note?.trim() || i18n.t('sidebar.editNote')} aria-label={i18n.t('sidebar.editNote')} onclick={(ev) => quickAction(ev, 'note')}><StickyNote size={11} /></button>
      <button type="button" class="action-chip {(p.tags ?? []).length > 0 ? 'action-chip--active' : ''}" title={i18n.t('sidebar.editTags')} aria-label={i18n.t('sidebar.editTags')} onclick={(ev) => quickAction(ev, 'tags')}><Tags size={11} /></button>
      <button type="button" class="action-chip" title={i18n.t('sidebar.renameProfile')} aria-label={i18n.t('sidebar.renameProfile')} onclick={(ev) => quickAction(ev, 'rename')}><Pencil size={11} /></button>
      {#if p.kind === 'ssh'}
        <button type="button" class="action-chip" title={i18n.t('sidebar.openSftpBrowser')} aria-label={i18n.t('sidebar.openSftpBrowser')} onclick={(ev) => quickAction(ev, 'sftp')}><FolderOpen size={11} /></button>
      {/if}
      <button type="button" class="action-chip" title={i18n.t('sidebar.editIcon')} aria-label={i18n.t('sidebar.editIcon')} onclick={(ev) => quickAction(ev, 'icon')}><Image size={11} /></button>
    </div>
  {:else}
    <div class="profile-list-row-settings-actions shrink-0">
      <button type="button" class="action-chip" title={i18n.t('profiles.connect')} aria-label={i18n.t('profiles.connect')} onclick={() => onConnect?.()}><Plug size={12} /></button>
      <button type="button" class="action-chip" title={i18n.t('common.edit')} aria-label={i18n.t('common.edit')} onclick={() => onEdit?.()}><Pencil size={12} /></button>
      <button type="button" class="action-chip action-chip--danger" title={i18n.t('common.delete')} aria-label={i18n.t('common.delete')} onclick={() => onRemove?.()}><Trash2 size={12} /></button>
    </div>
  {/if}
</div>

{#if tagPopoverOpen && allTags.length > 0}
  <div
    use:portal
    class="profile-tag-all-popover"
    style:left="{tagPopoverPos.left}px"
    style:top="{tagPopoverPos.top}px"
    role="tooltip"
    aria-label={i18n.t('profileTags.allTags')}
    onmouseenter={cancelTagPopoverClose}
    onmouseleave={scheduleTagPopoverClose}
  >
    {#each allTags as tag (tag)}
      <ProfileTag {tag} compact={variant === 'sidebar'} />
    {/each}
  </div>
{/if}

<style>
  .profile-list-row {
    display: flex;
    align-items: center;
    gap: 6px;
    border-radius: 4px;
  }
  .profile-list-row--sidebar {
    gap: 6px;
    padding-left: calc(var(--depth, 0) * 10px);
  }
  .profile-list-row--settings {
    gap: 8px;
    padding: 6px 8px;
    margin-bottom: 4px;
    border: 1px solid var(--color-border-soft);
    background: var(--color-panel-2);
  }
  .profile-list-row-indent {
    width: 12px;
  }
  .profile-list-row-content {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .profile-list-row-main {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    min-width: 0;
    padding: 4px 0;
    background: transparent;
    border: none;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .profile-list-row--sidebar .profile-list-row-main {
    padding: 4px 0;
  }
  .profile-list-row-title {
    font-size: 11.5px;
    color: var(--color-fg);
  }
  .profile-inline-rename {
    font-size: 11.5px;
    padding: 1px 4px;
    height: 20px;
  }
  .profile-list-row--settings .profile-list-row-title {
    font-size: 12.5px;
    font-weight: 500;
  }
  .profile-list-row-endpoint {
    font-size: 10px;
    color: var(--color-fg-muted);
  }
  .profile-list-row-connect-hint {
    font-size: 9.5px;
    color: var(--color-accent);
    opacity: 0.75;
  }
  .profile-list-row--settings .profile-list-row-endpoint {
    font-size: 11px;
    font-family: var(--font-mono, ui-monospace, monospace);
  }
  .profile-list-row-note {
    font-size: 10px;
    color: var(--color-fg-muted);
    opacity: 0.85;
  }
  .profile-list-row-tags {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
    margin-top: 2px;
    padding-left: calc(var(--profile-tag-icon-offset, 18px));
    overflow: visible;
  }
  .profile-list-row--sidebar {
    --profile-tag-icon-offset: 18px;
  }
  .profile-list-row--settings {
    --profile-tag-icon-offset: 20px;
  }
  .profile-tag-more-wrap {
    position: relative;
    display: inline-flex;
  }
  .profile-tag-more {
    font-size: 10px;
    line-height: 1.2;
    color: var(--color-fg-muted);
    padding: 1px 5px;
    border-radius: 4px;
    background: color-mix(in srgb, var(--color-fg-muted) 12%, transparent);
    border: none;
    cursor: default;
  }
  .profile-tag-more-wrap:hover .profile-tag-more,
  .profile-tag-more[aria-expanded='true'] {
    color: var(--color-accent);
    background: color-mix(in srgb, var(--color-accent) 14%, transparent);
  }
  .profile-tag-all-popover {
    position: fixed;
    z-index: 120;
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    max-width: min(280px, calc(100vw - 16px));
    padding: 6px 8px;
    border-radius: 6px;
    border: 1px solid var(--color-border-soft);
    background: var(--color-panel);
    box-shadow: 0 4px 16px rgb(0 0 0 / 0.22);
    pointer-events: auto;
  }
  .profile-list-row-health-details {
    margin-top: 4px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: 10.5px;
    color: var(--color-fg-muted);
  }
  .profile-list-row-check {
    opacity: 0.55;
  }
  .profile-list-row-check.always-visible,
  .profile-list-row:hover .profile-list-row-check,
  .profile-list-row--focused .profile-list-row-check,
  .profile-list-row--selected .profile-list-row-check {
    opacity: 1;
  }
  .profile-list-row--sidebar:hover {
    background: var(--color-panel-2);
  }
  .profile-list-row--focused {
    background: color-mix(in srgb, var(--color-accent) 10%, var(--color-panel-2));
    outline: 1px solid color-mix(in srgb, var(--color-accent) 45%, transparent);
    outline-offset: -1px;
  }
  .profile-list-row--selected {
    background: color-mix(in srgb, var(--color-accent) 18%, var(--color-panel-2));
  }
  .profile-list-row--settings.profile-list-row--selected {
    border-color: color-mix(in srgb, var(--color-accent) 35%, var(--color-border-soft));
  }
  .health-chip {
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
  }
  .health-chip.ok { color: var(--color-success); }
  .health-chip.warning { color: var(--color-warning); }
  .health-chip.error { color: var(--color-danger); }
  .profile-action-chips,
  .profile-list-row-settings-actions {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .profile-action-chips {
    opacity: 0;
    pointer-events: none;
    padding-right: 2px;
  }
  .profile-list-row:hover .profile-action-chips,
  .profile-list-row--focused .profile-action-chips,
  .profile-list-row--selected .profile-action-chips,
  .profile-list-row:focus-within .profile-action-chips {
    opacity: 1;
    pointer-events: auto;
  }
  .action-chip {
    display: inline-grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border-radius: 4px;
    color: var(--color-fg-muted);
    background: transparent;
    border: none;
    padding: 0;
    cursor: pointer;
  }
  .action-chip--active {
    color: var(--color-accent);
  }
  .action-chip--danger:hover {
    color: var(--color-danger);
  }
  .action-chip:hover {
    color: var(--color-accent);
    background: var(--color-panel-2);
  }
</style>
