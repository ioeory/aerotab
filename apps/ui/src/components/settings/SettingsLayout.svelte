<script lang="ts">
  // Tabby-style multi-section settings modal: left navigation pane lists
  // categories; the selected section component is mounted on the right.
  // Replaces the legacy single-page SettingsPanel.svelte.

  import { onMount, onDestroy, tick } from 'svelte';
  import { X, RotateCcw, Search } from '@lucide/svelte';
  import { settingsText as text } from './settingsMessages';
  import { applyStoredSettingsToUi } from '../../lib/applyStoredSettings';
  import type { RpcClient } from '../../lib/rpc';
  import { i18n } from '../../lib/i18n.svelte';
  import { settingsCoord } from '../../lib/settingsStore.svelte';
  import { appConfirm, confirmUi } from '../../lib/confirm.svelte';

  import ApplicationSection from './sections/ApplicationSection.svelte';
  import AppearanceSection from './sections/AppearanceSection.svelte';
  import ProfilesSection from './sections/ProfilesSection.svelte';
  import TerminalSection from './sections/TerminalSection.svelte';
  import AISection from './sections/AISection.svelte';
  import McpSection from './sections/McpSection.svelte';
  import ColorSchemeSection from './sections/ColorSchemeSection.svelte';
  import ConfigSyncSection from './sections/ConfigSyncSection.svelte';
  import HotkeysSection from './sections/HotkeysSection.svelte';
  import PluginsSection from './sections/PluginsSection.svelte';
  import ShellSection from './sections/ShellSection.svelte';
  import SshSection from './sections/SshSection.svelte';
  import VaultSection from './sections/VaultSection.svelte';
  import WindowSection from './sections/WindowSection.svelte';
  import ConfigFileSection from './sections/ConfigFileSection.svelte';

  type SectionId =
    | 'application'
    | 'appearance'
    | 'profiles'
    | 'terminal'
    | 'ai'
    | 'mcp'
    | 'colorscheme'
    | 'configsync'
    | 'hotkeys'
    | 'plugins'
    | 'shell'
    | 'ssh'
    | 'vault'
    | 'window'
    | 'configfile';

  interface Props {
    rpc: RpcClient;
    buildId: string;
    initialSection?: SectionId;
    onClose: () => void;
    onError: (msg: string) => void;
    onSettingsChanged: () => void;
  }
  let { rpc, buildId, initialSection = 'appearance', onClose, onError: reportError, onSettingsChanged }: Props = $props();
  let errorMessage = $state('');
  function onError(message: string) {
    errorMessage = message;
    reportError(message);
  }

  interface NavEntry {
    id: SectionId;
    labelKey: string;
  }

  const groups: { titleKey: string; entries: NavEntry[] }[] = [
    {
      titleKey: 'settings.group.general',
      entries: [
        { id: 'application', labelKey: 'settings.nav.application' },
        { id: 'appearance', labelKey: 'settings.nav.appearance' },
        { id: 'profiles', labelKey: 'settings.nav.profiles' },
        { id: 'terminal', labelKey: 'settings.nav.terminal' },
        { id: 'ai', labelKey: 'settings.nav.ai' },
        { id: 'mcp', labelKey: 'settings.nav.mcp' },
        { id: 'colorscheme', labelKey: 'settings.nav.colorScheme' },
        { id: 'configsync', labelKey: 'settings.nav.configSync' },
        { id: 'hotkeys', labelKey: 'settings.nav.hotkeys' },
        { id: 'plugins', labelKey: 'settings.nav.plugins' },
      ],
    },
    {
      titleKey: 'settings.group.advanced',
      entries: [
        { id: 'shell', labelKey: 'settings.nav.shell' },
        { id: 'ssh', labelKey: 'settings.nav.ssh' },
        { id: 'vault', labelKey: 'settings.nav.vault' },
        { id: 'window', labelKey: 'settings.nav.window' },
        { id: 'configfile', labelKey: 'settings.nav.configFile' },
      ],
    },
  ];

  let active = $state<SectionId>('appearance');
  let visited = $state<SectionId[]>([]);
  let query = $state('');
  let saved = $state(false);
  let resetting = $state(false);
  let epoch = $state(0);
  let content: HTMLDivElement;
  let dialog: HTMLDivElement;
  onMount(() => {
    const origin = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    dialog.querySelector<HTMLElement>('input[type=search]')?.focus();
    return () => { if (origin?.isConnected) origin.focus(); };
  });
  const sections = groups.flatMap(group => group.entries);
  const searchDescriptions: Record<SectionId, string[]> = {
    application: ['application.general', 'application.language'],
    appearance: ['settings.appearance.mode', 'settings.appearance.light', 'settings.appearance.dark', 'settings.appearance.system', 'settings.appearance.font', 'settings.appearance.family', 'settings.appearance.fallback', 'settings.appearance.size', 'settings.appearance.weight', 'settings.appearance.boldWeight', 'settings.appearance.ligatures', 'settings.appearance.cursor', 'settings.appearance.cursorShape', 'settings.appearance.linePadding', 'settings.appearance.contrast', 'settings.appearance.accent', 'settings.appearance.accentHint', 'settings.appearance.customCss', 'settings.appearance.cssHint'],
    profiles: ['settings.nav.profiles'], terminal: ['settings.terminal.experimentalTransfers'],
    ai: ['settings.nav.ai'], mcp: ['mcp.tokenHint'], colorscheme: ['settings.nav.colorScheme'],
    configsync: ['settings.nav.configSync'], hotkeys: ['settings.nav.hotkeys'], plugins: ['settings.nav.plugins'],
    shell: ['settings.nav.shell'], ssh: ['settings.nav.ssh'], vault: ['settings.nav.vault'],
    window: ['window.frameTransparency', 'window.layout', 'window.restartHint'], configfile: ['settings.nav.configFile'],
  };
  function matches(entry: NavEntry) {
    const needle = query.trim().toLocaleLowerCase();
    return !needle || [entry.id, text(entry.labelKey), ...searchDescriptions[entry.id].map(text)].join(' ').toLocaleLowerCase().includes(needle);
  }
  function navigate(id: SectionId) { active = id; content?.scrollTo({ top: 0 }); }
  $effect(() => { if (!resetting && !visited.includes(active)) visited = [...visited, active]; });
  onDestroy(() => { settingsCoord.markClean(); });

  $effect(() => {
    active = initialSection;
  });

  async function save() {
    if (settingsCoord.saving) return;
    saved = false;
    errorMessage = '';
    try {
      await settingsCoord.saveAll();
      saved = !settingsCoord.dirty;
      onSettingsChanged();
    } catch (e) {
      if (settingsCoord.failedSection) navigate(settingsCoord.failedSection as SectionId);
      onError(`save: ${(e as Error).message}`);
    }
  }

  async function reset() {
    if (!(await appConfirm(i18n.t('settings.resetConfirm'), { danger: true, confirmLabel: i18n.t('common.delete') }))) return;
    resetting = true;
    try {
      await rpc.call('settings.reset');
      // Destroy previews before applying persisted defaults, then mount fresh drafts.
      visited = [];
      epoch += 1;
      await tick();
      await applyStoredSettingsToUi(rpc);
      visited = [active];
      settingsCoord.markClean();
      settingsCoord.bumpRev();
      onSettingsChanged();
    } catch (e) {
      onError((e as Error).message);
    } finally { resetting = false; }
  }

  async function requestClose() {
    if (settingsCoord.saving || resetting) return;
    if (settingsCoord.dirty) {
      if (!(await appConfirm(i18n.t('settings.unsavedCloseConfirm')))) return;
    }
    // Roll back application/window live previews as well as the appearance draft.
    onClose();
    await tick();
    await applyStoredSettingsToUi(rpc);
  }

  function onOverlayKeydown(ev: KeyboardEvent) {
    if (confirmUi.pending || ev.defaultPrevented) return;
    if (ev.key === 'Tab') {
      const focusable = [...dialog.querySelectorAll<HTMLElement>('button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), a[href], [tabindex="0"]')].filter(el => el.getClientRects().length > 0);
      const first = focusable[0];
      const last = focusable.at(-1);
      if (ev.shiftKey && (document.activeElement === first || !dialog.contains(document.activeElement))) {
        ev.preventDefault(); last?.focus();
      } else if (!ev.shiftKey && (document.activeElement === last || !dialog.contains(document.activeElement))) {
        ev.preventDefault(); first?.focus();
      }
    }
    if (ev.key === 'Escape' && !ev.defaultPrevented && !confirmUi.pending) {
      ev.preventDefault();
      void requestClose();
    }
  }
