import type { ImportCandidate } from './importTypes';
import { parseGroupSegments } from './profileTree';

export interface ImportPreviewFolder {
  name: string;
  path: string;
  folders: ImportPreviewFolder[];
  candidates: ImportCandidate[];
}

function ensureFolder(
  map: Map<string, ImportPreviewFolder>,
  root: ImportPreviewFolder,
  path: string,
  name: string,
): ImportPreviewFolder {
  const existing = map.get(path);
  if (existing) return existing;

  const parentPath = path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '';
  const parent = parentPath
    ? ensureFolder(map, root, parentPath, parentPath.split('/').pop() ?? '')
    : root;
  const node: ImportPreviewFolder = { name, path, folders: [], candidates: [] };
  parent.folders.push(node);
  map.set(path, node);
  return node;
}

function sortFolder(node: ImportPreviewFolder): void {
  node.candidates.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }));
  node.folders.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }));
  for (const child of node.folders) sortFolder(child);
}

export function buildImportPreviewTree(candidates: ImportCandidate[]): ImportPreviewFolder {
  const root: ImportPreviewFolder = { name: '', path: '', folders: [], candidates: [] };
  const map = new Map<string, ImportPreviewFolder>([['', root]]);

  for (const candidate of candidates) {
    const segments = parseGroupSegments(candidate.group);
    if (segments.length === 0) {
      root.candidates.push(candidate);
      continue;
    }
    let path = '';
    for (const segment of segments) {
      path = path ? `${path}/${segment}` : segment;
      ensureFolder(map, root, path, segment);
    }
    map.get(path)!.candidates.push(candidate);
  }

  sortFolder(root);
  return root;
}

export function collectFolderPaths(folder: ImportPreviewFolder): string[] {
  const out: string[] = [];
  function walk(node: ImportPreviewFolder) {
    if (node.path) out.push(node.path);
    for (const child of node.folders) walk(child);
  }
  for (const child of folder.folders) walk(child);
  return out;
}

export function collectCandidatesInFolder(folder: ImportPreviewFolder): ImportCandidate[] {
  const out = [...folder.candidates];
  for (const child of folder.folders) {
    out.push(...collectCandidatesInFolder(child));
  }
  return out;
}

export function importableCandidates(candidates: ImportCandidate[]): ImportCandidate[] {
  return candidates.filter((c) => c.status === 'ready' || c.status === 'duplicate');
}

export function invertImportSelection(
  selected: Set<string>,
  candidates: ImportCandidate[],
): Set<string> {
  const next = new Set(selected);
  for (const c of importableCandidates(candidates)) {
    if (next.has(c.sourceId)) next.delete(c.sourceId);
    else next.add(c.sourceId);
  }
  return next;
}

export type ImportPreviewRow =
  | { kind: 'ungrouped-header'; count: number; depth: number; key: string }
  | { kind: 'group'; folder: ImportPreviewFolder; depth: number; key: string }
  | { kind: 'candidate'; candidate: ImportCandidate; depth: number; key: string };

function appendFolderContents(
  folder: ImportPreviewFolder,
  depth: number,
  collapsed: ReadonlySet<string>,
  rows: ImportPreviewRow[],
): void {
  for (const candidate of folder.candidates) {
    rows.push({
      kind: 'candidate',
      candidate,
      depth,
      key: `c:${candidate.sourceId}`,
    });
  }
  for (const child of folder.folders) {
    rows.push({ kind: 'group', folder: child, depth, key: `g:${child.path}` });
    if (!collapsed.has(child.path)) {
      appendFolderContents(child, depth + 1, collapsed, rows);
    }
  }
}

/** Flat list of visible table rows (valid direct children of `<tbody>`). */
export function flattenVisibleImportRows(
  root: ImportPreviewFolder,
  collapsed: ReadonlySet<string>,
  showUngroupedLabel: boolean,
): ImportPreviewRow[] {
  const rows: ImportPreviewRow[] = [];

  if (root.candidates.length > 0) {
    if (showUngroupedLabel) {
      rows.push({
        kind: 'ungrouped-header',
        count: root.candidates.length,
        depth: 0,
        key: 'ungrouped',
      });
    }
    for (const candidate of root.candidates) {
      rows.push({
        kind: 'candidate',
        candidate,
        depth: 0,
        key: `c:${candidate.sourceId}`,
      });
    }
  }

  for (const child of root.folders) {
    rows.push({ kind: 'group', folder: child, depth: 0, key: `g:${child.path}` });
    if (!collapsed.has(child.path)) {
      appendFolderContents(child, 1, collapsed, rows);
    }
  }

  return rows;
}

export function folderSelectionState(
  folder: ImportPreviewFolder,
  selected: Set<string>,
): 'none' | 'partial' | 'all' {
  const importable = importableCandidates(collectCandidatesInFolder(folder));
  if (importable.length === 0) return 'none';
  const picked = importable.filter((c) => selected.has(c.sourceId)).length;
  if (picked === 0) return 'none';
  if (picked === importable.length) return 'all';
  return 'partial';
}
