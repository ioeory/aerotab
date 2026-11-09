/** Bounded in-memory geometry only; exported by the existing diagnostics action. */
interface GeometryEvent {
  reason: 'fit' | 'font' | 'theme' | 'dpr' | 'draw' | 'capture' | 'cancel';
  backend: string;
  revision: number;
  rect: number[];
  client: number[];
  scroll: number[];
  target: number[];
  applied: number[];
  dpr: number;
  canvas: number[];
  resets: number;
  navigation?: number[];
  workbench?: number[];
}
const events: (GeometryEvent & { at: number })[] = [];
export function recordTerminalGeometry(event: GeometryEvent) {
  const pair = (value: number[], count = 2) => value.slice(0, count).map(n => Number.isFinite(n) ? n : 0);
  events.push({ at: Date.now(), reason: event.reason, backend: event.backend,
    revision: event.revision, rect: pair(event.rect, 4), client: pair(event.client), scroll: pair(event.scroll),
    target: pair(event.target), applied: pair(event.applied), dpr: event.dpr,
    canvas: pair(event.canvas), resets: event.resets,
    navigation: pair(event.navigation ?? []), workbench: pair(event.workbench ?? [], 4) });
  if (events.length > 128) events.shift();
}
export function terminalGeometrySnapshot() {
  return events.map(event => ({ ...event, rect: [...event.rect], client: [...event.client], scroll: [...event.scroll],
    target: [...event.target], applied: [...event.applied], canvas: [...event.canvas],
    navigation: [...(event.navigation ?? [])], workbench: [...(event.workbench ?? [])] }));
}
