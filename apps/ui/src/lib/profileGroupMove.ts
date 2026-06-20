import type { RpcClient } from './rpc';
import type { StoredProfile } from './types';
import { normalizeGroupPath } from './profileTree';

/** Normalize prompt input to `profile.group` (null = ungrouped). */
export function normalizeProfileGroupInput(input: string): string | null {
  const trimmed = input.trim();
  if (!trimmed) return null;
  const lower = trimmed.toLowerCase();
  if (lower === '(ungrouped)' || lower === 'ungrouped') return null;
  return trimmed;
}

/** Replace a group path prefix (exact match or nested under prefix). */
export function replaceGroupPathPrefix(path: string, oldPrefix: string, newPrefix: string): string {
  if (path === oldPrefix || path.startsWith(`${oldPrefix}/`)) {
    return `${newPrefix}${path.slice(oldPrefix.length)}`;
  }
  return path;
}

/** Resolve rename prompt input to a full group path, or null if invalid. */
export function resolveRenamedGroupPath(sourcePath: string, input: string): string | null {
  const source = normalizeGroupPath(sourcePath);
  if (!source) return null;
  const trimmed = input.trim();
  if (!trimmed) return null;

  let target: string | null;
  if (/[/\\]/.test(trimmed)) {
    target = normalizeGroupPath(trimmed);
  } else {
    const parent = source.includes('/') ? source.slice(0, source.lastIndexOf('/')) : '';
    target = normalizeGroupPath(parent ? `${parent}/${trimmed}` : trimmed);
  }
  if (!target || target === source) return null;
  if (target.startsWith(`${source}/`) || source.startsWith(`${target}/`)) return null;
  return target;
}

/** Profiles whose group equals or nests under `groupPath`. */
export function profilesUnderGroupPath(profiles: StoredProfile[], groupPath: string): StoredProfile[] {
  const source = normalizeGroupPath(groupPath);
  if (!source) return [];
  return profiles.filter((p) => {
    const group = normalizeGroupPath(p.group);
    return group === source || group?.startsWith(`${source}/`);
  });
}

/** Default value for the move-to-group prompt. */
export function defaultGroupForMove(profiles: StoredProfile[]): string {
  if (profiles.length === 0) return '';
  const first = profiles[0]!.group ?? '';
  if (profiles.every((p) => (p.group ?? '') === first)) return first;
  return '';
}

/** Persist new group for each profile that changed. Returns how many were updated. */
export async function upsertProfilesGroup(
  rpc: RpcClient,
  profiles: StoredProfile[],
  group: string | null,
): Promise<number> {
  let moved = 0;
  for (const p of profiles) {
    if ((p.group ?? null) === group) continue;
    await rpc.call('profile.upsert', { ...p, group });
    moved += 1;
  }
  return moved;
}
