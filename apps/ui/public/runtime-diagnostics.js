/* Pre-module, opt-in content-free probe. One overwritten local snapshot; no RPC/network/input values. */
(() => {
  'use strict';
  // Opt in by launching the WebView URL with ?runtimeDiagnostics=1.
  // Default startup remains unchanged, including timers and capture handlers.
  if (window.__aerotabRuntimeDiagnostics) return;
  if (window.__AEROTAB_RUNTIME_DIAGNOSTICS__ !== true && !/(?:^|[?&])runtimeDiagnostics=1(?:&|$)/.test(location.search)) return;
  const events = [];
  const counts = { click: 0, contextmenu: 0, keydown: 0 };
  let phase = 'boot', beats = 0, last = performance.now(), maxLagMs = 0;
  const known = ['effect_update_depth_exceeded', 'state_unsafe_mutation', 'hydration_mismatch'];
  function record(kind, data) {
    events.push({ at: Date.now(), kind, ...data });
    if (events.length > 64) events.shift();
    persist();
  }
  function persist() {
    const value = snapshot();
    try { localStorage.setItem('aerotab.runtimeDiagnostics.v1', JSON.stringify(value)); } catch { /* Optional local-only storage. */ }
    if (window.__AEROTAB_RUNTIME_DIAGNOSTICS__ === true) {
      try { window.__TAURI_INTERNALS__?.invoke('runtime_diagnostic_snapshot', { snapshot: value }).catch(() => {}); } catch { /* Bootstrap may precede IPC. */ }
    }
  }
  function failure(kind, text, event) {
    // Never retain arbitrary exception messages/stacks: they may contain content.
    const value = String(text ?? '');
    const code = known.find(code => value.includes(code))
      ?? (/before initialization|uninitialized/i.test(value) ? 'initialization' : 'other');
    record(kind, { code, line: Number(event?.lineno) || 0, column: Number(event?.colno) || 0 });
  }
  addEventListener('error', event => failure('error', event.message, event), true);
  addEventListener('unhandledrejection', event => failure('rejection', event.reason?.message, event), true);
  for (const type of Object.keys(counts)) {
    addEventListener(type, event => {
      counts[type]++;
      if (counts[type] <= 10) record(type, { tag: /^[A-Z]{1,20}$/.test(event.target?.tagName ?? '') ? event.target.tagName : 'OTHER', prevented: !!event.defaultPrevented });
    }, true);
  }
  const timer = setInterval(() => {
    const now = performance.now();
    maxLagMs = Math.max(maxLagMs, Math.max(0, now - last - 2000));
    last = now; beats++; persist();
  }, 2000);
  function snapshot() {
    return { schema: 1, phase, beats, maxLagMs: Math.round(maxLagMs), hidden: !!document.hidden,
      modalOverlay: document.body?.dataset.modalOverlay === 'true',
      nativeOccluded: document.body?.dataset.nativeViewOccluded === 'true',
      dialogs: document.querySelectorAll('dialog[open]').length,
      tabLists: document.querySelectorAll('[data-aerotab-tab-list]').length,
      inert: document.querySelectorAll('[inert]').length,
      blockers: document.querySelectorAll('[data-native-view-blocker], [data-aerotab-modal], [data-aerotab-menu-open]').length,
      counts: { ...counts }, events: events.map(event => ({ ...event })) };
  }
  // Independent export chord, installed before App's capture hotkey handler.
  // Does not inspect ordinary keys or forward anything to a terminal.
  addEventListener('keydown', event => {
    if (!event.ctrlKey || !event.altKey || event.key !== 'F12') return;
    event.preventDefault(); event.stopImmediatePropagation();
    const url = URL.createObjectURL(new Blob([JSON.stringify(snapshot(), null, 2)], { type: 'application/json' }));
    const link = document.createElement('a');
    link.href = url; link.download = 'aerotab-runtime-diagnostics.json';
    link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
  }, true);
  window.__aerotabRuntimeDiagnostics = Object.freeze({ snapshot,
    mark(value) { if (['mounting', 'mounted', 'mount-failed'].includes(value)) { phase = value; record('phase', { value }); } },
    stop() { clearInterval(timer); },
  });
})();
