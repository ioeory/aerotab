// Hotkey registry + binding store (M5).
//
// The registry lists every user-rebindable action; the manager parses
// stored binding strings and routes keyboard events to action handlers.
//
// Binding strings are case-insensitive segments joined by `+`. Recognised
// modifiers: `Ctrl`, `Shift`, `Alt`, `Meta`. The terminal key segment is
// either a single character (e.g. `T`, `,`) or a special key name from
// `KeyboardEvent.key` (e.g. `ArrowRight`, `Tab`, `Enter`, `F1`).

export interface ActionDef {
  id: string;
  label: string;
  category: string;
  defaultBindings: string[];
}

export const ACTIONS: ActionDef[] = [
  { id: 'new-tab',     label: 'New local tab',         category: 'Tabs',     defaultBindings: ['Ctrl+Shift+T'] },
  { id: 'open-profile', label: 'Open profile picker…',  category: 'Tabs',     defaultBindings: ['Ctrl+T'] },
  { id: 'close-pane',  label: 'Close current pane',    category: 'Tabs',     defaultBindings: ['Ctrl+W'] },
  { id: 'next-tab',    label: 'Next tab',              category: 'Tabs',     defaultBindings: ['Ctrl+Tab'] },
  { id: 'prev-tab',    label: 'Previous tab',          category: 'Tabs',     defaultBindings: ['Ctrl+Shift+Tab'] },
  { id: 'split-right', label: 'Split pane right',      category: 'Panes',    defaultBindings: ['Ctrl+Shift+D'] },
  { id: 'split-left',  label: 'Split pane left',       category: 'Panes',    defaultBindings: ['Ctrl+Shift+A'] },
  { id: 'split-down',  label: 'Split pane down',       category: 'Panes',    defaultBindings: ['Ctrl+Shift+E'] },
  { id: 'split-up',    label: 'Split pane up',         category: 'Panes',    defaultBindings: ['Ctrl+Shift+W'] },
  { id: 'maximize-pane', label: 'Maximize / restore pane', category: 'Panes', defaultBindings: ['Alt+Z'] },
  { id: 'open-sftp', label: 'Open SFTP for current SSH pane', category: 'Panes', defaultBindings: ['Ctrl+Alt+F'] },
  { id: 'toggle-sftp-dock', label: 'Collapse / expand SFTP dock', category: 'Panes', defaultBindings: ['Ctrl+Alt+E'] },
  { id: 'toggle-broadcast', label: 'Toggle broadcast input to SSH panes', category: 'Panes', defaultBindings: ['Ctrl+Shift+B'] },
  { id: 'focus-left',  label: 'Focus pane left',       category: 'Panes',    defaultBindings: ['Alt+ArrowLeft'] },
  { id: 'focus-right', label: 'Focus pane right',      category: 'Panes',    defaultBindings: ['Alt+ArrowRight'] },
  { id: 'focus-up',    label: 'Focus pane up',         category: 'Panes',    defaultBindings: ['Alt+ArrowUp'] },
  { id: 'focus-down',  label: 'Focus pane down',       category: 'Panes',    defaultBindings: ['Alt+ArrowDown'] },
  { id: 'next-pane',   label: 'Focus next pane',       category: 'Panes',    defaultBindings: ['Alt+]'] },
  { id: 'prev-pane',   label: 'Focus previous pane',   category: 'Panes',    defaultBindings: ['Alt+['] },
  { id: 'palette',     label: 'Command palette',       category: 'App',      defaultBindings: ['Ctrl+Shift+P'] },
  { id: 'settings',    label: 'Open settings',         category: 'App',      defaultBindings: ['Ctrl+,'] },
  { id: 'toggle-sidebar', label: 'Toggle sidebar',      category: 'App',      defaultBindings: ['Ctrl+Alt+S'] },
  { id: 'search',      label: 'Search in pane',        category: 'Terminal', defaultBindings: ['Ctrl+F'] },
];

interface ParsedBinding {
  ctrl: boolean;
  shift: boolean;
  alt: boolean;
  meta: boolean;
  key: string; // canonical: single-char lowercase, or KeyboardEvent.key for named keys
}

