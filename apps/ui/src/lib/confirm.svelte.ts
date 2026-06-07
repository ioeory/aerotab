/** In-app confirm / prompt dialogs (replaces native `confirm` / `prompt`). */

export type ConfirmOptions = {
  title?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  /** Style the primary action as destructive (e.g. delete). */
  danger?: boolean;
  /** Optional viewport position for focus-anchored dialogs. */
  position?: { x: number; y: number };
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
  position?: { x: number; y: number };
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
  position?: { x: number; y: number };
  resolve: (value: string | null) => void;
};

export type PendingDialog = PendingConfirm | PendingPrompt;

class ConfirmUi {
  pending = $state<PendingDialog | null>(null);
  /** Set when confirm/cancel handlers run; suppresses spurious `dialog` close cancel. */
  private settled = false;
  /** Bound from AppConfirmDialog for prompt input. */
  promptValue = $state('');

  confirm(message: string, options: ConfirmOptions = {}): Promise<boolean> {
    return new Promise((resolve) => {
      this.settled = false;
      this.pending = {
        kind: 'confirm',
        message,
        title: options.title ?? null,
        confirmLabel: options.confirmLabel ?? '',
        cancelLabel: options.cancelLabel ?? '',
        danger: !!options.danger,
        position: options.position,
        resolve,
      };
    });
  }

  prompt(message: string, options: PromptOptions = {}): Promise<string | null> {
    this.promptValue = options.defaultValue ?? '';
    return new Promise((resolve) => {
      this.settled = false;
      this.pending = {
        kind: 'prompt',
        message,
        title: options.title ?? null,
        defaultValue: options.defaultValue ?? '',
        placeholder: options.placeholder ?? null,
        confirmLabel: options.confirmLabel ?? '',
        cancelLabel: options.cancelLabel ?? '',
        position: options.position,
        resolve,
      };
    });
  }

  finishConfirm(ok: boolean): void {
    const p = this.pending;
    if (!p || p.kind !== 'confirm') return;
    this.settled = true;
    this.pending = null;
    p.resolve(ok);
  }

  finishPrompt(value: string | null): void {
    const p = this.pending;
    if (!p || p.kind !== 'prompt') return;
    this.settled = true;
    this.pending = null;
    p.resolve(value);
  }

  cancel(): void {
    const p = this.pending;
    if (!p) return;
    this.settled = true;
    this.pending = null;
    if (p.kind === 'confirm') p.resolve(false);
    else p.resolve(null);
  }

  /** True after confirm/cancel; ignore native `close` until next dialog opens. */
  wasSettled(): boolean {
    return this.settled;
  }

  resetSettled(): void {
    this.settled = false;
  }
}

export const confirmUi = new ConfirmUi();

export function appConfirm(message: string, options?: ConfirmOptions): Promise<boolean> {
  return confirmUi.confirm(message, options);
}

export function appPrompt(message: string, options?: PromptOptions): Promise<string | null> {
  return confirmUi.prompt(message, options);
}
