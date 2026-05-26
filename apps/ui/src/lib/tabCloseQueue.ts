import { notifySessionsClosing } from './sessionLifecycle';
import type { Tab } from './tabs.svelte';

export interface TabCloseSink {
  removeTabIds: (tabIds: string[]) => void;
  clearSftpTabIds: (tabIds: string[]) => void;
  clearRestoreSessionIds: (sessionIds: string[]) => void;
  closeSessions: (sessionIds: string[]) => void;
}

type PendingClose = { tabId: string; paneIds: string[] };

const pending: PendingClose[] = [];
let rafId: number | null = null;

/** Tabs removed per animation frame (spreads xterm teardown). */
const TABS_PER_FRAME = 2;

function scheduleFlush(sink: TabCloseSink) {
  if (rafId != null) return;
  rafId = requestAnimationFrame(() => {
    rafId = null;
    const chunk = pending.splice(0, TABS_PER_FRAME);
    if (chunk.length === 0) return;
    const tabIds = chunk.map((c) => c.tabId);
    const paneIds = chunk.flatMap((c) => c.paneIds);
    notifySessionsClosing(paneIds);
    sink.removeTabIds(tabIds);
    sink.clearSftpTabIds(tabIds);
    sink.clearRestoreSessionIds(paneIds);
    sink.closeSessions(paneIds);
    if (pending.length > 0) scheduleFlush(sink);
  });
}

export function queueTabsClose(tabList: Tab[], sink: TabCloseSink): void {
  if (tabList.length === 0) return;
  const alreadyQueued = new Set(pending.map((c) => c.tabId));
  for (const tab of tabList) {
    if (alreadyQueued.has(tab.id)) continue;
    alreadyQueued.add(tab.id);
    pending.push({ tabId: tab.id, paneIds: tab.panes.map((p) => p.id) });
  }
  scheduleFlush(sink);
}

export function queueTabClose(tab: Tab, sink: TabCloseSink): void {
  queueTabsClose([tab], sink);
}
