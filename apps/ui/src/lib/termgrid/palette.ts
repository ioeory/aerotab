/**
 * Resolves engine colour codes (`kind << 24 | value`) to RGB.
 *
 * kind 0 = named: 0–15 ANSI, 256 fg, 257 bg, 258 cursor, 259–266 dim ANSI,
 * 267 bright fg, 268 dim fg; kind 1 = indexed 0–255; kind 2 = 0xRRGGBB.
 */

export const COLOR_KIND_SHIFT = 24;
export const COLOR_NAMED = 0;
export const COLOR_INDEXED = 1;
export const COLOR_RGB = 2;

export const NAMED_FG = 256;
export const NAMED_BG = 257;
export const NAMED_CURSOR = 258;
export const NAMED_DIM_BASE = 259;
export const NAMED_BRIGHT_FG = 267;
export const NAMED_DIM_FG = 268;

/** xterm-style theme record (subset of xterm.js `ITheme`). */
export interface ThemeColors {
  background?: string;
  foreground?: string;
  cursor?: string;
  cursorAccent?: string;
  selectionBackground?: string;
  black?: string;
  red?: string;
  green?: string;
  yellow?: string;
  blue?: string;
  magenta?: string;
  cyan?: string;
  white?: string;
  brightBlack?: string;
  brightRed?: string;
  brightGreen?: string;
  brightYellow?: string;
  brightBlue?: string;
  brightMagenta?: string;
  brightCyan?: string;
  brightWhite?: string;
  [key: string]: string | undefined;
}

const ANSI_KEYS = [
  'black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white',
  'brightBlack', 'brightRed', 'brightGreen', 'brightYellow', 'brightBlue',
  'brightMagenta', 'brightCyan', 'brightWhite',
] as const;

const DEFAULT_ANSI = [
  0x000000, 0xcd0000, 0x00cd00, 0xcdcd00, 0x0000ee, 0xcd00cd, 0x00cdcd, 0xe5e5e5,
  0x7f7f7f, 0xff0000, 0x00ff00, 0xffff00, 0x5c5cff, 0xff00ff, 0x00ffff, 0xffffff,
];

export interface Rgba {
  r: number;
  g: number;
  b: number;
  a: number;
}

/** Parse `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb()`/`rgba()` and `transparent`. */
export function parseCssColor(input: string | undefined, fallback: number): Rgba {
  if (!input) return unpack(fallback, 1);
  const s = input.trim().toLowerCase();
  if (s === 'transparent') return { r: 0, g: 0, b: 0, a: 0 };
  if (s.startsWith('#')) {
    const hex = s.slice(1);
    if (hex.length === 3 || hex.length === 4) {
      const r = parseInt(hex[0]! + hex[0]!, 16);
      const g = parseInt(hex[1]! + hex[1]!, 16);
      const b = parseInt(hex[2]! + hex[2]!, 16);
      const a = hex.length === 4 ? parseInt(hex[3]! + hex[3]!, 16) / 255 : 1;
      return { r, g, b, a };
    }
    if (hex.length === 6 || hex.length === 8) {
      const v = parseInt(hex.slice(0, 6), 16);
      const a = hex.length === 8 ? parseInt(hex.slice(6, 8), 16) / 255 : 1;
      if (!Number.isNaN(v)) return unpack(v, a);
    }
    return unpack(fallback, 1);
  }
  const m = /^rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*(?:,\s*([\d.]+)\s*)?\)$/.exec(s);
  if (m) {
    return {
      r: clamp255(parseFloat(m[1]!)),
      g: clamp255(parseFloat(m[2]!)),
      b: clamp255(parseFloat(m[3]!)),
      a: m[4] !== undefined ? Math.min(1, Math.max(0, parseFloat(m[4]))) : 1,
    };
  }
  return unpack(fallback, 1);
}

function clamp255(v: number): number {
  return Math.max(0, Math.min(255, Math.round(v)));
}

function unpack(v: number, a: number): Rgba {
  return { r: (v >> 16) & 0xff, g: (v >> 8) & 0xff, b: v & 0xff, a };
}

export function packRgb(c: Rgba): number {
  return ((c.r & 0xff) << 16) | ((c.g & 0xff) << 8) | (c.b & 0xff);
}

export function rgbaToCss(c: Rgba): string {
  return c.a >= 1 ? `rgb(${c.r},${c.g},${c.b})` : `rgba(${c.r},${c.g},${c.b},${c.a.toFixed(3)})`;
}

/**
 * Palette with a resolve cache. `resolve()` returns a packed 0xRRGGBB
 * number (always opaque); the terminal background's alpha is tracked
 * separately in `backgroundAlpha` for translucent windows.
 */
