import type { StoredProfile } from './types';

export function toggleProfileInSelection(selected: Set<string>, id: string): Set<string> {
  const next = new Set(selected);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  return next;
}

export function selectAllProfiles(profiles: StoredProfile[]): Set<string> {
  return new Set(profiles.map((p) => p.id));
}

export function invertProfileSelection(selected: Set<string>, profiles: StoredProfile[]): Set<string> {
  const next = new Set<string>();
  for (const p of profiles) {
    if (!selected.has(p.id)) next.add(p.id);
  }
  return next;
}

export function profilesFromSelection(all: StoredProfile[], selected: Set<string>): StoredProfile[] {
  if (selected.size === 0) return [];
  return all.filter((p) => selected.has(p.id));
}

export function rangeSelectProfiles(
  profiles: StoredProfile[],
  anchorId: string | null,
  targetId: string,
): Set<string> {
  if (!anchorId) return new Set([targetId]);
  const ids = profiles.map((p) => p.id);
  const a = ids.indexOf(anchorId);
  const b = ids.indexOf(targetId);
  if (a < 0 || b < 0) return new Set([targetId]);
  const lo = Math.min(a, b);
  const hi = Math.max(a, b);
  return new Set(ids.slice(lo, hi + 1));
}
