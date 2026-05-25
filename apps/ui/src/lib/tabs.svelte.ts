// Per-tab Svelte 5 stores. A tab owns a pane tree so splits can be applied
// relative to the currently focused pane instead of changing the whole tab's
// global direction.

import { uuidv4 } from './rpc';
import type { SessionMeta } from './types';

export type SplitDir = 'row' | 'col';
export type SplitSide = 'before' | 'after';
export type PaneDropSide = 'left' | 'right' | 'up' | 'down';
export type FocusDirection = 'left' | 'right' | 'up' | 'down';

export interface PaneLeaf {
  type: 'leaf';
  id: string;
  pane: SessionMeta;
}

export interface PaneSplit {
  type: 'split';
  id: string;
  direction: SplitDir;
  children: PaneNode[];
  ratios: number[];
}

export type PaneNode = PaneLeaf | PaneSplit;

interface PaneRect {
  id: string;
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Tab {
  id: string;
  layout: PaneNode;
  /** Flattened compatibility view used by tab chrome, restore, and actions. */
  panes: SessionMeta[];
  /** Session id of the currently focused pane within this tab. */
  activePaneId: string;
  /** Full-screen pane view state; sessions remain alive. */
  maximizedPaneId?: string | null;
  /** Human-friendly title (defaults to the first pane's title). */
  title: string;
  /** Per-pane activity flags: 'output' since last focus or 'bell' if BEL seen. */
  activity: Record<string, 'output' | 'bell' | undefined>;
}

function leaf(session: SessionMeta): PaneLeaf {
  return { type: 'leaf', id: session.id, pane: session };
}

function split(direction: SplitDir, children: PaneNode[], ratios?: number[]): PaneSplit {
  const even = 1 / Math.max(1, children.length);
  return {
    type: 'split',
    id: uuidv4(),
    direction,
    children,
    ratios: ratios?.length === children.length ? ratios : children.map(() => even),
  };
}

function flatten(node: PaneNode): SessionMeta[] {
  if (node.type === 'leaf') return [node.pane];
  return node.children.flatMap(flatten);
}

function contains(node: PaneNode, sessionId: string): boolean {
  if (node.type === 'leaf') return node.pane.id === sessionId;
  return node.children.some((child) => contains(child, sessionId));
}

function insertRelative(
  node: PaneNode,
  targetId: string,
  session: SessionMeta,
  direction: SplitDir,
  side: SplitSide,
): { node: PaneNode; inserted: boolean } {
  if (node.type === 'leaf') {
    if (node.pane.id !== targetId) return { node, inserted: false };
    const nextLeaf = leaf(session);
    const children = side === 'before' ? [nextLeaf, node] : [node, nextLeaf];
    return { node: split(direction, children), inserted: true };
  }

  const children = node.children.slice();
  for (let i = 0; i < children.length; i++) {
    const child = children[i];
    if (!child) continue;
    const result = insertRelative(child, targetId, session, direction, side);
    if (!result.inserted) continue;
    children[i] = result.node;
    return { node: { ...node, children }, inserted: true };
  }
  return { node, inserted: false };
}

function directionForDropSide(side: PaneDropSide): SplitDir {
  return side === 'left' || side === 'right' ? 'row' : 'col';
}

function splitSideForDropSide(side: PaneDropSide): SplitSide {
  return side === 'left' || side === 'up' ? 'before' : 'after';
}

function removeLeaf(node: PaneNode, sessionId: string): { node: PaneNode | null; removed: SessionMeta | null } {
  if (node.type === 'leaf') {
    return node.pane.id === sessionId ? { node: null, removed: node.pane } : { node, removed: null };
  }

  const children: PaneNode[] = [];
  const ratios: number[] = [];
  let removed: SessionMeta | null = null;
  for (let i = 0; i < node.children.length; i++) {
    const child = node.children[i];
    if (!child) continue;
    const result = removeLeaf(child, sessionId);
    if (result.removed) removed = result.removed;
    if (result.node) {
      children.push(result.node);
      ratios.push(node.ratios[i] ?? 1);
    }
  }
  if (!removed) return { node, removed: null };
  if (children.length === 0) return { node: null, removed };
  if (children.length === 1) return { node: children[0] ?? null, removed };
  const total = ratios.reduce((sum, value) => sum + value, 0) || children.length;
  return { node: { ...node, children, ratios: ratios.map((value) => value / total) }, removed };
}

function updateSplitRatios(node: PaneNode, splitId: string, ratios: number[]): PaneNode {
  if (node.type === 'leaf') return node;
  if (node.id === splitId) return { ...node, ratios };
  return { ...node, children: node.children.map((child) => updateSplitRatios(child, splitId, ratios)) };
}

function findLeafNode(node: PaneNode, sessionId: string): PaneLeaf | null {
  if (node.type === 'leaf') return node.pane.id === sessionId ? node : null;
  for (const child of node.children) {
    const found = findLeafNode(child, sessionId);
    if (found) return found;
  }
  return null;
}

function replaceLeafSession(node: PaneNode, oldId: string, session: SessionMeta): PaneNode {
  if (node.type === 'leaf') {
    if (node.pane.id !== oldId) return node;
    return { type: 'leaf', id: session.id, pane: session };
  }
  return {
    ...node,
    children: node.children.map((child) => replaceLeafSession(child, oldId, session)),
  };
}

function collectRects(node: PaneNode, rect: Omit<PaneRect, 'id'>, out: PaneRect[]) {
  if (node.type === 'leaf') {
    out.push({ id: node.pane.id, ...rect });
    return;
  }
  const total = node.ratios.reduce((sum, value) => sum + value, 0) || node.children.length || 1;
  let offset = 0;
  for (let i = 0; i < node.children.length; i++) {
    const child = node.children[i];
    if (!child) continue;
    const share = (node.ratios[i] ?? 1) / total;
    if (node.direction === 'row') {
      collectRects(child, { x: rect.x + rect.w * offset, y: rect.y, w: rect.w * share, h: rect.h }, out);
    } else {
      collectRects(child, { x: rect.x, y: rect.y + rect.h * offset, w: rect.w, h: rect.h * share }, out);
    }
    offset += share;
  }
}

function overlap(aStart: number, aSize: number, bStart: number, bSize: number): number {
  return Math.max(0, Math.min(aStart + aSize, bStart + bSize) - Math.max(aStart, bStart));
}

class TabStore {
  tabs = $state<Tab[]>([]);
  activeId = $state<string | null>(null);
  revision = $state(0);

