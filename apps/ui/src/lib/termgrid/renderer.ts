import type { GridState } from './frame';
import type { Palette } from './palette';
import type { CellMetrics, FontSpec } from './fontMetrics';
import type { CursorState } from './cellStyle';

export interface RenderScene {
  grid: GridState;
  palette: Palette;
  metrics: CellMetrics;
  spec: FontSpec;
  cursor: CursorState | null;
  dpr: number;
  /** Draw runs as strings so font ligatures can form (Canvas 2D only). */
  ligatures: boolean;
}

export interface GridRenderer {
  readonly kind: 'webgl' | 'canvas2d';
  /** Apply new geometry; the canvas backing store is resized to cols×rows cells. */
  resize(cols: number, rows: number, metrics: CellMetrics, dpr: number): void;
  setFont(spec: FontSpec, metrics: CellMetrics): void;
  render(scene: RenderScene): void;
  dispose(): void;
}
