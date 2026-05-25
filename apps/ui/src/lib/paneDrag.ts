/** Pane drag-and-drop helpers (WebView2 needs text/plain + in-memory source). */

export const PANE_DRAG_MIME = 'application/x-aerotab-pane';
const PLAIN_PREFIX = 'aerotab-pane:';

export interface PaneDragPayload {
  tabId: string;
  paneId: string;
}

let activeDrag: PaneDragPayload | null = null;
let globalHandlersInstalled = false;

/** Keep drop cursor valid over terminal canvas / empty areas (WebView2). */
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

export function beginPaneDrag(tabId: string, paneId: string, dt: DataTransfer | null): void {
  activeDrag = { tabId, paneId };
  if (!dt) return;
  const raw = JSON.stringify(activeDrag);
  dt.setData(PANE_DRAG_MIME, raw);
  dt.setData('text/plain', `${PLAIN_PREFIX}${raw}`);
  // WebView2 accepts copyMove more reliably than move-only.
  dt.effectAllowed = 'copyMove';
  try {
    dt.setDragImage(document.createElement('div'), 0, 0);
  } catch {
    /* ignore */
  }
}

export function endPaneDrag(): void {
  activeDrag = null;
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