  nodeKey(node: PaneNode): string {
    return node.id;
  }

  nodeContains(node: PaneNode, sessionId: string): boolean {
    return contains(node, sessionId);
  }

  paneIndex(tab: Tab, sessionId: string): number {
    return tab.panes.findIndex((pane) => pane.id === sessionId);
  }

  firstPane(tab: Tab): SessionMeta | undefined {
    return tab.panes[0];
  }

  activePane(tab: Tab): SessionMeta | undefined {
    return tab.panes.find((pane) => pane.id === tab.activePaneId);
  }

  private refreshTab(tab: Tab) {
    tab.panes = flatten(tab.layout);
    if (!tab.panes.some((pane) => pane.id === tab.activePaneId)) {
      tab.activePaneId = tab.panes[0]?.id ?? '';
    }
    if (tab.maximizedPaneId && !tab.panes.some((pane) => pane.id === tab.maximizedPaneId)) {
      tab.maximizedPaneId = null;
    }
    const first = tab.panes[0];
    if (first && !tab.title) tab.title = first.title || first.id.slice(0, 8);
  }

  private bump() {
    this.revision++;
  }

  /** Open a new tab containing a single pane. */
  add(session: SessionMeta): Tab {
    const tab: Tab = {
      id: session.id,
      layout: leaf(session),
      panes: [session],
      activePaneId: session.id,
      maximizedPaneId: null,
      title: session.title || session.id.slice(0, 8),
      activity: {},
    };
    this.tabs.push(tab);
    this.activeId = tab.id;
    this.bump();
    return tab;
  }

