/** Canvas 2D grid renderer — fallback for WebGL / translucency / ligatures. */

import { cellStyle, type CursorState } from './cellStyle';
import {
  CURSOR_BEAM,
  CURSOR_BLOCK,
  CURSOR_HOLLOW,
  CURSOR_UNDERLINE,
  type GridState,
} from './frame';
import { fontString, type CellMetrics, type FontSpec } from './fontMetrics';
import { rgbToCss, type Palette } from './palette';
import type { GridRenderer, RenderScene } from './renderer';

export class Canvas2DRenderer implements GridRenderer {
  readonly kind = 'canvas2d' as const;
  private canvas: HTMLCanvasElement;
  private ctx: CanvasRenderingContext2D;
  private cols = 0;
  private rows = 0;
  private metrics: CellMetrics = {
    width: 9,
    height: 18,
    baseline: 14,
    deviceWidth: 9,
    deviceHeight: 18,
  };
  private spec: FontSpec = {
    family: 'monospace',
    size: 13,
    weight: 400,
    weightBold: 700,
    linePadding: 0,
    dpr: 1,
  };
  private dpr = 1;

  constructor(canvas: HTMLCanvasElement) {
    this.canvas = canvas;
    const ctx = canvas.getContext('2d', { alpha: true });
    if (!ctx) throw new Error('termgrid: 2d context unavailable');
    this.ctx = ctx;
  }

  resize(cols: number, rows: number, metrics: CellMetrics, dpr: number) {
    this.cols = cols;
    this.rows = rows;
    this.metrics = metrics;
    this.dpr = Math.max(1, dpr);
    const width = Math.max(1, Math.round(cols * metrics.deviceWidth));
    const height = Math.max(1, Math.round(rows * metrics.deviceHeight));
    // Even assigning the same size erases a canvas backing store.
    if (this.canvas.width !== width) this.canvas.width = width;
    if (this.canvas.height !== height) this.canvas.height = height;
    this.canvas.style.width = `${cols * metrics.width}px`;
    this.canvas.style.height = `${rows * metrics.height}px`;
    this.ctx.setTransform(1, 0, 0, 1, 0, 0);
  }

  setFont(spec: FontSpec, metrics: CellMetrics) {
    this.spec = spec;
    this.metrics = metrics;
  }

  render(scene: RenderScene) {
    const { grid, palette, cursor, ligatures } = scene;
    const ctx = this.ctx;
    const cw = this.metrics.deviceWidth;
    const ch = this.metrics.deviceHeight;
    const cssCw = this.metrics.width;
    const cssCh = this.metrics.height;
    const dpr = this.dpr;

    // Clear / fill background.
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    if (palette.backgroundAlpha < 1) {
      ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);
      if (palette.backgroundAlpha > 0) {
        ctx.fillStyle = `rgba(${(palette.background >> 16) & 0xff},${(palette.background >> 8) & 0xff},${palette.background & 0xff},${palette.backgroundAlpha})`;
        ctx.fillRect(0, 0, this.canvas.width, this.canvas.height);
      }
    } else {
      ctx.fillStyle = rgbToCss(palette.background);
      ctx.fillRect(0, 0, this.canvas.width, this.canvas.height);
    }

    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.textBaseline = 'alphabetic';
    ctx.textAlign = 'left';

    for (let y = 0; y < grid.rows; y++) {
      if (ligatures) {
        this.drawLigatureRow(grid, y, palette, cursor, cssCw, cssCh);
      } else {
        this.drawCellRow(grid, y, palette, cursor, cssCw, cssCh);
      }
    }

