/** Pure Markdown text transforms for the notes textarea; the stored Markdown is never re-serialized. */
export type FormatKind = 'heading' | 'bold' | 'italic' | 'strike' | 'code' | 'quote' | 'unordered' | 'ordered' | 'task' | 'codeblock' | 'separator' | 'table' | 'link' | 'image';
export interface TextEdit { text: string; start: number; end: number }
export interface ShortcutEvent { key: string; ctrlKey?: boolean; metaKey?: boolean; shiftKey?: boolean; altKey?: boolean }

const INLINE: Partial<Record<FormatKind, string>> = { bold: '**', italic: '*', strike: '~~', code: '`' };
const LIST_MARKER = /^(\s*)(?:[-*+] \[[ xX]\] |[-*+] |\d+[.)] )/;
const LINE_KIND: Partial<Record<FormatKind, RegExp>> = {
  unordered: /^\s*[-*+] (?!\[[ xX]\] )/,
  ordered: /^\s*\d+[.)] /,
  task: /^\s*[-*+] \[[ xX]\] /,
  quote: /^\s*> ?/,
};
const URL_LIKE = /^(https?:\/\/|mailto:)\S+$/i;

function lineStartOf(text: string, index: number): number { return text.lastIndexOf('\n', index - 1) + 1; }
function lineEndOf(text: string, index: number): number { const end = text.indexOf('\n', index); return end < 0 ? text.length : end; }
function starsAround(text: string, start: number, end: number): number {
  let before = 0, after = 0;
  while (text[start - before - 1] === '*') before++;
  while (text[end + after] === '*') after++;
  return Math.min(before, after);
}

function inline(text: string, start: number, end: number, marker: string): TextEdit {
  const m = marker.length;
  const wrapped = marker === '*' ? starsAround(text, start, end) % 2 === 1
    : marker === '**' ? starsAround(text, start, end) >= 2
    : text.slice(start - m, start) === marker && text.slice(end, end + m) === marker;
  if (wrapped) return { text: text.slice(0, start - m) + text.slice(start, end) + text.slice(end + m), start: start - m, end: end - m };
  return { text: text.slice(0, start) + marker + text.slice(start, end) + marker + text.slice(end), start: start + m, end: end + m };
}

function lines(text: string, start: number, end: number, kind: FormatKind): TextEdit {
  const from = lineStartOf(text, start);
  const to = lineEndOf(text, end > start && text[end - 1] === '\n' ? end - 1 : end);
  const original = text.slice(from, to).split('\n');
  let next: string[];
  if (kind === 'heading') {
    next = original.map(line => {
      const level = /^(#{1,6}) /.exec(line)?.[1]?.length ?? 0;
      const body = level ? line.slice(level + 1) : line;
      return level >= 3 ? body : '#'.repeat(level + 1) + ' ' + body;
    });
  } else {
    const kindPattern = LINE_KIND[kind]!;
    const strip = kind === 'quote' ? kindPattern : LIST_MARKER;
    const remove = original.every(line => kindPattern.test(line));
    next = original.map((line, index) => {
      const indent = /^\s*/.exec(line)![0];
      const body = strip.test(line) ? line.replace(strip, '') : line.slice(indent.length);
      if (remove) return indent + body;
      const marker = kind === 'ordered' ? `${index + 1}. ` : kind === 'task' ? '- [ ] ' : kind === 'quote' ? '> ' : '- ';
      return indent + marker + body;
    });
  }
  const replaced = next.join('\n');
  const firstDelta = next[0]!.length - original[0]!.length;
  const startOut = Math.max(from, start + firstDelta);
  const endOut = start === end ? startOut : Math.max(startOut, end + replaced.length - (to - from));
  return { text: text.slice(0, from) + replaced + text.slice(to), start: startOut, end: endOut };
}

function block(text: string, start: number, end: number, kind: FormatKind): TextEdit {
  const before = text.slice(0, start), body = text.slice(start, end), after = text.slice(end);
  const lead = before === '' ? '' : '\n'.repeat(Math.max(0, 2 - (/\n*$/.exec(before)![0].length)));
  const leadingAfter = /^\n*/.exec(after)![0].length;
  const trail = after === '' ? '\n' : '\n'.repeat(Math.max(0, 2 - leadingAfter));
  let insert: string, selStart: number, selEnd: number;
  if (kind === 'codeblock') { insert = '```\n' + body + '\n```'; selStart = 4; selEnd = 4 + body.length; }
  else if (kind === 'table') { insert = '| Column 1 | Column 2 |\n| --- | --- |\n|  |  |'; selStart = 2; selEnd = 10; }
  else { insert = '---'; selStart = selEnd = insert.length + trail.length; }
  const offset = before.length + lead.length;
  return { text: before + lead + insert + trail + after, start: offset + selStart, end: offset + selEnd };
}

function link(text: string, start: number, end: number, image: boolean): TextEdit {
  const bang = image ? '!' : '', selected = text.slice(start, end);
  if (URL_LIKE.test(selected)) {
    const caret = start + bang.length + 1;
    return { text: text.slice(0, start) + `${bang}[](${selected})` + text.slice(end), start: caret, end: caret };
  }
  const urlStart = start + bang.length + 1 + selected.length + 2;
  return { text: text.slice(0, start) + `${bang}[${selected}](https://)` + text.slice(end), start: urlStart, end: urlStart + 'https://'.length };
}

export function applyFormat(text: string, start: number, end: number, kind: FormatKind): TextEdit {
  const marker = INLINE[kind];
  if (marker) return inline(text, start, end, marker);
  if (kind === 'link' || kind === 'image') return link(text, start, end, kind === 'image');
  if (kind === 'codeblock' || kind === 'table' || kind === 'separator') return block(text, start, end, kind);
  return lines(text, start, end, kind);
}

/** Enter at the end of a list/quote line continues it; Enter on an empty item ends the list. */
export function continueList(text: string, start: number, end: number): TextEdit | null {
  if (start !== end) return null;
  const from = lineStartOf(text, start);
  if (lineEndOf(text, start) !== start) return null;
  const match = /^(\s*)(>\s?|([-*+]) \[[ xX]\] |[-*+] |(\d+)([.)]) )(.*)$/.exec(text.slice(from, start));
  if (!match) return null;
  const [, indent, marker, taskBullet, number, delimiter, rest] = match;
  if (!rest?.trim()) return { text: text.slice(0, from) + text.slice(start), start: from, end: from };
  const nextMarker = taskBullet ? `${taskBullet} [ ] ` : number ? `${Number(number) + 1}${delimiter} ` : marker;
  const insert = '\n' + (indent ?? '') + (nextMarker ?? ''), caret = start + insert.length;
  return { text: text.slice(0, start) + insert + text.slice(start), start: caret, end: caret };
}

export function shortcutFormat(event: ShortcutEvent): FormatKind | null {
  if (!(event.ctrlKey || event.metaKey) || event.altKey) return null;
  const key = event.key.toLowerCase();
  if (event.shiftKey) return key === 'x' ? 'strike' : null;
  return ({ b: 'bold', i: 'italic', k: 'link', e: 'code' } as Record<string, FormatKind>)[key] ?? null;
}
