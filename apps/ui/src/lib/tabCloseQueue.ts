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

/** Tabs removed per frame when flushing a large batch (close all / close others). */
const BATCH_TABS_PER_FRAME = 1;

function dropPending(tabId: string) {
  for (let i = pending.length - 1; i >= 0; i--) {
    if (pending[i]!.tabId === tabId) pending.splice(i, 1);
  }
}

/** Immediate UI teardown for a single tab (keyboard / × click). */
export function closeTabImmediate(tab: Tab, sink: TabCloseSink): void {
  dropPending(tab.id);
  const paneIds = tab.panes.map((p) => p.id);
  notifySessionsClosing(paneIds);
  sink.removeTabIds([tab.id]);
  sink.clearSftpTabIds([tab.id]);
  sink.clearRestoreSessionIds(paneIds);
  sink.closeSessions(paneIds);
}

function scheduleBatchFlush(sink: TabCloseSink) {
  if (rafId != null) return;
  rafId = requestAnimationFrame(() => {
    rafId = null;
    const chunk = pending.splice(0, BATCH_TABS_PER_FRAME);
    if (chunk.length === 0) return;
    const tabIds = chunk.map((c) => c.tabId);
    const paneIds = chunk.flatMap((c) => c.paneIds);
    notifySessionsClosing(paneIds);
    sink.removeTabIds(tabIds);
    sink.clearSftpTabIds(tabIds);
    sink.clearRestoreSessionIds(paneIds);
    sink.closeSessions(paneIds);
    if (pending.length > 0) scheduleBatchFlush(sink);
  });
}

export function queueTabsClose(tabList: Tab[], sink: TabCloseSink): void {
  if (tabList.length === 0) return;
  if (tabList.length === 1) {
    closeTabImmediate(tabList[0]!, sink);
    return;
  }
  const alreadyQueued = new Set(pending.map((c) => c.tabId));
  for (const tab of tabList) {
    if (alreadyQueued.has(tab.id)) continue;
    alreadyQueued.add(tab.id);
    pending.push({ tabId: tab.id, paneIds: tab.panes.map((p) => p.id) });
  }
  scheduleBatchFlush(sink);
}

export function queueTabClose(tab: Tab, sink: TabCloseSink): void {
  closeTabImmediate(tab, sink);
}
