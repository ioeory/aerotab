import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

test('desktopFocus invokes desktop_focus_webview before modal DOM focus', async () => {
  const calls = [];
  const prev = globalThis.window;
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: async (cmd) => { calls.push(cmd); },
    },
  };
  try {
    const { focusWebviewDesktop } = await import(`./desktopFocus.ts?t=${Date.now()}`);
    await focusWebviewDesktop();
    assert.deepEqual(calls, ['desktop_focus_webview']);
  } finally {
    if (prev === undefined) delete globalThis.window;
    else globalThis.window = prev;
  }
});

test('aerotab-app registers desktop_focus_webview for native-gpu modal handoff', () => {
  const app = readFileSync(new URL('../../../../src-tauri/src/bin/aerotab-app.rs', import.meta.url), 'utf8');
  assert.match(app, /fn desktop_focus_webview/);
  assert.match(app, /desktop_focus_webview,/);
  // Must MoveFocus the Webview — WebviewWindow::set_focus only focuses the frame.
  assert.match(app, /as_ref\(\)\s*\n?\s*\.set_focus\(\)/);
  assert.doesNotMatch(app, /SetFocus\(hwnd\)/);
});

test('window reactivation restores current pane with overlay/input guards and cleanup', () => {
  const app = readFileSync(new URL('../App.svelte', import.meta.url), 'utf8');
  const backend = readFileSync(new URL('../../../../src-tauri/src/bin/aerotab-app.rs', import.meta.url), 'utf8');
  // Windows routes through the guard's deduped emitter (guard emit + tauri
  // Focused would double-fire now that Chromium raises GotFocus again);
  // blur is only forwarded when the app really lost foreground (IME
  // composition windows must not be reported as window blur). Other
  // platforms keep the direct emit. Either way the Focused handler must
  // forward the event to the frontend restore chain.
  assert.match(backend, /WindowEvent::Focused\(focused\)[\s\S]*?(?:focus_guard::emit_window_focus\(focused\)|focus_guard::emit_window_focus\(\*focused\)|window\.emit\("aerotab:window-focus", focused\))/);
  assert.match(backend, /foreground_is_main_window\(\)/);
  // The Focused handler runs nested inside Win32 message dispatch: a
  // synchronous MoveFocus there oscillates parent/child focus (vault modal
  // flicker) and can deadlock the UI thread. The retry must go through IPC.
  const handler = backend.slice(backend.indexOf('.on_window_event('), backend.indexOf('.on_menu_event('));
  assert.doesNotMatch(handler, /set_focus\(\)/, 'on_window_event must not issue synchronous MoveFocus');
  assert.match(app, /listenNativeEvent<boolean>\('aerotab:window-focus', restore\)/);
  assert.match(app, /!focused \|\| document.hidden \|\| isOverlayBlockingInput\(\)/);
  assert.match(app, /host && shouldAutoFocusTerminal\(host\)/);
  const command = backend.slice(backend.indexOf('fn desktop_focus_webview('), backend.indexOf('/// Convert a DOM rect'));
  assert.doesNotMatch(command, /window\.set_focus\(\)/, 'late request must not foreground the app');
  assert.match(command, /GetForegroundWindow/);
});

test('modalFocus awaits handoff then forces desktop webview focus', () => {
  const modal = readFileSync(new URL('./modalFocus.ts', import.meta.url), 'utf8');
  assert.match(modal, /focusWebviewDesktop/);
  assert.match(modal, /waitForNativeOsFocusHandoff/);
});