  /** Open a new tab from a prebuilt pane tree, used by session workspace restore. */
  addLayout(
    title: string,
    layout: PaneNode,
    activePaneId?: string,
    maximizedPaneId?: string | null,
  ): Tab {
    const panes = flatten(layout);
    const first = panes[0];
    if (!first) throw new Error('cannot add empty tab layout');
    const tab: Tab = {
      id: first.id,
      layout,
      panes,
      activePaneId: activePaneId && panes.some((pane) => pane.id === activePaneId) ? activePaneId : first.id,
      maximizedPaneId: maximizedPaneId && panes.some((pane) => pane.id === maximizedPaneId) ? maximizedPaneId : null,
      title: title || first.title || first.id.slice(0, 8),
      activity: {},
    };
    this.tabs.push(tab);
    this.activeId = tab.id;
    this.bump();
    return tab;
  }

  /** Split the active pane in an existing tab and focus the new pane. */
  addPane(tabId: string, session: SessionMeta, direction: SplitDir = 'row', side: SplitSide = 'after') {
    const tab = this.tabs.find((candidate) => candidate.id === tabId);
    if (!tab) return;
    const target = tab.activePaneId || tab.panes[0]?.id;
    if (!target) return;
    const result = insertRelative(tab.layout, target, session, direction, side);
    if (!result.inserted) return;
    tab.layout = result.node;
    tab.activePaneId = session.id;
    tab.maximizedPaneId = null;
    this.activeId = tab.id;
    this.refreshTab(tab);
    this.bump();
  }

  /** Remove a tab entirely. */
  remove(id: string): Tab | undefined {
    const i = this.tabs.findIndex((tab) => tab.id === id);
    if (i < 0) return;
    const [removed] = this.tabs.splice(i, 1);
    if (this.activeId === id) {
      const next = this.tabs[Math.min(i, this.tabs.length - 1)];
      this.activeId = next?.id ?? null;
    }
    this.bump();
    return removed;
  }

  removePane(tabId: string, sessionId: string): { sessionId: string; tabClosed: boolean } | null {
    const tab = this.tabs.find((candidate) => candidate.id === tabId);
    if (!tab) return null;
    const oldIndex = this.paneIndex(tab, sessionId);
    const wasActive = tab.activePaneId === sessionId;
    if (oldIndex < 0) return null;
    const result = removeLeaf(tab.layout, sessionId);
    if (!result.removed) return null;
    if (!result.node) {
      this.remove(tabId);
      return { sessionId, tabClosed: true };
    }
    tab.layout = result.node;
    this.refreshTab(tab);
    if (wasActive || !tab.activePaneId) {
      const next = tab.panes[Math.min(oldIndex, tab.panes.length - 1)];
      if (next) tab.activePaneId = next.id;
    }
    this.bump();
    return { sessionId, tabClosed: false };
  }

  movePane(tabId: string, sourceId: string, targetId: string, side: PaneDropSide): boolean {
    return this.movePaneBetweenTabs(tabId, sourceId, tabId, targetId, side);
  }

