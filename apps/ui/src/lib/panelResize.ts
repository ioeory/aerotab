/** Pointer-drag horizontal resize; `growWhenDraggingRight` matches left-side panels (sidebar). */
export function startHorizontalPanelResize(
  ev: PointerEvent,
  opts: {
    startWidthPx: number;
    minPx: number;
    maxPx: number;
    growWhenDraggingRight: boolean;
    onWidth: (widthPx: number) => void;
    onEnd?: () => void;
  },
): void {
  ev.preventDefault();
  const startX = ev.clientX;
  const { startWidthPx, minPx, maxPx, growWhenDraggingRight, onWidth, onEnd } = opts;
  const onMove = (move: PointerEvent) => {
    const rawDelta = move.clientX - startX;
    const delta = growWhenDraggingRight ? rawDelta : -rawDelta;
    onWidth(Math.max(minPx, Math.min(maxPx, startWidthPx + delta)));
  };
  const onUp = () => {
    window.removeEventListener('pointermove', onMove);
    window.removeEventListener('pointerup', onUp);
    onEnd?.();
  };
  window.addEventListener('pointermove', onMove);
  window.addEventListener('pointerup', onUp);
}