</script>

<svelte:window onkeydown={onOverlayKeydown} />

<div class="settings-overlay-root fixed inset-0 z-50" data-aerotab-modal="" data-settings-layout="">
  <!-- Full-screen backdrop: grid centering left gaps where wheel reached the terminal. -->
  <button
    type="button"
    class="settings-overlay-backdrop absolute inset-0 border-0 p-0 bg-black/60 cursor-default"
    aria-label={i18n.t('common.close')}
    onclick={() => { void requestClose(); }}
    onwheel={(e) => e.stopPropagation()}
  ></button>
  <div class="relative z-10 flex min-h-full w-full items-center justify-center p-6 pointer-events-none">
  <div
    bind:this={dialog}
    class="panel settings-overlay-panel pointer-events-auto flex flex-col overflow-hidden"
    role="dialog"
    aria-modal="true"
    aria-label={i18n.t('settings.title')}
  >
    <header class="settings-header">
      <div><h1>{i18n.t('settings.title')}</h1><p class="settings-description">{text('settings.draftHint')}</p></div>
      <span class="build-id">{buildId}</span>
      <button type="button" class="btn-ghost close-button" disabled={settingsCoord.saving || resetting}
        onclick={() => { void requestClose(); }} aria-label={i18n.t('common.close')}><X size={20} /></button>
    </header>

    {#if errorMessage}
      <div class="settings-error" role="alert">
        <span>{errorMessage}</span>
        <button type="button" class="btn-ghost" aria-label={i18n.t('common.close')} onclick={() => { errorMessage = ''; }}><X size={18} /></button>
      </div>
    {/if}
    <div class="settings-body">
      <nav class="settings-navigation settings-overlay-scroll" aria-label={i18n.t('settings.title')}>
        <label class="settings-search">
          <Search size={18} aria-hidden="true" />
          <input type="search" bind:value={query} placeholder={text('settings.search')} aria-label={text('settings.searchHint')} />
        </label>
        {#each groups as g (g.titleKey)}
          {#if g.entries.some(matches)}
            <h2 class="nav-group">{i18n.t(g.titleKey)}</h2>
            {#each g.entries.filter(matches) as e (e.id)}
              <button type="button" class="settings-nav-item" class:active={active === e.id}
                aria-current={active === e.id ? 'page' : undefined} aria-controls={`settings-section-${e.id}`}
                onclick={() => navigate(e.id)}>{i18n.t(e.labelKey)}</button>
            {/each}
          {/if}
        {/each}
        {#if !sections.some(matches)}<p class="search-empty" role="status">{i18n.t('common.noMatches')}</p>{/if}
      </nav>

      <div bind:this={content} class="settings-content settings-overlay-scroll">
        {#key epoch}
        {#each visited as section (section)}
        <div id={`settings-section-${section}`} hidden={active !== section} class="settings-section-container">
        {#if section === 'application'}
          <ApplicationSection {rpc} {buildId} {onError} />
        {:else if section === 'appearance'}
          <AppearanceSection {rpc} {onError} />
        {:else if section === 'profiles'}
          <ProfilesSection {rpc} {onError} />
        {:else if section === 'terminal'}
          <TerminalSection {rpc} {onError} />
        {:else if section === 'ai'}
          <AISection {rpc} {onError} />
        {:else if section === 'mcp'}
          <McpSection {rpc} {onError} />
        {:else if section === 'colorscheme'}
          <ColorSchemeSection {rpc} {onError} />
        {:else if section === 'configsync'}
          <ConfigSyncSection {rpc} {onError} onSyncApplied={onSettingsChanged} />
        {:else if section === 'hotkeys'}
          <HotkeysSection {rpc} {onError} />
        {:else if section === 'plugins'}
          <PluginsSection {rpc} {onError} />
        {:else if section === 'shell'}
          <ShellSection {rpc} {onError} />
        {:else if section === 'ssh'}
          <SshSection {rpc} {onError} />
        {:else if section === 'vault'}
          <VaultSection {rpc} {onError} />
        {:else if section === 'window'}
          <WindowSection {rpc} {onError} />
        {:else if section === 'configfile'}
          <ConfigFileSection {rpc} {onError} onChanged={onSettingsChanged} />
        {/if}
        </div>
        {/each}
        {/key}
      </div>
    </div>

    <footer class="settings-footer">
      <button type="button" onclick={reset} disabled={settingsCoord.saving || resetting} class="btn-secondary">
        <RotateCcw size={18} /> {i18n.t('common.reset')}
      </button>
      <span class="save-feedback" role="status" aria-live="polite">
        {settingsCoord.saving ? i18n.t('common.saving') : settingsCoord.dirty ? i18n.t('settings.unsaved') : saved ? text('settings.saved') : ''}
      </span>
      <div class="footer-actions">
        <button type="button" disabled={settingsCoord.saving || resetting} onclick={() => { void requestClose(); }} class="btn-secondary">{i18n.t('common.close')}</button>
        <button type="button" onclick={save} class="btn-primary" disabled={!settingsCoord.dirty || settingsCoord.saving || resetting}>
          {settingsCoord.saving ? i18n.t('common.saving') : text('settings.saveAll')}
        </button>
      </div>
    </footer>
  </div>
  </div>
</div>

<style>
  .settings-overlay-panel { width: min(1120px, 100%); height: min(900px, calc(100dvh - 48px)); font-size: 16px; line-height: 1.55; color: var(--color-fg); font-family: var(--font-ui, system-ui, sans-serif); }
  .settings-header { display: flex; align-items: center; gap: 16px; padding: 22px 28px; border-bottom: 1px solid var(--color-border); }
  .settings-header h1 { font-size: 26px; line-height: 1.3; font-weight: 600; margin: 0; }
  .settings-description { font-size: 14px; margin: 6px 0 0; }
  .build-id { margin-left: auto; font-size: 12px; overflow-wrap: anywhere; }
  .close-button { flex-shrink: 0; width: 44px; height: 44px; display: grid; place-items: center; }
  .settings-error { display: flex; align-items: center; gap: 12px; padding: 12px 24px; border-bottom: 1px solid var(--color-danger); background: var(--color-panel); color: var(--color-fg); font-size: 14px; max-height: 140px; overflow-y: auto; }
  .settings-error span { flex: 1; min-width: 0; overflow-wrap: anywhere; }
  .settings-body { display: grid; grid-template-columns: 252px minmax(0, 1fr); flex: 1; min-height: 0; }
  .settings-navigation { overflow-y: auto; padding: 18px 12px; border-right: 1px solid var(--color-border); background: var(--color-panel-2); }
  .settings-search { display: flex; align-items: center; gap: 8px; padding: 0 10px; min-height: 44px; border: 1px solid var(--color-border); border-radius: 8px; background: var(--color-bg); }
  .settings-search input { width: 100%; min-width: 0; border: 0; background: transparent; color: var(--color-fg); font: inherit; outline: none; }
  .settings-search:focus-within { outline: 2px solid var(--color-accent); outline-offset: 2px; }
  .nav-group { font-size: 14px; font-weight: 600; margin: 24px 12px 8px; }
  .settings-nav-item { width: 100%; text-align: left; min-height: 44px; padding: 10px 12px; border-radius: 8px; border: 1px solid transparent; font: inherit; overflow-wrap: anywhere; }
  .settings-nav-item:hover { background: var(--color-panel); }
  .settings-nav-item.active { background: color-mix(in srgb, var(--color-accent) 14%, var(--color-panel)); border-color: var(--color-accent); font-weight: 600; }
  .settings-content { overflow-y: auto; padding: 28px 32px 40px; min-width: 0; }
  .settings-section-container { max-width: 760px; margin-inline: auto; }
  .settings-section-container[hidden] { display: none !important; }
  .search-empty { padding: 12px; font-size: 14px; }
  .settings-footer { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; padding: 16px 24px; border-top: 1px solid var(--color-border); background: var(--color-panel); }
  .save-feedback { font-size: 14px; }
  .footer-actions { display: flex; flex-wrap: wrap; gap: 10px; margin-left: auto; }
  .settings-overlay-panel :global(button) { min-height: 44px; font-size: 16px; line-height: 1.5; }
  .settings-overlay-panel :global(button:focus-visible) { outline: 2px solid var(--color-accent); outline-offset: 3px; }
  .settings-content :global(.settings-section) { font-size: 16px; line-height: 1.55; gap: 24px; }
  .settings-content :global(.settings-section h2) { font-size: 26px; line-height: 1.3; font-weight: 600; margin-bottom: 12px; }
  .settings-content :global(.settings-section .section-h), .settings-content :global(.settings-section h3) { font-size: 20px; line-height: 1.4; font-weight: 600; text-transform: none; letter-spacing: normal; color: var(--color-fg); margin-top: 20px; margin-bottom: 12px; }
  .settings-content :global(.settings-section .lbl), .settings-content :global(.settings-section .row-label), .settings-content :global(.settings-section label) { font-size: 16px; line-height: 1.55; color: var(--color-fg); }
  .settings-content :global(.settings-section .help), .settings-content :global(.settings-section .hint), .settings-content :global(.settings-section .placeholder) { font-size: 14px; line-height: 1.6; color: var(--color-fg); opacity: 1; }
  .settings-content :global(.settings-section p), .settings-content :global(.settings-section li), .settings-content :global(.settings-section td), .settings-content :global(.settings-section th), .settings-content :global(.settings-section small) { font-size: 14px; line-height: 1.6; }
  .settings-content :global(.settings-section input:not([type=checkbox]):not([type=radio])), .settings-content :global(.settings-section select), .settings-content :global(.settings-section textarea) { min-height: 44px; max-width: 100%; font-size: 16px; line-height: 1.5; }
  .settings-content :global(.settings-section [class*="text-["]) { font-size: 14px !important; line-height: 1.6; }
  .settings-content :global(.settings-section .row) { min-height: 44px; flex-wrap: wrap; gap: 12px; }
  .settings-content :global(.settings-section input[type=checkbox]), .settings-content :global(.settings-section input[type=radio]) { width: 20px; height: 20px; flex-shrink: 0; }
  .settings-content :global(.settings-section .row-label) { flex: 1 1 220px; }
  @media (max-width: 760px) {
    .settings-body { grid-template-columns: 190px minmax(0, 1fr); }
    .settings-content { padding: 20px 16px 32px; }
    .settings-header { padding: 16px; }
    .build-id { display: none; }
    .settings-footer { padding: 12px 16px; }
  }
  @media (max-width: 540px) {
    .settings-body { grid-template-columns: 1fr; grid-template-rows: auto minmax(0, 1fr); }
    .settings-navigation { max-height: 180px; border-right: 0; border-bottom: 1px solid var(--color-border); }
    .settings-nav-item { width: auto; margin: 2px; }
    .nav-group { margin-top: 12px; }
    .settings-description { display: none; }
  }
</style>

