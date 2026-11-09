export function resizeTransaction(start: number, min: number, max: number, preview: (v: number) => void, end: (v: number) => void = () => {}) {
  let value = start, done = false;
  return { move(delta: number) { if (!done) { value = Math.max(min, Math.min(max, start + delta)); preview(value); } }, commit() { if (!done) { done = true; end(value); } }, cancel() { if (!done) { done = true; preview(start); } } };
}
export function keyboardResize(key: string, value: number, min: number, max: number, axis: 'x' | 'y', shift: boolean): number | null {
  if (key === 'Home') return min; if (key === 'End') return max;
  const keys = axis === 'x' ? ['ArrowLeft', 'ArrowRight'] : ['ArrowUp', 'ArrowDown'];
  if (!keys.includes(key)) return null;
  return Math.max(min, Math.min(max, value + (key === keys[0] ? -1 : 1) * (shift ? 50 : 10)));
}
/** Shared, cancel-safe pointer transaction. Geometry is in CSS pixels, never device pixels. */
export function pointerResize(event: PointerEvent, opts: { value: number; min: number; max: number; axis: 'x' | 'y'; sign?: number; scale?: number; preview: (v: number) => void; end?: (v: number) => void }): () => void {
  if (event.button !== 0) return () => {};
  event.preventDefault(); event.stopPropagation();
  // Capture synchronously, before a layout update can move the native child HWND
  // under the pointer. Window listeners alone cannot receive that child's mouseup.
  const target = event.currentTarget as HTMLElement | null;
  try { target?.setPointerCapture(event.pointerId); }
  catch { return () => {}; }
  const transaction = resizeTransaction(opts.value, opts.min, opts.max, opts.preview, opts.end);
  const start = opts.axis === 'x' ? event.clientX : event.clientY;
  let done = false;
  const move = (e: PointerEvent) => {
    if (e.pointerId !== event.pointerId) return;
    // Recover a missed mouseup without applying any unpressed movement.
    if (e.pointerType === 'mouse' && (e.buttons & 1) === 0) { finish(true); return; }
    transaction.move(((opts.axis === 'x' ? e.clientX : e.clientY) - start) * (opts.sign ?? 1) / (opts.scale ?? 1));
  };
  const finish = (commit: boolean) => {
    if (done) return;
    done = true;
    window.removeEventListener('pointermove', move, true);
    window.removeEventListener('pointerup', up, true);
    window.removeEventListener('pointercancel', pointerCancel, true);
    window.removeEventListener('blur', cancel);
    window.removeEventListener('keydown', key, true);
    target?.removeEventListener('lostpointercapture', pointerCancel);
    try { if (target?.hasPointerCapture(event.pointerId)) target.releasePointerCapture(event.pointerId); } catch { /* Target may have been removed. */ }
    if (commit) transaction.commit(); else transaction.cancel();
  };
  const cancel = () => finish(false);
  const pointerCancel = (e: PointerEvent) => { if (e.pointerId === event.pointerId) cancel(); };
  const up = (e: PointerEvent) => { if (e.pointerId === event.pointerId) finish(true); };
  const key = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.preventDefault(); cancel(); } };
  target?.addEventListener('lostpointercapture', pointerCancel);
  window.addEventListener('pointermove', move, true);
  window.addEventListener('pointerup', up, true);
  window.addEventListener('pointercancel', pointerCancel, true);
  window.addEventListener('blur', cancel);
  window.addEventListener('keydown', key, true);
  return cancel;
}
