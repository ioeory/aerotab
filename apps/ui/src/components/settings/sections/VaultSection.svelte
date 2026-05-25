<script lang="ts">
  // Vault — M10. Master-password protected secret store. Talks to the
  // `vault.*` IPC family in src-tauri/src/commands.rs which is backed by
  // src-tauri/src/vault.rs (ChaCha20-Poly1305 + Argon2 envelope).
  import { onMount } from 'svelte';
  import { KeyRound, Lock, Unlock, Plus, Trash2, Eye, EyeOff, Pencil } from '@lucide/svelte';
  import type { RpcClient } from '../../../lib/rpc';
  import { i18n } from '../../../lib/i18n.svelte';
  import { appConfirm } from '../../../lib/confirm.svelte';

  interface Props { rpc: RpcClient; onError: (msg: string) => void }
  let { rpc, onError }: Props = $props();

  type EntryKind = 'password' | 'note' | 'token' | 'key';

  interface EntryMeta {
    id: string;
    label: string;
    kind: EntryKind;
    username?: string | null;
  }
  interface Entry extends EntryMeta { secret: string }

  interface Status {
    configured: boolean;
    initialized: boolean;
    unlocked: boolean;
  }

  let status = $state<Status>({ configured: false, initialized: false, unlocked: false });
  let busy = $state(false);
  let entries = $state<EntryMeta[]>([]);

  // unlock / init form
  let password = $state('');
  let confirmPassword = $state('');

  // entry editor
  let editing = $state<Entry | null>(null);
  let revealedId = $state<string | null>(null);
  let revealedSecret = $state('');

  // change password
  let showChangePw = $state(false);
  let oldPw = $state('');
  let newPw = $state('');
  let newPw2 = $state('');

  async function refreshStatus() {
    try {
      status = await rpc.call<Status>('vault.status', {});
      if (status.unlocked) await refreshList();
      else entries = [];
    } catch (e) { onError(`vault.status: ${(e as Error).message}`); }
  }

  async function refreshList() {
    try {
      entries = await rpc.call<EntryMeta[]>('vault.list', {});
    } catch (e) { onError(`vault.list: ${(e as Error).message}`); }
  }

  async function initialize() {
    if (!password) { onError('Password required'); return; }
    if (password !== confirmPassword) { onError('Passwords do not match'); return; }
    busy = true;
    try {
      await rpc.call('vault.initialize', { password });
      await rpc.call('vault.unlock', { password });
      password = ''; confirmPassword = '';
      await refreshStatus();
    } catch (e) { onError(`vault.initialize: ${(e as Error).message}`); }
    finally { busy = false; }
  }

  async function unlock() {
    if (!password) return;
    busy = true;
    try {
      await rpc.call('vault.unlock', { password });
      password = '';
      await refreshStatus();
    } catch (e) { onError(`vault.unlock: ${(e as Error).message}`); }
    finally { busy = false; }
  }

  async function lock() {
    try {
      await rpc.call('vault.lock', {});
      revealedId = null; revealedSecret = ''; editing = null;
      await refreshStatus();
    } catch (e) { onError(`vault.lock: ${(e as Error).message}`); }
  }

  function newEntry() {
    editing = { id: '', label: '', kind: 'password', username: '', secret: '' };
    revealedId = null; revealedSecret = '';
  }

  async function editEntry(meta: EntryMeta) {
    try {
      const full = await rpc.call<Entry>('vault.get', { id: meta.id });
      editing = { ...full, username: full.username ?? '' };
    } catch (e) { onError(`vault.get: ${(e as Error).message}`); }
  }

  async function saveEntry() {
    if (!editing) return;
    if (!editing.label) { onError('Label required'); return; }
    busy = true;
    try {
      const payload = {
        id: editing.id,
        label: editing.label,
        kind: editing.kind,
        username: editing.username || null,
        secret: editing.secret,
      };
      await rpc.call('vault.put', payload);
      editing = null;
      await refreshList();
    } catch (e) { onError(`vault.put: ${(e as Error).message}`); }
    finally { busy = false; }
  }

  async function deleteEntry(id: string) {
    if (!(await appConfirm(i18n.t('vault.deleteEntryConfirm'), { danger: true, confirmLabel: i18n.t('common.delete') }))) return;
    try {
      await rpc.call('vault.remove', { id });
      if (revealedId === id) { revealedId = null; revealedSecret = ''; }
      await refreshList();
    } catch (e) { onError(`vault.remove: ${(e as Error).message}`); }
  }

  async function reveal(meta: EntryMeta) {
    if (revealedId === meta.id) { revealedId = null; revealedSecret = ''; return; }
    try {
      const full = await rpc.call<Entry>('vault.get', { id: meta.id });
      revealedId = meta.id;
      revealedSecret = full.secret;
    } catch (e) { onError(`vault.get: ${(e as Error).message}`); }
  }

  async function copySecret(meta: EntryMeta) {
    try {
      const full = await rpc.call<Entry>('vault.get', { id: meta.id });
      await navigator.clipboard.writeText(full.secret);
    } catch (e) { onError(`copy: ${(e as Error).message}`); }
  }

  async function changePassword() {
    if (!oldPw || !newPw) { onError('Both passwords required'); return; }
    if (newPw !== newPw2) { onError('New passwords do not match'); return; }
    busy = true;
    try {
      await rpc.call('vault.changePassword', { oldPassword: oldPw, newPassword: newPw });
      oldPw = ''; newPw = ''; newPw2 = '';
      showChangePw = false;
    } catch (e) { onError(`vault.changePassword: ${(e as Error).message}`); }
    finally { busy = false; }
  }

  onMount(() => { void refreshStatus(); });