  /** Move a pane within one tab or into another tab (merge / reposition). */
  movePaneBetweenTabs(
    sourceTabId: string,
    sourceId: string,
    targetTabId: string,
    targetId: string,
    side: PaneDropSide,
  ): boolean {
    if (sourceId === targetId && sourceTabId === targetTabId) return false;

    const sourceTab = this.tabs.find((candidate) => candidate.id === sourceTabId);
    const targetTab = this.tabs.find((candidate) => candidate.id === targetTabId);
    if (!sourceTab || !targetTab) return false;
    if (!sourceTab.panes.some((pane) => pane.id === sourceId)) return false;
    if (sourceTabId === targetTabId && !targetTab.panes.some((pane) => pane.id === targetId)) {
      return false;
    }

    const removed = removeLeaf(sourceTab.layout, sourceId);
    if (!removed.removed) return false;

    if (!removed.node) {
      this.remove(sourceTabId);
    } else {
      sourceTab.layout = removed.node;
      if (sourceTab.activePaneId === sourceId) {
        const next = sourceTab.panes[0];
        if (next) sourceTab.activePaneId = next.id;
      }
      if (sourceTab.maximizedPaneId === sourceId) sourceTab.maximizedPaneId = null;
      this.refreshTab(sourceTab);
    }

    let targetLayout = targetTab.layout;
    if (targetTab.panes.length === 0) {
      targetTab.layout = leaf(removed.removed);
      targetTab.activePaneId = removed.removed.id;
      targetTab.maximizedPaneId = null;
      this.activeId = targetTabId;
      this.refreshTab(targetTab);
      this.bump();
      return true;
    }

    const anchorId =
      targetTab.panes.some((pane) => pane.id === targetId)
        ? targetId
        : targetTab.activePaneId;
    const inserted = insertRelative(
      targetLayout,
      anchorId,
      removed.removed,
      directionForDropSide(side),
      splitSideForDropSide(side),
    );
    if (!inserted.inserted) return false;

    targetTab.layout = inserted.node;
    targetTab.activePaneId = sourceId;
    targetTab.maximizedPaneId = null;
    this.activeId = targetTabId;
    this.refreshTab(targetTab);
    this.bump();
    return true;
  }

  /** Drop a pane onto a tab strip to merge into that tab. */
  mergePaneIntoTab(sourceTabId: string, sourceId: string, targetTabId: string, side: PaneDropSide = 'right'): boolean {
    const targetTab = this.tabs.find((candidate) => candidate.id === targetTabId);
    if (!targetTab || targetTab.panes.length === 0) return false;
    const anchorId = targetTab.activePaneId || targetTab.panes[0]!.id;
    return this.movePaneBetweenTabs(sourceTabId, sourceId, targetTabId, anchorId, side);
  }

  activate(id: string) {
    if (this.tabs.some((tab) => tab.id === id)) this.activeId = id;
  }

  focusPane(tabId: string, sessionId: string) {
    const tab = this.tabs.find((candidate) => candidate.id === tabId);
    if (!tab) return;
    if (tab.panes.some((pane) => pane.id === sessionId)) {
      tab.activePaneId = sessionId;
      this.activeId = tab.id;
      this.clearActivity(sessionId);
    }
  }

  resizeSplit(tabId: string, splitId: string, ratios: number[]) {
    const tab = this.tabs.find((candidate) => candidate.id === tabId);
    if (!tab) return;
    tab.layout = updateSplitRatios(tab.layout, splitId, ratios);
  }

  focusDirectional(tabId: string, direction: FocusDirection) {
    const tab = this.tabs.find((candidate) => candidate.id === tabId);
    if (!tab || tab.maximizedPaneId) return;
    const rects: PaneRect[] = [];
    collectRects(tab.layout, { x: 0, y: 0, w: 1, h: 1 }, rects);
    const current = rects.find((rect) => rect.id === tab.activePaneId);
    if (!current) return;
    const cx = current.x + current.w / 2;
    const cy = current.y + current.h / 2;
    let best: { id: string; score: number } | null = null;
    for (const candidate of rects) {
      if (candidate.id === current.id) continue;
      const candidateCx = candidate.x + candidate.w / 2;
      const candidateCy = candidate.y + candidate.h / 2;
      let gap = 0;
      let lateral = 0;
      let aligned = 0;
      if (direction === 'left') {
        if (candidate.x + candidate.w > current.x + 0.0001) continue;
        gap = current.x - (candidate.x + candidate.w);
        lateral = Math.abs(candidateCy - cy);
        aligned = overlap(current.y, current.h, candidate.y, candidate.h);
      } else if (direction === 'right') {
        if (candidate.x < current.x + current.w - 0.0001) continue;
        gap = candidate.x - (current.x + current.w);
        lateral = Math.abs(candidateCy - cy);
        aligned = overlap(current.y, current.h, candidate.y, candidate.h);
      } else if (direction === 'up') {
        if (candidate.y + candidate.h > current.y + 0.0001) continue;
        gap = current.y - (candidate.y + candidate.h);
        lateral = Math.abs(candidateCx - cx);
        aligned = overlap(current.x, current.w, candidate.x, candidate.w);
      } else {
        if (candidate.y < current.y + current.h - 0.0001) continue;
        gap = candidate.y - (current.y + current.h);
        lateral = Math.abs(candidateCx - cx);
        aligned = overlap(current.x, current.w, candidate.x, candidate.w);
      }
      const score = gap * 1000 + lateral - aligned;
      if (!best || score < best.score) best = { id: candidate.id, score };
    }
    if (best) this.focusPane(tab.id, best.id);
  }

