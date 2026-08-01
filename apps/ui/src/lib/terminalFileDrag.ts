/** Terminal trzsz file drag-and-drop helpers (WebView2 / WebKitGTK / WKWebView). */

import { b64encode, tauriInvoke } from './rpc';
import { isPaneDragActive, PANE_DRAG_MIME } from './paneDrag';

function dragTypes(ev: DragEvent): readonly string[] {
  const types = ev.dataTransfer?.types;
  if (!types) return [];
  const out: string[] = [];
  for (let i = 0; i < types.length; i++) {
    const t = types[i];
    if (t) out.push(t);
  }
  return out;
}

function isPaneReorderDrag(ev: DragEvent): boolean {
  const types = dragTypes(ev);
  if (types.includes(PANE_DRAG_MIME)) return true;
  return types.some((t) => t.includes('aerotab-pane'));
}

/**
 * OS file drags often expose an empty `types` list during dragover on Tauri
 * WebView / WebKitGTK; SFTP panes accept unconditionally — mirror that here.
 */
export function isOsFileDragEvent(ev: DragEvent): boolean {
  if (isPaneDragActive() || isPaneReorderDrag(ev)) return false;
  const types = dragTypes(ev);
  if (types.length === 0) return true;
  return types.some((t) => t === 'Files' || t === 'application/x-moz-file');
}

export function allowTerminalFileDragOver(ev: DragEvent, enabled: boolean): void {
  if (!enabled || !isOsFileDragEvent(ev)) return;
  ev.preventDefault();
  ev.stopPropagation();
  if (ev.dataTransfer) ev.dataTransfer.dropEffect = 'copy';
}

/** Tauri / Electron expose absolute paths on dropped File objects (Windows/Linux). */
export function filePathsFromDataTransfer(dt: DataTransfer): string[] {
  const paths: string[] = [];
  for (let i = 0; i < dt.files.length; i++) {
    const path = (dt.files[i] as File & { path?: string }).path;
    if (typeof path === 'string' && path.length > 0) paths.push(path);
  }
  return paths;
}

/**
 * Resolve native paths for a drop. macOS WKWebView does not expose `File.path`;
 * stage blobs into a temp file via Tauri so trzsz nodefs can read them.
 */
export async function resolveDropFilePaths(dt: DataTransfer): Promise<string[]> {
  const paths = filePathsFromDataTransfer(dt);
  if (paths.length > 0) return paths;
  if (dt.files.length === 0) return [];

  const staged: string[] = [];
  for (let i = 0; i < dt.files.length; i++) {
    const file = dt.files.item(i);
    if (!file) continue;
    const buf = await file.arrayBuffer();
    const invoke = tauriInvoke<string>;
    if (!invoke) {
      throw new Error('Native file staging requires the Tauri app');
    }
    const path = await invoke('local_stage_drop_file', {
      name: file.name || `drop-${i}`,
      data: b64encode(new Uint8Array(buf)),
    });
    if (!path) {
      throw new Error('Failed to stage dropped file');
    }
    staged.push(path);
  }
  return staged;
}

/**
 * WKWebView navigates to dropped images/files unless dragover/drop call
 * preventDefault at the document level (terminal capture alone is not enough).
 */
export function installWebKitFileDropGuard(): () => void {
  const opts: AddEventListenerOptions = { capture: true };
  const guard = (ev: DragEvent) => {
    if (!isOsFileDragEvent(ev)) return;
    ev.preventDefault();
  };
  document.addEventListener('dragenter', guard, opts);
  document.addEventListener('dragover', guard, opts);
  document.addEventListener('drop', guard, opts);
  return () => {
    document.removeEventListener('dragenter', guard, opts);
    document.removeEventListener('dragover', guard, opts);
    document.removeEventListener('drop', guard, opts);
  };
}

export function installTerminalFileDrag(
  root: HTMLElement,
  handlers: {
    onDragOver: (ev: DragEvent) => void;
    onDrop: (ev: DragEvent) => void;
  },
): () => void {
  const onDragEnter = (ev: DragEvent) => handlers.onDragOver(ev);
  const opts: AddEventListenerOptions = { capture: true };
  root.addEventListener('dragenter', onDragEnter, opts);
  root.addEventListener('dragover', handlers.onDragOver, opts);
  root.addEventListener('drop', handlers.onDrop, opts);
  return () => {
    root.removeEventListener('dragenter', onDragEnter, opts);
    root.removeEventListener('dragover', handlers.onDragOver, opts);
    root.removeEventListener('drop', handlers.onDrop, opts);
  };
}
