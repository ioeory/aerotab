import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const read = path => readFileSync(new URL(path, import.meta.url), 'utf8');
test('notes use the shared semantic tokens without legacy variables or literal colors', () => {
  for (const name of ['NoteEditor', 'NotesTree', 'NotesWorkspace', 'SessionNotesPanel']) {
    const css = read(`../components/notes/${name}.svelte`).split('<style>')[1];
    assert.doesNotMatch(css, /var\(--(?:border|bg-secondary|bg-hover|fg-primary)|#[a-f0-9]{3,8}\b/i);
  }
});
test('terminal follow mode resolves saved UI preferences and listens for system appearance changes', () => {
  const source = read('../components/TerminalPane.svelte');
  assert.match(source, /resolveAppearanceTheme\(normalizeAppearancePreferences\(ui.value, th.value\)/);
  assert.match(source, /appearanceMedia\.addEventListener\('change'/);
  assert.match(source, /appearanceMedia\.removeEventListener\('change'/);
});
test('Hosts keeps global navigation untouched and exposes folder CRUD and current-directory drops', () => {
  const source = read('../components/Sidebar.svelte');
  assert.doesNotMatch(source, /<ResourceNavigation|showGroups/);
  assert.match(source, /deleteGroup\(groupPath\)/);
  assert.match(source, /handleDropToGroup\(variant === 'workspace' \? catalog.group/);
  assert.match(source, /folders=\{directoryFolders\}/);
});
