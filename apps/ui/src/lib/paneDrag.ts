/** Pane drag-and-drop helpers (pointer drag for WebView2; HTML5 fallback). */

import type { PaneDropSide } from './tabs.svelte';

export const PANE_DRAG_MIME = 'application/x-aerotab-pane';
const PLAIN_PREFIX = 'aerotab-pane:';

export interface PaneDragPayload {
  tabId: string;
  paneId: string;
}

export type PaneDropHit =
  | { kind: 'pane'; tabId: string; paneId: string; side: PaneDropSide }
  | { kind: 'tab'; tabId: string };

let activeDrag: PaneDragPayload | null = null;
let pointerDrag: PaneDragPayload | null = null;
let currentHit: PaneDropHit | null = null;
let globalHandlersInstalled = false;
let pointerMoveHandler: ((ev: PointerEvent) => void) | null = null;
let pointerUpHandler: ((ev: PointerEvent) => void) | null = null;

type HitListener = () => void;
const hitListeners = new Set<HitListener>();

type DropListener = (detail: { source: PaneDragPayload; hit: PaneDropHit }) => void;
const dropListeners = new Set<DropListener>();

const DRAG_THRESHOLD_PX = 5;

function notifyHit() {
  for (const fn of hitListeners) fn();
}

function dropSideFromRect(rect: DOMRect, x: number, y: number): PaneDropSide {
  const rx = (x - rect.left) / Math.max(1, rect.width);
  const ry = (y - rect.top) / Math.max(1, rect.height);
  const distances: Array<[PaneDropSide, number]> = [
    ['left', rx],
    ['right', 1 - rx],
    ['up', ry],
    ['down', 1 - ry],
  ];
  distances.sort((a, b) => a[1] - b[1]);
  return distances[0]?.[0] ?? 'right';
}

function setCurrentHit(hit: PaneDropHit | null) {
  const prev = currentHit;
  if (
    prev?.kind === hit?.kind
    && prev?.kind === 'pane'
    && hit?.kind === 'pane'
    && prev.tabId === hit.tabId
    && prev.paneId === hit.paneId
    && prev.side === hit.side
  ) {
    return;
  }
  if (prev?.kind === 'tab' && hit?.kind === 'tab' && prev.tabId === hit.tabId) {
    return;
  }
  currentHit = hit;
  notifyHit();
}

export function hitTestPaneDrop(
  x: number,
  y: number,
  source: PaneDragPayload,
): PaneDropHit | null {
  const el = document.elementFromPoint(x, y);
  if (!el) return null;

  const paneEl = el.closest('[data-pane-drop-pane]') as HTMLElement | null;
  if (paneEl) {
    const tabId = paneEl.dataset.paneDropTab;
    const paneId = paneEl.dataset.paneDropPane;
    if (!tabId || !paneId) return null;
    if (tabId === source.tabId && paneId === source.paneId) return null;
    const side = dropSideFromRect(paneEl.getBoundingClientRect(), x, y);
    return { kind: 'pane', tabId, paneId, side };
  }

  const tabEl = el.closest('[data-tab-drop]') as HTMLElement | null;
  if (tabEl) {
    const tabId = tabEl.dataset.tabDrop;
    if (!tabId) return null;
    return { kind: 'tab', tabId };
  }

  return null;
}

function setBodyDragActive(active: boolean) {
  if (typeof document === 'undefined') return;
  if (active) document.body.dataset.paneDragActive = 'true';
  else delete document.body.dataset.paneDragActive;
}

function finishPointerDrag(clientX: number, clientY: number) {
  const source = pointerDrag;
  if (!source) return;
  const hit = hitTestPaneDrop(clientX, clientY, source) ?? currentHit;
  pointerDrag = null;
  activeDrag = null;
  setCurrentHit(null);
  setBodyDragActive(false);
  if (hit) {
    for (const fn of dropListeners) fn({ source, hit });
  }
}

function teardownPointerListeners() {
  if (pointerMoveHandler) {
    window.removeEventListener('pointermove', pointerMoveHandler, true);
    pointerMoveHandler = null;
  }
  if (pointerUpHandler) {
    window.removeEventListener('pointerup', pointerUpHandler, true);
    window.removeEventListener('pointercancel', pointerUpHandler, true);
    pointerUpHandler = null;
  }
}

/** Keep drop cursor valid over terminal canvas / empty areas (HTML5 path). */
export function installPaneDragGlobalHandlers(): void {
  if (globalHandlersInstalled || typeof document === 'undefined') return;
  globalHandlersInstalled = true;
  document.addEventListener(
    'dragover',
    (ev) => {
      if (!activeDrag) return;
      ev.preventDefault();
      if (ev.dataTransfer) ev.dataTransfer.dropEffect = 'move';
    },
    true,
  );
  document.addEventListener(
    'dragenter',
    (ev) => {
      if (!activeDrag) return;
      ev.preventDefault();
    },
    true,
  );
}

