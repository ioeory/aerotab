/**
 * Shared DOM marker so native GPU surfaces stop reclaiming OS focus while any
 * modal/overlay/menu owns the keyboard (command palette, settings, quick connect…).
 * Mirrors `aerotabResizing` on documentElement.
 */

export function syncAerotabOverlayMarker(open: boolean): void {
  if (typeof document === 'undefined') return;
  const root = document.documentElement;
  if (!root) return;
  if (open) root.dataset.aerotabOverlayOpen = '';
  else delete root.dataset.aerotabOverlayOpen;
}

export function isAerotabOverlayOpen(): boolean {
  if (typeof document === 'undefined') return false;
  return document.documentElement?.dataset.aerotabOverlayOpen !== undefined;
}
