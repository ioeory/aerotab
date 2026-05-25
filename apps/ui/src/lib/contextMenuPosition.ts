/** Clamp a context menu position so it stays inside the viewport. */
export function clampMenuToViewport(
  x: number,
  y: number,
  el: HTMLElement | null,
  pad = 8,
): { x: number; y: number } {
  if (!el) return { x, y };
  const maxX = Math.max(pad, window.innerWidth - el.offsetWidth - pad);
  const maxY = Math.max(pad, window.innerHeight - el.offsetHeight - pad);
  return {
    x: Math.min(Math.max(pad, x), maxX),
    y: Math.min(Math.max(pad, y), maxY),
  };
}
