import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';

function tabActivation() {
  const source = readFileSync(new URL('../components/TabBar.svelte', import.meta.url), 'utf8');
  const body = source.match(/function activateTab\(tabId: string\) \{[\s\S]*?\n  \}/)?.[0];
  assert.ok(body, 'exercise the actual TabBar activation handler');
  const callbacks = [];
  const focused = [];
  let blocking = false;
  let activeId;
  const context = {
    tabs: { tabs: [{ id: 'tab', activePaneId: 'pane' }], activate(id) { activeId = id; } },
    requestAnimationFrame(callback) { callbacks.push(callback); },
    dispatchFocusPane(id) { focused.push(id); },
    isOverlayBlockingInput: () => blocking,
  };
  const js = ts.transpileModule(`${body}\nthis.activate = activateTab;`, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  runInNewContext(js, context);
  return { activate: context.activate, callbacks, focused, block() { blocking = true; }, active: () => activeId };
}

test('tab activation must not reclaim focus after a menu opens before the next frame', () => {
  const env = tabActivation();
  env.activate('tab');
  assert.equal(env.active(), 'tab');
  env.block(); // The menu action focuses its first item during the DOM update.
  env.callbacks.shift()();
  assert.deepEqual(env.focused, []);
});

test('tab activation still focuses its terminal when no overlay opens', () => {
  const env = tabActivation();
  env.activate('tab');
  env.callbacks.shift()();
  assert.deepEqual(env.focused, ['pane']);
});

async function setup() {
  const { workspaceMenu } = await import('./workspaceMenu.ts');
  const listeners = new Map();
  const doc = { activeElement: null };
  const item = (disabled = false) => ({
    disabled, hidden: false, tabIndex: 0,
    focus() { doc.activeElement = this; },
    setAttribute(name, value) { this[name] = value; },
  });
  const previous = item();
  previous.focus();
  const items = [item(), item(true), item()];
  const node = {
    ownerDocument: doc,
    classList: { add() {} },
    querySelectorAll: () => items,
    contains: (el) => items.includes(el),
    addEventListener: (name, cb) => listeners.set(name, cb),
    removeEventListener: (name) => listeners.delete(name),
  };
  let closed = 0;
  const action = workspaceMenu(node, () => { closed++; });
  const key = (key) => {
    const event = { key, prevented: false, stopped: false, preventDefault() { this.prevented = true; }, stopPropagation() { this.stopped = true; } };
    listeners.get('keydown')?.(event);
    return event;
  };
  return { action, items, doc, key, previous, closed: () => closed };
}

test('menus focus the first enabled action and navigate with wrapping, Home and End', async () => {
  const env = await setup();
  assert.equal(env.doc.activeElement, env.items[0]);
  assert.equal(env.items[0].role, 'menuitem');
  env.key('ArrowDown');
  assert.equal(env.doc.activeElement, env.items[2]);
  env.key('ArrowDown');
  assert.equal(env.doc.activeElement, env.items[0]);
  env.key('ArrowUp');
  assert.equal(env.doc.activeElement, env.items[2]);
  env.key('Home');
  assert.equal(env.doc.activeElement, env.items[0]);
  env.key('End');
  assert.equal(env.doc.activeElement, env.items[2]);
});

test('Escape closes without leaking keys and restores the actual opening focus', async () => {
  const env = await setup();
  const ev = env.key('Escape');
  assert.equal(ev.prevented, true);
  assert.equal(ev.stopped, true);
  assert.equal(env.closed(), 1);
  env.action.destroy();
  assert.equal(env.doc.activeElement, env.previous);
});

test('outside focus is preserved and all menu keys remain isolated from terminal shortcuts', async () => {
  const env = await setup();
  assert.equal(env.key('a').stopped, true);
  const outside = {};
  env.doc.activeElement = outside;
  env.action.destroy();
  assert.equal(env.doc.activeElement, outside);
});

test('Tab dismisses the menu without trapping keyboard users', async () => {
  const env = await setup();
  const ev = env.key('Tab');
  assert.equal(ev.prevented, false);
  assert.equal(env.closed(), 1);
});
