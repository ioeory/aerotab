<script lang="ts">
  import { Plus, Pencil, Trash2, Terminal as TerminalIcon, Server, Usb, FolderOpen, Settings as SettingsIcon, Star } from '@lucide/svelte';
  import type { RpcClient } from '../lib/rpc';
  import type { SessionMeta, StoredProfile } from '../lib/types';
  import { tabs } from '../lib/tabs.svelte';
  import { dispatchFocusPane } from '../lib/focusPane';
  import { sortProfiles } from '../lib/profileMeta';
  import { i18n } from '../lib/i18n.svelte';
  import ProfileIcon from './ProfileIcon.svelte';

  interface Props {
    rpc: RpcClient;
    onError: (msg: string) => void;
    openProfileModal: (existing?: StoredProfile) => void;
    openSerialModal: () => void;
    openSftp: (p: StoredProfile) => void;
    openSettings: () => void;
  }
  let { rpc, onError, openProfileModal, openSerialModal, openSftp, openSettings }: Props = $props();

  let profiles = $state<StoredProfile[]>([]);
  const visibleProfiles = $derived(sortProfiles(profiles));

  export async function refresh() {
    try {
      profiles = await rpc.call<StoredProfile[]>('profile.list');
    } catch {
      profiles = [];
    }
  }
  $effect(() => {
    void refresh();
  });

  async function openLocal() {
    try {
      const meta = await rpc.call<SessionMeta>('session.openLocal', { title: 'local' });
      tabs.add(meta);
    } catch (e) {
      onError(`local: ${(e as Error).message}`);
    }
  }

  async function openProfile(p: StoredProfile, mode: 'new-tab' | 'split-right' | 'split-down' = 'new-tab') {
    try {
      const meta = await rpc.call<SessionMeta>('session.openSsh', {
        title: p.name,
        rows: 24,
        cols: 80,
        profile: p.ssh,
      });
      const activeTab = tabs.tabs.find((t) => t.id === tabs.activeId);
      if (mode !== 'new-tab' && activeTab) {
        tabs.addPane(activeTab.id, { ...meta, profileId: p.id, sshProfile: p.ssh }, mode === 'split-down' ? 'col' : 'row');
      } else {
        tabs.add({ ...meta, profileId: p.id, sshProfile: p.ssh });
      }
    } catch (e) {
      onError(`ssh: ${(e as Error).message}`);
    }
  }

  // Profile context menu state (right-click).
  let menuOpen = $state(false);
  let menuX = $state(0);
  let menuY = $state(0);
  let menuProfile = $state<StoredProfile | null>(null);

  function showMenu(p: StoredProfile, ev: MouseEvent) {
    ev.preventDefault();
    ev.stopPropagation();
    menuProfile = p;
    menuX = ev.clientX;
    menuY = ev.clientY;
    menuOpen = true;
  }
  function closeMenu() { menuOpen = false; menuProfile = null; }

  // Each menu action captures the profile into a local before calling
  // closeMenu(), because `menuProfile` is reactive state and would otherwise
  // be read as `null` by the time the action consumes it.
  function menuOpenInNewTab(p: StoredProfile) { closeMenu(); void openProfile(p, 'new-tab'); }
  function menuSplitRight(p: StoredProfile) { closeMenu(); void openProfile(p, 'split-right'); }
  function menuSplitDown(p: StoredProfile) { closeMenu(); void openProfile(p, 'split-down'); }
  function menuOpenSftp(p: StoredProfile) { closeMenu(); openSftp(p); }
  function menuEdit(p: StoredProfile) { closeMenu(); void editProfile(p); }
  function menuDelete(p: StoredProfile, ev: Event) { closeMenu(); void deleteProfile(p, ev); }

  async function latestProfile(p: StoredProfile): Promise<StoredProfile> {
    try {
      return await rpc.call<StoredProfile>('profile.get', { id: p.id });
    } catch (e) {
      onError(`profile refresh: ${(e as Error).message}`);
      return p;
    }
  }

  async function editProfile(p: StoredProfile) {
    const tab = tabs.tabs.find((candidate) =>
      candidate.panes.some((pane) => pane.profileId === p.id),
    );
    if (tab) {
      tabs.activate(tab.id);
      const pane = tab.panes.find((candidate) => candidate.profileId === p.id);
      if (pane) {
        tabs.focusPane(tab.id, pane.id);
        requestAnimationFrame(() => dispatchFocusPane(pane.id));
      }
    }
    openProfileModal(await latestProfile(p));
  }

  async function deleteProfile(p: StoredProfile, ev: Event) {
    ev.stopPropagation();
    if (!confirm(i18n.t('sidebar.deleteProfileConfirm', { name: p.name }))) return;
    try {
      await rpc.call('profile.delete', { id: p.id });
      await refresh();
    } catch (e) {
      onError((e as Error).message);
    }
  }
