import type { StoredProfile } from './types';
import { sortProfiles } from './profileMeta';

/** Split profile.group into nested folder segments (supports `/`, `\`, ` / `). */
export function parseGroupSegments(group: string | null | undefined): string[] {
  const raw = group?.trim();
  if (!raw) return [];
  return raw
    .split(/[/\\]+/)
    .flatMap((part) => part.split(/\s*\/\s*/))
    .map((s) => s.trim())
    .filter(Boolean);
}

export interface ProfileTreeFolder {
  /** Display name for this level. */
  name: string;
  /** Stable path key joined with `/` (empty string = virtual root). */
  path: string;
  folders: ProfileTreeFolder[];
  profiles: StoredProfile[];
}

const ROOT: ProfileTreeFolder = { name: '', path: '', folders: [], profiles: [] };

function ensureFolder(
  map: Map<string, ProfileTreeFolder>,
  root: ProfileTreeFolder,
  path: string,
  name: string,
): ProfileTreeFolder {
  const existing = map.get(path);
  if (existing) return existing;

  const parentPath = path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '';
  const parent = parentPath ? ensureFolder(map, root, parentPath, parentPath.split('/').pop() ?? '') : root;
  const node: ProfileTreeFolder = { name, path, folders: [], profiles: [] };
  parent.folders.push(node);
  map.set(path, node);
  return node;
}

function sortFolder(node: ProfileTreeFolder): void {
  node.profiles = sortProfiles(node.profiles);
  node.folders.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }));
  for (const child of node.folders) sortFolder(child);
}

/** Build group → subgroup → … → profiles tree from profile.group paths. */
export function buildProfileTree(profiles: StoredProfile[]): ProfileTreeFolder {
  const root: ProfileTreeFolder = {
    name: '',
    path: '',
    folders: [],
    profiles: [],
  };
  const map = new Map<string, ProfileTreeFolder>([['', root]]);

  for (const profile of profiles) {
    const segments = parseGroupSegments(profile.group);
    if (segments.length === 0) {
      root.profiles.push(profile);
      continue;
    }
    let path = '';
    for (const segment of segments) {
      path = path ? `${path}/${segment}` : segment;
      ensureFolder(map, root, path, segment);
    }
    map.get(path)!.profiles.push(profile);
  }

  sortFolder(root);
  return root;
}

/** All profiles in a folder node and its subfolders (depth-first). */
export function collectProfilesInFolder(folder: ProfileTreeFolder): StoredProfile[] {
  const out: StoredProfile[] = [...folder.profiles];
  for (const child of folder.folders) {
    out.push(...collectProfilesInFolder(child));
  }
  return out;
}

/** All folder paths that contain at least one profile (for expand-on-search). */
export function collectFolderPaths(node: ProfileTreeFolder): string[] {
  const paths: string[] = [];
  function walk(folder: ProfileTreeFolder) {
    if (folder.path) paths.push(folder.path);
    for (const child of folder.folders) walk(child);
  }
  for (const child of node.folders) walk(child);
  return paths;
}

/** Folder paths along a profile group (for expand-after-save). */
export function expandPathsForGroup(group: string | null | undefined): Set<string> {
  const expanded = new Set<string>();
  let path = '';
  for (const segment of parseGroupSegments(group)) {
    path = path ? `${path}/${segment}` : segment;
    expanded.add(path);
  }
  return expanded;
}

/** Paths that should stay expanded so every visible match stays reachable. */
export function expandPathsForMatches(
  profiles: StoredProfile[],
  matches: (p: StoredProfile) => boolean,
): Set<string> {
  const expanded = new Set<string>();
  for (const profile of profiles) {
    if (!matches(profile)) continue;
    const segments = parseGroupSegments(profile.group);
    let path = '';
    for (const segment of segments) {
      path = path ? `${path}/${segment}` : segment;
      expanded.add(path);
    }
  }
  return expanded;
}

export function loadCollapsedPaths(): Set<string> {
  try {
    const raw = localStorage.getItem('aerotab.sidebar.collapsedGroups');
    if (!raw) return new Set();
    const arr = JSON.parse(raw) as unknown;
    if (!Array.isArray(arr)) return new Set();
    return new Set(arr.filter((x): x is string => typeof x === 'string'));
  } catch {
    return new Set();
  }
}

export function saveCollapsedPaths(collapsed: Set<string>): void {
  try {
    localStorage.setItem(
      'aerotab.sidebar.collapsedGroups',
      JSON.stringify([...collapsed]),
    );
  } catch {
    /* ignore quota / private mode */
  }
}

export { ROOT as PROFILE_TREE_ROOT };
