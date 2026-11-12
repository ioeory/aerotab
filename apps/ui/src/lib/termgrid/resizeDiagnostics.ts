/** Bounded in-memory metadata only; never record terminal content or credentials. */
export interface TerminalPipelineState {
  receivedFrames: number;
  paintedFrames: number;
  lastFrameAt: number;
  lastPaintAt: number;
  awaitingFullBase: boolean;
  captureInFlight: boolean;
  paintPending: boolean;
  disposed: boolean;
  backend: string;
}
const pipelines = new Set<() => TerminalPipelineState>();
export function registerTerminalPipeline(read: () => TerminalPipelineState): () => void {
  pipelines.add(read);
  return () => { pipelines.delete(read); };
}
export function terminalPipelineSnapshot(): TerminalPipelineState[] {
  return [...pipelines].slice(-128).map(read => {
    const state = read();
    // Explicit allowlist: do not spread objects that may gain sensitive fields.
    return { receivedFrames: state.receivedFrames, paintedFrames: state.paintedFrames,
      lastFrameAt: state.lastFrameAt, lastPaintAt: state.lastPaintAt,
      awaitingFullBase: state.awaitingFullBase, captureInFlight: state.captureInFlight,
      paintPending: state.paintPending, disposed: state.disposed, backend: state.backend };
  });
}
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
