export interface ActionPosition {
  x: number;
  y: number;
}

export function positionFromMouseEvent(ev: MouseEvent): ActionPosition {
  return { x: Math.round(ev.clientX), y: Math.round(ev.clientY) };
}

export function positionFromRect(rect: DOMRect, anchor: 'center' | 'bottom-start' = 'center'): ActionPosition {
  if (anchor === 'bottom-start') {
    return { x: Math.round(rect.left), y: Math.round(rect.bottom + 6) };
  }
  return {
    x: Math.round(rect.left + rect.width / 2),
    y: Math.round(rect.top + rect.height / 2),
  };
}

export function focusedElementPosition(selector: string, container?: ParentNode | null): ActionPosition | undefined {
  const root = container ?? document;
  const el = root.querySelector<HTMLElement>(selector);
  const rect = el?.getBoundingClientRect();
  if (!rect || rect.width === 0 || rect.height === 0) return undefined;
  return positionFromRect(rect);
}

export function activeElementPosition(): ActionPosition | undefined {
  const active = document.activeElement as HTMLElement | null;
  const rect = active?.getBoundingClientRect();
  if (!rect || rect.width === 0 || rect.height === 0) return undefined;
  return positionFromRect(rect, 'bottom-start');
}

export function clampActionPosition(
  position: ActionPosition,
  width: number,
  height: number,
  pad = 8,
): ActionPosition {
  return {
    x: Math.min(Math.max(pad, position.x), Math.max(pad, window.innerWidth - width - pad)),
    y: Math.min(Math.max(pad, position.y), Math.max(pad, window.innerHeight - height - pad)),
  };
}