export function subscribePaneDragHit(fn: HitListener): () => void {
  hitListeners.add(fn);
  return () => hitListeners.delete(fn);
}

export function getPaneDragHit(): PaneDropHit | null {
  return currentHit;
}

export function subscribePanePointerDrop(fn: DropListener): () => void {
  dropListeners.add(fn);
  return () => dropListeners.delete(fn);
}

export function isPointerPaneDragActive(): boolean {
  return pointerDrag !== null;
}

/** Primary pane move handle (WebView2 / Win10-safe). */
export function startPointerPaneDrag(
  tabId: string,
  paneId: string,
  ev: PointerEvent,
  handle: HTMLElement,
): void {
  if (ev.button !== 0) return;
  const startX = ev.clientX;
  const startY = ev.clientY;
  let started = false;

  const onMove = (move: PointerEvent) => {
    if (!started) {
      const dx = move.clientX - startX;
      const dy = move.clientY - startY;
      if (dx * dx + dy * dy < DRAG_THRESHOLD_PX * DRAG_THRESHOLD_PX) return;
      started = true;
      pointerDrag = { tabId, paneId };
      activeDrag = pointerDrag;
      setBodyDragActive(true);
      try {
        handle.setPointerCapture(move.pointerId);
      } catch {
        /* ignore */
      }
    }
    move.preventDefault();
    if (!pointerDrag) return;
    setCurrentHit(hitTestPaneDrop(move.clientX, move.clientY, pointerDrag));
  };

  const onUp = (up: PointerEvent) => {
    window.removeEventListener('pointermove', onMove, true);
    teardownPointerListeners();
    if (started) {
      try {
        handle.releasePointerCapture(up.pointerId);
      } catch {
        /* ignore */
      }
      finishPointerDrag(up.clientX, up.clientY);
    } else {
      pointerDrag = null;
      activeDrag = null;
      setCurrentHit(null);
      setBodyDragActive(false);
    }
  };

  pointerMoveHandler = onMove;
  pointerUpHandler = onUp;
  window.addEventListener('pointermove', onMove, true);
  window.addEventListener('pointerup', onUp, true);
  window.addEventListener('pointercancel', onUp, true);
}

export function beginPaneDrag(tabId: string, paneId: string, dt: DataTransfer | null): void {
  activeDrag = { tabId, paneId };
  if (!dt) return;
  const raw = JSON.stringify(activeDrag);
  dt.setData(PANE_DRAG_MIME, raw);
  dt.setData('text/plain', `${PLAIN_PREFIX}${raw}`);
  dt.effectAllowed = 'copyMove';
  try {
    dt.setDragImage(document.createElement('div'), 0, 0);
  } catch {
    /* ignore */
  }
}

export function endPaneDrag(): void {
  activeDrag = null;
  pointerDrag = null;
  setCurrentHit(null);
  setBodyDragActive(false);
  teardownPointerListeners();
}

export function getActivePaneDrag(): PaneDragPayload | null {
  return activeDrag;
}

export function isPaneDragActive(): boolean {
  return activeDrag !== null;
}

export function isPaneDragEvent(ev: DragEvent): boolean {
  if (activeDrag) return true;
  const types = ev.dataTransfer?.types;
  if (!types) return false;
  for (let i = 0; i < types.length; i++) {
    const t = types[i] ?? '';
    if (t === PANE_DRAG_MIME || t.includes('aerotab-pane')) return true;
  }
  const plain = types.includes('text/plain') || types.includes('Text');
  if (!plain) return false;
  return true;
}

export function allowPaneDragOver(ev: DragEvent): void {
  if (!activeDrag) return;
  ev.preventDefault();
  if (ev.dataTransfer) ev.dataTransfer.dropEffect = 'move';
}

export function readPaneDragData(ev: DragEvent): PaneDragPayload | null {
  if (activeDrag) return activeDrag;
  const dt = ev.dataTransfer;
  if (!dt) return null;
  let raw = dt.getData(PANE_DRAG_MIME);
  if (!raw) {
    const plain = dt.getData('text/plain') || dt.getData('Text');
    if (plain.startsWith(PLAIN_PREFIX)) raw = plain.slice(PLAIN_PREFIX.length);
  }
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    if (typeof parsed.tabId === 'string' && typeof parsed.paneId === 'string') {
      return { tabId: parsed.tabId, paneId: parsed.paneId };
    }
  } catch {
    return null;
  }
  return null;
}
