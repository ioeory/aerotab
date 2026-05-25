import type { StoredProfile } from './types';

/** Strip trailing " copy" / " (n)" suffixes before generating a duplicate name. */
export function duplicateNameBase(name: string): string {
  const trimmed = name.trim() || 'profile';
  return trimmed
    .replace(/\s+\(\d+\)$/u, '')
    .replace(/\s+copy(?:\s+\d+)?$/iu, '')
    .trim() || trimmed;
}

/** Pick a unique profile name among existing entries (e.g. "server" → "server copy"). */
export function suggestDuplicateProfileName(baseName: string, existingNames: string[]): string {
  const taken = new Set(existingNames.map((n) => n.trim().toLowerCase()).filter(Boolean));
  const root = duplicateNameBase(baseName);
  let candidate = `${root} copy`;
  if (!taken.has(candidate.toLowerCase())) return candidate;
  for (let i = 2; i < 1000; i++) {
    candidate = `${root} copy ${i}`;
    if (!taken.has(candidate.toLowerCase())) return candidate;
  }
  return `${root} copy ${Date.now()}`;
}

/** Deep-clone a stored profile with a new id and display name (not persisted). */
export function cloneProfileAsNew(src: StoredProfile, newName: string, newId: string): StoredProfile {
  const raw = JSON.parse(JSON.stringify(src)) as StoredProfile;
  raw.id = newId;
  raw.name = newName;
  raw.favorite = false;
  return raw;
}

export const BUILTIN_PROFILE_ICONS = [
  'server',
  'database',
  'cloud',
  'router',
  'key',
  'terminal',
  'cpu',
  'cluster',
] as const;

export function normalizeTags(tags: string[] | null | undefined): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const raw of tags ?? []) {
    const tag = raw.trim();
    const key = tag.toLowerCase();
    if (!tag || seen.has(key)) continue;
    seen.add(key);
    out.push(tag);
  }
  return out;
}

export function parseTagsInput(text: string): string[] {
  return normalizeTags(text.split(/[\n,;]+/));
}

export function formatTags(tags: string[] | null | undefined): string {
  return normalizeTags(tags).join(', ');
}

export function profileGroupName(profile: StoredProfile): string {
  return profile.group?.trim() || '(Ungrouped)';
}

export function sortProfiles(profiles: StoredProfile[]): StoredProfile[] {
  return [...profiles].sort((a, b) => {
    if (!!a.favorite !== !!b.favorite) return a.favorite ? -1 : 1;
    return a.name.localeCompare(b.name, undefined, { sensitivity: 'base' });
  });
}

export function profileEndpointLabel(profile: StoredProfile): string {
  if (profile.kind === 'ssh') {
    const port = profile.ssh.port === 22 ? '' : `:${profile.ssh.port}`;
    return `${profile.ssh.user}@${profile.ssh.host}${port}`;
  }
  if (profile.kind === 'rdp') {
    return `RDP · ${profile.rdp.host}:${profile.rdp.port}`;
  }
  return `VNC · ${profile.vnc.host}:${profile.vnc.port}`;
}

function profileText(profile: StoredProfile): string {
  const endpoint =
    profile.kind === 'ssh'
      ? [profile.ssh.host, profile.ssh.user, String(profile.ssh.port)]
      : profile.kind === 'rdp'
        ? ['rdp', profile.rdp.host, String(profile.rdp.port)]
        : ['vnc', profile.vnc.host, String(profile.vnc.port)];
  return [
    profile.name,
    profile.kind,
    profile.group ?? '',
    ...endpoint,
    ...(profile.tags ?? []),
    profile.icon?.value ?? '',
    profile.favorite ? 'favorite starred pinned' : '',
  ].join(' ').toLowerCase();
}

function matchField(profile: StoredProfile, key: string, value: string): boolean {
  const needle = value.trim().toLowerCase();
  if (!needle) return true;
  if (key === 'tag' || key === 'tags') {
    return (profile.tags ?? []).some((tag) => tag.toLowerCase().includes(needle));
  }
  if (key === 'group') return (profile.group ?? '').toLowerCase().includes(needle);
  if (key === 'host') {
    const host =
      profile.kind === 'ssh' ? profile.ssh.host
        : profile.kind === 'rdp' ? profile.rdp.host
          : profile.vnc.host;
    return host.toLowerCase().includes(needle);
  }
  if (key === 'user') {
    return profile.kind === 'ssh' && profile.ssh.user.toLowerCase().includes(needle);
  }
  if (key === 'kind') return profile.kind.includes(needle);
  if (key === 'icon') return (profile.icon?.value ?? '').toLowerCase().includes(needle);
  if (key === 'fav' || key === 'favorite') return !!profile.favorite && !['0', 'false', 'no'].includes(needle);
  return profileText(profile).includes(`${key}:${needle}`);
}

export function matchesProfileQuery(profile: StoredProfile, query: string): boolean {
  const tokens = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (tokens.length === 0) return true;
  const haystack = profileText(profile);
  return tokens.every((token) => {
    if (token.startsWith('#')) {
      const needle = token.slice(1);
      return (profile.tags ?? []).some((tag) => tag.toLowerCase().includes(needle));
    }
    const sep = token.indexOf(':');
    if (sep > 0) return matchField(profile, token.slice(0, sep), token.slice(sep + 1));
    return haystack.includes(token);
  });
}

export function summarizeProfiles(profiles: StoredProfile[]) {
  const groups = new Set<string>();
  const tags = new Set<string>();
  let favorites = 0;
  for (const profile of profiles) {
    groups.add(profileGroupName(profile));
    if (profile.favorite) favorites += 1;
    for (const tag of normalizeTags(profile.tags)) tags.add(tag.toLowerCase());
  }
  return { groups: groups.size, tags: tags.size, favorites };
}