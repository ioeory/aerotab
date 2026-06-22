/**
 * Detect app-level modals/dialogs so xterm does not steal focus from their inputs.
 * Matches `body[data-modal-overlay]` (App.svelte) plus native `<dialog open>` and
 * `data-aerotab-modal` markers on standalone overlays.
 */

const MODAL_FOCUS_DELAYS_MS = [0, 50, 120] as const;

export function isModalOverlayActive(): boolean {
  if (typeof document === 'undefined') return false;
  if (document.body.dataset.modalOverlay === 'true') return true;
  if (document.querySelector('dialog[open]')) return true;
  if (document.querySelector('[data-aerotab-modal]')) return true;
  return false;
}

/** Block global hotkeys when modals, context menus, or tab list overlays are open. */
export function isOverlayBlockingInput(): boolean {
  if (isModalOverlayActive()) return true;
  if (typeof document === 'undefined') return false;
  // `data-aerotab-context-menu` marks zones that *allow* app context menus (TabBar, Sidebar,
  // terminal host) — not open menus. Only block on explicit open-menu markers.
  if (document.querySelector('[data-aerotab-tab-list]')) return true;
  if (document.querySelector('[data-aerotab-menu-open]')) return true;
  return false;
}

export function shouldFocusTerminal(): boolean {
  return !isModalOverlayActive();
}

export function focusTerminalIfAllowed(term: { focus: () => void } | null | undefined): void {
  if (!term || !shouldFocusTerminal()) return;
  term.focus();
}

/** Re-assert focus on a modal field after xterm delayed fit/focus runs. */
export function scheduleModalFieldFocus(focus: () => void): void {
  if (typeof window === 'undefined') {
    focus();
    return;
  }
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      focus();
      for (const ms of MODAL_FOCUS_DELAYS_MS) {
        window.setTimeout(() => {
          if (isModalOverlayActive()) focus();
        }, ms);
      }
    });
  });
}
