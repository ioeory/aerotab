import { BUILTIN_PROFILE_ICONS } from './profileMeta';
import type { ProfileIcon as ProfileIconData, StoredProfile } from './types';

export type ProfileKind = 'ssh' | 'rdp' | 'vnc';

export interface VisualOverrides {
  groupColors?: Record<string, string>;
  tagColors?: Record<string, string>;
}

export interface VisualTone {
  fg: string;
  bg: string;
  border: string;
}

/** Semantic icon colors — infrastructure roles on a terminal/network ops palette. */
const BUILTIN_ICON_TONES: Record<(typeof BUILTIN_PROFILE_ICONS)[number], VisualTone> = {
  server: { fg: '#7aa2f7', bg: '#7aa2f71a', border: '#7aa2f742' },
  database: { fg: '#bb9af7', bg: '#bb9af71a', border: '#bb9af742' },
  cloud: { fg: '#7dcfff', bg: '#7dcfff1a', border: '#7dcfff42' },
  router: { fg: '#ff9e64', bg: '#ff9e641a', border: '#ff9e6442' },
  key: { fg: '#e0af68', bg: '#e0af681a', border: '#e0af6842' },
  terminal: { fg: '#9ece6a', bg: '#9ece6a1a', border: '#9ece6a42' },
  cpu: { fg: '#2ac3de', bg: '#2ac3de1a', border: '#2ac3de42' },
  cluster: { fg: '#73daca', bg: '#73daca1a', border: '#73daca42' },
  desktop: { fg: '#c0caf5', bg: '#c0caf51a', border: '#c0caf542' },
  globe: { fg: '#b4f9f8', bg: '#b4f9f81a', border: '#b4f9f842' },
  lock: { fg: '#f7768e', bg: '#f7768e1a', border: '#f7768e42' },
};

/** Connection kind — quick scan for SSH vs remote desktop entries. */
export const PROFILE_KIND_TONES: Record<ProfileKind, VisualTone> = {
  ssh: { fg: '#9ece6a', bg: '#9ece6a14', border: '#9ece6a38' },
  rdp: { fg: '#7dcfff', bg: '#7dcfff14', border: '#7dcfff38' },
  vnc: { fg: '#ff9e64', bg: '#ff9e6414', border: '#ff9e6438' },
};

const GROUP_TONES: VisualTone[] = [
  { fg: '#7aa2f7', bg: '#7aa2f712', border: '#7aa2f740' },
  { fg: '#9ece6a', bg: '#9ece6a12', border: '#9ece6a40' },
  { fg: '#bb9af7', bg: '#bb9af712', border: '#bb9af740' },
  { fg: '#7dcfff', bg: '#7dcfff12', border: '#7dcfff40' },
  { fg: '#ff9e64', bg: '#ff9e6412', border: '#ff9e6440' },
  { fg: '#e0af68', bg: '#e0af6812', border: '#e0af6840' },
  { fg: '#73daca', bg: '#73daca12', border: '#73daca40' },
  { fg: '#f7768e', bg: '#f7768e12', border: '#f7768e40' },
];

const TAG_TONES: VisualTone[] = [
  { fg: '#9ece6a', bg: '#9ece6a18', border: '#9ece6a45' },
  { fg: '#7aa2f7', bg: '#7aa2f718', border: '#7aa2f745' },
  { fg: '#bb9af7', bg: '#bb9af718', border: '#bb9af745' },
  { fg: '#7dcfff', bg: '#7dcfff18', border: '#7dcfff45' },
  { fg: '#ff9e64', bg: '#ff9e6418', border: '#ff9e6445' },
  { fg: '#e0af68', bg: '#e0af6818', border: '#e0af6845' },
  { fg: '#73daca', bg: '#73daca18', border: '#73daca45' },
  { fg: '#f7768e', bg: '#f7768e18', border: '#f7768e45' },
  { fg: '#2ac3de', bg: '#2ac3de18', border: '#2ac3de45' },
  { fg: '#c0caf5', bg: '#c0caf518', border: '#c0caf545' },
];

const INITIAL_TONES: VisualTone[] = GROUP_TONES;

export const PRESET_VISUAL_COLORS = GROUP_TONES.map((t) => t.fg);

const NEUTRAL_TONE: VisualTone = {
  fg: '#a9b1d6',
  bg: '#1a1e27',
  border: '#2a2f3b',
};

function hashString(input: string): number {
  let hash = 0;
  for (let i = 0; i < input.length; i++) {
    hash = (hash * 31 + input.charCodeAt(i)) >>> 0;
  }
  return hash;
}

function toneFromPalette(key: string, palette: VisualTone[]): VisualTone {
  if (!key.trim()) return palette[0] ?? NEUTRAL_TONE;
  return palette[hashString(key.trim().toLowerCase()) % palette.length] ?? NEUTRAL_TONE;
}

export function normalizeGroupKey(path: string): string {
  return path
    .trim()
    .replace(/\\/g, '/')
    .replace(/\/+/g, '/')
    .replace(/^\/+|\/+$/g, '')
    .toLowerCase();
}

