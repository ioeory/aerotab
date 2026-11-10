import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { compile } from 'svelte/compiler';
const panel = readFileSync(new URL('../components/HostSystemPanel.svelte', import.meta.url), 'utf8');
test('system details use themed responsive cards, custom fine bars and no browser default meters', () => {
  assert.doesNotThrow(() => compile(panel, { filename: 'HostSystemPanel.svelte', generate: 'server' }));
  assert.doesNotMatch(panel, /<meter|<progress/);
  assert.match(panel, /SystemMetricCard/);
  assert.match(panel, /@container[^}]*max-width:\s*3[0-9][0-9]px/);
  assert.match(panel, /--color-fg/);
  assert.match(panel, /isVirtualMount/);
});
