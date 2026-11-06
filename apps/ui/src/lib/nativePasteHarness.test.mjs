import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const source = readFileSync(new URL('../../../../src-tauri/src/bin/aerotab-app.rs', import.meta.url), 'utf8');
const minimal = source.slice(source.indexOf("if(location.search.includes('pasteMinimal=1'))"), source.indexOf("phase='dynamic revision barrier';"));
test('minimal harness isolates cancel and stale writes, paste writer, then real GTK Enter before PTY assertion', () => {
  const isolated = minimal.indexOf('minimal cancel or old generation wrote before approval');
  const writer = minimal.indexOf('minimal paste writer increment not exactly one before Enter');
  const enter = minimal.indexOf("phase:'paste submit enter'");
  const marker = minimal.indexOf('minimal approved PTY marker once');
  assert.ok(isolated >= 0 && isolated < writer && writer < enter && enter < marker);
  assert.match(minimal, /after.sessionWriteCount!==pasteWritten.sessionWriteCount\+1/);
  assert.match(minimal, /after.flow.reserved!==pasteWritten.flow.reserved\+1/);
  assert.doesNotMatch(minimal, /session\.write|await sleep\(/);
});
