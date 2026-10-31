/** Cell geometry derived from a CSS font. */

export interface FontSpec {
  family: string;
  /** CSS px. */
  size: number;
  weight: number;
  weightBold: number;
  /** Extra vertical padding per cell (CSS px, applied top and bottom). */
  linePadding: number;
  /** Device pixel ratio the metrics were measured at. */
  dpr: number;
}

export interface CellMetrics {
  /** CSS px. */
  width: number;
  height: number;
  /** Baseline offset from the cell top (CSS px). */
  baseline: number;
  /** Device px (rounded). */
  deviceWidth: number;
  deviceHeight: number;
}

export function fontString(spec: FontSpec, bold: boolean, italic: boolean, sizeOverride?: number): string {
  const weight = bold ? spec.weightBold : spec.weight;
  const style = italic ? 'italic ' : '';
  return `${style}${weight} ${sizeOverride ?? spec.size}px ${spec.family}`;
}

let scratch: CanvasRenderingContext2D | null = null;

function scratchCtx(): CanvasRenderingContext2D | null {
  if (scratch) return scratch;
  if (typeof document === 'undefined') return null;
  const c = document.createElement('canvas');
  c.width = 64;
  c.height = 64;
  scratch = c.getContext('2d');
  return scratch;
}

/**
 * Measure the monospace cell. Width comes from the advance of `W`; height
 * from font bounding box ascent/descent when available (falls back to a
 * 1.2 line-height heuristic). Values are snapped so that `cols × width`
 * maps to whole device pixels and glyphs stay crisp.
 */
export function measureCell(spec: FontSpec): CellMetrics {
  const ctx = scratchCtx();
  const dpr = Math.max(1, spec.dpr || 1);
  let advance = spec.size * 0.6;
  let ascent = spec.size * 0.9;
  let descent = spec.size * 0.3;
  if (ctx) {
    ctx.font = fontString(spec, false, false);
    const m = ctx.measureText('W');
    if (m.width > 0) advance = m.width;
    const fa = (m as TextMetrics & { fontBoundingBoxAscent?: number }).fontBoundingBoxAscent;
    const fd = (m as TextMetrics & { fontBoundingBoxDescent?: number }).fontBoundingBoxDescent;
    if (typeof fa === 'number' && typeof fd === 'number' && fa + fd > 0) {
      ascent = fa;
      descent = fd;
    } else {
      // actualBoundingBox of a tall/deep sample as a fallback.
      const s = ctx.measureText('Mgjq|');
      if (s.actualBoundingBoxAscent > 0) ascent = s.actualBoundingBoxAscent + 1;
      if (s.actualBoundingBoxDescent > 0) descent = s.actualBoundingBoxDescent + 1;
    }
  }
  const deviceWidth = Math.max(1, Math.round(advance * dpr));
  const rawHeight = ascent + descent + spec.linePadding * 2;
  const deviceHeight = Math.max(1, Math.round(rawHeight * dpr));
  const width = deviceWidth / dpr;
  const height = deviceHeight / dpr;
  const baseline = spec.linePadding + ascent + (height - rawHeight) / 2;
  return { width, height, baseline, deviceWidth, deviceHeight };
}
