// Built-in colour themes for the AeroTab UI. Themes drive a small set of
// CSS custom properties on the document root; xterm picks up the matching
// palette via `applyTheme` callers reading the same constants.

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
  label: 'Tokyo Night',
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
  label: 'Solarized Dark',
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
  label: 'Gruvbox Dark',
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
  label: 'One Light',
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
  label: 'Termora Dark',
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
  label: 'Termora Light',
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

export function applyTheme(theme: Theme) {
  const r = document.documentElement;
  r.style.setProperty('--color-bg', theme.bg);
  r.style.setProperty('--color-panel', theme.panel);
  r.style.setProperty('--color-panel-2', theme.panel2);
  r.style.setProperty('--color-bg-soft', theme.bgSoft ?? theme.bg);
  r.style.setProperty('--color-surface-raised', theme.surfaceRaised ?? theme.panel2);
  r.style.setProperty('--color-fg', theme.fg);
  r.style.setProperty('--color-fg-muted', theme.fgMuted);
  r.style.setProperty('--color-border', theme.border);
  r.style.setProperty('--color-border-soft', theme.borderSoft);
  r.style.setProperty('--color-accent', theme.accent);
  r.style.setProperty('--color-danger', theme.danger);
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
