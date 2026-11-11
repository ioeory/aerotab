<script lang="ts">
  import { X } from '@lucide/svelte';
  import type { RpcClient } from '../lib/rpc';
  import { i18n } from '../lib/i18n.svelte';
  import { unlockVaultWithOptions } from '../lib/vaultBootstrap';

  interface Props {
    rpc: RpcClient;
    onError: (msg: string) => void;
    onUnlocked?: () => void;
    onSkip?: () => void;
  }
  let { rpc, onError, onUnlocked, onSkip }: Props = $props();

  let dialog: HTMLDialogElement | null = null;
  let password = $state('');
  let busy = $state(false);

  export async function open() {
    password = '';
    dialog?.showModal();
    requestAnimationFrame(() => {
      document.getElementById('vum-password')?.focus();
    });
  }

  function close() {
    password = '';
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
      const ok = await unlockVaultWithOptions(rpc, password);
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

<dialog bind:this={dialog} aria-labelledby="vault-unlock-title" class="w-[min(520px,calc(100vw-32px))] min-w-0 max-h-[calc(100vh-32px)] overflow-y-auto" onclose={() => { password = ''; onSkip?.(); }}>
  <form onsubmit={submit} class="p-5">
    <div class="flex items-center justify-between mb-2">
      <h2 id="vault-unlock-title" class="text-[length:var(--text-section,20px)] font-semibold text-[var(--color-accent)]">
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
    <p class="text-[length:var(--text-ui,16px)] text-[var(--color-fg-muted)] mb-3 leading-relaxed">
      {i18n.t('vault.unlockPromptBody')}
    </p>
    <label for="vum-password" class="block text-[length:var(--text-compact,14px)] text-[var(--color-fg-muted)] mb-1">
      {i18n.t('vault.unlockPromptPassword')}
    </label>
    <input
      id="vum-password"
      type="password"
      bind:value={password}
      class="input w-full mb-3"
      autocomplete="current-password"
    />
    <div class="flex flex-wrap justify-end gap-2">
      <button type="button" class="btn-secondary" disabled={busy} onclick={skip}>
        {i18n.t('vault.unlockPromptSkip')}
      </button>
      <button type="submit" class="btn-primary" disabled={busy}>
        {busy ? i18n.t('vault.unlockPromptUnlocking') : i18n.t('vault.unlockPromptSubmit')}
      </button>
    </div>
  </form>
</dialog>

