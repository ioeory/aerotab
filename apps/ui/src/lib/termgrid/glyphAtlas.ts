/**
 * Glyph atlas: rasterises characters with Canvas 2D into a single texture
 * page (white glyphs on transparent, grayscale AA) and hands out UV rects.
 * Keyed by (codepoint, bold, italic, wide). When the page fills up it is
 * cleared and callers get `generation` bumped so they re-upload.
 */

import { fontString, type CellMetrics, type FontSpec } from './fontMetrics';

export interface GlyphRect {
  /** Device px in the atlas. */
  x: number;
  y: number;
  w: number;
  h: number;
  /** Horizontal offset to apply when drawing (device px; negative for overhang). */
  dx: number;
}

const ATLAS_SIZE = 2048;
/** Extra device px around each glyph so antialiasing/overhang is not clipped. */
const PAD = 2;

type Canvas2D = HTMLCanvasElement | OffscreenCanvas;

function makeCanvas(w: number, h: number): Canvas2D {
  if (typeof OffscreenCanvas !== 'undefined') return new OffscreenCanvas(w, h);
  const c = document.createElement('canvas');
  c.width = w;
  c.height = h;
  return c;
}

export class GlyphAtlas {
  readonly canvas: Canvas2D;
  private ctx: OffscreenCanvasRenderingContext2D | CanvasRenderingContext2D;
  private map = new Map<number, GlyphRect>();
  private shelfX = PAD;
  private shelfY = PAD;
  private shelfH = 0;
  /** Bumped whenever the page is cleared or the font changes. */
  generation = 0;
  /** True when pixels changed since the last `markUploaded()`. */
  dirty = false;
  private spec: FontSpec;
  private metrics: CellMetrics;

  constructor(spec: FontSpec, metrics: CellMetrics) {
    this.canvas = makeCanvas(ATLAS_SIZE, ATLAS_SIZE);
    const ctx = this.canvas.getContext('2d', { willReadFrequently: false });
    if (!ctx) throw new Error('termgrid: 2d context unavailable for glyph atlas');
    this.ctx = ctx as OffscreenCanvasRenderingContext2D | CanvasRenderingContext2D;
    this.spec = spec;
    this.metrics = metrics;
  }

  get size(): number {
    return ATLAS_SIZE;
  }

  setFont(spec: FontSpec, metrics: CellMetrics) {
    this.spec = spec;
    this.metrics = metrics;
    this.clear();
  }

  clear() {
    this.ctx.clearRect(0, 0, ATLAS_SIZE, ATLAS_SIZE);
    this.map.clear();
    this.shelfX = PAD;
    this.shelfY = PAD;
    this.shelfH = 0;
    this.generation++;
    this.dirty = true;
  }

  markUploaded() {
    this.dirty = false;
  }

  private key(cp: number, bold: boolean, italic: boolean, wide: boolean): number {
    // 21-bit codepoint + 3 style bits.
    return (cp << 3) | (bold ? 1 : 0) | (italic ? 2 : 0) | (wide ? 4 : 0);
  }

  /** Returns null for blank glyphs (space / control). */
  get(cp: number, bold: boolean, italic: boolean, wide: boolean): GlyphRect | null {
    if (cp === 0x20 || cp < 0x20 || cp === 0x7f || cp === 0xa0) return null;
    const k = this.key(cp, bold, italic, wide);
    const hit = this.map.get(k);
    if (hit) return hit;
    const rect = this.rasterise(cp, bold, italic, wide);
    if (rect) this.map.set(k, rect);
    return rect;
  }

  private rasterise(cp: number, bold: boolean, italic: boolean, wide: boolean): GlyphRect | null {
    const dpr = Math.max(1, this.spec.dpr || 1);
    const cellW = this.metrics.deviceWidth * (wide ? 2 : 1);
    const cellH = this.metrics.deviceHeight;
    const w = cellW + PAD * 2;
    const h = cellH + PAD * 2;
    if (this.shelfX + w > ATLAS_SIZE) {
      this.shelfX = PAD;
      this.shelfY += this.shelfH + PAD;
      this.shelfH = 0;
    }
    if (this.shelfY + h > ATLAS_SIZE) {
      // Page full: start over. Caller sees `generation` change and redraws.
      this.clear();
      if (h > ATLAS_SIZE) return null;
    }
    const x = this.shelfX;
    const y = this.shelfY;
    this.shelfX += w + PAD;
    this.shelfH = Math.max(this.shelfH, h);

    const ctx = this.ctx;
    ctx.save();
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.font = fontString(this.spec, bold, italic);
    ctx.textBaseline = 'alphabetic';
    ctx.textAlign = 'left';
    ctx.fillStyle = '#ffffff';
    const str = String.fromCodePoint(cp);
    // Draw into the padded slot; position in CSS px (transform handles dpr).
    ctx.fillText(str, (x + PAD) / dpr, (y + PAD) / dpr + this.metrics.baseline);
    ctx.restore();
    this.dirty = true;
    return { x, y, w, h, dx: -PAD };
  }
}