</script>

<div class="settings-section">
  <h2 class="flex items-center gap-2"><KeyRound size={16} /> Vault</h2>

  {#if !status.configured}
    <p class="hint">Vault store not available. Restart the app to retry initialization.</p>
  {:else if !status.initialized}
    <div class="section-h">Set up vault</div>
    <p class="hint">
      Choose a strong master password. It encrypts everything you store in the vault
      using ChaCha20-Poly1305 + Argon2. If you lose it, the data is unrecoverable.
    </p>
    <label class="row">
      <span class="row-label">Master password</span>
      <input type="password" bind:value={password} disabled={busy} />
    </label>
    <label class="row">
      <span class="row-label">Confirm</span>
      <input type="password" bind:value={confirmPassword} disabled={busy} />
    </label>
    <div class="actions">
      <button class="btn primary" disabled={busy} onclick={() => void initialize()}>
        <KeyRound size={14} /> Initialize vault
      </button>
    </div>
  {:else if !status.unlocked}
    <div class="section-h">Unlock vault</div>
    <label class="row">
      <span class="row-label">Master password</span>
      <input type="password" bind:value={password} disabled={busy}
             onkeydown={(e) => { if (e.key === 'Enter') void unlock(); }} />
    </label>
    <div class="actions">
      <button class="btn primary" disabled={busy} onclick={() => void unlock()}>
        <Unlock size={14} /> Unlock
      </button>
    </div>
  {:else}
    <div class="toolbar">
      <button class="btn primary" onclick={newEntry}><Plus size={14} /> New entry</button>
      <button class="btn" onclick={() => (showChangePw = !showChangePw)}>Change password</button>
      <button class="btn" onclick={() => void lock()}><Lock size={14} /> Lock</button>
    </div>

    {#if showChangePw}
      <div class="section-h">Change master password</div>
      <label class="row">
        <span class="row-label">Current password</span>
        <input type="password" bind:value={oldPw} disabled={busy} />
      </label>
      <label class="row">
        <span class="row-label">New password</span>
        <input type="password" bind:value={newPw} disabled={busy} />
      </label>
      <label class="row">
        <span class="row-label">Confirm new password</span>
        <input type="password" bind:value={newPw2} disabled={busy} />
      </label>
      <div class="actions">
        <button class="btn primary" disabled={busy} onclick={() => void changePassword()}>
          Apply
        </button>
      </div>
    {/if}

    {#if editing}
      <div class="section-h">{editing.id ? 'Edit entry' : 'New entry'}</div>
      <label class="row">
        <span class="row-label">Label</span>
        <input type="text" bind:value={editing.label} disabled={busy} />
      </label>
      <label class="row">
        <span class="row-label">Kind</span>
        <select bind:value={editing.kind} disabled={busy}>
          <option value="password">Password</option>
          <option value="token">Token</option>
          <option value="key">Private key</option>
          <option value="note">Note</option>
        </select>
      </label>
      <label class="row">
        <span class="row-label">Username (optional)</span>
        <input type="text" bind:value={editing.username} disabled={busy} />
      </label>
      <label class="row">
        <span class="row-label">Secret</span>
        {#if editing.kind === 'note' || editing.kind === 'key'}
          <textarea rows="6" bind:value={editing.secret} disabled={busy}></textarea>
        {:else}
          <input type="password" bind:value={editing.secret} disabled={busy} />
        {/if}
      </label>
      <div class="actions">
        <button class="btn primary" disabled={busy} onclick={() => void saveEntry()}>Save</button>
        <button class="btn" disabled={busy} onclick={() => (editing = null)}>Cancel</button>
      </div>
    {/if}

    <div class="section-h">Entries ({entries.length})</div>
    {#if entries.length === 0}
      <p class="hint">No entries yet. Click <em>New entry</em> to add one.</p>
    {:else}
      <table class="entries">
        <thead>
          <tr><th>Label</th><th>Kind</th><th>Username</th><th>Secret</th><th></th></tr>
        </thead>
        <tbody>
          {#each entries as e (e.id)}
            <tr>
              <td>{e.label}</td>
              <td><span class="kind">{e.kind}</span></td>
              <td>{e.username ?? ''}</td>
              <td class="secret-col">
                {#if revealedId === e.id}
                  <code>{revealedSecret}</code>
                {:else}
                  <code class="masked">••••••••</code>
                {/if}
              </td>
              <td class="actions-col">
                <button class="icon" title="Reveal" onclick={() => void reveal(e)}>
                  {#if revealedId === e.id}<EyeOff size={14} />{:else}<Eye size={14} />{/if}
                </button>
                <button class="icon" title="Copy" onclick={() => void copySecret(e)}>📋</button>
                <button class="icon" title="Edit" onclick={() => void editEntry(e)}><Pencil size={14} /></button>
                <button class="icon danger" title="Delete" onclick={() => void deleteEntry(e.id)}><Trash2 size={14} /></button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  {/if}
</div>

<style>
  .section-h {
    margin-top: 16px;
    margin-bottom: 6px;
    font-size: 11.5px;
    text-transform: uppercase;
    color: var(--color-fg-muted);
    letter-spacing: 0.04em;
  }
  .hint { color: var(--color-fg-muted); font-size: 12px; margin: 6px 0; }
  .row {
    display: grid;
    grid-template-columns: 220px 1fr;
    align-items: center;
    gap: 10px;
    padding: 4px 0;
  }
  .row-label { font-size: 12.5px; }
  .row input[type='text'],
  .row input[type='password'],
  .row select,
  .row textarea {
    padding: 4px 8px;
    background: var(--color-bg-soft);
    color: var(--color-fg);
    border: 1px solid var(--color-border);
    border-radius: 4px;
    font-size: 12.5px;
    width: 100%;
    max-width: 380px;
    font-family: inherit;
  }
  .row textarea { font-family: var(--font-mono, monospace); resize: vertical; }
  .row input:focus, .row select:focus, .row textarea:focus {
    outline: none; border-color: var(--color-accent);
  }
  .actions { margin-top: 10px; display: flex; gap: 8px; }
  .toolbar { display: flex; gap: 8px; margin: 8px 0 4px; }
  .btn {
    display: inline-flex; align-items: center; gap: 4px;
    padding: 5px 10px;
    background: var(--color-bg-soft);
    color: var(--color-fg);
    border: 1px solid var(--color-border);
    border-radius: 4px;
    font-size: 12.5px;
    cursor: pointer;
  }
  .btn:hover { border-color: var(--color-accent); }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
  .btn.primary { background: var(--color-accent); color: var(--color-bg); border-color: var(--color-accent); }
  table.entries { width: 100%; border-collapse: collapse; font-size: 12.5px; margin-top: 6px; }
  table.entries th, table.entries td {
    text-align: left; padding: 6px 8px; border-bottom: 1px solid var(--color-border);
  }
  table.entries th { color: var(--color-fg-muted); font-weight: 500; font-size: 11.5px; }
  .kind { font-size: 11px; padding: 1px 6px; border-radius: 3px; background: var(--color-bg-soft); color: var(--color-fg-muted); }
  .secret-col code { font-family: var(--font-mono, monospace); }
  .masked { color: var(--color-fg-muted); }
  .actions-col { white-space: nowrap; }
  .icon {
    display: inline-flex; align-items: center; justify-content: center;
    width: 24px; height: 24px;
    background: transparent; border: 1px solid transparent; border-radius: 3px;
    color: var(--color-fg-muted); cursor: pointer;
  }
  .icon:hover { color: var(--color-fg); background: var(--color-bg-soft); border-color: var(--color-border); }
  .icon.danger:hover { color: #e06c75; }
</style>
