import { tabs } from './tabs.svelte';
import { dispatchFocusPane } from './focusPane';

/** Activate the tab/pane tied to a saved profile (if any session is open). */
export function focusProfileInTabs(profileId: string): void {
  const tab = tabs.tabs.find((t) => t.panes.some((p) => p.profileId === profileId));
  if (!tab) return;
  tabs.activate(tab.id);
  const pane = tab.panes.find((p) => p.profileId === profileId);
  if (!pane) return;
  tabs.focusPane(tab.id, pane.id);
  requestAnimationFrame(() => dispatchFocusPane(pane.id));
}