</script>

<aside class="w-[240px] shrink-0 border-r border-[var(--color-border-soft)] bg-[var(--color-panel)] flex flex-col">
  <div class="px-4 py-3 border-b border-[var(--color-border-soft)] flex items-center gap-2">
    <div class="w-6 h-6 rounded-md bg-[var(--color-accent)] text-[var(--color-bg)] grid place-items-center font-bold text-[12px]">›_</div>
    <h1 class="text-[13px] font-semibold tracking-wide">Tabby v2</h1>
    <button
      type="button"
      onclick={openSettings}
      class="ml-auto p-1 rounded text-[var(--color-fg-muted)] hover:text-[var(--color-fg)] hover:bg-[var(--color-panel-2)]"
      title={i18n.t('sidebar.settings')}
      aria-label={i18n.t('sidebar.settings')}
    >
      <SettingsIcon size={14} />
    </button>
  </div>

  <div class="px-2 py-2 flex flex-col gap-1">
    <button
      type="button"
      onclick={openLocal}
      class="flex items-center gap-2 px-3 py-2 rounded-md text-[12.5px] text-left
             hover:bg-[var(--color-panel-2)] text-[var(--color-fg)]"
    >
      <TerminalIcon size={14} class="text-[var(--color-accent)]" />
      <span class="flex-1">{i18n.t('sidebar.newLocalShell')}</span>
      <Plus size={12} class="text-[var(--color-fg-muted)]" />
    </button>
    <button
      type="button"
      onclick={() => openProfileModal()}
      class="flex items-center gap-2 px-3 py-2 rounded-md text-[12.5px] text-left
             hover:bg-[var(--color-panel-2)] text-[var(--color-fg)]"
    >
      <Server size={14} class="text-[var(--color-accent)]" />
      <span class="flex-1">{i18n.t('sidebar.newSshProfile')}</span>
      <Plus size={12} class="text-[var(--color-fg-muted)]" />
    </button>
    <button
      type="button"
      onclick={openSerialModal}
      class="flex items-center gap-2 px-3 py-2 rounded-md text-[12.5px] text-left
             hover:bg-[var(--color-panel-2)] text-[var(--color-fg)]"
    >
      <Usb size={14} class="text-[var(--color-accent)]" />
      <span class="flex-1">{i18n.t('sidebar.newSerialConnection')}</span>
      <Plus size={12} class="text-[var(--color-fg-muted)]" />
    </button>
  </div>

  <div class="px-3 pt-3 pb-1 text-[10.5px] uppercase tracking-[0.12em] text-[var(--color-fg-muted)]">
    {i18n.t('sidebar.sshProfiles')}
  </div>
  <div class="flex-1 overflow-y-auto px-2 pb-3 flex flex-col gap-0.5">
    {#each visibleProfiles as p (p.id)}
      <div class="group flex items-center gap-1 rounded-md hover:bg-[var(--color-panel-2)]"
           role="presentation"
           oncontextmenu={(e) => showMenu(p, e)}>
        <div class="pl-2">
          <ProfileIcon icon={p.icon} name={p.name} size={13} />
        </div>
        <button
          type="button"
          onclick={() => openProfile(p)}
          class="flex-1 min-w-0 text-left px-3 py-1.5 text-[12px]"
          title={i18n.t('sidebar.profileTooltip', { user: p.ssh.user, host: p.ssh.host, port: p.ssh.port })}
        >
          <div class="flex items-center gap-1 truncate text-[var(--color-fg)]">
            <span class="truncate">{p.name}</span>
            {#if p.favorite}
              <Star size={10} class="shrink-0 text-[var(--color-accent)]" fill="currentColor" />
            {/if}
          </div>
          <div class="truncate text-[10.5px] text-[var(--color-fg-muted)]">
            {p.ssh.user}@{p.ssh.host}:{p.ssh.port}
          </div>
          {#if (p.tags ?? []).length > 0}
            <div class="mt-1 flex gap-1 overflow-hidden">
              {#each (p.tags ?? []).slice(0, 3) as tag (tag)}
                <span class="shrink-0 max-w-[64px] truncate rounded-full border border-[var(--color-border-soft)] px-1.5 text-[9.5px] text-[var(--color-fg-muted)]">{tag}</span>
              {/each}
            </div>
          {/if}
        </button>
        <button
          type="button"
          class="opacity-0 group-hover:opacity-100 p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-accent)]"
          onclick={(e) => { e.stopPropagation(); openSftp(p); }}
          title={i18n.t('sidebar.openSftpBrowser')}
          aria-label={i18n.t('sidebar.openSftpBrowser')}
        >
          <FolderOpen size={12} />
        </button>
        <button
          type="button"
          class="opacity-0 group-hover:opacity-100 p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-accent)]"
          onclick={(e) => { e.stopPropagation(); void editProfile(p); }}
          title={i18n.t('common.edit')}
          aria-label={i18n.t('sidebar.editProfile')}
        >
          <Pencil size={12} />
        </button>
        <button
          type="button"
          class="opacity-0 group-hover:opacity-100 p-1 mr-1 text-[var(--color-fg-muted)] hover:text-[var(--color-danger)]"
          onclick={(e) => deleteProfile(p, e)}
          title={i18n.t('common.delete')}
          aria-label={i18n.t('sidebar.deleteProfile')}
        >
          <Trash2 size={12} />
        </button>
      </div>
    {:else}
      <div class="px-3 py-2 text-[11.5px] text-[var(--color-fg-muted)] italic">
        {i18n.t('sidebar.noProfiles')}
      </div>
    {/each}
  </div>
</aside>

{#if menuOpen && menuProfile}
  {@const mp = menuProfile}
  <div role="presentation" class="fixed inset-0 z-[55]" onclick={closeMenu}
       oncontextmenu={(e) => { e.preventDefault(); closeMenu(); }}>
    <div role="menu" tabindex="-1"
         class="absolute min-w-[200px] bg-[var(--color-panel)] border border-[var(--color-border)]
                rounded shadow-xl py-1 text-[12.5px] text-[var(--color-fg)]"
         style="left:{menuX}px; top:{menuY}px;"
          onkeydown={(e) => e.stopPropagation()}
         onclick={(e) => e.stopPropagation()}>
      <button type="button" class="menu-item" onclick={() => menuOpenInNewTab(mp)}>
        {i18n.t('sidebar.openInNewTab')}
      </button>
      <button type="button" class="menu-item" onclick={() => menuSplitRight(mp)}>
        {i18n.t('sidebar.splitRightCurrent')}
      </button>
      <button type="button" class="menu-item" onclick={() => menuSplitDown(mp)}>
        {i18n.t('sidebar.splitDownCurrent')}
      </button>
      <div class="my-1 border-t border-[var(--color-border-soft)]"></div>
      <button type="button" class="menu-item" onclick={() => menuOpenSftp(mp)}>
        {i18n.t('sidebar.openSftpBrowser')}
      </button>
      <div class="my-1 border-t border-[var(--color-border-soft)]"></div>
      <button type="button" class="menu-item" onclick={() => menuEdit(mp)}>
        {i18n.t('sidebar.editProfile')}...
      </button>
      <button type="button" class="menu-item text-[var(--color-danger)]"
              onclick={(e) => menuDelete(mp, e)}>
        {i18n.t('sidebar.deleteProfile')}
      </button>
    </div>
  </div>
{/if}
