<script lang="ts">
  // Tabby-style multi-section settings modal: left navigation pane lists
  // categories; the selected section component is mounted on the right.
  // Replaces the legacy single-page SettingsPanel.svelte.

  import { X, RotateCcw } from '@lucide/svelte';
  import type { RpcClient } from '../../lib/rpc';
  import { i18n } from '../../lib/i18n.svelte';
  import { settingsCoord } from '../../lib/settingsStore.svelte';

  import ApplicationSection from './sections/ApplicationSection.svelte';
  import AppearanceSection from './sections/AppearanceSection.svelte';
  import ProfilesSection from './sections/ProfilesSection.svelte';
  import TerminalSection from './sections/TerminalSection.svelte';
  import AISection from './sections/AISection.svelte';
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
  let { rpc, buildId, initialSection = 'appearance', onClose, onError, onSettingsChanged }: Props = $props();

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

  $effect(() => {
    active = initialSection;
  });

  async function save() {
    try {
      await settingsCoord.saveAll();
      onSettingsChanged();
    } catch (e) {
      onError(`save: ${(e as Error).message}`);
    }
  }

  async function reset() {
    if (!confirm(i18n.t('settings.resetConfirm'))) return;
    try {
      await rpc.call('settings.reset');
      settingsCoord.markClean();
      settingsCoord.bumpRev();
      onSettingsChanged();
    } catch (e) {
      onError((e as Error).message);
    }
  }
</script>

<div
  class="fixed inset-0 bg-black/60 z-50 grid place-items-center p-6"
  role="dialog"
  aria-modal="true"
  aria-label={i18n.t('settings.title')}
>
  <div
    class="panel w-full max-w-[920px] h-[80vh] flex flex-col overflow-hidden"
  >
    <header class="flex items-center gap-2 px-4 py-2.5 border-b border-[var(--color-border-soft)]">
      <div class="text-[var(--color-accent)] font-semibold text-[13px]">{i18n.t('settings.title')}</div>
      <div class="text-[10.5px] text-[var(--color-fg-muted)]">{buildId}</div>
      {#if settingsCoord.dirty}
        <span class="text-[10.5px] uppercase tracking-[0.12em] text-[var(--color-accent)]">
          {i18n.t('settings.unsaved')}
        </span>
      {/if}
      <button
        type="button"
        class="btn-ghost ml-auto p-1"
        onclick={onClose}
        aria-label={i18n.t('common.close')}
      >
        <X size={14} />
      </button>
    </header>

    <div class="flex-1 min-h-0 grid grid-cols-[200px_1fr]">
      <nav class="overflow-y-auto border-r border-[var(--color-border-soft)]
                  bg-[var(--color-panel-2)] py-3">
        {#each groups as g (g.titleKey)}
          <div class="px-3 pt-2 pb-1 shell-section-title">
            {i18n.t(g.titleKey)}
          </div>
          {#each g.entries as e (e.id)}
            <button
              type="button"
              class="shell-nav-item {active === e.id ? 'active' : ''}"
              onclick={() => (active = e.id)}
            >
              {i18n.t(e.labelKey)}
            </button>
          {/each}
        {/each}
      </nav>

      <div class="overflow-y-auto p-5 text-[12.5px]">
        {#if active === 'application'}
          <ApplicationSection {rpc} {buildId} {onError} />
        {:else if active === 'appearance'}
          <AppearanceSection {rpc} {onError} />
        {:else if active === 'profiles'}
          <ProfilesSection {rpc} {onError} />
        {:else if active === 'terminal'}
          <TerminalSection {rpc} {onError} />
        {:else if active === 'ai'}
          <AISection {rpc} {onError} />
        {:else if active === 'colorscheme'}
          <ColorSchemeSection {rpc} {onError} />
        {:else if active === 'configsync'}
          <ConfigSyncSection {rpc} {onError} onSyncApplied={onSettingsChanged} />
        {:else if active === 'hotkeys'}
          <HotkeysSection {rpc} {onError} />
        {:else if active === 'plugins'}
          <PluginsSection {rpc} {onError} />
        {:else if active === 'shell'}
          <ShellSection {rpc} {onError} />
        {:else if active === 'ssh'}
          <SshSection {rpc} {onError} />
        {:else if active === 'vault'}
          <VaultSection {rpc} {onError} />
        {:else if active === 'window'}
          <WindowSection {rpc} {onError} />
        {:else if active === 'configfile'}
          <ConfigFileSection {rpc} {onError} onChanged={onSettingsChanged} />
        {/if}
      </div>
    </div>

    <footer class="flex items-center gap-2 px-4 py-2.5 border-t border-[var(--color-border-soft)]">
      <button type="button" onclick={reset} class="btn-secondary flex items-center gap-1.5">
        <RotateCcw size={12} /> {i18n.t('common.reset')}
      </button>
      <div class="ml-auto flex items-center gap-2">
        <button type="button" onclick={onClose} class="btn-secondary">{i18n.t('common.close')}</button>
        <button
          type="button"
          onclick={save}
          class="btn-primary"
          disabled={!settingsCoord.dirty}
        >
          {i18n.t('common.save')}
        </button>
      </div>
    </footer>
  </div>
</div>

