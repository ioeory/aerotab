<script lang="ts">
  // Tabby-style multi-section settings modal: left navigation pane lists
  // categories; the selected section component is mounted on the right.
  // Replaces the legacy single-page SettingsPanel.svelte.

  import { X, RotateCcw } from '@lucide/svelte';
  import type { RpcClient } from '../../lib/rpc';
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

  interface Props {
    rpc: RpcClient;
    onClose: () => void;
    onError: (msg: string) => void;
    onSettingsChanged: () => void;
  }
  let { rpc, onClose, onError, onSettingsChanged }: Props = $props();

  const buildId = '0.1.17-ui-20260523';

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

  interface NavEntry {
    id: SectionId;
    label: string;
  }

  const groups: { title: string; entries: NavEntry[] }[] = [
    {
      title: 'General',
      entries: [
        { id: 'application', label: 'Application' },
        { id: 'appearance', label: 'Appearance' },
        { id: 'profiles', label: 'Profiles & connections' },
        { id: 'terminal', label: 'Terminal' },
        { id: 'ai', label: 'AI 助手' },
        { id: 'colorscheme', label: 'Color scheme' },
        { id: 'configsync', label: 'Config sync' },
        { id: 'hotkeys', label: 'Hotkeys' },
        { id: 'plugins', label: 'Plugins' },
      ],
    },
    {
      title: 'Advanced',
      entries: [
        { id: 'shell', label: 'Shell' },
        { id: 'ssh', label: 'SSH' },
        { id: 'vault', label: 'Vault' },
        { id: 'window', label: 'Window' },
        { id: 'configfile', label: 'Config file' },
      ],
    },
  ];

  let active = $state<SectionId>('appearance');

  async function save() {
    try {
      await settingsCoord.saveAll();
      onSettingsChanged();
    } catch (e) {
      onError(`save: ${(e as Error).message}`);
    }
  }

  async function reset() {
    if (!confirm('Reset all settings to defaults?')) return;
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
  aria-label="Settings"
>
  <div
    class="bg-[var(--color-panel)] border border-[var(--color-border)] rounded-lg shadow-2xl
           w-full max-w-[920px] h-[80vh] flex flex-col overflow-hidden"
  >
    <header class="flex items-center gap-2 px-4 py-2.5 border-b border-[var(--color-border-soft)]">
      <div class="text-[var(--color-accent)] font-semibold text-[13px]">Settings</div>
      <div class="text-[10.5px] text-[var(--color-fg-muted)]">{buildId}</div>
      {#if settingsCoord.dirty}
        <span class="text-[10.5px] uppercase tracking-[0.12em] text-[var(--color-accent)]">
          unsaved
        </span>
      {/if}
      <button
        type="button"
        class="ml-auto p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]"
        onclick={onClose}
        aria-label="Close"
      >
        <X size={14} />
      </button>
    </header>

    <div class="flex-1 min-h-0 grid grid-cols-[200px_1fr]">
      <nav class="overflow-y-auto border-r border-[var(--color-border-soft)]
                  bg-[var(--color-panel-2)] py-3">
        {#each groups as g (g.title)}
          <div class="px-3 pt-2 pb-1 text-[10px] uppercase tracking-[0.14em] text-[var(--color-fg-muted)]">
            {g.title}
          </div>
          {#each g.entries as e (e.id)}
            <button
              type="button"
              class="nav-btn {active === e.id ? 'active' : ''}"
              onclick={() => (active = e.id)}
            >
              {e.label}
            </button>
          {/each}
        {/each}
      </nav>

      <div class="overflow-y-auto p-5 text-[12.5px]">
        {#if active === 'application'}
          <ApplicationSection {rpc} {onError} />
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
          <ConfigSyncSection {rpc} {onError} />
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
        <RotateCcw size={12} /> Reset
      </button>
      <div class="ml-auto flex items-center gap-2">
        <button type="button" onclick={onClose} class="btn-secondary">Close</button>
        <button
          type="button"
          onclick={save}
          class="btn-primary"
          disabled={!settingsCoord.dirty}
        >
          Save
        </button>
      </div>
    </footer>
  </div>
</div>

<style>
  .nav-btn {
    display: block;
    width: 100%;
    text-align: left;
    padding: 6px 14px;
    font-size: 12px;
    color: var(--color-fg);
    background: transparent;
    border: none;
    cursor: pointer;
  }
  .nav-btn:hover {
    background: color-mix(in srgb, var(--color-accent) 10%, transparent);
  }
  .nav-btn.active {
    background: color-mix(in srgb, var(--color-accent) 22%, transparent);
    color: var(--color-accent);
    font-weight: 600;
  }
  .btn-primary {
    background: var(--color-accent);
    color: var(--color-bg);
    border: none;
    padding: 6px 14px;
    border-radius: var(--radius-sm);
    font-weight: 600;
    cursor: pointer;
  }
  .btn-primary[disabled] {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .btn-secondary {
    background: var(--color-panel-2);
    color: var(--color-fg);
    border: 1px solid var(--color-border);
    padding: 6px 14px;
    border-radius: var(--radius-sm);
    cursor: pointer;
  }
</style>
