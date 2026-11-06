import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
export const table = JSON.parse(readFileSync(new URL('../generated/unicode-width-0.2.3.json', import.meta.url), 'utf8'));
if (createHash('sha256').update(JSON.stringify(table.ranges)).digest('hex') !== table.metadata.sha256) throw new Error('Unicode range hash mismatch');

// Registered exclusively via xterm's public (proposed) unicode API. The 5.5
// charProperties packed ABI was checked against its installed UnicodeV6:
// shouldJoin=bit0, width=bits1..2, charKind=bits3..26. We intentionally join
// ONLY width-zero scalars, just as native Term::input attaches zerowidth chars.
// No UAX29 cluster folding: native does not collapse emoji ZWJ sequences either.
export class NativeUnicodeProvider {
  version = 'native-unicode-width-0.2.3-unicode18';
  wcwidth(cp) {
    let lo = 0, hi = table.ranges.length - 1;
    while (lo <= hi) {
      const mid = (lo + hi) >>> 1;
      const [start, end, width] = table.ranges[mid];
      if (cp < start) hi = mid - 1;
      else if (cp > end) lo = mid + 1;
      else return width;
    }
    return 0;
  }
  charProperties(cp, preceding) {
    let width = this.wcwidth(cp);
    const previousWidth = (preceding >>> 1) & 3;
    const join = width === 0 && previousWidth !== 0;
    if (join) width = previousWidth;
    return (width << 1) | Number(join);
  }
}
export function installNativeUnicode(term) {
  const provider = new NativeUnicodeProvider();
  term.unicode.register(provider);
  term.unicode.activeVersion = provider.version;
}
