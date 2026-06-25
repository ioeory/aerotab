<script lang="ts">
  import { ChevronDown, ChevronRight } from '@lucide/svelte';
  import type { RpcClient } from '../lib/rpc';
  import { pickPrivateKeyPath } from '../lib/localFiles';
  import { i18n } from '../lib/i18n.svelte';
  import type { ImportBatchAuthConfig, ImportBatchAuthMode } from '../lib/importAuth';

  interface Props {
    rpc: RpcClient;
    selectedCount: number;
    disabled?: boolean;
    onApply: (config: ImportBatchAuthConfig) => void;
    onMatchExisting: () => void;
    onError: (msg: string) => void;
    onSummary?: (msg: string) => void;
  }

  let {
    rpc,
    selectedCount,
    disabled = false,
    onApply,
    onMatchExisting,
    onError,
    onSummary,
  }: Props = $props();

  let expanded = $state(true);
  let userOverride = $state('');
  let mode = $state<ImportBatchAuthMode>('password');
  let password = $state('');
  let keyPath = $state('');
  let keyPassphrase = $state('');
  let vaultEntryId = $state('');
  let vaultPassphraseEntryId = $state('');
  let vaultEntries = $state<Array<{ id: string; label: string; kind: string }>>([]);
  let vaultUnlocked = $state(false);

  async function loadVaultEntries() {
    try {
      const st = await rpc.call<{ initialized: boolean; unlocked: boolean }>('vault.status', {});
      vaultUnlocked = st.initialized && st.unlocked;
      if (!vaultUnlocked) {
        vaultEntries = [];
        return;
      }
      vaultEntries = await rpc.call<Array<{ id: string; label: string; kind: string }>>('vault.list', {});
    } catch (e) {
      vaultUnlocked = false;
      vaultEntries = [];
      onError(`vault: ${(e as Error).message}`);
    }
  }

  $effect(() => {
    if (expanded) void loadVaultEntries();
  });

  async function browseKey() {
    try {
      const path = await pickPrivateKeyPath();
      if (path) keyPath = path;
    } catch (e) {
      onError(i18n.t('profileModal.browsePrivateKeyFailed', { message: (e as Error).message }));
    }
  }

  function applyBatch() {
    if (selectedCount === 0) {
      onSummary?.(i18n.t('import.batchAuth.noSelection'));
      return;
    }
    if (mode === 'password' && !password.trim()) {
      onError(i18n.t('import.batchAuth.passwordRequired'));
      return;
    }
    if (mode === 'key' && !keyPath.trim()) {
      onError(i18n.t('import.batchAuth.keyPathRequired'));
      return;
    }
    if (mode === 'vault' && !vaultEntryId.trim()) {
      onError(i18n.t('import.batchAuth.vaultEntryRequired'));
      return;
    }
    onApply({
      userOverride,
      mode,
      password,
      keyPath,
      keyPassphrase,
      vaultEntryId,
      vaultPassphraseEntryId,
    });
  }

  function getConfig(): ImportBatchAuthConfig {
    return {
      userOverride,
      mode,
      password,
      keyPath,
      keyPassphrase,
      vaultEntryId,
      vaultPassphraseEntryId,
    };
  }

  export { getConfig };

  const canApply = $derived(
    selectedCount > 0
      && !disabled
      && (mode === 'keep'
        ? userOverride.trim().length > 0
        : mode !== 'password' || password.trim().length > 0),
  );
</script>

