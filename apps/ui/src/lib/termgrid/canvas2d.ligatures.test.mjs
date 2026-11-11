import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';

const server = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true, include: [] }, server: { middlewareMode: true, hmr: false } });
const { Canvas2DRenderer } = await server.ssrLoadModule('/src/lib/termgrid/canvas2d.ts');
const { GridState, CF_BOLD, CF_WIDE_CHAR, CF_WIDE_CHAR_SPACER } = await server.ssrLoadModule('/src/lib/termgrid/frame.ts');
const { Palette } = await server.ssrLoadModule('/src/lib/termgrid/palette.ts');
await server.close();

// Simulate the browser's natural 9px advance against a device-snapped cell.
// Record the actual transformed text bounds, not implementation call syntax.
function render(dpr, wide = false) {
  let sx = 1, tx = 0;
  const stack = [], draws = [];
  const ctx = {
    setTransform(a, b, c, d, e) { sx = a; tx = e; },
    save() { stack.push([sx, tx]); },
    restore() { [sx, tx] = stack.pop(); },
    translate(x) { tx += sx * x; },
    scale(x) { sx *= x; },
    fillRect() {},
    measureText(text) { return { width: [...text].reduce((w, cp) => w + (cp === '中' ? 18 : 9), 0) }; },
    fillText(text, x) { draws.push({ text, left: tx + x * sx, right: tx + (x + this.measureText(text).width) * sx }); },
  };
  const canvas = { width: 0, height: 0, style: {}, getContext: () => ctx };
  const renderer = new Canvas2DRenderer(canvas);
  const cw = Math.round(9 * dpr) / dpr;
  const metrics = { width: cw, height: 20, baseline: 15, deviceWidth: Math.round(9 * dpr), deviceHeight: 20 * dpr };
  renderer.setFont({ family: 'monospace', size: 15, weight: 400, weightBold: 700, linePadding: 0, dpr }, metrics);
  const grid = new GridState();
  grid.resize(120, 1);
  grid.fg.fill(256);
  const prompt = 'root@unbuildserver:/home/repositories/jenkins/workspace/wm-teaching-workload_DEV/security-reports/';
  for (let x = 0; x < prompt.length; x++) grid.chars[x] = prompt.codePointAt(x);
  let boundary = prompt.length;
  if (wide) {
    grid.chars[boundary] = '中'.codePointAt(0);
    grid.flags[boundary] = CF_WIDE_CHAR;
    grid.flags[boundary + 1] = CF_WIDE_CHAR_SPACER;
    boundary += 2;
  }
  for (let x = 0; x < 5; x++) { grid.chars[boundary + x] = 'aetls'.codePointAt(x); grid.flags[boundary + x] = CF_BOLD; }
  renderer.resize(grid.cols, grid.rows, metrics, dpr);
  renderer.render({ grid, palette: new Palette(), cursor: null, ligatures: true });
  return { draws, boundary: boundary * cw * dpr, end: (boundary + 5) * cw * dpr };
}

for (const dpr of [1, 1.25, 1.5]) {
  for (const wide of [false, true]) {
    test(`ligature runs align with cells across prompt/style boundary: DPR ${dpr}, wide ${wide}`, () => {
      const { draws, boundary, end } = render(dpr, wide);
      assert.equal(draws.length, 2);
      assert.ok(Math.abs(draws[0].right - boundary) < 1e-8, `prompt ends at ${draws[0].right}, grid boundary ${boundary}`);
      assert.ok(Math.abs(draws[1].left - boundary) < 1e-8);
      assert.ok(Math.abs(draws[1].right - end) < 1e-8);
    });
  }
}
