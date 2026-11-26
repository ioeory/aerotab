import type { OpsEvent } from './opsProject';

/** Device-local database; deliberately outside settings and the config sync bridge. */
export async function openOpsProject(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open('aerotab-operations', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('events', { keyPath: 'id' });
    request.onblocked = () => reject(new Error('Operations database upgrade blocked'));
    request.onerror = () => reject(request.error);
    request.onsuccess = () => {
      const db = request.result;
      db.onversionchange = () => db.close();
      resolve(db);
    };
  });
}
function transaction<T>(db: IDBDatabase, mode: IDBTransactionMode, run: (store: IDBObjectStore, set: (result: T) => void) => void): Promise<T> {
  return new Promise((resolve, reject) => {
    const tx = db.transaction('events', mode);
    let result: T;
    tx.oncomplete = () => resolve(result);
    tx.onabort = () => reject(tx.error ?? new Error('Operations transaction aborted'));
    try { run(tx.objectStore('events'), value => { result = value; }); }
    catch (error) { tx.abort(); reject(error); }
  });
}
export const listOpsEvents = (db: IDBDatabase) => transaction<OpsEvent[]>(db, 'readonly', (store, set) => {
  const request = store.getAll();
  request.onsuccess = () => set(request.result);
});
export const writeOpsEvent = (db: IDBDatabase, event: OpsEvent, ifAbsent = false) => transaction<void>(db, 'readwrite', store => {
  if (!ifAbsent) { store.put(event); return; }
  const request = store.getKey(event.id);
  request.onsuccess = () => { if (request.result === undefined) store.add(event); };
});
