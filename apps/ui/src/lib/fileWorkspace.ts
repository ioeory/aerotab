export type FileSide = 'left' | 'right';
export interface FileTab { id: string; endpointId: string; path: string }
export interface FileLayout { version: 1; left: FileTab[]; right: FileTab[]; activeLeft: string; activeRight: string }
export class FileWorkspace {
  left: FileTab[] = []; right: FileTab[] = []; activeLeft = ''; activeRight = '';
  constructor(layout?: FileLayout) {
    if (layout?.version === 1) {
      for (const side of ['left', 'right'] as const) {
        this[side] = (layout[side] ?? []).filter(t => typeof t.id === 'string' && typeof t.endpointId === 'string' && typeof t.path === 'string').map(t => ({ id: t.id, endpointId: t.endpointId, path: t.path }));
      }
      this.activeLeft = layout.activeLeft; this.activeRight = layout.activeRight;
    }
  }
  add(side: FileSide, endpointId = '__local__', path = ''): FileTab {
    const tab = { id: `file-${Date.now()}-${Math.random().toString(36).slice(2)}`, endpointId, path };
    this[side].push(tab); this.activate(side, tab.id); return tab;
  }
  activate(side: FileSide, id: string) { if (this[side].some(t => t.id === id)) { if (side === 'left') this.activeLeft = id; else this.activeRight = id; } }
  active(side: FileSide) { return this[side].find(t => t.id === (side === 'left' ? this.activeLeft : this.activeRight)) ?? this[side][0]; }
  close(side: FileSide, id: string) { this[side] = this[side].filter(t => t.id !== id); if (!this.active(side) || !this[side].some(t => t.id === (side === 'left' ? this.activeLeft : this.activeRight))) { if (side === 'left') this.activeLeft = this[side][0]?.id ?? ''; else this.activeRight = this[side][0]?.id ?? ''; } }
  serialize(): FileLayout { return { version: 1, left: this.left.map(({ id, endpointId, path }) => ({ id, endpointId, path })), right: this.right.map(({ id, endpointId, path }) => ({ id, endpointId, path })), activeLeft: this.activeLeft, activeRight: this.activeRight }; }
}
