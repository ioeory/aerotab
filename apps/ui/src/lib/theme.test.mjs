import test from 'node:test';
import assert from 'node:assert/strict';
import { BUILTIN_THEMES, themeTokens, contrastRatio, xtermPalette, applyTheme } from './theme.ts';
import { normalizeAppearancePreferences, resolveAppearanceTheme, createAppearanceCoordinator } from './appearancePreferences.ts';

test('six stable palettes meet body AAA and help AA on every chrome surface', () => {
  assert.deepEqual(BUILTIN_THEMES.map(t => t.name).sort(), ['gruvbox-dark', 'one-light', 'solarized-dark', 'termora-dark', 'termora-light', 'tokyo-night']);
  for (const theme of BUILTIN_THEMES) {
    const tokens = themeTokens(theme);
    for (const surface of ['bg', 'panel', 'panel-2', 'surface-raised']) {
      assert.ok(contrastRatio(tokens.fg, tokens[surface]) >= 7, `${theme.name} body on ${surface}`);
      assert.ok(contrastRatio(tokens['fg-muted'], tokens[surface]) >= 4.5, `${theme.name} help on ${surface}`);
    }
    assert.ok(contrastRatio(tokens.accent, tokens['accent-fg']) >= 4.5);
    assert.match(tokens['accent-soft'], /^#[a-f0-9]{6}$/);
  }
});

test('custom accents do not mutate terminal ANSI or semantic status colours', () => {
  const prefs = normalizeAppearancePreferences({ mode: 'system', darkPalette: 'gruvbox-dark', lightPalette: 'one-light', accent: '#ffffff' });
  assert.equal(resolveAppearanceTheme(prefs, false).name, 'one-light');
  assert.equal(resolveAppearanceTheme(prefs, true).name, 'gruvbox-dark');
  for (const theme of BUILTIN_THEMES) {
    const before = xtermPalette(theme);
    const normal = themeTokens(theme);
    const custom = themeTokens(theme, prefs.accent);
    assert.ok(contrastRatio(custom.accent, custom['accent-fg']) >= 4.5);
    assert.ok(contrastRatio(custom['accent-text'], custom['panel-2']) >= 4.5);
    for (const key of ['danger', 'warning', 'success']) assert.equal(custom[key], normal[key]);
    assert.deepEqual(xtermPalette(theme), before);
  }
});

test('theme application resets accent overrides and paints UI tokens only', () => {
  const oldDocument = globalThis.document;
  const properties = new Map();
  const root = { style: { setProperty: (k, v) => properties.set(k, v) }, dataset: {} };
  globalThis.document = { documentElement: root };
  try {
    const theme = BUILTIN_THEMES[0];
    applyTheme(theme, '#ffffff');
    assert.equal(properties.get('--color-accent'), '#ffffff');
    applyTheme(theme);
    assert.equal(properties.get('--color-accent'), themeTokens(theme).accent);
    assert.equal(root.dataset.theme, theme.name);
    assert.equal(root.style.colorScheme, 'dark');
    assert.ok([...properties.keys()].every(k => k.startsWith('--color-')));
  } finally { globalThis.document = oldDocument; }
});

test('coordinator preview/cancel never saves and failed commit retains baseline', async () => {
  const painted = [];
  const c = createAppearanceCoordinator({ legacyTheme: 'tokyo-night', apply: p => painted.push(p.darkPalette) });
  c.preview({ darkPalette: 'gruvbox-dark' });
  c.cancel();
  assert.equal(painted.at(-1), 'tokyo-night');
  c.preview({ darkPalette: 'termora-dark' });
  await assert.rejects(c.commit(async () => { throw Error('offline'); }), /offline/);
  assert.equal(c.getSnapshot().applied.darkPalette, 'tokyo-night');
  await c.commit(async () => {});
  c.preview({ darkPalette: 'gruvbox-dark' });
  c.cancel();
  assert.equal(painted.at(-1), 'termora-dark');
});

test('concurrent saves deduplicate without losing a newer preview', async () => {
  const c = createAppearanceCoordinator({ apply: () => {} });
  c.preview({ darkPalette: 'gruvbox-dark' });
  let finish;
  const save = c.commit(() => new Promise(resolve => { finish = resolve; }));
  assert.equal(c.commit(() => { throw Error('duplicate'); }), save);
  c.preview({ darkPalette: 'tokyo-night' });
  finish(); await save;
  assert.equal(c.getSnapshot().applied.darkPalette, 'gruvbox-dark');
  assert.equal(c.getSnapshot().draft.darkPalette, 'tokyo-night');
  assert.equal(c.getSnapshot().previewing, true);
  c.cancel();
  assert.equal(c.getSnapshot().draft.darkPalette, 'gruvbox-dark');
});

test('system listener paints only in system mode and disposes cleanly', () => {
  let listener; let removed = false;
  const media = { matches: false, addEventListener: (_, cb) => { listener = cb; }, removeEventListener: (_, cb) => { removed = cb === listener; } };
  const painted = [];
  const c = createAppearanceCoordinator({ apply: (_, dark) => painted.push(dark), media });
  const count = painted.length;
  listener({ matches: true }); assert.equal(painted.length, count);
  c.preview({ mode: 'system' }); listener({ matches: false });
  assert.equal(painted.at(-1), false);
  c.dispose(); assert.equal(removed, true);
});
