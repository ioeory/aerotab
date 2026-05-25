/** In-app confirm / prompt dialogs (replaces native `confirm` / `prompt`). */

export type ConfirmOptions = {
  title?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  /** Style the primary action as destructive (e.g. delete). */
  danger?: boolean;
};

export type PromptOptions = ConfirmOptions & {
  defaultValue?: string;
  placeholder?: string;
};

type PendingConfirm = {
  kind: 'confirm';
  message: string;
  title: string | null;
  confirmLabel: string;
  cancelLabel: string;
  danger: boolean;
  resolve: (ok: boolean) => void;
};

type PendingPrompt = {
  kind: 'prompt';
  message: string;
  title: string | null;
  defaultValue: string;
  placeholder: string | null;
  confirmLabel: string;
  cancelLabel: string;
  resolve: (value: string | null) => void;
};

export type PendingDialog = PendingConfirm | PendingPrompt;

class ConfirmUi {
  pending = $state<PendingDialog | null>(null);
  /** Bound from AppConfirmDialog for prompt input. */
  promptValue = $state('');

  confirm(message: string, options: ConfirmOptions = {}): Promise<boolean> {
    return new Promise((resolve) => {
      this.pending = {
        kind: 'confirm',
        message,
        title: options.title ?? null,
        confirmLabel: options.confirmLabel ?? '',
        cancelLabel: options.cancelLabel ?? '',
        danger: !!options.danger,
        resolve,
      };
    });
  }

  prompt(message: string, options: PromptOptions = {}): Promise<string | null> {
    this.promptValue = options.defaultValue ?? '';
    return new Promise((resolve) => {
      this.pending = {
        kind: 'prompt',
        message,
        title: options.title ?? null,
        defaultValue: options.defaultValue ?? '',
        placeholder: options.placeholder ?? null,
        confirmLabel: options.confirmLabel ?? '',
        cancelLabel: options.cancelLabel ?? '',
        resolve,
      };
    });
  }

  finishConfirm(ok: boolean): void {
    const p = this.pending;
    if (!p || p.kind !== 'confirm') return;
    this.pending = null;
    p.resolve(ok);
  }

  finishPrompt(value: string | null): void {
    const p = this.pending;
    if (!p || p.kind !== 'prompt') return;
    this.pending = null;
    p.resolve(value);
  }

  cancel(): void {
    const p = this.pending;
    if (!p) return;
    this.pending = null;
    if (p.kind === 'confirm') p.resolve(false);
    else p.resolve(null);
  }
}

export const confirmUi = new ConfirmUi();

export function appConfirm(message: string, options?: ConfirmOptions): Promise<boolean> {
  return confirmUi.confirm(message, options);
}

export function appPrompt(message: string, options?: PromptOptions): Promise<string | null> {
  return confirmUi.prompt(message, options);
}
