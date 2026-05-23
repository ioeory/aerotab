import type { SessionMeta } from './types';
import type { Tab } from './tabs.svelte';

/** True when the pane can receive SSH broadcast input. */
export function isSshPane(p: SessionMeta): boolean {
  return p.kind === 'Ssh' || !!p.profileId || !!p.sshProfile;
}

/** Session ids in `tab` that should receive broadcast keystrokes. */
export function broadcastTargetIds(tab: Tab | undefined): string[] {
  if (!tab) return [];
  return tab.panes.filter(isSshPane).map((p) => p.id);
}
