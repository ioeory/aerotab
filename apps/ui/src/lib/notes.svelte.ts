import type { RpcClient } from './rpc';
import { emptySnapshot, NoteSaveQueue, type Note, type NoteFolder, type NotesSnapshot } from './notesCore';
export interface NoteDraft { value: Note; queue: NoteSaveQueue<Note>; status: 'saved' | 'dirty' | 'saving' | 'error'; timer?: ReturnType<typeof setTimeout> }
export class NotesStore {
  snapshot = $state<NotesSnapshot>(emptySnapshot());
  drafts = $state<Record<string, NoteDraft>>({});
  profileRemarks = $state<Record<string, { value: string; base: string; dirty: boolean }>>({});
  private loading: Promise<void> | null = null;
  private mutation = 0;
  constructor(public rpc: RpcClient) {}
  get dirty(): boolean { return Object.values(this.drafts).some(d => d.status !== 'saved') || Object.values(this.profileRemarks).some(d => d.dirty); }
  refresh(): Promise<void> {
    if (this.loading) return this.loading;
    this.loading = (async () => {
      let generation: number;
      do {
        generation = this.mutation;
        const value = await this.rpc.call<NotesSnapshot>('notes.list');
        if (generation === this.mutation) this.snapshot = value;
      } while (generation !== this.mutation);
    })().finally(() => { this.loading = null; });
    return this.loading;
  }
  draft(note: Note): NoteDraft {
    if (!this.drafts[note.id]) {
      const value = JSON.parse(JSON.stringify(note)) as Note;
      const queue = new NoteSaveQueue<Note>(async (draft, version) => {
        const saved = await this.rpc.call<Note>('notes.put', { note: draft, expectedVersion: version });
        this.mutation++;
        this.snapshot.notes = this.snapshot.notes.map(n => n.id === saved.id ? saved : n);
        const current = this.drafts[note.id];
        if (current) current.value.version = saved.version;
        return saved.version;
      }, note.version);
      this.drafts[note.id] = { value, queue, status: 'saved' };
    }
    const draft = this.drafts[note.id]!;
    if (!draft.queue.dirty && draft.value.version !== note.version) { draft.value = JSON.parse(JSON.stringify(note)); draft.queue.version = note.version; }
    return draft;
  }
  edit(id: string): void {
    const draft = this.drafts[id]; if (!draft) return; draft.queue.update(JSON.parse(JSON.stringify(draft.value))); draft.status = 'dirty';
    clearTimeout(draft.timer); draft.timer = setTimeout(() => { void this.save(id).catch(() => {}); }, 650);
  }
  async save(id: string): Promise<void> {
    const draft = this.drafts[id]; if (!draft) return;
    clearTimeout(draft.timer); draft.status = 'saving';
    try { await draft.queue.flush(); draft.status = draft.queue.dirty ? 'dirty' : 'saved'; } catch { draft.status = 'error'; throw Error('Note save failed; draft retained. Retry or export the draft before closing.'); }
  }
  async flush(): Promise<void> { await Promise.all(Object.keys(this.drafts).map(id => this.save(id))); }
  /** Parent Tauri onCloseRequested must await this and preventDefault on rejection. */
  async prepareClose(): Promise<void> {
    await this.flush();
    if (Object.values(this.profileRemarks).some(d => d.dirty)) throw Error('Unsaved host remark; save it before closing.');
  }
  async create(parent_id: string | null, title: string, folder = false): Promise<string> {
    const id = crypto.randomUUID();
    if (folder) await this.rpc.call('notes.putFolder', { folder: { id, parent_id, name: title }, expectedVersion: 0 });
    else await this.rpc.call('notes.put', { note: { id, parent_id, title, markdown: '', tags: [], profile_ids: [] }, expectedVersion: 0 });
    this.mutation++; await this.refresh(); return id;
  }
  async putFolder(folder: NoteFolder): Promise<void> { await this.rpc.call('notes.putFolder', { folder, expectedVersion: folder.version }); this.mutation++; await this.refresh(); }
  async move(note: Note, parent_id: string | null): Promise<void> {
    const draft = this.draft(note); draft.value.parent_id = parent_id; this.edit(note.id); await this.save(note.id); await this.refresh();
  }
  async remove(id: string, version: number, confirm: (impact: { notes: { id: string; title: string }[]; folders: string[]; revision: number }) => Promise<boolean>): Promise<void> {
    await this.flush();
    const impact = await this.rpc.call<{ notes: { id: string; title: string }[]; folders: string[]; revision: number }>('notes.deleteImpact', { id });
    if (!await confirm(impact)) return;
    // Save may advance the selected note's version.
    const current = this.snapshot.notes.find(n => n.id === id)?.version ?? version;
    await this.rpc.call('notes.delete', { id, expectedVersion: current, expectedRevision: impact.revision, confirmCascade: true });
    this.mutation++;
    for (const note of impact.notes) { clearTimeout(this.drafts[note.id]?.timer); delete this.drafts[note.id]; }
    await this.refresh();
  }
}
const stores = new WeakMap<RpcClient, NotesStore>();
export function getNotesStore(rpc: RpcClient): NotesStore {
  let store = stores.get(rpc);
  if (!store) {
    store = new NotesStore(rpc); stores.set(rpc, store);
    if (typeof window !== 'undefined') window.addEventListener('beforeunload', event => { if (store!.dirty) { event.preventDefault(); event.returnValue = ''; } });
  }
  return store;
}
