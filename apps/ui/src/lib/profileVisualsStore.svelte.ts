import type { RpcClient } from './rpc';
import { normalizeGroupKey, normalizeTagKey, type VisualOverrides } from './profileVisuals';

export interface ProfileVisualsSettings {
  showSshKindBadge: boolean;
  groupColors: Record<string, string>;
  tagColors: Record<string, string>;
}

const SETTINGS_KEY = 'profiles.visuals';

const DEFAULTS: ProfileVisualsSettings = {
  showSshKindBadge: false,
  groupColors: {},
  tagColors: {},
};

export const PROFILE_VISUALS_EXPORT_VERSION = 1;

export interface ProfileVisualsExport {
  version: number;
  showSshKindBadge?: boolean;
  groupColors?: Record<string, string>;
  tagColors?: Record<string, string>;
}

function parseVisualsImport(raw: unknown): Partial<ProfileVisualsSettings> | null {
  if (!raw || typeof raw !== 'object') return null;
  const obj = raw as Record<string, unknown>;
  const version = typeof obj.version === 'number' ? obj.version : 1;
  if (version !== PROFILE_VISUALS_EXPORT_VERSION) return null;
  const out: Partial<ProfileVisualsSettings> = {};
  if (typeof obj.showSshKindBadge === 'boolean') {
    out.showSshKindBadge = obj.showSshKindBadge;
  }
  if (obj.groupColors && typeof obj.groupColors === 'object') {
    out.groupColors = sanitizeColorMap(obj.groupColors as Record<string, string>);
  }
  if (obj.tagColors && typeof obj.tagColors === 'object') {
    out.tagColors = sanitizeColorMap(obj.tagColors as Record<string, string>);
  }
  return out;
}

class ProfileVisualsStore {
  loaded = $state(false);
  showSshKindBadge = $state(DEFAULTS.showSshKindBadge);
  groupColors = $state<Record<string, string>>({});
  tagColors = $state<Record<string, string>>({});

  get overrides(): VisualOverrides {
    return {
      groupColors: this.groupColors,
      tagColors: this.tagColors,
    };
  }

  apply(raw: Partial<ProfileVisualsSettings> | null | undefined) {
    this.showSshKindBadge = raw?.showSshKindBadge ?? DEFAULTS.showSshKindBadge;
    this.groupColors = sanitizeColorMap(raw?.groupColors);
    this.tagColors = sanitizeColorMap(raw?.tagColors);
  }

  toSettings(): ProfileVisualsSettings {
    return {
      showSshKindBadge: this.showSshKindBadge,
      groupColors: { ...this.groupColors },
      tagColors: { ...this.tagColors },
    };
  }

  async load(rpc: RpcClient) {
    try {
      const r = await rpc.call<{ value: unknown }>('settings.get', { key: SETTINGS_KEY });
      if (r.value && typeof r.value === 'object') {
        this.apply(r.value as Partial<ProfileVisualsSettings>);
      } else {
        this.apply(DEFAULTS);
      }
    } catch {
      this.apply(DEFAULTS);
    } finally {
      this.loaded = true;
    }
  }

  async save(rpc: RpcClient) {
    await rpc.call('settings.set', { key: SETTINGS_KEY, value: this.toSettings() });
    notifyChanged();
  }

  async setGroupColor(rpc: RpcClient, groupPath: string, color: string | null) {
    const key = normalizeGroupKey(groupPath);
    if (!key) return;
    const next = { ...this.groupColors };
    if (color) next[key] = color;
    else delete next[key];
    this.groupColors = next;
    await this.save(rpc);
  }

  async setTagColor(rpc: RpcClient, tag: string, color: string | null) {
    const key = normalizeTagKey(tag);
    if (!key) return;
    const next = { ...this.tagColors };
    if (color) next[key] = color;
    else delete next[key];
    this.tagColors = next;
    await this.save(rpc);
  }

  async resetGroupColors(rpc: RpcClient) {
    this.groupColors = {};
    await this.save(rpc);
  }

  async resetTagColors(rpc: RpcClient) {
    this.tagColors = {};
    await this.save(rpc);
  }

  /** Remap custom group colors when a folder path is renamed. */
  async renameGroupPaths(rpc: RpcClient, oldPath: string, newPath: string) {
    const oldKey = normalizeGroupKey(oldPath);
    const newKey = normalizeGroupKey(newPath);
    if (!oldKey || !newKey || oldKey === newKey) return;
    const next: Record<string, string> = {};
    for (const [key, color] of Object.entries(this.groupColors)) {
      let remapped = key;
      if (key === oldKey || key.startsWith(`${oldKey}/`)) {
        remapped = `${newKey}${key.slice(oldKey.length)}`;
      }
      next[remapped] = color;
    }
    this.groupColors = next;
    await this.save(rpc);
  }

  async setShowSshKindBadge(rpc: RpcClient, value: boolean) {
    this.showSshKindBadge = value;
    await this.save(rpc);
  }

  exportPayload(): ProfileVisualsExport {
    return {
      version: PROFILE_VISUALS_EXPORT_VERSION,
      ...this.toSettings(),
    };
  }

  async importPayload(rpc: RpcClient, raw: unknown, merge = true) {
    const incoming = parseVisualsImport(raw);
    if (!incoming) {
      throw new Error('invalid profile visuals export');
    }
    if (merge) {
      if (incoming.groupColors) {
        this.groupColors = { ...this.groupColors, ...incoming.groupColors };
      }
      if (incoming.tagColors) {
        this.tagColors = { ...this.tagColors, ...incoming.tagColors };
      }
    } else {
      this.apply({
        showSshKindBadge: incoming.showSshKindBadge ?? this.showSshKindBadge,
        groupColors: incoming.groupColors ?? {},
        tagColors: incoming.tagColors ?? {},
      });
    }
    if (typeof incoming.showSshKindBadge === 'boolean') {
      this.showSshKindBadge = incoming.showSshKindBadge;
    }
    await this.save(rpc);
  }
}

function sanitizeColorMap(raw: Record<string, string> | undefined): Record<string, string> {
  if (!raw || typeof raw !== 'object') return {};
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(raw)) {
    if (typeof v === 'string' && /^#[0-9a-f]{6}$/i.test(v.trim())) {
      out[k] = v.trim().toLowerCase();
    }
  }
  return out;
}

function notifyChanged() {
  if (typeof document !== 'undefined') {
    document.dispatchEvent(new CustomEvent('aerotab:profile-visuals-changed'));
  }
}

export const profileVisualsStore = new ProfileVisualsStore();
