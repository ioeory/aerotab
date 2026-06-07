/** Read a local text file (Tauri native picker or browser fallback). */

import { b64decode, tauriInvoke } from './rpc';

/** Max size for PEM/SSH private key imports (Vault + profile helpers). */
export const MAX_PRIVATE_KEY_FILE_BYTES = 512 * 1024;

const KEY_FILE_ACCEPT = '.pem,.key,.ppk,.pub,.txt,.asc';

function readViaBrowserFileInput(): Promise<string | null> {
  return new Promise((resolve, reject) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = KEY_FILE_ACCEPT;
    input.onchange = () => {
      const file = input.files?.[0];
      if (!file) {
        resolve(null);
        return;
      }
      if (file.size > MAX_PRIVATE_KEY_FILE_BYTES) {
        reject(new Error('file_too_large'));
        return;
      }
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result ?? ''));
      reader.onerror = () => reject(new Error('read_failed'));
      reader.readAsText(file);
    };
    input.click();
  });
}

async function readPathAsUtf8(path: string): Promise<string> {
  const stat = await tauriInvoke<{ size: number }>('local_stat', { path });
  if (!stat) throw new Error('stat_failed');
  if (stat.size > MAX_PRIVATE_KEY_FILE_BYTES) throw new Error('file_too_large');
  const chunk = await tauriInvoke<{ data: string }>('local_read_chunk', {
    path,
    offset: 0,
    len: stat.size,
  });
  if (!chunk) throw new Error('read_failed');
  return new TextDecoder().decode(b64decode(chunk.data));
}

function pickPrivateKeyPathBrowser(): Promise<string | null> {
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = KEY_FILE_ACCEPT;
    input.onchange = () => {
      const file = input.files?.[0];
      resolve(file?.name ?? null);
    };
    input.click();
  });
}

/** Pick a private-key file and return its absolute path, or `null` if cancelled. */
export async function pickPrivateKeyPath(): Promise<string | null> {
  const picked = await tauriInvoke<string | null>('pick_open_private_key_file');
  if (picked !== null) return picked || null;

  const paths = await tauriInvoke<string[] | null>('pick_open_files', { directory: false });
  if (paths === null) return pickPrivateKeyPathBrowser();
  return paths.length ? paths[0]! : null;
}

/** Pick a private-key file and return its UTF-8 text, or `null` if cancelled. */
export async function pickAndReadPrivateKeyFile(): Promise<string | null> {
  const picked = await tauriInvoke<string | null>('pick_open_private_key_file');
  if (picked !== null) {
    if (!picked) return null;
    return readPathAsUtf8(picked);
  }

  const paths = await tauriInvoke<string[] | null>('pick_open_files', { directory: false });
  if (paths === null) return readViaBrowserFileInput();
  if (!paths.length) return null;
  return readPathAsUtf8(paths[0]!);
}


/** Pick an image file for profile icons and return its path or object URL fallback. */
export async function pickIconFilePath(): Promise<string | null> {
  const paths = await tauriInvoke<string[] | null>('pick_open_files', { directory: false });
  if (paths !== null) return paths.length ? paths[0]! : null;
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = 'image/png,image/svg+xml,image/webp,image/jpeg,.png,.svg,.webp,.jpg,.jpeg,.ico';
    input.onchange = () => {
      const file = input.files?.[0];
      resolve(file ? URL.createObjectURL(file) : null);
    };
    input.click();
  });
}
