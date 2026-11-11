import type { SshAuth, SshProfileSpec, StoredProfile, ProfileIcon, SshInheritField, ProfileGroupDefaults } from './types';

/** Shared group resolver: flags affect only fields explicitly present in that entry. */
export function resolveGroupDefaults(group: string | null, groups: ProfileGroupDefaults) {
  const values: import('./types').GroupSshDefaults = {};
  const sources: Partial<Record<SshInheritField, string>> = {};
  const locked: Partial<Record<SshInheritField, string>> = {};
  const forced: Partial<Record<SshInheritField, string>> = {};
  const hostValues: import('./types').GroupSshDefaults = {};
  const fields: SshInheritField[] = ['user', 'port', 'auth', 'proxy_id', 'jump'];
  for (const { path } of profileGroupBreadcrumbs(group)) {
    const own = groups[path];
    if (!own) continue;
    for (const field of fields) {
      const key = field === 'jump' ? 'jump_profile_ids' : field;
      if (own[key] === undefined) continue;
      if (!locked[field]) {
        Object.assign(values, { [key]: JSON.parse(JSON.stringify(own[key])) });
        sources[field] = path;
        if (own.force_subgroups === true) locked[field] = path;
      }
      if (own.force_hosts === true && !forced[field]) {
        Object.assign(hostValues, { [key]: values[key] });
        forced[field] = sources[field];
      }
    }
  }
  return { values, sources, locked, forced, hostValues };
}

/** UI preview only; backend independently resolves saved profiles at connection time. */
export function previewProfileInheritance(profile: StoredProfile, groups: ProfileGroupDefaults) {
  const copy = JSON.parse(JSON.stringify(profile)) as StoredProfile;
  const sources: Partial<Record<SshInheritField, string>> = {};
  const forced: Partial<Record<SshInheritField, string>> = {};
  if (copy.kind !== 'ssh') return { profile: copy, sources, forced };
  const group = resolveGroupDefaults(copy.group ?? null, groups);
  const fields = new Set<SshInheritField>([...(copy.ssh.inherit ?? []), ...Object.keys(group.forced) as SshInheritField[]]);
  for (const field of fields) {
    const key = field === 'jump' ? 'jump_profile_ids' : field;
    const value = (group.forced[field] ? group.hostValues : group.values)[key];
    if (value === undefined) continue;
    if (field === 'jump') { copy.ssh.jump_via = []; copy.ssh.jump_profile_ids = [...value as string[]]; }
    else Object.assign(copy.ssh, { [field]: JSON.parse(JSON.stringify(value)) });
    sources[field] = group.forced[field] ?? group.sources[field];
    if (group.forced[field]) forced[field] = group.forced[field];
  }
  return { profile: copy, sources, forced };
}

export function profileGroupBreadcrumbs(group: string | null): Array<{ name: string; path: string }> {
  const parts = (group ?? '').split(/[/\\>]+/).map(part => part.trim()).filter(Boolean);
  return parts.map((name, index) => ({ name, path: parts.slice(0, index + 1).join('/') }));
}

/** Preserve credentials/nested hops when the text representation has not changed. */
export async function resolveProfileEditorJumps(lines: string, originalLines: string, original: SshProfileSpec[], resolve: () => Promise<SshProfileSpec[]>): Promise<SshProfileSpec[]> {
  const jumps = lines === originalLines ? original : await resolve();
  return JSON.parse(JSON.stringify(jumps));
}

/** One serializable draft shared by the editor and any future embedded form. */
export interface ProfileEditorDraft {
  schemaVersion: number;
  kind: StoredProfile['kind'];
  name: string;
  group: string;
  tags: string[];
  note: string;
  favorite: boolean;
  icon: ProfileIcon | null;
  host: string;
  port: number;
  user: string;
  auth: ProfileAuthDraft;
  jumps: SshProfileSpec[];
  jumpProfileIds: string[];
  proxyId: string;
  inherit: SshInheritField[];
  remoteSshProfileId: string;
  localBindPort: number | '';
}

