// Stable theme IDs and terminal palettes. UI-only tokens are deliberately
// separate so appearance previews never mutate a terminal's ANSI palette.

export interface Theme {
  name: string;
  label: string;
  bg: string;
  panel: string;
  panel2: string;
  bgSoft?: string;
  surfaceRaised?: string;
  fg: string;
  fgMuted: string;
  border: string;
  borderSoft: string;
  accent: string;
  danger: string;
  /** xterm ansi (black..white, brightBlack..brightWhite). */
  ansi: string[];
}

export const TOKYO_NIGHT: Theme = {
  name: 'tokyo-night',
  label: 'Slate',
  bg: '#0b0d12',
  panel: '#13161d',
  panel2: '#1a1e27',
  fg: '#c0caf5',
  fgMuted: '#6b7894',
  border: '#2a2f3a',
  borderSoft: '#1f232c',
  accent: '#7aa2f7',
  danger: '#f7768e',
  ansi: [
    '#15161e', '#f7768e', '#9ece6a', '#e0af68',
    '#7aa2f7', '#bb9af7', '#7dcfff', '#a9b1d6',
    '#414868', '#f7768e', '#9ece6a', '#e0af68',
    '#7aa2f7', '#bb9af7', '#7dcfff', '#c0caf5',
  ],
};

export const SOLARIZED_DARK: Theme = {
  name: 'solarized-dark',
  label: 'Deep teal',
  bg: '#002b36',
  panel: '#073642',
  panel2: '#0a4150',
  fg: '#93a1a1',
  fgMuted: '#586e75',
  border: '#0e4a59',
  borderSoft: '#093540',
  accent: '#268bd2',
  danger: '#dc322f',
  ansi: [
    '#073642', '#dc322f', '#859900', '#b58900',
    '#268bd2', '#d33682', '#2aa198', '#eee8d5',
    '#002b36', '#cb4b16', '#586e75', '#657b83',
    '#839496', '#6c71c4', '#93a1a1', '#fdf6e3',
  ],
};

export const GRUVBOX_DARK: Theme = {
  name: 'gruvbox-dark',
  label: 'Warm graphite',
  bg: '#1d2021',
  panel: '#282828',
  panel2: '#32302f',
  fg: '#ebdbb2',
  fgMuted: '#928374',
  border: '#3c3836',
  borderSoft: '#282828',
  accent: '#fabd2f',
  danger: '#fb4934',
  ansi: [
    '#282828', '#cc241d', '#98971a', '#d79921',
    '#458588', '#b16286', '#689d6a', '#a89984',
    '#928374', '#fb4934', '#b8bb26', '#fabd2f',
    '#83a598', '#d3869b', '#8ec07c', '#ebdbb2',
  ],
};

export const ONE_LIGHT: Theme = {
  name: 'one-light',
  label: 'Warm light',
  bg: '#fafafa',
  panel: '#eaeaeb',
  panel2: '#e0e0e2',
  fg: '#383a42',
  fgMuted: '#8b8e94',
  border: '#c8c8c9',
  borderSoft: '#dadada',
  accent: '#4078f2',
  danger: '#e45649',
  ansi: [
    '#fafafa', '#e45649', '#50a14f', '#c18401',
    '#4078f2', '#a626a4', '#0184bc', '#383a42',
    '#a0a1a7', '#e45649', '#50a14f', '#c18401',
    '#4078f2', '#a626a4', '#0184bc', '#090a0b',
  ],
};

export const TERMORA_DARK: Theme = {
  name: 'termora-dark',
  label: 'Graphite',
  bg: '#0f1117',
  panel: '#171a22',
  panel2: '#20242e',
  bgSoft: '#121620',
  surfaceRaised: '#1c212b',
  fg: '#d8dee9',
  fgMuted: '#8f9bad',
  border: '#303642',
  borderSoft: '#252b35',
  accent: '#5b8def',
  danger: '#e06c75',
  ansi: [
    '#1b1f27', '#e06c75', '#98c379', '#d19a66',
    '#61afef', '#c678dd', '#56b6c2', '#abb2bf',
    '#5c6370', '#e06c75', '#98c379', '#e5c07b',
    '#61afef', '#c678dd', '#56b6c2', '#d8dee9',
  ],
};

