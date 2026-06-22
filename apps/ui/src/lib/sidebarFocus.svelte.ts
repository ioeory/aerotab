/** Sidebar profile keyboard focus (for global profile hotkeys). */
class SidebarFocusStore {
  focusedProfileId = $state<string | null>(null);

  setFocused(profileId: string | null) {
    this.focusedProfileId = profileId;
  }
}

export const sidebarFocus = new SidebarFocusStore();
