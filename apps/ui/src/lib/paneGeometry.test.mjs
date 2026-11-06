import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import ts from 'typescript';

const text = await readFile(new URL('../components/PaneGrid.svelte', import.meta.url), 'utf8');
const sync = text.slice(text.indexOf('  function syncPaneBoxes('), text.indexOf('  function scheduleSync('));
const js = ts.transpileModule(sync + '\nsyncPaneBoxes(); return paneBox;', {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None },
}).outputText;

for (const zoom of [0.6, 0.8, 1, 1.25, 1.4]) {
  test(`pane geometry uses local CSS pixels at UI zoom ${zoom}`, () => {
    const grid = {
      offsetWidth: 1000, offsetHeight: 600,
      getBoundingClientRect: () => ({ left: 100, top: 50, width: 1000 * zoom, height: 600 * zoom }),
      querySelector: () => ({ getBoundingClientRect: () => ({
        left: 100 + 400 * zoom, top: 50 + 30 * zoom, width: 600 * zoom, height: 570 * zoom,
      }) }),
    };
    const run = new Function('gridEl', 'tab', 'paneBox', 'CSS', 'getComputedStyle', js);
    const boxes = run(grid, { panes: [{ id: 's' }] }, {}, { escape: v => v }, () => ({ display: 'block' }));
    for (const [key, expected] of Object.entries({ left: 400, top: 30, width: 600, height: 570 })) {
      assert.ok(Math.abs(boxes.s[key] - expected) < 0.001, `${key}: ${boxes.s[key]} != ${expected}`);
    }
  });
}