export const TERMORA_LIGHT: Theme = {
  name: 'termora-light',
  label: 'Cloud',
  bg: '#f6f7fb',
  panel: '#ffffff',
  panel2: '#eef1f6',
  bgSoft: '#f0f2f7',
  surfaceRaised: '#ffffff',
  fg: '#252a33',
  fgMuted: '#677183',
  border: '#d4d9e3',
  borderSoft: '#e2e6ee',
  accent: '#356dd9',
  danger: '#c4454d',
  ansi: [
    '#252a33', '#c4454d', '#4d7c2f', '#a36500',
    '#356dd9', '#8f4bb8', '#1f7a8c', '#677183',
    '#8a94a6', '#c4454d', '#4d7c2f', '#a36500',
    '#356dd9', '#8f4bb8', '#1f7a8c', '#111827',
  ],
};

export const BUILTIN_THEMES: Theme[] = [
  TERMORA_DARK,
  TOKYO_NIGHT,
  SOLARIZED_DARK,
  GRUVBOX_DARK,
  ONE_LIGHT,
  TERMORA_LIGHT,
];

export type ThemeMode = 'dark' | 'light';

export interface UiPalette {
  label: string;
  mode: ThemeMode;
  bg: string;
  panel: string;
  panel2: string;
  raised: string;
  fg: string;
  muted: string;
  border: string;
  accent: string;
}

/** Netcatty-inspired neutral surfaces, with the six persisted IDs intact. */
export const UI_PALETTES: Record<string, UiPalette> = {
  'termora-dark': { label: 'Graphite', mode: 'dark', bg: '#17191c', panel: '#1d2024', panel2: '#272b30', raised: '#2b3036', fg: '#f0f1f3', muted: '#b5bcc6', border: '#49515c', accent: '#a9bdf0' },
  'tokyo-night': { label: 'Slate', mode: 'dark', bg: '#171c25', panel: '#1d2430', panel2: '#273140', raised: '#2b3545', fg: '#f0f3fa', muted: '#bbc5d6', border: '#4b5c73', accent: '#a5c2ff' },
  'solarized-dark': { label: 'Deep teal', mode: 'dark', bg: '#142323', panel: '#1b2b2c', panel2: '#253637', raised: '#293c3d', fg: '#edf5f2', muted: '#b7ccc6', border: '#4e6863', accent: '#9acdc5' },
  'gruvbox-dark': { label: 'Warm graphite', mode: 'dark', bg: '#211e1b', panel: '#292521', panel2: '#35302a', raised: '#3b352e', fg: '#f7f1e8', muted: '#cdc0ae', border: '#6b5e4e', accent: '#e9c28b' },
  'one-light': { label: 'Warm light', mode: 'light', bg: '#f7f5f1', panel: '#fffefa', panel2: '#ede9e2', raised: '#ffffff', fg: '#282621', muted: '#625d54', border: '#b4ada1', accent: '#735027' },
  'termora-light': { label: 'Cloud', mode: 'light', bg: '#f4f6f9', panel: '#ffffff', panel2: '#e9edf3', raised: '#ffffff', fg: '#202833', muted: '#536174', border: '#aab5c4', accent: '#315eac' },
};

export function themeMode(theme: Theme): ThemeMode {
  return UI_PALETTES[theme.name]?.mode ?? (contrastRatio(theme.bg, '#ffffff') < 4.5 ? 'light' : 'dark');
}

function rgb(hex: string): number[] {
  return [1, 3, 5].map(i => parseInt(hex.slice(i, i + 2), 16));
}