function parseBinding(s: string): ParsedBinding | null {
  const segs = s.split('+').map((p) => p.trim()).filter(Boolean);
  if (segs.length === 0) return null;
  let ctrl = false, shift = false, alt = false, meta = false;
  let key = '';
  for (const seg of segs) {
    const low = seg.toLowerCase();
    if (low === 'ctrl' || low === 'control') ctrl = true;
    else if (low === 'shift') shift = true;
    else if (low === 'alt' || low === 'option') alt = true;
    else if (low === 'meta' || low === 'cmd' || low === 'super' || low === 'win') meta = true;
    else key = seg; // last non-modifier wins
  }
  if (!key) return null;
  // Canonicalise single-char keys to lowercase; preserve named keys verbatim.
  if (key.length === 1) key = key.toLowerCase();
  return { ctrl, shift, alt, meta, key };
}

function matches(ev: KeyboardEvent, b: ParsedBinding): boolean {
  if (b.ctrl !== ev.ctrlKey) return false;
  if (b.alt !== ev.altKey) return false;
  if (b.meta !== ev.metaKey) return false;
  if (b.shift !== ev.shiftKey) return false;
  // For single-character bindings, compare case-insensitively against
  // ev.key. Otherwise compare verbatim against the named key.
  if (b.key.length === 1) {
    return ev.key.toLowerCase() === b.key;
  }
  return ev.key === b.key;
}

/** Format an event back into a binding string (used by the recorder UI). */
export function formatEvent(ev: KeyboardEvent): string | null {
  const parts: string[] = [];
  if (ev.ctrlKey) parts.push('Ctrl');
  if (ev.shiftKey) parts.push('Shift');
  if (ev.altKey) parts.push('Alt');
  if (ev.metaKey) parts.push('Meta');
  const k = ev.key;
  if (!k || k === 'Control' || k === 'Shift' || k === 'Alt' || k === 'Meta') return null;
  parts.push(k.length === 1 ? k.toUpperCase() : k);
  return parts.join('+');
}

export class HotkeyManager {
  /** action id -> list of bindings */
  private bindings = new Map<string, ParsedBinding[]>();
  /** action id -> handler */
  private handlers = new Map<string, () => void>();

  constructor() {
    this.resetToDefaults();
  }

  resetToDefaults(): void {
    this.bindings.clear();
    for (const a of ACTIONS) {
      this.bindings.set(a.id, a.defaultBindings.map(parseBinding).filter((b): b is ParsedBinding => !!b));
    }
  }

  /** Replace bindings from a `{actionId: string[]}` map. Missing entries
   *  fall back to defaults. */
  loadFromMap(m: Record<string, string[]> | null | undefined): void {
    this.resetToDefaults();
    if (!m) return;
    for (const a of ACTIONS) {
      const raw = m[a.id];
      if (Array.isArray(raw)) {
        const parsed = raw.map(parseBinding).filter((b): b is ParsedBinding => !!b);
        this.bindings.set(a.id, parsed);
      }
    }
  }

  /** Serialise bindings into a `{actionId: string[]}` map for persistence. */
  toMap(): Record<string, string[]> {
    const out: Record<string, string[]> = {};
    for (const a of ACTIONS) {
      const list = this.bindings.get(a.id) ?? [];
      out[a.id] = list.map((b) => {
        const parts: string[] = [];
        if (b.ctrl) parts.push('Ctrl');
        if (b.shift) parts.push('Shift');
        if (b.alt) parts.push('Alt');
        if (b.meta) parts.push('Meta');
        parts.push(b.key.length === 1 ? b.key.toUpperCase() : b.key);
        return parts.join('+');
      });
    }
    return out;
  }

  /** Set bindings for one action (in display form). */
  setBindings(actionId: string, strings: string[]): void {
    const parsed = strings.map(parseBinding).filter((b): b is ParsedBinding => !!b);
    this.bindings.set(actionId, parsed);
  }

  getBindings(actionId: string): string[] {
    const list = this.bindings.get(actionId) ?? [];
    return list.map((b) => {
      const parts: string[] = [];
      if (b.ctrl) parts.push('Ctrl');
      if (b.shift) parts.push('Shift');
      if (b.alt) parts.push('Alt');
      if (b.meta) parts.push('Meta');
      parts.push(b.key.length === 1 ? b.key.toUpperCase() : b.key);
      return parts.join('+');
    });
  }

  registerHandler(actionId: string, fn: () => void): void {
    this.handlers.set(actionId, fn);
  }

  /** Returns true if the event was consumed. */
  dispatch(ev: KeyboardEvent): boolean {
    for (const [actionId, list] of this.bindings) {
      for (const b of list) {
        if (matches(ev, b)) {
          const fn = this.handlers.get(actionId);
          if (fn) {
            ev.preventDefault();
            fn();
            return true;
          }
        }
      }
    }
    return false;
  }
}

export const hotkeys = new HotkeyManager();
