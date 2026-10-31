/** Shared per-cell colour / decoration resolution for both renderers. */

import {
  CF_BOLD,
  CF_DIM,
  CF_HIDDEN,
  CF_INVERSE,
  CF_ITALIC,
  CF_SELECTED,
  CF_WIDE_CHAR,
  CF_WIDE_CHAR_SPACER,
  CF_LEADING_WIDE_CHAR_SPACER,
  CF_ANY_UNDERLINE,
  CF_DOUBLE_UNDERLINE,
  CF_STRIKEOUT,
  CURSOR_BLOCK,
  CURSOR_HIDDEN,
  type GridState,
} from './frame';
import { blendOver, dim, type Palette, COLOR_KIND_SHIFT, COLOR_RGB } from './palette';

export interface CursorState {
  col: number;
  line: number;
  shape: number;
  /** False while the blink timer hides the cursor. */
  visible: boolean;
  focused: boolean;
}

export interface CellStyle {
  fg: number;
  bg: number;
  /** Background differs from the terminal default and must be painted. */
  paintBg: boolean;
  bold: boolean;
  italic: boolean;
  wide: boolean;
  /** True for the trailing half of a wide char (never draw a glyph). */
  spacer: boolean;
  drawGlyph: boolean;
  underline: boolean;
  doubleUnderline: boolean;
  strike: boolean;
  /** Cell is under a block cursor (glyph uses cursorAccent). */
  blockCursor: boolean;
}

const scratch: CellStyle = {
  fg: 0,
  bg: 0,
  paintBg: false,
  bold: false,
  italic: false,
  wide: false,
  spacer: false,
  drawGlyph: false,
  underline: false,
  doubleUnderline: false,
  strike: false,
  blockCursor: false,
};

/**
 * Resolve the style of cell `idx`. Returns a shared scratch object — copy
 * fields out before the next call.
 */
export function cellStyle(
  grid: GridState,
  idx: number,
  col: number,
  row: number,
  palette: Palette,
  cursor: CursorState | null,
): CellStyle {
  const flags = grid.flags[idx] ?? 0;
  const cp = grid.chars[idx] ?? 0x20;
  const fgCode = grid.fg[idx] ?? 0;
  const bgCode = grid.bg[idx] ?? 0;
  const bold = (flags & CF_BOLD) !== 0;
  let fg = bold && palette.boldIsBright ? palette.brighten(fgCode) : palette.resolve(fgCode, true);
  let bg = palette.resolve(bgCode, false);
  if (flags & CF_DIM) fg = dim(fg);
  if (flags & CF_INVERSE) {
    const t = fg;
    fg = bg;
    bg = t;
  }
  let paintBg = (flags & CF_INVERSE) !== 0 || bgCode !== 0x101 || bg !== palette.background;
  if (flags & CF_SELECTED) {
    bg = blendOver(bg, palette.selection);
    paintBg = true;
  }
  const isCursorCell =
    cursor !== null
    && cursor.visible
    && cursor.shape !== CURSOR_HIDDEN
    && cursor.line === row
    && (cursor.col === col || (cursor.col === col - 1 && (flags & CF_WIDE_CHAR_SPACER) !== 0));
  let blockCursor = false;
  if (isCursorCell && cursor && cursor.shape === CURSOR_BLOCK && cursor.focused) {
    blockCursor = true;
    bg = palette.cursor;
    fg = palette.cursorAccent;
    paintBg = true;
  }
  const spacer = (flags & (CF_WIDE_CHAR_SPACER | CF_LEADING_WIDE_CHAR_SPACER)) !== 0;
  scratch.fg = fg;
  scratch.bg = bg;
  scratch.paintBg = paintBg;
  scratch.bold = bold;
  scratch.italic = (flags & CF_ITALIC) !== 0;
  scratch.wide = (flags & CF_WIDE_CHAR) !== 0;
  scratch.spacer = spacer;
  scratch.drawGlyph = !spacer && (flags & CF_HIDDEN) === 0 && cp > 0x20 && cp !== 0x7f && cp !== 0xa0;
  scratch.underline = (flags & CF_ANY_UNDERLINE) !== 0;
  scratch.doubleUnderline = (flags & CF_DOUBLE_UNDERLINE) !== 0;
  scratch.strike = (flags & CF_STRIKEOUT) !== 0;
  scratch.blockCursor = blockCursor;
  return scratch;
}

/** True when `code` encodes an explicit RGB (truecolor) value. */
export function isRgbCode(code: number): boolean {
  return code >>> COLOR_KIND_SHIFT === COLOR_RGB;
}
