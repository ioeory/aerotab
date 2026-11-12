export type MermaidTheme = 'default' | 'dark';
export interface MermaidRenderer { render(source: string, theme: MermaidTheme): Promise<Node> }
export interface MermaidBlock { source: string; connected(): boolean; show(svg: Node): void; fail(detail: string): void }
export interface HydrateOptions { theme: MermaidTheme; isStale(): boolean; blockedMessage?: string; load?: () => Promise<MermaidRenderer> }

export const mermaidTheme = (mode: string | undefined): MermaidTheme => mode === 'light' ? 'default' : 'dark';

const idPrefix = `aerotab-mermaid-${Date.now().toString(36)}`;
let idSeq = 0;
export const nextMermaidId = () => `${idPrefix}-${++idSeq}`;

export function mermaidErrorDetail(error: unknown): string {
  const raw = error instanceof Error ? error.message
    : typeof error === 'string' ? error
    : typeof (error as { str?: unknown } | null)?.str === 'string' ? (error as { str: string }).str
    : '';
  return (raw.split('\n')[0] ?? '').trim().slice(0, 160);
}

/** Removes every CSS construct that could fetch a resource; only fragment references like url(#id) survive. */
export function stripRemoteCss(css: string): string {
  return css
    .replace(/[^;{}]*\\[^;{}]*/g, '')
    .replace(/@import[^;]*;?/gi, '')
    .replace(/@font-face\s*\{[^}]*\}/gi, '')
    .replace(/(?:-webkit-)?image-set\s*\([^)]*\)/gi, 'none')
    .replace(/\bsrc\s*\([^)]*\)/gi, 'none')
    .replace(/url\s*\(\s*(?!['"]?\s*#)[^)]*\)/gi, 'none');
}

/**
 * Mermaid lays diagrams out inside the live document, so image shapes and CSS from
 * style/classDef/directives would fetch before the SVG can be sanitized. Such sources are refused.
 */
export function referencesExternalResource(source: string): boolean {
  const decoded = source
    .replace(/&#x([0-9a-f]+);?/gi, (_, hex: string) => String.fromCodePoint(parseInt(hex, 16) % 0x110000))
    .replace(/&?#(\d+);/g, (_, dec: string) => String.fromCodePoint(Number(dec) % 0x110000))
    .replace(/\\([0-9a-f]{1,6})\s?/gi, (_, hex: string) => String.fromCodePoint(parseInt(hex, 16) % 0x110000))
    .replace(/\\(.)/g, '$1');
  return /(?:url|image-set|src)\s*\(|@import|\bimg\s*["']?\s*:/i.test(decoded);
}

/** Renders diagrams one by one; results are dropped once the preview re-renders or unmounts. */
export async function hydrateMermaid(blocks: MermaidBlock[], { theme, isStale, blockedMessage = '', load = () => import('./notesMermaidRender') }: HydrateOptions): Promise<void> {
  if (!blocks.length || isStale()) return;
  const renderable = blocks.filter(block => {
    if (!referencesExternalResource(block.source)) return true;
    block.fail(blockedMessage);
    return false;
  });
  if (!renderable.length) return;
  let renderer: MermaidRenderer;
  try { renderer = await load(); } catch (error) {
    if (!isStale()) for (const block of renderable) if (block.connected()) block.fail(mermaidErrorDetail(error));
    return;
  }
  for (const block of renderable) {
    if (isStale()) return;
    if (!block.connected()) continue;
    try {
      const svg = await renderer.render(block.source, theme);
      if (!isStale() && block.connected()) block.show(svg);
    } catch (error) {
      if (!isStale() && block.connected()) block.fail(mermaidErrorDetail(error));
    }
  }
}

/** DOM adapter for placeholders produced by `mermaidPlaceholder` in notesCore. */
export function mermaidBlocks(root: ParentNode, errorLabel: string): MermaidBlock[] {
  return [...root.querySelectorAll<HTMLElement>('div[data-mermaid]')].map(el => ({
    source: el.querySelector('code')?.textContent ?? '',
    connected: () => el.isConnected,
    show(svg) {
      const figure = document.createElement('div');
      figure.className = 'mermaid-svg';
      figure.append(svg);
      el.dataset.mermaidState = 'ok';
      el.replaceChildren(figure);
    },
    fail(detail) {
      const note = document.createElement('p');
      note.className = 'mermaid-error';
      note.textContent = detail ? `${errorLabel}: ${detail}` : errorLabel;
      el.dataset.mermaidState = 'error';
      el.querySelector('.mermaid-error')?.remove();
      el.append(note);
    },
  }));
}