    this.drawNonBlockCursor(cursor, cssCw, cssCh, palette);
    grid.clearDirty();
  }

  private drawCellRow(
    grid: GridState,
    y: number,
    palette: Palette,
    cursor: CursorState | null,
    cssCw: number,
    cssCh: number,
  ) {
    const ctx = this.ctx;
    const base = y * grid.cols;
    for (let x = 0; x < grid.cols; x++) {
      const idx = base + x;
      const style = cellStyle(grid, idx, x, y, palette, cursor);
      if (style.paintBg) {
        ctx.fillStyle = rgbToCss(style.bg);
        ctx.fillRect(x * cssCw, y * cssCh, cssCw * (style.wide ? 2 : 1), cssCh);
      }
      if (!style.drawGlyph) continue;
      const cp = grid.chars[idx] ?? 0x20;
      ctx.fillStyle = rgbToCss(style.fg);
      ctx.font = fontString(this.spec, style.bold, style.italic);
      ctx.fillText(String.fromCodePoint(cp), x * cssCw, y * cssCh + this.metrics.baseline);
      this.drawDecorations(style, x * cssCw, y * cssCh, cssCw * (style.wide ? 2 : 1), cssCh);
    }
  }

  private drawLigatureRow(
    grid: GridState,
    y: number,
    palette: Palette,
    cursor: CursorState | null,
    cssCw: number,
    cssCh: number,
  ) {
    const ctx = this.ctx;
    const base = y * grid.cols;
    let runStart = 0;
    let runText = '';
    let runFg = 0;
    let runBold = false;
    let runItalic = false;

    const flush = (end: number) => {
      if (!runText) return;
      ctx.fillStyle = rgbToCss(runFg);
      ctx.font = fontString(this.spec, runBold, runItalic);
      ctx.fillText(runText, runStart * cssCw, y * cssCh + this.metrics.baseline);
      runText = '';
      void end;
    };

    for (let x = 0; x < grid.cols; x++) {
      const idx = base + x;
      const style = cellStyle(grid, idx, x, y, palette, cursor);
      if (style.paintBg) {
        flush(x);
        ctx.fillStyle = rgbToCss(style.bg);
        ctx.fillRect(x * cssCw, y * cssCh, cssCw * (style.wide ? 2 : 1), cssCh);
      }
      if (style.spacer) continue;
      if (!style.drawGlyph) {
        flush(x);
        continue;
      }
      const cp = grid.chars[idx] ?? 0x20;
      if (
        runText
        && (style.fg !== runFg || style.bold !== runBold || style.italic !== runItalic)
      ) {
        flush(x);
      }
      if (!runText) {
        runStart = x;
        runFg = style.fg;
        runBold = style.bold;
        runItalic = style.italic;
      }
      runText += String.fromCodePoint(cp);
      if (style.underline || style.strike || style.doubleUnderline) {
        flush(x + 1);
        this.drawDecorations(style, x * cssCw, y * cssCh, cssCw * (style.wide ? 2 : 1), cssCh);
      }
    }
    flush(grid.cols);
  }

  private drawDecorations(
    style: ReturnType<typeof cellStyle>,
    x: number,
    y: number,
    w: number,
    h: number,
  ) {
    const ctx = this.ctx;
    ctx.strokeStyle = rgbToCss(style.fg);
    ctx.lineWidth = 1;
    if (style.underline || style.doubleUnderline) {
      const uy = y + h - 1.5;
      ctx.beginPath();
      ctx.moveTo(x, uy);
      ctx.lineTo(x + w, uy);
      ctx.stroke();
      if (style.doubleUnderline) {
        ctx.beginPath();
        ctx.moveTo(x, uy - 2);
        ctx.lineTo(x + w, uy - 2);
        ctx.stroke();
      }
    }
    if (style.strike) {
      const sy = y + h / 2;
      ctx.beginPath();
      ctx.moveTo(x, sy);
      ctx.lineTo(x + w, sy);
      ctx.stroke();
    }
  }

  private drawNonBlockCursor(
    cursor: CursorState | null,
    cssCw: number,
    cssCh: number,
    palette: Palette,
  ) {
    if (!cursor || !cursor.visible || cursor.shape === CURSOR_BLOCK) return;
    if (cursor.col < 0 || cursor.line < 0) return;
    const ctx = this.ctx;
    const x = cursor.col * cssCw;
    const y = cursor.line * cssCh;
    ctx.fillStyle = rgbToCss(palette.cursor);
    ctx.strokeStyle = rgbToCss(palette.cursor);
    switch (cursor.shape) {
      case CURSOR_UNDERLINE:
        ctx.fillRect(x, y + cssCh - 2, cssCw, 2);
        break;
      case CURSOR_BEAM:
        ctx.fillRect(x, y, 2, cssCh);
        break;
      case CURSOR_HOLLOW:
        ctx.lineWidth = 1;
        ctx.strokeRect(x + 0.5, y + 0.5, cssCw - 1, cssCh - 1);
        break;
      default:
        break;
    }
  }

  dispose() {
    // canvas owned by caller
  }
}
