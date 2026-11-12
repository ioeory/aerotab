import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const pane = readFileSync(new URL('../components/TerminalPane.svelte', import.meta.url), 'utf8');
const view = readFileSync(new URL('../components/NativeTerminalView.svelte', import.meta.url), 'utf8');
const grid = readFileSync(new URL('../components/PaneGrid.svelte', import.meta.url), 'utf8');
const host = readFileSync(new URL('./nativeViewHost.ts', import.meta.url), 'utf8');
const controller = readFileSync(new URL('./termgrid/controller.ts', import.meta.url), 'utf8');

test('activating a pane always schedules native/webview terminal focus', () => {
  assert.match(pane, /becameActive[\s\S]*nativeView\?\.focus\(\)/);
  assert.match(pane, /aerotab:focus-pane[\s\S]*nativeView\?\.focus\(\)/);
  // Do not require `active` before matching sessionId — Svelte may flush later.
  const focusListener = pane.match(/const focusListener = \(ev: Event\) => \{[\s\S]*?\n    \};/)?.[0] ?? '';
  assert.match(focusListener, /detail\?\.sessionId && detail\.sessionId !== session\.id/);
  assert.doesNotMatch(focusListener, /if \(!active\) return;/);
});

test('native surface focus export forces hostAdapter SetFocus, not click-only', () => {
  const body = view.match(/export function focus\(opts\?: \{ force\?: boolean \}\) \{[\s\S]*?\n  \}/)?.[0] ?? '';
  assert.match(body, /hostAdapter\?\.focus/);
  assert.match(body, /setFocusAllowed/);
  assert.match(body, /tick\(\)\.then/);
  assert.match(body, /requestSurfaceFocus/);
  assert.match(body, /if \(canFocusSurface\(\)\) return apply/);
  // Controller path stays for webview; native must not rely on click-only.
  assert.match(controller, /Native focus remains OS SetFocus via hostAdapter/);
});

test('focus-pane and becameActive force native focus past the active prop race', () => {
  assert.match(pane, /nativeView\?\.focus\(\{ force: true \}\)/);
  assert.match(view, /becameActive already has active=true/);
  assert.match(controller, /Release DOM focus so keydown on a stale IME/);
});

test('inactive WebView panes keep pointer events so the first click hits the canvas', () => {
  // native-surface still uses pointer-events-none (OS HWND owns the click).
  assert.match(pane, /active \|\| nativeBackend !== 'native-surface' \? '' : 'pointer-events-none'/);
  assert.match(controller, /this\.opts\.onActivate\?\.\(\)/);
  assert.match(view, /onActivate: \(\) => onActivate\?\.\(\)/);
});

test('pane chrome and native activation both dispatch focus into the new pane', () => {
  assert.match(grid, /dispatchFocusPane\(sessionId\)/);
  assert.match(grid, /onActivate=\{\(\) => \{[\s\S]*dispatchFocusPane\(pane\.id\)/);
});

test('hostAdapter focus chains a fresh SetFocus after an in-flight attempt settles', () => {
  const body = host.match(/focus\(\): Promise<NativeFocusResult> \{[\s\S]*?\n    \},/)?.[0] ?? '';
  assert.match(body, /native_view_focus/);
  assert.match(body, /focusQueued|focusing =/);
  assert.match(body, /await (?:focusing|updating)/);
});
