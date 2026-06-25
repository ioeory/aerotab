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

function importCandidateText(candidate: ImportCandidate): string {
  const parts = [
    candidate.name,
    candidate.group ?? '',
    candidate.kind,
    candidate.note ?? '',
    candidate.status,
    candidate.errorMessage ?? '',
    ...candidate.tags,
  ];
  if (candidate.profile?.kind === 'ssh') {
    const ssh = candidate.profile.ssh;
    parts.push(ssh.host, ssh.user, String(ssh.port));
  }
  return parts.join(' ').toLowerCase();
}

/** Filter import preview rows by free-text query (name, host, user, group, tags, status). */
export function matchesImportCandidateQuery(candidate: ImportCandidate, query: string): boolean {
  const tokens = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (tokens.length === 0) return true;
  const haystack = importCandidateText(candidate);
  return tokens.every((token) => {
    if (token.startsWith('#')) {
      const needle = token.slice(1);
      return candidate.tags.some((tag) => tag.toLowerCase().includes(needle));
    }
    if (token === 'duplicate' || token === '重复') {
      return candidate.status === 'duplicate';
    }
    if (token === 'ready' || token === '可导入') {
      return candidate.status === 'ready';
    }
    if (token === 'error' || token === '错误') {
      return candidate.status === 'error';
    }
    return haystack.includes(token);
  });
}

export function countMatchingImportCandidates(
  candidates: ImportCandidate[],
  query: string,
): number {
  const q = query.trim();
  if (!q) return candidates.length;
  return candidates.filter((c) => matchesImportCandidateQuery(c, q)).length;
}

function folderHasMatchingCandidate(folder: ImportPreviewFolder, query: string): boolean {
  if (folder.candidates.some((c) => matchesImportCandidateQuery(c, query))) return true;
  return folder.folders.some((child) => folderHasMatchingCandidate(child, query));
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
  query = '',
): void {
  const filtering = query.trim().length > 0;
  for (const candidate of folder.candidates) {
    if (filtering && !matchesImportCandidateQuery(candidate, query)) continue;
    rows.push({
      kind: 'candidate',
      candidate,
      depth,
      key: `c:${candidate.sourceId}`,
    });
  }
  for (const child of folder.folders) {
    if (filtering && !folderHasMatchingCandidate(child, query)) continue;
    rows.push({ kind: 'group', folder: child, depth, key: `g:${child.path}` });
    const expanded = filtering || !collapsed.has(child.path);
    if (expanded) {
      appendFolderContents(child, depth + 1, collapsed, rows, query);
    }
  }
}

/** Flat list of visible table rows (valid direct children of `<tbody>`). */
export function flattenVisibleImportRows(
  root: ImportPreviewFolder,
  collapsed: ReadonlySet<string>,
  showUngroupedLabel: boolean,
  query = '',
): ImportPreviewRow[] {
  const rows: ImportPreviewRow[] = [];
  const filtering = query.trim().length > 0;
  const ungrouped = filtering
    ? root.candidates.filter((c) => matchesImportCandidateQuery(c, query))
    : root.candidates;

  if (ungrouped.length > 0) {
    if (showUngroupedLabel && !filtering) {
      rows.push({
        kind: 'ungrouped-header',
        count: ungrouped.length,
        depth: 0,
        key: 'ungrouped',
      });
    }
    for (const candidate of ungrouped) {
      rows.push({
        kind: 'candidate',
        candidate,
        depth: 0,
        key: `c:${candidate.sourceId}`,
      });
    }
  }

  for (const child of root.folders) {
    if (filtering && !folderHasMatchingCandidate(child, query)) continue;
    rows.push({ kind: 'group', folder: child, depth: 0, key: `g:${child.path}` });
    const expanded = filtering || !collapsed.has(child.path);
    if (expanded) {
      appendFolderContents(child, 1, collapsed, rows, query);
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
