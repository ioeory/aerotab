import type { SshAuth, SshProfileSpec, StoredProfile, ProfileIcon } from './types';

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
    remoteSshProfileId: remote?.ssh_profile_id ?? '', localBindPort: remote?.local_bind_port ?? '',
  };
}

export function profileFromEditorDraft(draft: ProfileEditorDraft, id: string): StoredProfile {
  const base = {
    schemaVersion: draft.schemaVersion, id, name: draft.name || 'profile', group: draft.group.trim() || null,
    tags: [...draft.tags], note: draft.note.trim() || null, favorite: draft.favorite,
    icon: draft.icon ? { ...draft.icon } : null,
  };
  if (draft.kind === 'ssh') return { ...base, kind: 'ssh', ssh: { host: draft.host, port: Number(draft.port) || 22, user: draft.user, auth: authFromDraft(draft.auth), jump_via: JSON.parse(JSON.stringify(draft.jumps)), ...(draft.jumpProfileIds.length ? { jump_profile_ids: [...draft.jumpProfileIds] } : {}), ...(draft.proxyId ? { proxy_id: draft.proxyId } : {}) } };
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