  toggleMaximize(tabId: string, sessionId?: string) {
    const tab = this.tabs.find((candidate) => candidate.id === tabId);
    if (!tab) return;
    const paneId = sessionId ?? tab.activePaneId;
    if (!paneId || !tab.panes.some((pane) => pane.id === paneId)) return;
    tab.maximizedPaneId = tab.maximizedPaneId === paneId ? null : paneId;
    tab.activePaneId = paneId;
    this.activeId = tab.id;
    this.bump();
  }

  findLeaf(tab: Tab, sessionId: string): PaneLeaf | null {
    return findLeafNode(tab.layout, sessionId);
  }

  /** Swap a pane's live session id/metadata without changing layout structure. */
  replacePaneSession(tabId: string, oldSessionId: string, session: SessionMeta) {
    const tab = this.tabs.find((candidate) => candidate.id === tabId);
    if (!tab) return;
    if (!tab.panes.some((pane) => pane.id === oldSessionId)) return;
    tab.layout = replaceLeafSession(tab.layout, oldSessionId, session);
    if (tab.activePaneId === oldSessionId) tab.activePaneId = session.id;
    if (tab.maximizedPaneId === oldSessionId) tab.maximizedPaneId = session.id;
    const { [oldSessionId]: _drop, ...restActivity } = tab.activity;
    tab.activity = restActivity;
    this.refreshTab(tab);
    this.bump();
  }

  move(fromIdx: number, toIdx: number) {
    if (fromIdx === toIdx) return;
    if (fromIdx < 0 || fromIdx >= this.tabs.length) return;
    if (toIdx < 0 || toIdx >= this.tabs.length) return;
    const [moved] = this.tabs.splice(fromIdx, 1);
    if (moved) {
      this.tabs.splice(toIdx, 0, moved);
      this.bump();
    }
  }

  /** Find which tab contains a given session id (linear scan; tab counts are small). */
  tabOf(sessionId: string): Tab | undefined {
    return this.tabs.find((tab) => tab.panes.some((pane) => pane.id === sessionId));
  }

  /** Mark a pane as having unread activity. Bell takes precedence over output. */
  markActivity(sessionId: string, kind: 'output' | 'bell') {
    const tab = this.tabOf(sessionId);
    if (!tab) return;
    if (this.activeId === tab.id && tab.activePaneId === sessionId) return;
    const existing = tab.activity[sessionId];
    if (kind === 'bell' || existing !== 'bell') {
      tab.activity = { ...tab.activity, [sessionId]: kind };
    }
  }

  clearActivity(sessionId: string) {
    const tab = this.tabOf(sessionId);
    if (!tab || !tab.activity[sessionId]) return;
    const { [sessionId]: _drop, ...rest } = tab.activity;
    tab.activity = rest;
  }

  /** Aggregate activity for a tab (worst of all panes). */
  tabActivity(tab: Tab): 'output' | 'bell' | null {
    let any: 'output' | 'bell' | null = null;
    for (const pane of tab.panes) {
      const activity = tab.activity[pane.id];
      if (activity === 'bell') return 'bell';
      if (activity === 'output') any = 'output';
    }
    return any;
  }
}

export const tabs = new TabStore();