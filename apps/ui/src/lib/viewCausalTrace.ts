/** Debug harness must explicitly install this sink. Release/default path is inert. */
export interface ViewTraceEvent {
  stage: string; epoch?: number; controller?: number; request?: number;
  status?: string; cancelled?: boolean;
  layout?: ReadonlyArray<LayoutSample>;
  reason?: string;
}
interface LayoutSample {
  role: string; rect: number[]; client: number[]; scroll: number[];
  display: string; position: string; overflowX: string; overflowY: string;
  fontSize: number; lineHeight: number; paddingY: number; borderY: number;
  hidden: boolean;
}
export function createViewCausalBuffer(capacity = 128) {
  const events: Readonly<ViewTraceEvent & { order: number }>[] = [];
  let order = 0;
  return Object.freeze({
    record(event: ViewTraceEvent) {
      // Copy an allowlist only; caller extras, text, errors and RPC payloads never enter the buffer.
      const stages = ['layout.sample', 'pane.report', 'store.report', 'reload.request', 'reload.apply', 'reload.applied',
        'pane.stage.begin', 'pane.stage.success', 'pane.stage.catch', 'rollback.restore-status',
        'transaction.current', 'transaction.prepare', 'transaction.prepared', 'transaction.drain', 'transaction.drained',
        'transaction.commit', 'transaction.committed', 'transaction.rollback', 'transaction.rolled-back'];
      const clean: ViewTraceEvent & { order: number } = { order: ++order, stage: stages.includes(event.stage) ? event.stage : 'unknown' };
      for (const key of ['epoch', 'controller', 'request'] as const) if (Number.isSafeInteger(event[key])) clean[key] = event[key];
      if (['current', 'disposed', 'target-error', 'controller', 'config', 'geometry', 'visibility', 'pending', 'native', 'native-surface', 'unavailable', 'xterm-canvas', 'xterm-webgl', 'xterm-dom'].includes(event.status ?? '')) clean.status = event.status;
      if (typeof event.cancelled === 'boolean') clean.cancelled = event.cancelled;
      if (['capture', 'current', 'prepared', 'resize', 'mutation', 'rollback'].includes(event.reason ?? '')) clean.reason = event.reason;
      if (event.stage === 'layout.sample' && Array.isArray(event.layout)) {
        const tokens = ['block','flex','grid','none','inline','inline-flex','absolute','relative','static','fixed','hidden','visible','auto','scroll','clip'];
        clean.layout = event.layout.slice(0, 12).filter(s => ['surface','host','pane','grid','slot','main','content','footer','canvas','chip','viewport'].includes(s.role)).map(s => ({
          role: s.role,
          rect: s.rect?.slice(0,4).map((n: number)=>Number.isFinite(n)?n:0),
          client: s.client?.slice(0,2).map((n: number)=>Number.isFinite(n)?n:0),
          scroll: s.scroll?.slice(0,2).map((n: number)=>Number.isFinite(n)?n:0),
          display: tokens.includes(s.display)?s.display:'other', position: tokens.includes(s.position)?s.position:'other',
          overflowX: tokens.includes(s.overflowX)?s.overflowX:'other', overflowY: tokens.includes(s.overflowY)?s.overflowY:'other',
          fontSize: Number.isFinite(s.fontSize)?s.fontSize:0, lineHeight: Number.isFinite(s.lineHeight)?s.lineHeight:0,
          paddingY: Number.isFinite(s.paddingY)?s.paddingY:0, borderY: Number.isFinite(s.borderY)?s.borderY:0, hidden: s.hidden === true,
        }));
      }
      events.push(Object.freeze(clean));
      if (events.length > Math.max(1, Math.min(256, capacity))) events.shift();
    },
    snapshot: () => events.map(event => ({ ...event, ...(event.layout ? { layout: event.layout.map(s=>({...s,rect:[...s.rect],client:[...s.client],scroll:[...s.scroll]})) } : {}) })),
  });
}
const identities = new WeakMap<object, number>();
let identity = 0;
export function traceController(value: object | null): number {
  if (!value) return 0;
  let id = identities.get(value);
  if (!id) { id = ++identity; identities.set(value, id); }
  return id;
}
/** Debug-only synchronous reads; no observer, timer, DOM mutation or focus change. */
export function traceLayout(reason: string, surface: HTMLElement | null) {
  if (typeof window === 'undefined' || !surface) return;
  const host = window as unknown as { __aerotabDebugDrawObserver?: { causalTrace?: boolean } };
  if (host.__aerotabDebugDrawObserver?.causalTrace !== true) return;
  try {
    const main = surface.closest('main');
    const grid = surface.closest('[data-pane-drop-pane]')?.parentElement ?? surface.parentElement?.parentElement?.parentElement;
    const nodes: [string, Element | null][] = [
      ['surface',surface],['host',surface.closest('.terminal-host')],['pane',surface.parentElement?.parentElement ?? null],
      ['grid',grid ?? null],['slot',grid?.querySelector('[data-pane-slot]') ?? null],
      ['main',main],['content',main?.firstElementChild ?? null],['footer',main?.querySelector('footer') ?? null],
      ['chip',main?.querySelector('footer button[disabled]') ?? null],['canvas',surface.querySelector('canvas')],
    ];
    const layout = nodes.filter((entry): entry is [string, HTMLElement] => entry[1] instanceof HTMLElement).map(([role,node]) => {
      const r = node.getBoundingClientRect(), s = getComputedStyle(node);
      const num = (v: string) => parseFloat(v) || 0;
      return { role, rect:[r.x,r.y,r.width,r.height],client:[node.clientWidth,node.clientHeight],scroll:[node.scrollWidth,node.scrollHeight],
        display:s.display,position:s.position,overflowX:s.overflowX,overflowY:s.overflowY,fontSize:num(s.fontSize),lineHeight:num(s.lineHeight),
        paddingY:num(s.paddingTop)+num(s.paddingBottom),borderY:num(s.borderTopWidth)+num(s.borderBottomWidth),hidden:node.hidden };
    });
    traceView({stage:'layout.sample',reason,layout});
  } catch { /* Never affect the transaction. */ }
}

export function traceView(event: ViewTraceEvent) {
  if (typeof window === 'undefined') return;
  const host = window as unknown as {
    __aerotabDebugDrawObserver?: { causalTrace?: boolean };
    __aerotabViewCausalTrace?: ReturnType<typeof createViewCausalBuffer>;
  };
  // The capability is installed only by Rust debug_assertions + E2E-env harness.
  try {
    if (!host.__aerotabViewCausalTrace && host.__aerotabDebugDrawObserver?.causalTrace === true) {
      Object.defineProperty(host, '__aerotabViewCausalTrace', { value: createViewCausalBuffer(), configurable: false, writable: false });
    }
    host.__aerotabViewCausalTrace?.record(event);
  } catch { /* diagnostics must never affect control flow */ }
}
