import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const load = async (file, replace = s => s) => {
  const source = replace(readFileSync(new URL(file, import.meta.url), 'utf8'));
  const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
  return import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);
};
const api = await load('./notesMermaid.ts');
const core = await load('./notesCore.ts', s => s.replace("from 'marked'", `from '${import.meta.resolve('marked')}'`));

test('only fenced blocks whose info string starts with mermaid become diagram placeholders', () => {
  assert.equal(core.isMermaidLang('mermaid'), true);
  assert.equal(core.isMermaidLang('Mermaid title="x"'), true);
  assert.equal(core.isMermaidLang('mermaidjs'), false);
  assert.equal(core.isMermaidLang('js'), false);
  assert.equal(core.isMermaidLang(undefined), false);
  const html = core.renderMarkdown('```mermaid\ngraph TD; A-->B\n```\n\n~~~mermaid\npie\n~~~\n\n```js\nmermaid\n```\n\n    mermaid indented\n\n`mermaid`');
  assert.equal(html.match(/data-mermaid/g)?.length, 2);
  assert.ok(html.includes('<code class="language-js">'));
  assert.ok(!core.renderMarkdown('no diagrams here').includes('data-mermaid'));
});

test('placeholder keeps the escaped original source as an inert code block', () => {
  const html = core.mermaidPlaceholder('graph TD\nA["<img src=x onerror=alert(1)>"] --> B & C\n');
  assert.ok(html.startsWith('<div class="mermaid-diagram" data-mermaid>'));
  assert.ok(html.includes('<pre><code class="language-mermaid">'));
  assert.ok(!/<img\b/.test(html)); assert.ok(html.includes('&lt;img src=x onerror=alert(1)&gt;'));
  assert.ok(html.includes('B &amp; C'));
  const rendered = core.renderMarkdown('```mermaid\n<script>alert(1)</script>\n```');
  assert.ok(!rendered.includes('<script>')); assert.ok(rendered.includes('&lt;script&gt;'));
});

test('diagram ids are unique and CSS-selector safe', () => {
  const ids = new Set(Array.from({ length: 50 }, () => api.nextMermaidId()));
  assert.equal(ids.size, 50);
  for (const id of ids) assert.match(id, /^aerotab-mermaid-[a-z0-9]+-\d+$/);
});

test('error detail is a short single line and tolerates odd throwables', () => {
  assert.equal(api.mermaidErrorDetail(new Error('Parse error on line 2:\n...A-->\n----^')), 'Parse error on line 2:');
  assert.equal(api.mermaidErrorDetail('boom'), 'boom');
  assert.equal(api.mermaidErrorDetail({ str: 'Lexical error' }), 'Lexical error');
  assert.equal(api.mermaidErrorDetail(null), '');
  assert.equal(api.mermaidErrorDetail(new Error('x'.repeat(500))).length, 160);
});

test('theme follows the app light/dark mode, dark by default', () => {
  assert.equal(api.mermaidTheme('light'), 'default');
  assert.equal(api.mermaidTheme('dark'), 'dark');
  assert.equal(api.mermaidTheme(undefined), 'dark');
});

test('CSS sanitizer drops every remote fetch vector but keeps local url(#ref)', () => {
  const css = api.stripRemoteCss('@import url(https://x/a.css); #a .n{fill:url(#grad);stroke:url( "https://t/x.png" )} @font-face{font-family:x;src:url(//t/f.woff)} .b{background:image-set("https://t/a.png" 1x)} .c{fill:u\\72l(https://t)} .d{color:red}');
  assert.ok(!/https?:|\/\/t/.test(css), css);
  assert.ok(!/@import|@font-face|image-set|\\/i.test(css), css);
  assert.ok(css.includes('url(#grad)')); assert.ok(css.includes('.d{color:red}'));
  assert.equal(api.stripRemoteCss('fill:#fff;stroke:#000'), 'fill:#fff;stroke:#000');
});

const block = (source, connected = true) => {
  const b = { source, shown: null, failed: null, connected: () => connected, show: v => { b.shown = v; }, fail: d => { b.failed = d; } };
  return b;
};