export function normalizeTagKey(tag: string): string {
  return tag.trim().toLowerCase();
}

function parseHexColor(input: string): string | null {
  const raw = input.trim();
  const m = raw.match(/^#?([0-9a-f]{3}|[0-9a-f]{6})$/i);
  if (!m) return null;
  let hex = m[1]!;
  if (hex.length === 3) {
    hex = hex.split('').map((c) => c + c).join('');
  }
  return `#${hex.toLowerCase()}`;
}

/** Build a full tone from a single accent hex (for custom group/tag colors). */
export function toneFromColor(input: string): VisualTone {
  const fg = parseHexColor(input) ?? PRESET_VISUAL_COLORS[0] ?? '#7aa2f7';
  const r = parseInt(fg.slice(1, 3), 16);
  const g = parseInt(fg.slice(3, 5), 16);
  const b = parseInt(fg.slice(5, 7), 16);
  return {
    fg,
    bg: `rgba(${r}, ${g}, ${b}, 0.11)`,
    border: `rgba(${r}, ${g}, ${b}, 0.32)`,
  };
}

function customGroupTone(groupPath: string, overrides?: VisualOverrides): VisualTone | null {
  if (!overrides?.groupColors) return null;
  const key = normalizeGroupKey(groupPath);
  const direct = overrides.groupColors[key] ?? overrides.groupColors[groupPath.trim()];
  if (!direct) return null;
  return toneFromColor(direct);
}

function customTagTone(tag: string, overrides?: VisualOverrides): VisualTone | null {
  if (!overrides?.tagColors) return null;
  const key = normalizeTagKey(tag);
  const direct = overrides.tagColors[key] ?? overrides.tagColors[tag.trim()];
  if (!direct) return null;
  return toneFromColor(direct);
}

export function builtinIconTone(iconValue: string | undefined | null): VisualTone {
  const key = (iconValue ?? 'server').trim().toLowerCase();
  if (key in BUILTIN_ICON_TONES) {
    return BUILTIN_ICON_TONES[key as keyof typeof BUILTIN_ICON_TONES];
  }
  return BUILTIN_ICON_TONES.server;
}

export function profileKindTone(kind: ProfileKind | undefined): VisualTone {
  return kind ? PROFILE_KIND_TONES[kind] : PROFILE_KIND_TONES.ssh;
}

export function nameInitialTone(name: string): VisualTone {
  const initial = name.trim().charAt(0).toUpperCase();
  if (!initial) return NEUTRAL_TONE;
  return toneFromPalette(initial, INITIAL_TONES);
}

export function groupTone(groupPath: string, overrides?: VisualOverrides): VisualTone {
  const custom = customGroupTone(groupPath, overrides);
  if (custom) return custom;
  const key = normalizeGroupKey(groupPath);
  if (!key) return NEUTRAL_TONE;
  return toneFromPalette(groupPath, GROUP_TONES);
}

export function tagTone(tag: string, overrides?: VisualOverrides): VisualTone {
  const custom = customTagTone(tag, overrides);
  if (custom) return custom;
  return toneFromPalette(tag, TAG_TONES);
}

export function resolveProfileIconTone(
  icon: ProfileIconData | null | undefined,
  name: string,
  kind?: ProfileKind,
): VisualTone {
  const value = (icon?.value ?? '').trim();
  const normalized = value.toLowerCase();

  if (icon?.kind === 'emoji' && value) {
    return { fg: '#e6e9ef', bg: '#1f2330', border: '#2a2f3b' };
  }
  if ((icon?.kind === 'file' || icon?.kind === 'data' || icon?.kind === 'remote' || icon?.kind === 'selfhst') && value) {
    return NEUTRAL_TONE;
  }
  if (icon?.kind === 'builtin' || BUILTIN_PROFILE_ICONS.includes(normalized as (typeof BUILTIN_PROFILE_ICONS)[number])) {
    return builtinIconTone(normalized || 'server');
  }
  if (normalized && normalized !== 'server') {
    return builtinIconTone(normalized);
  }
  if (kind && kind !== 'ssh') {
    return profileKindTone(kind);
  }
  if (!value && name.trim()) {
    return nameInitialTone(name);
  }
  return builtinIconTone('server');
}

export function visualStyle(tone: VisualTone): string {
  return `--profile-tone-fg:${tone.fg};--profile-tone-bg:${tone.bg};--profile-tone-border:${tone.border};`;
}

export function profileIconStyle(
  icon: ProfileIconData | null | undefined,
  name: string,
  kind?: ProfileKind,
): string {
  return visualStyle(resolveProfileIconTone(icon, name, kind));
}

export function tagStyle(tag: string, overrides?: VisualOverrides): string {
  return visualStyle(tagTone(tag, overrides));
}

export function groupStyle(groupPath: string, overrides?: VisualOverrides): string {
  return visualStyle(groupTone(groupPath, overrides));
}

export function profileKindLabel(kind: ProfileKind): string {
  return kind.toUpperCase();
}

export function storedProfileKind(profile: StoredProfile): ProfileKind {
  return profile.kind;
}
