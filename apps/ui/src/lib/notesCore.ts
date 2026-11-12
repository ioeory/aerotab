import { Marked, Renderer } from 'marked';

export interface Note { id: string; parent_id: string | null; title: string; markdown: string; tags: string[]; profile_ids: string[]; version: number; updated_at: number }
export interface NoteFolder { id: string; parent_id: string | null; name: string; version: number; updated_at: number; recovered_parent_id?: string | null }
export interface NotesSnapshot { notes: Note[]; folders: NoteFolder[]; revision: number }
export const emptySnapshot = (): NotesSnapshot => ({ notes: [], folders: [], revision: 0 });
const escape = (text: string) => text.replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);
/** No raw HTML or remote image loading; only explicit HTTP(S)/mailto links. */
const renderer = new Renderer();
renderer.html = ({ text }) => escape(text);
renderer.image = ({ text }) => escape(text);
renderer.link = function ({ href, tokens }) {
  const label = this.parser.parseInline(tokens);
  if (!/^(https?:\/\/|mailto:)/i.test(href) || /[\u0000-\u0020\u007f]/.test(href)) return label;
  try {
    const url = new URL(href);
    if (!['https:', 'http:', 'mailto:'].includes(url.protocol) || url.username || url.password) return label;
  } catch { return label; }
  return `<a href="${escape(href)}" target="_blank" rel="noopener noreferrer" referrerpolicy="no-referrer">${label}</a>`;
};
export const isMermaidLang = (lang?: string) => (lang ?? '').match(/^\S*/)![0].toLowerCase() === 'mermaid';
/** Inert code block; the preview later swaps it for a sanitized SVG and keeps it as the error fallback. */
export const mermaidPlaceholder = (code: string) => `<div class="mermaid-diagram" data-mermaid><pre><code class="language-mermaid">${escape(code)}</code></pre></div>\n`;
const defaultCode = renderer.code.bind(renderer);
renderer.code = token => isMermaidLang(token.lang) ? mermaidPlaceholder(token.text) : defaultCode(token);
const markdownParser = new Marked({ renderer, gfm: true, async: false });
export function renderMarkdown(markdown: string): string {
  return markdownParser.parse(markdown) as string;
}
export function safeFilename(value: string): string {
  let name = value.replace(/[<>:"/\\|?*\u0000-\u001f]/g, '').replace(/^\.+|[ .]+$/g, '').trim() || 'note';
  if (/^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i.test(name)) name = '_' + name;
  return name.slice(0, 120);
}
export function uniqueFilename(title: string, used: Set<string>): string {
  const base = safeFilename(title); let name = `${base}.md`; let index = 2;
  while ([...used].some(item => item.toLowerCase() === name.toLowerCase())) name = `${base} (${index++}).md`;
  used.add(name); return name;
}
/** One in-flight request per note. Failed writes never clear the latest draft. */
export class NoteSaveQueue<T> {
  private current: T | undefined;
  private generation = 0;
  private savedGeneration = 0;
  private pending: Promise<void> | null = null;
  constructor(private save: (draft: T, version: number) => Promise<number>, public version: number) {}
  get dirty(): boolean { return this.generation !== this.savedGeneration; }
  update(draft: T): void { this.current = draft; this.generation++; }
  flush(): Promise<void> {
    if (this.pending) return this.pending;
    this.pending = (async () => {
      while (this.dirty) {
        const generation = this.generation; const draft = this.current!;
        this.version = await this.save(draft, this.version);
        this.savedGeneration = generation;
      }
    })().finally(() => { this.pending = null; });
    return this.pending;
  }
}
