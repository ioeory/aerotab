import { tauriInvoke } from './rpc';
import { safeFilename, uniqueFilename, type Note, type NotesSnapshot } from './notesCore';
import type { NotesStore } from './notes.svelte';
import { nm } from './notesMessages';
const MAX = 2 * 1024 * 1024;
function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const pending = tauriInvoke<T>(command, args);
  if (!pending) return Promise.reject(Error('Markdown file operations require the desktop app.'));
  return pending;
}
function encode(text: string): string { const bytes = new TextEncoder().encode(text); let result = ''; for (let i = 0; i < bytes.length; i += 8192) result += String.fromCharCode(...bytes.subarray(i, i + 8192)); return btoa(result); }
async function readMarkdown(path: string): Promise<string> {
  if (!/\.md$/i.test(path)) throw Error(nm('importLimit'));
  const stat = await invoke<{ size: number; kind: string }>('local_stat', { path });
  if (stat.kind !== 'file' || stat.size > MAX) throw Error(nm('importLimit'));
  const bytes = new Uint8Array(stat.size); let offset = 0;
  while (offset < stat.size) {
    const result = await invoke<{ data: string }>('local_read_chunk', { path, offset, len: Math.min(65536, stat.size - offset) });
    const chunk = Uint8Array.from(atob(result.data), c => c.charCodeAt(0));
    if (!chunk.length || chunk.length > stat.size - offset) throw Error(nm('importLimit'));
    bytes.set(chunk, offset); offset += chunk.length;
  }
  return new TextDecoder('utf-8', { fatal: true }).decode(bytes);
}
export async function importNotes(store: NotesStore, parent_id: string | null): Promise<void> {
  const paths = await invoke<string[] | null>('pick_open_files', { directory: false });
  if (!paths) return;
  for (const path of paths) {
    const markdown = await readMarkdown(path); const title = path.split(/[\\/]/).pop()!.replace(/\.md$/i, '') || nm('newNote');
    await store.rpc.call('notes.put', { note: { id: crypto.randomUUID(), parent_id, title, markdown, tags: [], profile_ids: [] }, expectedVersion: 0 });
    await store.refresh();
  }
}
export async function exportNote(note: Note): Promise<void> {
  const path = await invoke<string | null>('pick_save_file', { defaultName: `${safeFilename(note.title)}.md` });
  if (path) await invoke('local_write_chunk', { path, offset: 0, data: encode(note.markdown), create: true });
}
/** Uses relative-path Tauri helpers. Exports into a fresh, non-colliding child directory. */
export async function exportFolder(snapshot: NotesSnapshot, folderId: string | null): Promise<void> {
  const baseDir = await invoke<string | null>('pick_directory'); if (!baseDir) return;
  const existing = await invoke<string[]>('local_read_dir', { path: baseDir });
  const title = folderId ? snapshot.folders.find(f => f.id === folderId)!.name : nm('title');
  const rootBase = safeFilename(title); let root = rootBase; let counter = 2;
  while (existing.some(name => name.toLowerCase() === root.toLowerCase())) root = `${rootBase} (${counter++})`;
  const walk = async (parent: string | null, relative: string[]): Promise<void> => {
    await invoke('local_mkdir_relative', { baseDir, relative });
    const used = new Set<string>();
    for (const folder of snapshot.folders.filter(f => f.parent_id === parent)) {
      let name = safeFilename(folder.name); let suffix = 2;
      while ([...used].some(n => n.toLowerCase() === name.toLowerCase())) name = `${safeFilename(folder.name)} (${suffix++})`;
      used.add(name); await walk(folder.id, [...relative, name]);
    }
    for (const note of snapshot.notes.filter(n => n.parent_id === parent)) {
      const name = uniqueFilename(note.title, used);
      await invoke('local_write_relative_chunk', { baseDir, relative: [...relative, name], offset: 0, data: encode(note.markdown), create: true });
    }
  };
  await walk(folderId, [root]);
}