test('hydrate skips loading mermaid entirely when there are no diagrams', async () => {
  let loads = 0;
  await api.hydrateMermaid([], { theme: 'dark', isStale: () => false, load: async () => { loads++; return { render: async () => 'svg' }; } });
  assert.equal(loads, 0);
});

test('hydrate renders each diagram, isolating parse errors to their own block', async () => {
  const ok = block('graph TD; A-->B'), bad = block('graph ???'), later = block('pie');
  const themes = [];
  await api.hydrateMermaid([ok, bad, later], { theme: 'default', isStale: () => false, load: async () => ({
    render: async (src, theme) => { themes.push(theme); if (src.includes('???')) throw new Error('Parse error\nmore'); return `svg:${src}`; },
  }) });
  assert.equal(ok.shown, 'svg:graph TD; A-->B'); assert.equal(bad.shown, null);
  assert.equal(bad.failed, 'Parse error'); assert.equal(later.shown, 'svg:pie');
  assert.deepEqual(themes, ['default', 'default', 'default']);
});

test('hydrate never inserts stale results after re-render or unmount', async () => {
  let stale = false; let release;
  const first = block('a'), second = block('b');
  const pending = api.hydrateMermaid([first, second], { theme: 'dark', isStale: () => stale, load: async () => ({
    render: async src => { if (src === 'a') await new Promise(r => release = r); return src; },
  }) });
  await new Promise(r => setTimeout(r, 0)); stale = true; release(); await pending;
  assert.equal(first.shown, null); assert.equal(second.shown, null); assert.equal(first.failed, null);
  const detached = block('c', false);
  await api.hydrateMermaid([detached], { theme: 'dark', isStale: () => false, load: async () => ({ render: async s => s }) });
  assert.equal(detached.shown, null);
});

test('a failed chunk load marks every diagram as failed but keeps the source', async () => {
  const a = block('a'), b = block('b');
  await api.hydrateMermaid([a, b], { theme: 'dark', isStale: () => false, load: async () => { throw new Error('chunk 404'); } });
  assert.equal(a.failed, 'chunk 404'); assert.equal(b.failed, 'chunk 404');
});

test('sources that would fetch during layout are detected, including escaped forms', () => {
  for (const src of [
    'flowchart LR\n  X@{ img: "https://t/i.png", label: "x" }',
    'flowchart LR\n  X@{ "img" : "//t/i.png" }',
    'graph TD\n  style A fill:url(https://t/a.png)',
    'graph TD\n  classDef c fill:U\\72 L(https://t)',
    'graph TD\n  classDef c fill:u\\rl(https://t)',
    '%%{init: {"themeCSS": "@import \'https://t/x.css\';"}}%%\ngraph TD; A-->B',
    '%%{init: {"themeVariables": {"primaryColor": "image-set(\'https://t\' 1x)"}}}%%\npie',
    'graph TD\n  style A fill:url&#40;https://t)',
    'graph TD\n  style A fill:url#40;https://t)',
  ]) assert.equal(api.referencesExternalResource(src), true, src);
  for (const src of ['graph TD; A-->B', 'graph TD\n  A["image label: ok"] --> B\n  style A fill:#f9f,stroke:#333', 'sequenceDiagram\n  Alice->>Bob: see src/lib\n  Bob-->>Alice: C:\\temp'])
    assert.equal(api.referencesExternalResource(src), false, src);
});

test('blocked diagrams fail with the blocked message and never reach mermaid', async () => {
  const bad = block('graph TD\n  X@{ img: "https://t/i.png" }'), good = block('graph TD; A-->B'); const rendered = [];
  await api.hydrateMermaid([bad, good], { theme: 'dark', blockedMessage: 'external blocked', isStale: () => false, load: async () => ({ render: async s => { rendered.push(s); return s; } }) });
  assert.equal(bad.failed, 'external blocked'); assert.deepEqual(rendered, ['graph TD; A-->B']);
  let loads = 0; const only = block('style A fill:url(https://t)');
  await api.hydrateMermaid([only], { theme: 'dark', blockedMessage: 'x', isStale: () => false, load: async () => { loads++; return { render: async s => s }; } });
  assert.equal(loads, 0); assert.equal(only.failed, 'x');
});
