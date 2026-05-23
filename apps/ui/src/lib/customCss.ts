// Custom CSS injection helper for the Appearance settings.
//
// A single <style id="user-custom-css"> tag is maintained on document.head.
// `applyCustomCss` replaces its content; `applyLigatures` writes a separate
// rule that enables OpenType ligatures on xterm rows when ligatures are on.

const CUSTOM_ID = 'user-custom-css';
const LIG_ID = 'xterm-ligatures';

function ensureStyle(id: string): HTMLStyleElement {
  let el = document.getElementById(id) as HTMLStyleElement | null;
  if (!el) {
    el = document.createElement('style');
    el.id = id;
    document.head.appendChild(el);
  }
  return el;
}

export function applyCustomCss(css: string): void {
  const el = ensureStyle(CUSTOM_ID);
  el.textContent = css ?? '';
}

export function applyLigatures(enabled: boolean): void {
  const el = ensureStyle(LIG_ID);
  el.textContent = enabled
    ? '.xterm .xterm-rows { font-feature-settings: "liga" 1, "calt" 1; font-variant-ligatures: contextual; }'
    : '.xterm .xterm-rows { font-feature-settings: "liga" 0, "calt" 0; font-variant-ligatures: none; }';
}