export function profileToEditorDraft(profile?: StoredProfile, group = ''): ProfileEditorDraft {
  const endpoint = profile?.kind === 'ssh' ? profile.ssh : profile?.kind === 'rdp' ? profile.rdp : profile?.kind === 'vnc' ? profile.vnc : undefined;
  const remote = profile?.kind === 'rdp' ? profile.rdp : profile?.kind === 'vnc' ? profile.vnc : undefined;
  return {
    schemaVersion: profile?.schemaVersion ?? 1, kind: profile?.kind ?? 'ssh', name: profile?.name ?? '',
    group: profile?.group ?? group, tags: [...(profile?.tags ?? [])], note: profile?.note ?? '', favorite: !!profile?.favorite,
    icon: profile?.icon ? { ...profile.icon } : null, host: endpoint?.host ?? '', port: endpoint?.port ?? 22,
    user: profile?.kind === 'ssh' ? profile.ssh.user : '',
    auth: authToDraft(profile?.kind === 'ssh' ? profile.ssh.auth : { Password: { secret: '' } }),
    jumps: profile?.kind === 'ssh' ? JSON.parse(JSON.stringify(profile.ssh.jump_via ?? [])) : [],
    jumpProfileIds: profile?.kind === 'ssh' ? [...(profile.ssh.jump_profile_ids ?? [])] : [],
    proxyId: profile?.kind === 'ssh' ? profile.ssh.proxy_id ?? '' : '',
    inherit: profile?.kind === 'ssh' ? [...(profile.ssh.inherit ?? [])] : !profile && group.trim() ? ['user', 'port', 'auth', 'proxy_id', 'jump'] : [],
    remoteSshProfileId: remote?.ssh_profile_id ?? '', localBindPort: remote?.local_bind_port ?? '',
  };
}

export function profileFromEditorDraft(draft: ProfileEditorDraft, id: string): StoredProfile {
  const base = {
    schemaVersion: draft.schemaVersion, id, name: draft.name || 'profile', group: draft.group.trim() || null,
    tags: [...draft.tags], note: draft.note.trim() || null, favorite: draft.favorite,
    icon: draft.icon ? { ...draft.icon } : null,
  };
  if (draft.kind === 'ssh') return { ...base, kind: 'ssh', ssh: { ...(draft.inherit.length ? { inherit: [...draft.inherit] } : {}), host: draft.host, port: Number(draft.port) || 22, user: draft.user, auth: authFromDraft(draft.auth), jump_via: JSON.parse(JSON.stringify(draft.jumps)), ...(draft.jumpProfileIds.length ? { jump_profile_ids: [...draft.jumpProfileIds] } : {}), ...(draft.proxyId ? { proxy_id: draft.proxyId } : {}) } };
  const remote = { host: draft.host, port: Number(draft.port) || (draft.kind === 'rdp' ? 3389 : 5900), ssh_profile_id: draft.remoteSshProfileId.trim() || null, local_bind_port: draft.localBindPort === '' ? undefined : Number(draft.localBindPort) || undefined };
  return draft.kind === 'rdp' ? { ...base, kind: 'rdp', rdp: remote } : { ...base, kind: 'vnc', vnc: remote };
}

export type ResourceView = 'grid' | 'list' | 'tree';
export function normalizeResourceView(value: unknown): ResourceView {
  return value === 'list' || value === 'tree' ? value : 'grid';
}

/** null = all resources, empty string = ungrouped; groups include their descendants. */
export function profileInScope(profile: StoredProfile, group: string | null, favorites: boolean): boolean {
  if (favorites && !profile.favorite) return false;
  if (group === null) return true;
  const path = (profile.group ?? '').split(/[/\\>]+/).map(part => part.trim()).filter(Boolean).join('/');
  return group === '' ? path === '' : path === group || path.startsWith(`${group}/`);
}