<div class="import-batch-auth mb-3 rounded border border-[var(--color-border-soft)] bg-[var(--color-panel-2)]">
  <button
    type="button"
    class="w-full flex items-center gap-2 px-3 py-2 text-left text-[12px] font-medium text-[var(--color-fg)]"
    onclick={() => { expanded = !expanded; }}
    aria-expanded={expanded}
  >
    {#if expanded}
      <ChevronDown size={14} class="text-[var(--color-fg-muted)]" />
    {:else}
      <ChevronRight size={14} class="text-[var(--color-fg-muted)]" />
    {/if}
    {i18n.t('import.batchAuth.title')}
    <span class="text-[11px] font-normal text-[var(--color-fg-muted)]">
      ({i18n.t('import.batchAuth.selected', { count: selectedCount })})
    </span>
  </button>

  {#if expanded}
    <div class="px-3 pb-3 pt-0 space-y-2 border-t border-[var(--color-border-soft)]">
      <p class="text-[10.5px] text-[var(--color-fg-muted)] pt-2">{i18n.t('import.batchAuth.hint')}</p>

      <div class="grid grid-cols-1 sm:grid-cols-2 gap-2">
        <label class="block">
          <span class="block text-[11px] text-[var(--color-fg-muted)] mb-1">{i18n.t('import.batchAuth.userOverride')}</span>
          <input type="text" class="input w-full text-[12px]" bind:value={userOverride} placeholder={i18n.t('import.batchAuth.userPlaceholder')} />
        </label>
        <label class="block">
          <span class="block text-[11px] text-[var(--color-fg-muted)] mb-1">{i18n.t('import.batchAuth.authMode')}</span>
          <select class="input w-full text-[12px]" bind:value={mode}>
            <option value="keep">{i18n.t('import.batchAuth.modeKeep')}</option>
            <option value="password">{i18n.t('import.batchAuth.modePassword')}</option>
            <option value="key">{i18n.t('import.batchAuth.modeKey')}</option>
            <option value="agent">{i18n.t('import.batchAuth.modeAgent')}</option>
            <option value="vault">{i18n.t('import.batchAuth.modeVault')}</option>
          </select>
        </label>
      </div>

      {#if mode === 'password'}
        <label class="block">
          <span class="block text-[11px] text-[var(--color-fg-muted)] mb-1">{i18n.t('import.batchAuth.password')}</span>
          <input type="password" class="input w-full text-[12px]" bind:value={password} autocomplete="new-password" />
        </label>
      {:else if mode === 'key'}
        <div class="grid grid-cols-1 sm:grid-cols-[1fr_auto] gap-2 items-end">
          <label class="block min-w-0">
            <span class="block text-[11px] text-[var(--color-fg-muted)] mb-1">{i18n.t('import.batchAuth.keyPath')}</span>
            <input type="text" class="input w-full text-[12px] font-mono" bind:value={keyPath} placeholder="~/.ssh/id_ed25519" />
          </label>
          <button type="button" class="btn-secondary text-[11px] px-2 py-1.5 shrink-0" onclick={() => { void browseKey(); }}>
            {i18n.t('profileModal.browsePrivateKey')}
          </button>
        </div>
        <label class="block">
          <span class="block text-[11px] text-[var(--color-fg-muted)] mb-1">{i18n.t('profileModal.keyPassphrase')}</span>
          <input type="password" class="input w-full text-[12px]" bind:value={keyPassphrase} autocomplete="new-password" />
        </label>
      {:else if mode === 'vault'}
        {#if !vaultUnlocked}
          <p class="text-[10.5px] text-[var(--color-fg-muted)]">{i18n.t('profileModal.vaultLockedHint')}</p>
        {:else}
          <label class="block">
            <span class="block text-[11px] text-[var(--color-fg-muted)] mb-1">{i18n.t('profileModal.vaultEntry')}</span>
            <select class="input w-full text-[12px]" bind:value={vaultEntryId}>
              <option value="">{i18n.t('import.batchAuth.vaultEntryPlaceholder')}</option>
              {#each vaultEntries as ve (ve.id)}
                <option value={ve.id}>{ve.label} ({ve.kind})</option>
              {/each}
            </select>
          </label>
          <label class="block">
            <span class="block text-[11px] text-[var(--color-fg-muted)] mb-1">{i18n.t('profileModal.vaultPassphraseEntry')}</span>
            <select class="input w-full text-[12px]" bind:value={vaultPassphraseEntryId}>
              <option value="">{i18n.t('import.batchAuth.vaultPassOptional')}</option>
              {#each vaultEntries.filter((e) => e.kind === 'password') as ve (ve.id)}
                <option value={ve.id}>{ve.label}</option>
              {/each}
            </select>
          </label>
        {/if}
      {/if}

      <div class="flex flex-wrap gap-2 pt-1">
        <button type="button" class="btn-secondary text-[11px] px-2.5 py-1" disabled={!canApply} onclick={applyBatch}>
          {i18n.t('import.batchAuth.apply', { count: selectedCount })}
        </button>
        <button type="button" class="btn-ghost text-[11px] px-2.5 py-1" disabled={disabled || selectedCount === 0} onclick={onMatchExisting}>
          {i18n.t('import.batchAuth.matchExisting')}
        </button>
      </div>
    </div>
  {/if}
</div>
