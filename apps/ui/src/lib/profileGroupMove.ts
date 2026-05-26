import type { RpcClient } from './rpc';
import type { StoredProfile } from './types';

/** Normalize prompt input to `profile.group` (null = ungrouped). */
export function normalizeProfileGroupInput(input: string): string | null {
  const trimmed = input.trim();
  if (!trimmed) return null;
  const lower = trimmed.toLowerCase();
  if (lower === '(ungrouped)' || lower === 'ungrouped') return null;
  return trimmed;
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
