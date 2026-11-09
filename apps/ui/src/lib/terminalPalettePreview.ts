/** Window-local terminal-only preview. null means use persisted settings; '' means follow UI. */
let preview: string | null = null;
export function setTerminalPalettePreview(value: string | null): void {
  preview = value;
  if (typeof document !== 'undefined') document.dispatchEvent(new CustomEvent('aerotab:terminal-palette-preview'));
}
export function resolveTerminalPaletteName(persisted: string): string { return preview ?? persisted; }

export function createTerminalPaletteDraft(initial: string, effects: { render: (value: string | null) => void; persist: (value: string) => Promise<unknown> }) {
  let applied = initial;
  let draft = initial;
  let pending: Promise<string> | null = null;
  return {
    preview(value: string) { draft = value; effects.render(value); return draft; },
    cancel() { draft = applied; effects.render(null); return draft; },
    apply(): Promise<string> {
      if (pending) return pending;
      const saved = draft;
      pending = Promise.resolve().then(() => effects.persist(saved)).then(() => {
        applied = saved; effects.render(null); return applied;
      }, error => { draft = applied; effects.render(applied); throw error; }).finally(() => { pending = null; });
      return pending;
    },
  };
}
