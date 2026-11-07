export type AuxiliaryPanel = 'system' | 'notes' | 'sftp';
export type AuxiliaryPanels = Record<string, AuxiliaryPanel>;

/** Exactly one auxiliary view owns the space beside a terminal tab. */
export function toggleAuxiliaryPanel(current: AuxiliaryPanels, tabId: string, panel: AuxiliaryPanel): AuxiliaryPanels {
  const next = { ...current };
  if (next[tabId] === panel) delete next[tabId];
  else next[tabId] = panel;
  return next;
}

export function pruneAuxiliaryPanels(current: AuxiliaryPanels, liveTabIds: string[]): AuxiliaryPanels {
  const live = new Set(liveTabIds);
  return Object.fromEntries(Object.entries(current).filter(([id]) => live.has(id)));
}
