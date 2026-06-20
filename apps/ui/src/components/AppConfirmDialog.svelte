<script lang="ts">
  import { tick } from 'svelte';
  import { confirmUi } from '../lib/confirm.svelte';
  import { i18n } from '../lib/i18n.svelte';
  import { portal } from '../lib/portal';

  let dialog: HTMLDialogElement | null = null;
  let promptInput: HTMLInputElement | null = null;
  let promptTextarea: HTMLTextAreaElement | null = null;

  const pending = $derived(confirmUi.pending);
  const isPrompt = $derived(pending?.kind === 'prompt');

  function positionDialog() {
    if (!dialog || !pending?.position) return;
    const pad = 8;
    const w = dialog.offsetWidth;
    const h = dialog.offsetHeight;
    let x = pending.position.x;
    let y = pending.position.y;
    if (x + w + pad > window.innerWidth) x = Math.max(pad, window.innerWidth - w - pad);
    if (y + h + pad > window.innerHeight) y = Math.max(pad, window.innerHeight - h - pad);
    x = Math.max(pad, x);
    y = Math.max(pad, y);
    dialog.style.left = `${x}px`;
    dialog.style.top = `${y}px`;
  }

  $effect(() => {
    if (!pending) {
      dialog?.close();
      return;
    }
    if (pending.kind === 'prompt') {
      confirmUi.promptValue = pending.defaultValue;
    }
    void tick().then(() => {
      dialog?.showModal();
      if (pending?.position) positionDialog();
      if (pending.kind === 'prompt') {
        if (pending.multiline) {
          promptTextarea?.focus();
          promptTextarea?.select();
        } else {
          promptInput?.focus();
          promptInput?.select();
        }
      }
    });
  });

  function confirmLabel(): string {
    const custom = pending?.confirmLabel?.trim();
    if (custom) return custom;
    return i18n.t('common.ok');
  }

  function cancelLabel(): string {
    const custom = pending?.cancelLabel?.trim();
    if (custom) return custom;
    return i18n.t('common.cancel');
  }

  function dialogStyle(): string {
    const pos = pending?.position;
    if (!pos) return '';
    return 'margin: 0;';
  }

  function onConfirm(ev?: MouseEvent) {
    ev?.stopPropagation();
    ev?.preventDefault();
    if (!pending) return;
    if (pending.kind === 'confirm') confirmUi.finishConfirm(true);
    else confirmUi.finishPrompt(confirmUi.promptValue.trim() || null);
  }

  function onCancel(ev?: MouseEvent) {
    ev?.stopPropagation();
    ev?.preventDefault();
    confirmUi.cancel();
  }

  function onDialogClose() {
    if (!confirmUi.wasSettled() && confirmUi.pending) {
      confirmUi.cancel();
    }
    confirmUi.resetSettled();
  }

  function onKeydown(ev: KeyboardEvent) {
    if (ev.key === 'Escape') {
      ev.preventDefault();
      onCancel();
    }
  }
  function dialogLabelledBy(): string {
    return pending?.title ? 'app-confirm-title' : 'app-confirm-message';
  }

  function dialogDescribedBy(): string | undefined {
    return pending?.title ? 'app-confirm-message' : undefined;
  }
</script>

{#if pending}
  <div use:portal class="contents">
    <dialog
      bind:this={dialog}
      class="app-confirm-dialog {pending?.position ? 'app-confirm-dialog--anchored' : ''} min-w-[360px] max-w-[min(440px,calc(100vw-32px))]"
      style={dialogStyle()}
      onclose={onDialogClose}
      onkeydown={onKeydown}
      aria-labelledby={dialogLabelledBy()}
      aria-describedby={dialogDescribedBy()}
    >
      <div class="p-5">
        {#if pending.title}
          <h2 id="app-confirm-title" class="text-[14px] font-semibold text-[var(--color-fg)] mb-2">
            {pending.title}
          </h2>
        {/if}
        <p id="app-confirm-message" class="text-[13px] leading-relaxed text-[var(--color-fg)] whitespace-pre-wrap">
          {pending.message}
        </p>
        {#if isPrompt && pending.kind === 'prompt'}
          {#if pending.multiline}
            <textarea
              bind:this={promptTextarea}
              bind:value={confirmUi.promptValue}
              class="input w-full mt-4 resize-y min-h-[72px]"
              rows={pending.rows}
              placeholder={pending.placeholder ?? ''}
              onkeydown={(e) => {
                if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
                  e.preventDefault();
                  onConfirm();
                }
              }}
            ></textarea>
          {:else}
            <input
              bind:this={promptInput}
              bind:value={confirmUi.promptValue}
              type="text"
              class="input w-full mt-4"
              placeholder={pending.placeholder ?? ''}
              onkeydown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault();
                  onConfirm();
                }
              }}
            />
          {/if}
        {/if}
        <div class="flex justify-end gap-2 mt-5">
          <button type="button" class="btn-secondary" onclick={onCancel}>
            {cancelLabel()}
          </button>
          <button
            type="button"
            class={pending.kind === 'confirm' && pending.danger ? 'btn-danger' : 'btn-primary'}
            onclick={onConfirm}
          >
            {confirmLabel()}
          </button>
        </div>
      </div>
    </dialog>
  </div>
{/if}