export function contrastRatio(a: string, b: string): number {
  const luminance = (hex: string) => rgb(hex).map(v => {
    const c = v / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  }).reduce((sum, c, i) => sum + c * ([0.2126, 0.7152, 0.0722][i] ?? 0), 0);
  const x = luminance(a);
  const y = luminance(b);
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

export function normalizeAccent(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  if (/^#[0-9a-f]{6}$/i.test(value)) return value.toLowerCase();
  if (/^#[0-9a-f]{3}$/i.test(value)) return '#' + [...value.slice(1)].map(c => c + c).join('').toLowerCase();
  return null;
}

function mix(a: string, b: string, ratio: number): string {
  const other = rgb(b);
  return '#' + rgb(a).map((c, i) => Math.round(c * ratio + (other[i] ?? 0) * (1 - ratio)).toString(16).padStart(2, '0')).join('');
}

export function themeTokens(theme: Theme, accentOverride?: string | null): Record<string, string> {
  const p = UI_PALETTES[theme.name] ?? {
    label: theme.label, mode: themeMode(theme), bg: theme.bg, panel: theme.panel,
    panel2: theme.panel2, raised: theme.surfaceRaised ?? theme.panel2,
    fg: theme.fg, muted: theme.fgMuted, border: theme.border, accent: theme.accent,
  };
  const light = p.mode === 'light';
  const accent = normalizeAccent(accentOverride) ?? p.accent;
  const accentFg = contrastRatio(accent, '#ffffff') >= contrastRatio(accent, '#17191c') ? '#ffffff' : '#17191c';
  // A white accent on a light palette still needs a visible focus/label colour.
  const accentText = contrastRatio(accent, p.panel2) >= 4.5 ? accent : p.accent;
  return {
    bg: p.bg, panel: p.panel, 'panel-2': p.panel2, 'bg-soft': mix(p.panel, p.bg, 0.5),
    'surface-raised': p.raised, fg: p.fg, 'fg-muted': p.muted, 'fg-subtle': p.muted,
    border: p.border, 'border-soft': mix(p.border, p.panel, 0.6),
    accent, 'accent-fg': accentFg, 'accent-text': accentText,
    'accent-soft': mix(accent, p.panel, light ? 0.10 : 0.16),
    'surface-hover': mix(p.fg, p.panel, light ? 0.07 : 0.08),
    'surface-selected': mix(accent, p.panel, light ? 0.12 : 0.18),
    'focus-ring': accentText, 'fg-disabled': p.muted, 'surface-disabled': p.panel2,
    success: light ? '#27643d' : '#a4d7ad', warning: light ? '#805612' : '#e9c386',
    danger: light ? '#b22e3e' : '#f2a3ab', info: light ? '#315eac' : '#a5c2ff',
    'danger-soft': mix(light ? '#b22e3e' : '#f2a3ab', p.panel, 0.12),
    'scrollbar-thumb': p.border, 'scrollbar-hover': p.muted,
  };
}

/** Paints UI chrome only; never sends terminal configuration or ANSI updates. */
export function applyTheme(theme: Theme, accentOverride?: string | null) {
  if (typeof document === 'undefined') return;
  const r = document.documentElement;
  for (const [key, value] of Object.entries(themeTokens(theme, accentOverride))) {
    r.style.setProperty(`--color-${key}`, value);
  }
  r.style.colorScheme = themeMode(theme);
  r.dataset.theme = theme.name;
  r.dataset.themeMode = themeMode(theme);
}

export function xtermPalette(theme: Theme) {
  return {
    background: theme.bg,
    foreground: theme.fg,
    cursor: theme.accent,
    cursorAccent: theme.bg,
    selectionBackground: theme.accent + '55',
    black: theme.ansi[0],
    red: theme.ansi[1],
    green: theme.ansi[2],
    yellow: theme.ansi[3],
    blue: theme.ansi[4],
    magenta: theme.ansi[5],
    cyan: theme.ansi[6],
    white: theme.ansi[7],
    brightBlack: theme.ansi[8],
    brightRed: theme.ansi[9],
    brightGreen: theme.ansi[10],
    brightYellow: theme.ansi[11],
    brightBlue: theme.ansi[12],
    brightMagenta: theme.ansi[13],
    brightCyan: theme.ansi[14],
    brightWhite: theme.ansi[15],
  };
}
