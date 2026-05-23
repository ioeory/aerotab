// Terminal color schemes (M4).
//
// These are *palette-only* schemes: they affect the xterm cell colours
// (background, foreground, cursor, 16 ANSI slots) but leave the application
// chrome alone. The chrome (panels, sidebar, borders) is governed by the
// app theme in `theme.ts`. This split mirrors Tabby's "Appearance" vs.
// "Color scheme" separation.
//
// To add a scheme, append a `ColorScheme` literal to `COLOR_SCHEMES`.
// All entries must have 16 ANSI colours in `ansi` (black..white,
// brightBlack..brightWhite).

export interface ColorScheme {
  name: string;
  label: string;
  background: string;
  foreground: string;
  cursor: string;
  selection: string;
  /** 16 entries: black..white, brightBlack..brightWhite. */
  ansi: string[];
}

function s(
  name: string, label: string,
  background: string, foreground: string,
  cursor: string, selection: string,
  ansi: string[],
): ColorScheme {
  return { name, label, background, foreground, cursor, selection, ansi };
}

// 30 hand-picked schemes covering the most-requested aesthetics.
// Sourced from iTerm2-Color-Schemes (MIT licensed) and the original
// authors' palettes. Extend the list to reach the full ~80 over time.
export const COLOR_SCHEMES: ColorScheme[] = [
  s('tokyo-night', 'Tokyo Night',
    '#1a1b26', '#a9b1d6', '#c0caf5', '#33467c',
    ['#15161e','#f7768e','#9ece6a','#e0af68','#7aa2f7','#bb9af7','#7dcfff','#a9b1d6',
     '#414868','#f7768e','#9ece6a','#e0af68','#7aa2f7','#bb9af7','#7dcfff','#c0caf5']),
  s('tokyo-night-storm', 'Tokyo Night Storm',
    '#24283b', '#a9b1d6', '#c0caf5', '#364a82',
    ['#1d202f','#f7768e','#9ece6a','#e0af68','#7aa2f7','#bb9af7','#7dcfff','#a9b1d6',
     '#414868','#f7768e','#9ece6a','#e0af68','#7aa2f7','#bb9af7','#7dcfff','#c0caf5']),
  s('solarized-dark', 'Solarized Dark',
    '#002b36', '#839496', '#93a1a1', '#073642',
    ['#073642','#dc322f','#859900','#b58900','#268bd2','#d33682','#2aa198','#eee8d5',
     '#002b36','#cb4b16','#586e75','#657b83','#839496','#6c71c4','#93a1a1','#fdf6e3']),
  s('solarized-light', 'Solarized Light',
    '#fdf6e3', '#657b83', '#586e75', '#eee8d5',
    ['#073642','#dc322f','#859900','#b58900','#268bd2','#d33682','#2aa198','#eee8d5',
     '#002b36','#cb4b16','#586e75','#657b83','#839496','#6c71c4','#93a1a1','#fdf6e3']),
  s('gruvbox-dark', 'Gruvbox Dark',
    '#282828', '#ebdbb2', '#ebdbb2', '#504945',
    ['#282828','#cc241d','#98971a','#d79921','#458588','#b16286','#689d6a','#a89984',
     '#928374','#fb4934','#b8bb26','#fabd2f','#83a598','#d3869b','#8ec07c','#ebdbb2']),
  s('gruvbox-light', 'Gruvbox Light',
    '#fbf1c7', '#3c3836', '#3c3836', '#d5c4a1',
    ['#fbf1c7','#cc241d','#98971a','#d79921','#458588','#b16286','#689d6a','#7c6f64',
     '#928374','#9d0006','#79740e','#b57614','#076678','#8f3f71','#427b58','#3c3836']),
  s('dracula', 'Dracula',
    '#282a36', '#f8f8f2', '#f8f8f2', '#44475a',
    ['#21222c','#ff5555','#50fa7b','#f1fa8c','#bd93f9','#ff79c6','#8be9fd','#f8f8f2',
     '#6272a4','#ff6e6e','#69ff94','#ffffa5','#d6acff','#ff92df','#a4ffff','#ffffff']),
  s('nord', 'Nord',
    '#2e3440', '#d8dee9', '#d8dee9', '#434c5e',
    ['#3b4252','#bf616a','#a3be8c','#ebcb8b','#81a1c1','#b48ead','#88c0d0','#e5e9f0',
     '#4c566a','#bf616a','#a3be8c','#ebcb8b','#81a1c1','#b48ead','#8fbcbb','#eceff4']),
  s('one-dark', 'One Dark',
    '#282c34', '#abb2bf', '#528bff', '#3e4451',
    ['#282c34','#e06c75','#98c379','#e5c07b','#61afef','#c678dd','#56b6c2','#abb2bf',
     '#5c6370','#e06c75','#98c379','#e5c07b','#61afef','#c678dd','#56b6c2','#ffffff']),
  s('one-light', 'One Light',
    '#fafafa', '#383a42', '#383a42', '#e5e5e6',
    ['#fafafa','#e45649','#50a14f','#c18401','#4078f2','#a626a4','#0184bc','#383a42',
     '#a0a1a7','#e45649','#50a14f','#c18401','#4078f2','#a626a4','#0184bc','#090a0b']),
  s('monokai', 'Monokai',
    '#272822', '#f8f8f2', '#f8f8f0', '#49483e',
    ['#272822','#f92672','#a6e22e','#f4bf75','#66d9ef','#ae81ff','#a1efe4','#f8f8f2',
     '#75715e','#f92672','#a6e22e','#f4bf75','#66d9ef','#ae81ff','#a1efe4','#f9f8f5']),
  s('tomorrow-night', 'Tomorrow Night',
    '#1d1f21', '#c5c8c6', '#c5c8c6', '#373b41',
    ['#1d1f21','#cc6666','#b5bd68','#f0c674','#81a2be','#b294bb','#8abeb7','#c5c8c6',
     '#969896','#cc6666','#b5bd68','#f0c674','#81a2be','#b294bb','#8abeb7','#ffffff']),
  s('tomorrow', 'Tomorrow',
    '#ffffff', '#4d4d4c', '#4d4d4c', '#d6d6d6',
    ['#000000','#c82829','#718c00','#eab700','#4271ae','#8959a8','#3e999f','#4d4d4c',
     '#8e908c','#c82829','#718c00','#eab700','#4271ae','#8959a8','#3e999f','#000000']),
  s('material', 'Material',
    '#263238', '#eeffff', '#ffcc00', '#314549',
    ['#000000','#ff5370','#c3e88d','#ffcb6b','#82aaff','#c792ea','#89ddff','#eeffff',
     '#546e7a','#ff5370','#c3e88d','#ffcb6b','#82aaff','#c792ea','#89ddff','#ffffff']),
  s('material-dark', 'Material Dark',
    '#212121', '#e5e5e5', '#ffcc00', '#3a3a3a',
    ['#000000','#ff5370','#c3e88d','#ffcb6b','#82aaff','#c792ea','#89ddff','#e5e5e5',
     '#545454','#ff5370','#c3e88d','#ffcb6b','#82aaff','#c792ea','#89ddff','#ffffff']),
  s('catppuccin-mocha', 'Catppuccin Mocha',
    '#1e1e2e', '#cdd6f4', '#f5e0dc', '#585b70',
    ['#45475a','#f38ba8','#a6e3a1','#f9e2af','#89b4fa','#f5c2e7','#94e2d5','#bac2de',
     '#585b70','#f38ba8','#a6e3a1','#f9e2af','#89b4fa','#f5c2e7','#94e2d5','#a6adc8']),
  s('catppuccin-latte', 'Catppuccin Latte',
    '#eff1f5', '#4c4f69', '#dc8a78', '#acb0be',
    ['#5c5f77','#d20f39','#40a02b','#df8e1d','#1e66f5','#ea76cb','#179299','#acb0be',
     '#6c6f85','#d20f39','#40a02b','#df8e1d','#1e66f5','#ea76cb','#179299','#bcc0cc']),
  s('ayu-dark', 'Ayu Dark',
    '#0a0e14', '#b3b1ad', '#e6b450', '#273747',
    ['#01060e','#ea6c73','#91b362','#f9af4f','#53bdfa','#fae994','#90e1c6','#c7c7c7',
     '#686868','#f07178','#c2d94c','#ffb454','#59c2ff','#ffee99','#95e6cb','#ffffff']),
  s('ayu-mirage', 'Ayu Mirage',
    '#1f2430', '#cbccc6', '#ffcc66', '#34455a',
    ['#191e2a','#ed8274','#a6cc70','#fad07b','#6dcbfa','#cfbafa','#90e1c6','#c7c7c7',
     '#686868','#f28779','#bae67e','#ffd580','#73d0ff','#d4bfff','#95e6cb','#ffffff']),
  s('snazzy', 'Snazzy',
    '#282a36', '#eff0eb', '#97979b', '#3f4451',
    ['#000000','#ff5c57','#5af78e','#f3f99d','#57c7ff','#ff6ac1','#9aedfe','#f1f1f0',
     '#686868','#ff5c57','#5af78e','#f3f99d','#57c7ff','#ff6ac1','#9aedfe','#eff0eb']),
  s('argonaut', 'Argonaut',
    '#0e1019', '#fffaf4', '#ff0018', '#1c4f70',
    ['#232323','#ff000f','#8ce10b','#ffb900','#008df8','#6d43a6','#00d8eb','#ffffff',
     '#444444','#ff2740','#abe15b','#ffd242','#0092ff','#9a5feb','#67fff0','#ffffff']),
  s('cobalt2', 'Cobalt2',
    '#132738', '#ffffff', '#f0cf06', '#1278a8',
    ['#000000','#ff0000','#38de21','#ffe50a','#1460d2','#ff005d','#00bbbb','#bbbbbb',
     '#555555','#f40e17','#3bd01d','#edc809','#5555ff','#ff55ff','#6ae3fa','#ffffff']),
  s('hopscotch', 'Hopscotch',
    '#322931', '#b9b5b8', '#b9b5b8', '#797379',
    ['#322931','#dd464c','#8fc13e','#fdcc59','#1290bf','#c85e7c','#149b93','#b9b5b8',
     '#797379','#fd8b19','#433b42','#5c545b','#989498','#d5d3d5','#b33508','#ffffff']),
  s('hybrid', 'Hybrid',
    '#1d1f21', '#c5c8c6', '#c5c8c6', '#373b41',
    ['#282a2e','#a54242','#8c9440','#de935f','#5f819d','#85678f','#5e8d87','#707880',
     '#373b41','#cc6666','#b5bd68','#f0c674','#81a2be','#b294bb','#8abeb7','#c5c8c6']),
  s('oceanic-next', 'Oceanic Next',
    '#1b2b34', '#cdd3de', '#6699cc', '#65737e',
    ['#29414f','#ec5f67','#99c794','#fac863','#6699cc','#c594c5','#5fb3b3','#65737e',
     '#405860','#ec5f67','#99c794','#fac863','#6699cc','#c594c5','#5fb3b3','#d8dee9']),
  s('spacegray', 'Spacegray',
    '#2b303b', '#c0c5ce', '#c0c5ce', '#4f5b66',
    ['#343d46','#bf616a','#a3be8c','#ebcb8b','#8fa1b3','#b48ead','#96b5b4','#c0c5ce',
     '#65737e','#bf616a','#a3be8c','#ebcb8b','#8fa1b3','#b48ead','#96b5b4','#eff1f5']),
  s('wombat', 'Wombat',
    '#171717', '#dedacf', '#bbbbbb', '#453b39',
    ['#000000','#ff615a','#b1e969','#ebd99c','#5da9f6','#e86aff','#82fff7','#dedacf',
     '#313131','#f58c80','#ddf88f','#eee5b2','#a5c7ff','#ddaaff','#b7fff9','#ffffff']),
  s('horizon', 'Horizon',
    '#1c1e26', '#cbced0', '#fcfcfa', '#272932',
    ['#0a0a0d','#e95678','#29d398','#fab795','#26bbd9','#ee64ac','#59e1e3','#cbced0',
     '#5b5858','#ec6a88','#3fdaa4','#fbc3a7','#3fc4de','#f075b5','#6be4e6','#d5d8da']),
  s('night-owl', 'Night Owl',
    '#011627', '#d6deeb', '#80a4c2', '#1d3b53',
    ['#011627','#ef5350','#22da6e','#addb67','#82aaff','#c792ea','#21c7a8','#ffffff',
     '#575656','#ef5350','#22da6e','#ffeb95','#82aaff','#c792ea','#7fdbca','#ffffff']),
  s('palenight', 'Palenight',
    '#292d3e', '#959dcb', '#ffcc00', '#34304a',
    ['#292d3e','#f07178','#c3e88d','#ffcb6b','#82aaff','#c792ea','#89ddff','#959dcb',
     '#676e95','#f07178','#c3e88d','#ffcb6b','#82aaff','#c792ea','#89ddff','#ffffff']),
];

export function colorSchemeByName(name: string | null | undefined): ColorScheme | undefined {
  if (!name) return undefined;
  return COLOR_SCHEMES.find((s) => s.name === name);
}

/** Convert to xterm theme shape (subset of ITheme). */
export function toXtermTheme(scheme: ColorScheme): Record<string, string> {
  const [black, red, green, yellow, blue, magenta, cyan, white,
         brBlack, brRed, brGreen, brYellow, brBlue, brMagenta, brCyan, brWhite] = scheme.ansi;
  return {
    background: scheme.background,
    foreground: scheme.foreground,
    cursor: scheme.cursor,
    cursorAccent: scheme.background,
    selectionBackground: scheme.selection,
    black: black!, red: red!, green: green!, yellow: yellow!,
    blue: blue!, magenta: magenta!, cyan: cyan!, white: white!,
    brightBlack: brBlack!, brightRed: brRed!, brightGreen: brGreen!,
    brightYellow: brYellow!, brightBlue: brBlue!, brightMagenta: brMagenta!,
    brightCyan: brCyan!, brightWhite: brWhite!,
  };
}
