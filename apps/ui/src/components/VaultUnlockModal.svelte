<script lang="ts">
  import { X } from '@lucide/svelte';
  import type { RpcClient } from '../lib/rpc';
  import { i18n } from '../lib/i18n.svelte';
  import {
    hasVaultKeyringSecret,
    loadVaultKeyringAccount,
    unlockVaultWithOptions,
  } from '../lib/vaultBootstrap';

  interface Props {
    rpc: RpcClient;
    onError: (msg: string) => void;
    onUnlocked?: () => void;
    onSkip?: () => void;
  }
  let { rpc, onError, onUnlocked, onSkip }: Props = $props();

  let dialog: HTMLDialogElement | null = null;
  let password = $state('');
  let saveToKeyring = $state(true);
  let busy = $state(false);
  let keyringAlreadySaved = $state(false);

  export async function open() {
    password = '';
    try {
      const account = await loadVaultKeyringAccount(rpc);
      keyringAlreadySaved = await hasVaultKeyringSecret(rpc, account);
      saveToKeyring = !keyringAlreadySaved;
    } catch {
      keyringAlreadySaved = false;
      saveToKeyring = true;
    }
    dialog?.showModal();
    requestAnimationFrame(() => {
      document.getElementById('vum-password')?.focus();
    });
  }

  function close() {
    dialog?.close();
  }

  function skip() {
    close();
    onSkip?.();
  }

  async function submit(ev: Event) {
    ev.preventDefault();
    if (!password.trim()) {
      onError(i18n.t('vault.unlockPromptPasswordRequired'));
      return;
    }
    busy = true;
    try {
      const account = await loadVaultKeyringAccount(rpc);
      const ok = await unlockVaultWithOptions(rpc, password, {
        saveToKeyring: saveToKeyring && !keyringAlreadySaved,
        account,
      });
      if (!ok) {
        onError(i18n.t('vault.unlockPromptFailed'));
        return;
      }
      password = '';
      close();
      onUnlocked?.();
    } catch (e) {
      onError(i18n.t('vault.unlockPromptFailedDetail', { message: (e as Error).message }));
    } finally {
      busy = false;
    }
  }
</script>

<dialog bind:this={dialog} class="min-w-[400px]" onclose={() => onSkip?.()}>
  <form onsubmit={submit} class="p-5">
    <div class="flex items-center justify-between mb-2">
      <h2 class="text-[14px] font-semibold text-[var(--color-accent)]">
        {i18n.t('vault.unlockPromptTitle')}
      </h2>
      <button
        type="button"
        onclick={skip}
        class="p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]"
        aria-label={i18n.t('common.close')}
      >
        <X size={14} />
      </button>
    </div>
    <p class="text-[12px] text-[var(--color-fg-muted)] mb-3 leading-relaxed">
      {i18n.t('vault.unlockPromptBody')}
    </p>
    <label for="vum-password" class="block text-[11px] text-[var(--color-fg-muted)] mb-1">
      {i18n.t('vault.unlockPromptPassword')}
    </label>
    <input
      id="vum-password"
      type="password"
      bind:value={password}
      class="input w-full mb-3"
      autocomplete="current-password"
    />
    {#if !keyringAlreadySaved}
      <label class="flex items-center gap-2 text-[12px] text-[var(--color-fg)] mb-4">
        <input type="checkbox" bind:checked={saveToKeyring} />
        {i18n.t('vault.saveToKeyringOnUnlock')}
      </label>
    {:else}
      <p class="text-[11px] text-[var(--color-fg-muted)] mb-4">
        {i18n.t('vault.keyringAlreadySavedHint')}
      </p>
    {/if}
    <div class="flex justify-end gap-2">
      <button type="button" class="btn-secondary" disabled={busy} onclick={skip}>
        {i18n.t('vault.unlockPromptSkip')}
      </button>
      <button type="submit" class="btn-primary" disabled={busy}>
        {busy ? i18n.t('vault.unlockPromptUnlocking') : i18n.t('vault.unlockPromptSubmit')}
      </button>
    </div>
  </form>
</dialog>

<style>
  .input {
    background: var(--color-bg);
    color: var(--color-fg);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    padding: 6px 10px;
    font: inherit;
  }
  .input:focus {
    outline: none;
    border-color: var(--color-accent);
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
  .btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }
  .btn-secondary {
    background: var(--color-panel-2);
    color: var(--color-fg);
    border: 1px solid var(--color-border);
    padding: 6px 14px;
    border-radius: var(--radius-sm);
    cursor: pointer;
  }
</style>
