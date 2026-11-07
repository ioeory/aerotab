import type { RpcClient } from './rpc';
import type { StoredProfile } from './types';
import { withRpcTimeout } from './rpcTimeout';
import { normalizeGroupPath } from './profileTree';
import { normalizeResourceView } from './profileCatalogHelpers';
import { notifyProfilesChanged } from './profileEvents';
import { monitoringProfileReferences } from './hostMonitoring.svelte';
import { i18n } from './i18n.svelte';

export function normalizeCatalogGroups(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return [...new Set(value.filter((path): path is string => typeof path === 'string')
    .map(path => normalizeGroupPath(path)).filter((path): path is string => !!path))]
    .sort((a, b) => a.localeCompare(b, undefined, { sensitivity: 'base' }));
}

export class ProfileCatalog {
  profiles = $state<StoredProfile[]>([]);
  groups = $state<string[]>([]);
  refreshing = $state(false);
  query = $state('');
  selectedIds = $state<Set<string>>(new Set());
  group = $state<string | null>(null);
  favoritesOnly = $state(false);
  recentOnly = $state(false);
  recentIds = $state<string[]>([]);
  view = $state(normalizeResourceView(this.readView()));
  private pending: Promise<void> | null = null;
  private invalidated = false;
  constructor(private rpc: RpcClient) {}

  private readView(): unknown {
    try { return localStorage.getItem('aerotab.hosts.view'); } catch { return null; }
  }
  setView(value: unknown): void {
    this.view = normalizeResourceView(value);
    try { localStorage.setItem('aerotab.hosts.view', this.view); } catch { /* Optional preference. */ }
  }

  /** Coalesces ordinary loads; explicit invalidation queues a fresh read after any in-flight snapshot. */
  refresh(options: { invalidate?: boolean } = {}): Promise<void> {
    if (options.invalidate) this.invalidated = true;
    if (this.pending) return this.pending;
    this.refreshing = true;
    this.pending = (async () => {
      do {
        this.invalidated = false;
        try {
          const [profiles, groups, recent] = await Promise.all([
            withRpcTimeout(this.rpc.call<StoredProfile[]>('profile.list'), 20_000, 'profile.list'),
            this.rpc.call<{ value: unknown }>('settings.get', { key: 'profile.groups' }).catch(() => ({ value: this.groups })),
            this.rpc.call<{ value: unknown }>('settings.get', { key: 'recentProfiles' }).catch(() => ({ value: this.recentIds })),
          ]);
          // A change notification makes this snapshot obsolete; publish only the fresh read.
          if (!this.invalidated) {
            this.profiles = profiles;
            this.groups = normalizeCatalogGroups(groups.value);
            this.recentIds = Array.isArray(recent.value) ? recent.value.filter((id): id is string => typeof id === 'string') : [];
          }
        } catch (error) {
          if (!this.invalidated) throw error;
        }
      } while (this.invalidated);
    })().finally(() => { this.refreshing = false; this.pending = null; });
    return this.pending;
  }

  /** Use after profile mutations/change notifications, not for concurrent initial consumers. */
  invalidate(): Promise<void> {
    return this.refresh({ invalidate: true });
  }

  async upsert(profile: StoredProfile): Promise<void> {
    // Svelte state proxies cannot be passed to the desktop serializer directly.
    const snapshot = JSON.parse(JSON.stringify(profile)) as StoredProfile;
    await this.rpc.call('profile.upsert', snapshot);
    notifyProfilesChanged({ profileId: snapshot.id, group: snapshot.group });
    await this.invalidate();
  }

  async remove(id: string): Promise<void> {
    const dependencies = monitoringProfileReferences(this.rpc, id);
    if (dependencies.length) throw Error(i18n.t('monitoring.profileReferenced', { names: dependencies.map(panel => panel.name).join(', ') }));
    await this.rpc.call('profile.delete', { id });
    this.selectedIds = new Set([...this.selectedIds].filter(value => value !== id));
    notifyProfilesChanged({ profileId: id });
    await this.invalidate();
  }

  recordRecent(id: string): void {
    this.recentIds = [id, ...this.recentIds.filter(value => value !== id)].slice(0, 8);
    void this.rpc.call('settings.set', { key: 'recentProfiles', value: this.recentIds }).catch(() => {});
  }

  async saveGroups(paths: string[]): Promise<void> {
    const groups = normalizeCatalogGroups(paths);
    await this.rpc.call('settings.set', { key: 'profile.groups', value: groups });
    this.groups = groups;
  }
}

const catalogs = new WeakMap<RpcClient, ProfileCatalog>();
/** Pass the same RpcClient to sidebar and workspace to share snapshots, search and selection. */
export function getProfileCatalog(rpc: RpcClient): ProfileCatalog {
  let catalog = catalogs.get(rpc);
  if (!catalog) { catalog = new ProfileCatalog(rpc); catalogs.set(rpc, catalog); }
  return catalog;
}
