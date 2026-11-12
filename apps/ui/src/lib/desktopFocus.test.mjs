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

test('modalFocus awaits handoff then forces desktop webview focus', () => {
  const modal = readFileSync(new URL('./modalFocus.ts', import.meta.url), 'utf8');
  assert.match(modal, /focusWebviewDesktop/);
  assert.match(modal, /waitForNativeOsFocusHandoff/);
});