/** null is the aggregate All view; '' is a real root directory. */
export function catalogGroupPaths(groups: string[], group: string | null, query: string): string[] {
  const needle = query.trim().toLowerCase();
  return groups.filter(path => (group === null || group === '' || path === group || path.startsWith(`${group}/`)) && (!needle || path.toLowerCase().includes(needle)));
}

export function catalogDirectory(profiles: StoredProfile[], groups: string[], group: string | null, query = ''): { profiles: StoredProfile[]; folders: Array<{ name: string; path: string }> } {
  const normalize = (path: string | null | undefined) => profileGroupBreadcrumbs(path ?? null).at(-1)?.path ?? '';
  const current = group === null ? null : normalize(group);
  const paths = new Set(groups.map(normalize));
  for (const profile of profiles) paths.add(normalize(profile.group));
  const folders = new Map<string, { name: string; path: string }>();
  const prefix = current ? `${current}/` : '';
  for (const path of paths) {
    if (!path.startsWith(prefix)) continue;
    const remaining = path.slice(prefix.length);
    const name = query.trim() ? remaining.split('/').at(-1) : remaining.split('/')[0];
    const target = query.trim() ? path : `${prefix}${name}`;
    if (name && (!query.trim() || path.toLowerCase().includes(query.trim().toLowerCase()))) folders.set(target, { name, path: target });
  }
  return {
    profiles: current === null ? profiles : profiles.filter(profile => normalize(profile.group) === current),
    folders: [...folders.values()].sort((a, b) => a.name.localeCompare(b.name)),
  };
}

/** A scoped tree is the same directory listing as grid/list, with each child
 * recursively expanded. Root keeps direct ungrouped hosts AND folder entries;
 * All is aggregate only in flat views, not duplicated into the tree root. */
export function buildCatalogTree(profiles: StoredProfile[], groups: string[], group: string | null): import('./profileTree').ProfileTreeFolder {
  const path = profileGroupBreadcrumbs(group).at(-1)?.path ?? '';
  const directory = catalogDirectory(profiles, groups, path);
  return {
    name: path.split('/').at(-1) ?? '', path, profiles: directory.profiles,
    folders: directory.folders.map(folder => buildCatalogTree(profiles, groups, folder.path)),
  };
}

export function reconcileCatalogSelection(selected: Set<string>, visible: StoredProfile[]): Set<string> {
  return new Set(visible.filter(profile => selected.has(profile.id)).map(profile => profile.id));
}

export interface ProfileAuthDraft {
  kind: 'password' | 'key' | 'vault' | 'agent';
  password: string;
  keyPath: string;
  keyPassphrase: string;
  vaultEntryId: string;
  vaultPassphraseEntryId: string;
}

export function authToDraft(auth: SshAuth): ProfileAuthDraft {
  const draft: ProfileAuthDraft = { kind: 'agent', password: '', keyPath: '', keyPassphrase: '', vaultEntryId: '', vaultPassphraseEntryId: '' };
  if (auth === 'Agent') return draft;
  if ('Password' in auth) return { ...draft, kind: 'password', password: auth.Password.secret };
  if ('PublicKey' in auth) return { ...draft, kind: 'key', keyPath: auth.PublicKey.key_path, keyPassphrase: auth.PublicKey.passphrase ?? '' };
  return { ...draft, kind: 'vault', vaultEntryId: auth.VaultRef.entry_id, vaultPassphraseEntryId: auth.VaultRef.passphrase_entry_id ?? '' };
}

export function authFromDraft(draft: ProfileAuthDraft): SshAuth {
  if (draft.kind === 'agent') return 'Agent';
  if (draft.kind === 'key') return { PublicKey: { key_path: draft.keyPath, passphrase: draft.keyPassphrase || undefined } };
  if (draft.kind === 'vault') return { VaultRef: { entry_id: draft.vaultEntryId.trim(), passphrase_entry_id: draft.vaultPassphraseEntryId.trim() || undefined } };
  return { Password: { secret: draft.password } };
}
