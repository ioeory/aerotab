import assert from 'node:assert/strict';
import test from 'node:test';
import { NativeUnicodeProvider, table } from './unicode-provider.mjs';

test('generated scalar ranges are complete gapless and use actual native Unicode 18 rules', () => {
  assert.equal(table.metadata.crate, 'unicode-width=0.2.3');
  assert.deepEqual(table.metadata.unicode, [18, 0, 0]);
  assert.equal(table.ranges[0][0], 0);
  assert.equal(table.ranges.at(-1)[1], 0x10ffff);
  for (let i = 1; i < table.ranges.length; i++) assert.equal(table.ranges[i][0], table.ranges[i - 1][1] + 1);
  const provider = new NativeUnicodeProvider();
  for (const [char, width] of [['中', 2], ['😀', 2], ['\u{1fae8}', 2], ['\u0301', 0], ['\ufe0f', 0], ['\u200d', 0]]) {
    assert.equal(provider.wcwidth(char.codePointAt(0)), width, char);
  }
  assert.equal(table.metadata.sha256.length, 64);
});