export class Palette {
  private ansi: number[] = DEFAULT_ANSI.slice();
  private cube: Uint32Array = buildCube();
  private cache = new Map<number, number>();
  foreground = 0xd8d8d8;
  background = 0x000000;
  backgroundAlpha = 1;
  cursor = 0xd8d8d8;
  cursorAccent = 0x000000;
  selection: Rgba = { r: 255, g: 255, b: 255, a: 0.3 };
  /** Bold text in ANSI 0–7 is promoted to the bright variant (xterm default). */
  boldIsBright = true;

  constructor(theme?: ThemeColors) {
    if (theme) this.setTheme(theme);
  }

  setTheme(theme: ThemeColors) {
    const fg = parseCssColor(theme.foreground, 0xd8d8d8);
    const bg = parseCssColor(theme.background, 0x000000);
    this.foreground = packRgb(fg);
    this.background = packRgb(bg);
    this.backgroundAlpha = bg.a;
    this.cursor = packRgb(parseCssColor(theme.cursor, this.foreground));
    this.cursorAccent = packRgb(parseCssColor(theme.cursorAccent, this.background));
    this.selection = parseCssColor(theme.selectionBackground, 0xffffff);
    if (theme.selectionBackground === undefined) this.selection.a = 0.3;
    for (let i = 0; i < 16; i++) {
      const key = ANSI_KEYS[i]!;
      this.ansi[i] = packRgb(parseCssColor(theme[key], DEFAULT_ANSI[i]!));
    }
    this.cache.clear();
  }

  /** Resolve an engine colour code to packed RGB. */
  resolve(code: number, isFg: boolean): number {
    const kind = code >>> COLOR_KIND_SHIFT;
    const value = code & 0xffffff;
    if (kind === COLOR_RGB) return value;
    const cached = this.cache.get(code);
    if (cached !== undefined) return cached;
    let out: number;
    if (kind === COLOR_INDEXED) {
      out = value < 16 ? this.ansi[value]! : this.cube[value]!;
    } else {
      out = this.resolveNamed(value, isFg);
    }
    this.cache.set(code, out);
    return out;
  }

  private resolveNamed(n: number, isFg: boolean): number {
    if (n < 16) return this.ansi[n]!;
    if (n === NAMED_FG) return this.foreground;
    if (n === NAMED_BG) return this.background;
    if (n === NAMED_CURSOR) return this.cursor;
    if (n >= NAMED_DIM_BASE && n < NAMED_DIM_BASE + 8) return dim(this.ansi[n - NAMED_DIM_BASE]!);
    if (n === NAMED_BRIGHT_FG) return this.foreground;
    if (n === NAMED_DIM_FG) return dim(this.foreground);
    return isFg ? this.foreground : this.background;
  }

  /** Bright variant of ANSI 0–7 for bold text. */
  brighten(code: number): number {
    const kind = code >>> COLOR_KIND_SHIFT;
    const value = code & 0xffffff;
    if (kind === COLOR_RGB) return value;
    if (value < 8) return this.ansi[value + 8]!;
    return this.resolve(code, true);
  }
}

export function dim(rgb: number): number {
  const r = Math.round(((rgb >> 16) & 0xff) * 0.66);
  const g = Math.round(((rgb >> 8) & 0xff) * 0.66);
  const b = Math.round((rgb & 0xff) * 0.66);
  return (r << 16) | (g << 8) | b;
}

/** Alpha-blend `top` (with alpha) over opaque `bottom`. */
export function blendOver(bottom: number, top: Rgba): number {
  const a = top.a;
  const r = Math.round(((bottom >> 16) & 0xff) * (1 - a) + top.r * a);
  const g = Math.round(((bottom >> 8) & 0xff) * (1 - a) + top.g * a);
  const b = Math.round((bottom & 0xff) * (1 - a) + top.b * a);
  return (r << 16) | (g << 8) | b;
}

export function rgbToCss(rgb: number): string {
  return `rgb(${(rgb >> 16) & 0xff},${(rgb >> 8) & 0xff},${rgb & 0xff})`;
}

function buildCube(): Uint32Array {
  const out = new Uint32Array(256);
  for (let i = 0; i < 16; i++) out[i] = DEFAULT_ANSI[i]!;
  for (let i = 16; i < 232; i++) {
    const n = i - 16;
    const r = Math.floor(n / 36);
    const g = Math.floor((n % 36) / 6);
    const b = n % 6;
    const f = (v: number) => (v === 0 ? 0 : 55 + v * 40);
    out[i] = (f(r) << 16) | (f(g) << 8) | f(b);
  }
  for (let i = 232; i < 256; i++) {
    const v = 8 + (i - 232) * 10;
    out[i] = (v << 16) | (v << 8) | v;
  }
  return out;
}
