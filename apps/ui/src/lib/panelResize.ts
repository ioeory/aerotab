import { pointerResize } from './splitter';
/** Compatibility wrappers for existing panel callers; share cancel-safe resize transactions. */
export function startHorizontalPanelResize(ev: PointerEvent, opts: { startWidthPx: number; minPx: number; maxPx: number; growWhenDraggingRight: boolean; onWidth: (width: number) => void; onEnd?: () => void }): void {
  pointerResize(ev, { axis: 'x', value: opts.startWidthPx, min: opts.minPx, max: opts.maxPx, sign: opts.growWhenDraggingRight ? 1 : -1, preview: opts.onWidth, end: opts.onEnd });
}
export function startVerticalPanelResize(ev: PointerEvent, opts: { startHeightPx: number; minPx: number; maxPx: number; onHeight: (height: number) => void; onEnd?: () => void }): void {
  pointerResize(ev, { axis: 'y', value: opts.startHeightPx, min: opts.minPx, max: opts.maxPx, sign: -1, preview: opts.onHeight, end: opts.onEnd });
}
