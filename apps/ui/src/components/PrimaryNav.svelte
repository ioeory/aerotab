<script lang="ts">
  import { Server, SquareTerminal, FolderOpen, Settings, PanelLeftClose, PanelLeftOpen, Palette, Activity, KeyRound, Network, ArrowLeftRight, FileCode, NotebookPen, ShieldCheck, ScrollText } from '@lucide/svelte';
  import { i18n } from '../lib/i18n.svelte';
  import Splitter from './Splitter.svelte';
  let width = $state(176);
  import type { WorkspacePage } from '../lib/workspacePages';
  const resources = [
    { id: 'monitoring', icon: Activity }, { id: 'keychain', icon: KeyRound },
    { id: 'proxies', icon: Network }, { id: 'forwarding', icon: ArrowLeftRight },
    { id: 'scripts', icon: FileCode }, { id: 'notes', icon: NotebookPen },
    { id: 'knownHosts', icon: ShieldCheck }, { id: 'logs', icon: ScrollText },
  ] as const;
  let { page, expanded, sessionCount, onNavigate, onToggle, onSettings, onAppearance }: {
    page: WorkspacePage; expanded: boolean; sessionCount: number;
    onNavigate: (page: WorkspacePage) => void;
    onToggle: () => void; onSettings: () => void; onAppearance: () => void;
  } = $props();
</script>

<nav style={`--primary-nav-width:${width}px`} class:collapsed={!expanded} class="primary-nav" aria-label={i18n.t('shell.navigation')} data-aerotab-focus-zone="navigation">
  <div class="brand"><span class="brand-mark">A</span>{#if expanded}<strong>AeroTab<span class="brand-dot">.</span></strong>{/if}</div>
  <div class="nav-items">
    <button class:active={page === 'hosts'} aria-current={page === 'hosts' ? 'page' : undefined} title={i18n.t('shell.hosts')} onclick={() => onNavigate('hosts')}><Server size={20}/>{#if expanded}<span>{i18n.t('shell.hosts')}</span>{/if}</button>
    <button class:active={page === 'sessions'} aria-current={page === 'sessions' ? 'page' : undefined} title={i18n.t('shell.sessions')} onclick={() => onNavigate('sessions')}><SquareTerminal size={20}/>{#if expanded}<span>{i18n.t('shell.sessions')}</span><small>{sessionCount}</small>{/if}</button>
    <button class:active={page === 'files'} aria-current={page === 'files' ? 'page' : undefined} title={i18n.t('shell.files')} onclick={() => onNavigate('files')}><FolderOpen size={20}/>{#if expanded}<span>{i18n.t('shell.files')}</span>{/if}</button>
    {#each resources as item (item.id)}
      <button class:active={page === item.id} aria-current={page === item.id ? 'page' : undefined} title={i18n.t(`shell.${item.id}`)} onclick={() => onNavigate(item.id)}>
        <item.icon size={20}/>{#if expanded}<span>{i18n.t(`shell.${item.id}`)}</span>{/if}
      </button>
    {/each}
  </div>
  <div class="nav-bottom">
    <button title={i18n.t('shell.appearance')} onclick={onAppearance}><Palette size={20}/>{#if expanded}<span>{i18n.t('shell.appearance')}</span>{/if}</button>
    <button title={i18n.t('shell.settings')} onclick={onSettings}><Settings size={20}/>{#if expanded}<span>{i18n.t('shell.settings')}</span>{/if}</button>
    <button title={i18n.t(expanded ? 'app.footer.hideSidebar' : 'app.footer.showSidebar')} onclick={onToggle}>{#if expanded}<PanelLeftClose size={20}/><span>{i18n.t('shell.collapse')}</span>{:else}<PanelLeftOpen size={20}/>{/if}</button>
  </div>
</nav>
{#if expanded}<div class="nav-resizer"><Splitter value={width} min={140} max={320} label={i18n.t('shell.navigation')} onChange={v=>width=v} storageKey="primary-nav" /></div>{/if}

<style>
  .primary-nav{width:var(--primary-nav-width,176px);flex-shrink:0;display:flex;flex-direction:column;padding:20px 10px 10px;background:var(--color-panel);border-right:1px solid var(--color-border-soft);gap:28px;overflow:hidden}
  .primary-nav.collapsed{width:60px;padding-inline:7px}
  .brand{display:flex;align-items:center;gap:10px;padding:0 8px;min-height:32px;white-space:nowrap;font-size:22px}.brand-mark{display:grid;place-items:center;background:var(--color-accent);color:var(--color-accent-fg,#101923);width:28px;height:28px;flex-shrink:0;border-radius:7px;font-size:18px;font-weight:700}.brand-dot{color:var(--color-accent)}
  .nav-items,.nav-bottom{display:flex;flex-direction:column;gap:6px}.nav-items{min-height:0;overflow-y:auto;flex-shrink:1}.nav-bottom{margin-top:auto}button{display:flex;align-items:center;gap:12px;min-height:44px;padding:10px 12px;border:0;border-radius:7px;background:transparent;color:var(--color-fg-muted);font:inherit;font-size:var(--text-ui,16px);cursor:pointer;text-align:left;white-space:nowrap}button:hover{background:var(--color-panel-2);color:var(--color-fg)}button.active{background:var(--color-accent-soft);color:var(--color-accent)}button:focus-visible{outline:2px solid var(--color-accent);outline-offset:-2px}small{margin-left:auto;font-size:12px;background:var(--color-panel-2);padding:1px 6px;border-radius:4px}.collapsed button{justify-content:center;padding:10px}.collapsed .brand{padding:0;justify-content:center}
  .nav-resizer{display:flex}
  @media(max-width:900px){.nav-resizer{display:none}.primary-nav{width:60px;padding-inline:7px}.primary-nav .brand strong,.primary-nav button span,.primary-nav button small{display:none}.primary-nav button{justify-content:center;padding:10px}.primary-nav .brand{padding:0;justify-content:center}}
</style>
