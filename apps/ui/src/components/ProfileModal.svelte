<script lang="ts">
  import { X } from '@lucide/svelte';
  import { tick } from 'svelte';
  import type { RpcClient } from '../lib/rpc';
  import type { StoredProfile } from '../lib/types';
  import { i18n } from '../lib/i18n.svelte';
  import ProfileEditorForm from './hosts/ProfileEditorForm.svelte';

  export interface ProfileModalOpenOptions {
    group?: string;
    duplicateFrom?: StoredProfile;
    existingNames?: string[];
  }
  let { rpc, onSaved, onClosed, onError, onOpenVault }: {
    rpc: RpcClient;
    onSaved: () => void;
    onClosed?: () => void;
    onError: (message: string) => void;
    onOpenVault?: () => void;
  } = $props();
  let dialog: HTMLDialogElement | null = null;
  let editor: ProfileEditorForm | undefined;
  let returnFocus: HTMLElement | null = null;
  let saving = $state(false);
  let titleKey = $state('profileModal.newTitle');
  let revision = 0;

  /** Compatibility facade: callers need not know whether the editor is a drawer or embedded form. */
  export function open(existing?: StoredProfile, options?: ProfileModalOpenOptions) {
    if (saving) return;
    const current = ++revision;
    if (!dialog?.open) returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    titleKey = options?.duplicateFrom ? 'profileModal.cloneTitle' : existing ? 'profileModal.editTitle' : 'profileModal.newTitle';
    editor?.load(existing, options);
    if (!dialog?.open) dialog?.showModal();
    void tick().then(() => {
      if (current === revision && dialog?.open) dialog.querySelector<HTMLInputElement>('#pm-name')?.focus();
    });
  }
  function close() { if (!saving) dialog?.close(); }
  function closed() {
    revision += 1;
    onClosed?.();
    if (returnFocus?.isConnected) returnFocus.focus({ preventScroll: true });
    returnFocus = null;
  }
  function saved() { onSaved(); dialog?.close(); }
  function openVault() { close(); onOpenVault?.(); }
</script>

<dialog bind:this={dialog} class="profile-drawer" aria-labelledby="profile-modal-title" aria-modal="true" onclose={closed} oncancel={(event) => { if (saving) event.preventDefault(); }}>
  <ProfileEditorForm bind:this={editor} {rpc} {onError} onSaved={saved} onCancel={close} onOpenVault={onOpenVault ? openVault : undefined} bind:saving>
    {#snippet header()}
      <header>
        <h2 id="profile-modal-title">{i18n.t(titleKey)}</h2>
        <button type="button" aria-label={i18n.t('common.close')} disabled={saving} onclick={close}><X size={20} /></button>
      </header>
    {/snippet}
  </ProfileEditorForm>
</dialog>

<style>
  dialog.profile-drawer { position: fixed; inset: 0 0 0 auto; margin: 0; width: min(620px, 100vw); min-width: 0; max-width: 100vw; height: 100dvh; max-height: 100dvh; padding: 0; border: 0; border-left: 1px solid var(--color-border); border-radius: 0; background: var(--color-panel); color: var(--color-fg); overflow-y: auto; font-size: 16px; line-height: 1.5; }
  .profile-drawer::backdrop { background: rgb(0 0 0 / 0.3); }
  header { position: sticky; top: 0; z-index: 2; margin: 0 -24px 20px; padding: 20px 24px; background: var(--color-panel); border-bottom: 1px solid var(--color-border-soft); display: flex; justify-content: space-between; align-items: center; gap: 12px; }
  h2 { font-size: 20px; line-height: 1.5; font-weight: 600; }
  header button { min-width: 40px; min-height: 40px; display: grid; place-items: center; color: var(--color-fg); border-radius: 6px; background: transparent; cursor: pointer; }
  button:focus-visible { outline: 2px solid var(--color-accent); }
</style>
