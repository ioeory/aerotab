import mermaid from 'mermaid';
import createDOMPurify from 'dompurify';
import { nextMermaidId, stripRemoteCss, type MermaidTheme } from './notesMermaid';

/** Private DOMPurify instance so these hooks never leak into mermaid's own sanitizer calls. */
const purify = createDOMPurify(window);
/** Elements that can navigate, fetch, embed HTML or animate attributes back to unsafe values. */
const FORBID_TAGS = ['a', 'image', 'use', 'foreignObject', 'feImage', 'script', 'iframe', 'animate', 'animateMotion', 'animateTransform', 'set', 'mpath', 'textPath'];
purify.addHook('uponSanitizeElement', (node, data) => {
  if (data.tagName === 'style' && node.textContent) node.textContent = stripRemoteCss(node.textContent);
});
purify.addHook('uponSanitizeAttribute', (_node, data) => {
  if (/^(?:xlink:)?href$|^on/i.test(data.attrName)) { data.keepAttr = false; return; }
  if (/url\s*\(|\\|@import|image-set|\bsrc\s*\(/i.test(data.attrValue)) data.attrValue = stripRemoteCss(data.attrValue);
});

let configuredTheme: MermaidTheme | undefined;
function configure(theme: MermaidTheme): void {
  if (configuredTheme === theme) return;
  mermaid.initialize({
    startOnLoad: false,
    securityLevel: 'strict',
    theme,
    htmlLabels: false,
    suppressErrorRendering: true,
    maxTextSize: 50_000,
    maxEdges: 500,
    secure: ['secure', 'securityLevel', 'startOnLoad', 'maxTextSize', 'suppressErrorRendering', 'maxEdges', 'htmlLabels', 'dompurifyConfig', 'themeCSS', 'fontFamily', 'altFontFamily', 'arrowMarkerAbsolute', 'legacyMathML', 'forceLegacyMathML'],
  });
  configuredTheme = theme;
}

/** SVG <style> applies document-wide; keep only rules scoped to this diagram (plus keyframes). */
function scopeCss(css: string, id: string): string {
  const sheet = new CSSStyleSheet();
  try { sheet.replaceSync(css); } catch { return ''; }
  const scope = `#${id}`;
  return [...sheet.cssRules]
    .filter(rule => rule instanceof CSSKeyframesRule
      || (rule instanceof CSSStyleRule && rule.selectorText.split(',').every(s => s.trim() === scope || s.trim().startsWith(`${scope} `) || s.trim().startsWith(`${scope}.`))))
    .map(rule => stripRemoteCss(rule.cssText))
    .join('\n');
}

/** Click/link directives wrap nodes in <a transform=…>; keep the geometry without the link. */
function unwrapLinks(svg: string): string {
  const doc = new DOMParser().parseFromString(svg, 'text/html');
  for (const link of doc.body.querySelectorAll('a')) {
    const group = doc.createElementNS('http://www.w3.org/2000/svg', 'g');
    for (const name of ['transform', 'class']) { const value = link.getAttribute(name); if (value !== null) group.setAttribute(name, value); }
    group.append(...link.childNodes);
    link.replaceWith(group);
  }
  return doc.body.innerHTML;
}

/** Mermaid needs a laid-out container to measure text; keep it off screen so it never flashes. */
function measureBox(): HTMLElement {
  let box = document.getElementById('aerotab-mermaid-measure');
  if (!box) {
    box = document.createElement('div');
    box.id = 'aerotab-mermaid-measure';
    box.setAttribute('aria-hidden', 'true');
    box.style.cssText = 'position:fixed;left:-10000px;top:0;width:1200px;visibility:hidden;pointer-events:none;';
    document.body.append(box);
  }
  return box;
}

export async function render(source: string, theme: MermaidTheme): Promise<Node> {
  configure(theme);
  const id = nextMermaidId();
  try {
    const { svg } = await mermaid.render(id, source, measureBox());
    const fragment = purify.sanitize(unwrapLinks(svg), {
      USE_PROFILES: { svg: true, svgFilters: true },
      FORBID_TAGS,
      RETURN_DOM_FRAGMENT: true,
    });
    for (const style of fragment.querySelectorAll('style')) style.textContent = scopeCss(style.textContent ?? '', id);
    return fragment;
  } finally {
    document.getElementById(id)?.remove();
    document.getElementById(`d${id}`)?.remove();
  }
}
