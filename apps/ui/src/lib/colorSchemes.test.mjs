import test from 'node:test';
import assert from 'node:assert/strict';
import { COLOR_SCHEMES, toXtermTheme } from './colorSchemes.ts';
import { contrastRatio } from './theme.ts';
test('at least sixty distinct complete terminal palettes with readable text', () => {
  assert.ok(COLOR_SCHEMES.length >= 60);
  assert.equal(new Set(COLOR_SCHEMES.map(s => s.name)).size, COLOR_SCHEMES.length);
  assert.equal(new Set(COLOR_SCHEMES.map(s => JSON.stringify(s))).size, COLOR_SCHEMES.length);
  for (const s of COLOR_SCHEMES) {
    assert.equal(s.ansi.length, 16);
    for (const c of [s.background, s.foreground, s.cursor, s.selection, ...s.ansi]) assert.match(c, /^#[0-9a-f]{6}$/i);
    if (s.name.startsWith('aero-')) assert.ok(contrastRatio(s.background, s.foreground) >= 4.5, s.name);
    assert.equal(toXtermTheme(s).background, s.background);
  }
});
