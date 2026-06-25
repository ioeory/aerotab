/** Sidebar profile keyboard focus (for global profile hotkeys). */
class SidebarFocusStore {
  focusedProfileId = $state<string | null>(null);

  setFocused(profileId: string | null) {
    this.focusedProfileId = profileId;
  }

  clearFocused() {
    this.focusedProfileId = null;
  }

  /** True when keyboard focus is inside the sidebar profile list. */
  isListFocused(): boolean {
    if (typeof document === 'undefined') return false;
    const el = document.activeElement;
    if (!el || !(el instanceof Element)) return false;
    return !!el.closest('[data-aerotab-sidebar-profiles]');
  }
}

export const sidebarFocus = new SidebarFocusStore();

export const SIDEBAR_PROFILES_SELECTOR = '[data-aerotab-sidebar-profiles]';

/** Drop sidebar profile focus when activating a terminal pane (Win10 WebView2 may keep focus on the row). */
export function releaseSidebarProfileFocus(): void {
  sidebarFocus.clearFocused();
  if (typeof document === 'undefined') return;
  const active = document.activeElement;
  if (active instanceof HTMLElement && active.closest(SIDEBAR_PROFILES_SELECTOR)) {
    active.blur();
  }
}
