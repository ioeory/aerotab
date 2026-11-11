<script lang="ts">
  import { onDestroy } from 'svelte';
  import type { RpcClient } from '../../lib/rpc';
  import type { StoredProfile, GroupSshDefaults } from '../../lib/types';
  import { resolveGroupDefaults } from '../../lib/profileCatalogHelpers';
  import { normalizeGroupPath } from '../../lib/profileTree';
  import { i18n } from '../../lib/i18n.svelte';

  type Defaults = GroupSshDefaults;
  type Field = Exclude<keyof Defaults, 'force_subgroups' | 'force_hosts'>;
  type DefaultsMap = Record<string, Defaults>;
  type VaultMeta = { id: string; label: string; kind: string };
  let { rpc, onSaved }: { rpc: RpcClient; onSaved?: (path: string) => void } = $props();
  let dialog: HTMLDialogElement;
  let path = $state('');
  let loading = $state(false);
  let loaded = $state(false);
  let saving = $state(false);
  let error = $state('');
  let revision = 0;
  let initialOwnSnapshot: string | undefined;
  let forceSubgroups = $state(false);
  let forceHosts = $state(false);
  let parentForced = $state<Partial<Record<Field, string>>>({});
  let parent = $state<Defaults>({});
  let sources = $state<Partial<Record<Field, string>>>({});
  let overrides = $state<Record<Field, boolean>>({ user: false, port: false, auth: false, proxy_id: false, jump_profile_ids: false });
  let user = $state('');
  let port = $state<number | undefined>(22);
  let authKind = $state('Agent');
  let keyPath = $state('');
  let vaultEntry = $state('');
  let passphraseEntry = $state('');
  let proxyId = $state('');
  let jumpIds = $state<string[]>([]);
  let profiles = $state<StoredProfile[]>([]);
  let proxies = $state<Array<{ id: string; name: string }>>([]);
  let vaultEntries = $state<VaultMeta[]>([]);
  let vaultUnlocked = $state(false);
  const fields: Field[] = ['user', 'port', 'auth', 'proxy_id', 'jump_profile_ids'];
  const fieldLabels: Record<Field, string> = {
    user: 'profileModal.user', port: 'profileModal.port', auth: 'profileModal.authMethod',
    proxy_id: 'groupDefaults.proxy', jump_profile_ids: 'groupDefaults.jumps',
  };

  async function readMap(): Promise<DefaultsMap> {
    const result = await rpc.call<{ value: unknown }>('settings.get', { key: 'profile.groupDefaults' });
    if (result.value == null) return {};
    if (typeof result.value !== 'object' || Array.isArray(result.value)) throw Error('invalid map');
    const map = result.value as DefaultsMap;
    for (const [key, value] of Object.entries(map)) {
      if (normalizeGroupPath(key) !== key || !value || typeof value !== 'object' || Array.isArray(value)) throw Error('invalid map');
    }
    return map;
  }

  export async function open(groupPath: string) {
    if (saving) return;
    const normalized = normalizeGroupPath(groupPath);
    if (!normalized) return;
    const request = ++revision;
    path = normalized;
    loaded = false;
    loading = true;
    error = '';
    parent = {};
    sources = {};
    if (!dialog.open) dialog.showModal();
    try {
      const [map, nextProfiles, nextProxies, status] = await Promise.all([
        readMap(), rpc.call<StoredProfile[]>('profile.list'),
        rpc.call<Array<{ id: string; name: string }>>('proxy.list'),
        rpc.call<{ initialized: boolean; unlocked: boolean }>('vault.status', {}),
      ]);
      if (request !== revision) return;
      const entries = status.initialized && status.unlocked ? await rpc.call<VaultMeta[]>('vault.list', {}) : [];
      if (request !== revision) return;
      profiles = nextProfiles;
      proxies = nextProxies;
      vaultUnlocked = status.initialized && status.unlocked;
      vaultEntries = entries;
      const parts = normalized.split('/');
      const resolved = resolveGroupDefaults(parts.slice(0, -1).join('/'), map);
      const effective = resolved.values;
      parent = effective;
      sources = {};
      parentForced = {};
      for (const field of fields) {
        const key = field === 'jump_profile_ids' ? 'jump' : field;
        sources[field] = resolved.sources[key];
        parentForced[field] = resolved.locked[key];
      }
      initialOwnSnapshot = JSON.stringify(map[normalized]);
      const own = map[normalized] ?? {};
      forceSubgroups = own.force_subgroups ?? false;
      forceHosts = own.force_hosts ?? false;
      overrides = Object.fromEntries(fields.map(field => [field, own[field] !== undefined])) as Record<Field, boolean>;
      user = own.user ?? effective.user ?? '';
      port = own.port ?? effective.port ?? 22;
      const auth = own.auth ?? effective.auth;
      authKind = !auth || auth === 'Agent' ? 'Agent' : 'VaultRef' in auth ? 'VaultRef' : 'PublicKey' in auth && !auth.PublicKey.passphrase ? 'PublicKey' : '';
      keyPath = auth && typeof auth === 'object' && 'PublicKey' in auth ? auth.PublicKey.key_path : '';
      vaultEntry = auth && typeof auth === 'object' && 'VaultRef' in auth ? auth.VaultRef.entry_id : '';
      passphraseEntry = auth && typeof auth === 'object' && 'VaultRef' in auth ? auth.VaultRef.passphrase_entry_id ?? '' : '';
      proxyId = (own.proxy_id !== undefined ? own.proxy_id : effective.proxy_id) ?? '';
      jumpIds = [...(own.jump_profile_ids ?? effective.jump_profile_ids ?? [])];
      loaded = true;
    } catch {
      if (request === revision) error = i18n.t('groupDefaults.loadFailed');
    } finally {
      if (request === revision) loading = false;
    }
  }

  function parentValue(field: Field): string {
    const value = parent[field];
    if (value === undefined) return i18n.t('groupDefaults.notSet');
    if (field === 'auth') {
      const auth = parent.auth;
      if (auth === 'Agent') return i18n.t('ssh.agent');
      if (auth && typeof auth === 'object' && 'VaultRef' in auth) return i18n.t('profileModal.vault');
      if (auth && typeof auth === 'object' && 'PublicKey' in auth) return i18n.t('profileModal.publicKey');
      return i18n.t('groupDefaults.protectedAuth');
    }
    if (field === 'proxy_id') return value === null ? i18n.t('groupDefaults.direct') : proxies.find(proxy => proxy.id === value)?.name ?? i18n.t('groupDefaults.unavailable');
    if (field === 'jump_profile_ids') return parent.jump_profile_ids?.length
      ? parent.jump_profile_ids.map(id => profiles.find(profile => profile.id === id)?.name ?? i18n.t('groupDefaults.unavailable')).join(' → ')
      : i18n.t('groupDefaults.direct');
    return String(value);
  }

  // Top-layer dialogs still inherit shell zoom; keep viewport sizing unscaled.
  function mountOutsideShell(node: HTMLDialogElement) {
    document.body.appendChild(node);
    return { destroy() { node.remove(); } };
  }

  function close() { if (!saving) dialog.close(); }
  function invalidate() { revision++; loaded = false; loading = false; }
  onDestroy(invalidate);

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (!loaded || loading || saving) return;
    const request = revision;
    const currentPath = path;
    const value: Defaults = {};
    error = '';
    if (overrides.user) {
      if (!user.trim()) { error = i18n.t('groupDefaults.invalid'); return; }
      value.user = user.trim();
    }
    if (overrides.port) {
      if (!port || !Number.isInteger(port) || port < 1 || port > 65535) { error = i18n.t('groupDefaults.invalid'); return; }
      value.port = port;
    }
    if (overrides.auth) {
      if (authKind === 'Agent') value.auth = 'Agent';
      else if (authKind === 'PublicKey' && keyPath.trim()) value.auth = { PublicKey: { key_path: keyPath.trim() } };
      else if (authKind === 'VaultRef' && vaultEntry) value.auth = { VaultRef: { entry_id: vaultEntry, ...(passphraseEntry ? { passphrase_entry_id: passphraseEntry } : {}) } };
      else { error = i18n.t('groupDefaults.invalid'); return; }
    }
    if (overrides.proxy_id) value.proxy_id = proxyId || null;
    if (overrides.jump_profile_ids) value.jump_profile_ids = [...jumpIds];
    if (forceSubgroups) value.force_subgroups = true;
    if (forceHosts) value.force_hosts = true;
    saving = true;
    try {
      // Read immediately before writing; never persist the map captured when opening.
      const latest = await readMap();
      if (request !== revision) return;
      if (JSON.stringify(latest[currentPath]) !== initialOwnSnapshot) {
        error = i18n.t('groupDefaults.saveConflict');
        return;
      }
      const next = { ...latest };
      if (Object.keys(value).length) Object.defineProperty(next, currentPath, { value, enumerable: true, configurable: true, writable: true });
      else delete next[currentPath];
      await rpc.call('settings.set', { key: 'profile.groupDefaults', value: next });
      if (request !== revision) return;
      saving = false;
      dialog.close();
      onSaved?.(currentPath);
    } catch {
      if (request === revision) error = i18n.t('groupDefaults.saveFailed');
    } finally {
      saving = false;
    }
  }
</script>

<dialog bind:this={dialog} use:mountOutsideShell class="group-defaults-dialog" aria-label={i18n.t('groupDefaults.title')} onclose={invalidate} oncancel={(event) => { if (saving) event.preventDefault(); }}>
  <form class="group-defaults-form" onsubmit={save}>
    <div class="group-defaults-body">
    <h2>{i18n.t('groupDefaults.title')}</h2>
    <p class="path">{path}</p>
    <p class="hint">{i18n.t('groupDefaults.hint')}</p>
    {#if loading}<p role="status">{i18n.t('common.loading')}</p>{/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <fieldset disabled={!loaded || loading || saving}>
      <label class="force"><input type="checkbox" bind:checked={forceSubgroups} />{i18n.t('groupDefaults.forceSubgroups')}</label>
      <label class="force"><input type="checkbox" bind:checked={forceHosts} />{i18n.t('groupDefaults.forceHosts')}</label>
      <p class="hint">{i18n.t('groupDefaults.forceHint')}</p>
      {#each fields as field (field)}
        <section>
          <label class="mode"><span>{i18n.t(fieldLabels[field])}</span>
            <select bind:value={overrides[field]}>
              <option value={false}>{i18n.t('groupDefaults.inherit')}</option>
              <option value={true}>{i18n.t('groupDefaults.override')}</option>
            </select>
          </label>
          <p class="hint">{i18n.t('groupDefaults.parentValue', { source: sources[field] ?? i18n.t('groupDefaults.notSet'), value: parentValue(field) })}</p>
          {#if parentForced[field]}<p class="hint">{i18n.t('groupDefaults.forcedBy', { source: parentForced[field] })}</p>{/if}
          {#if overrides[field]}
            {#if field === 'user'}
              <input aria-label={i18n.t('profileModal.user')} bind:value={user} required autocomplete="off" />
            {:else if field === 'port'}
              <input aria-label={i18n.t('profileModal.port')} type="number" min="1" max="65535" step="1" bind:value={port} required />
            {:else if field === 'auth'}
              <select aria-label={i18n.t('profileModal.authMethod')} bind:value={authKind}>
                {#if !authKind}<option value="" disabled>{i18n.t('groupDefaults.protectedAuth')}</option>{/if}
                <option value="Agent">{i18n.t('ssh.agent')}</option>
                <option value="VaultRef">{i18n.t('profileModal.vault')}</option>
                <option value="PublicKey">{i18n.t('profileModal.publicKey')}</option>
              </select>
              {#if authKind === 'PublicKey'}
                <label>{i18n.t('profileModal.privateKeyPath')}<input bind:value={keyPath} required autocomplete="off" /></label>
              {:else if authKind === 'VaultRef'}
                {#if !vaultUnlocked}<p class="hint">{i18n.t('profileModal.vaultLockedHint')}</p>{/if}
                <label>{i18n.t('profileModal.vaultEntry')}<select bind:value={vaultEntry} required>
                  <option value="">{i18n.t('groupDefaults.chooseVault')}</option>
                  {#if vaultEntry && !vaultEntries.some(entry => entry.id === vaultEntry)}<option value={vaultEntry}>{i18n.t('groupDefaults.retainedReference')}</option>{/if}
                  {#each vaultEntries.filter(entry => entry.kind === 'password' || entry.kind === 'key') as entry (entry.id)}<option value={entry.id}>{entry.label}</option>{/each}
                </select></label>
                <label>{i18n.t('profileModal.vaultPassphraseEntry')}<select bind:value={passphraseEntry}>
                  <option value="">{i18n.t('groupDefaults.none')}</option>
                  {#if passphraseEntry && !vaultEntries.some(entry => entry.id === passphraseEntry)}<option value={passphraseEntry}>{i18n.t('groupDefaults.retainedReference')}</option>{/if}
                  {#each vaultEntries.filter(entry => entry.kind === 'password') as entry (entry.id)}<option value={entry.id}>{entry.label}</option>{/each}
                </select></label>
              {/if}
              <p class="hint">{i18n.t('groupDefaults.vaultOnly')}</p>
            {:else if field === 'proxy_id'}
              <select aria-label={i18n.t('groupDefaults.proxy')} bind:value={proxyId}>
                <option value="">{i18n.t('groupDefaults.direct')}</option>
                {#if proxyId && !proxies.some(proxy => proxy.id === proxyId)}<option value={proxyId}>{i18n.t('groupDefaults.unavailable')}</option>{/if}
                {#each proxies as proxy (proxy.id)}<option value={proxy.id}>{proxy.name}</option>{/each}
              </select>
            {:else}
              <p class="hint">{i18n.t('groupDefaults.jumpOrder')}</p>
              {#each jumpIds as id, index (`${index}:${id}`)}
                <div class="jump">
                  <select aria-label={i18n.t('groupDefaults.hop', { index: index + 1 })} value={id} onchange={(event) => { jumpIds = jumpIds.map((old, i) => i === index ? event.currentTarget.value : old); }}>
                    {#if !profiles.some(profile => profile.kind === 'ssh' && profile.id === id)}<option value={id}>{i18n.t('groupDefaults.unavailable')}</option>{/if}
                    {#each profiles.filter(profile => profile.kind === 'ssh') as profile (profile.id)}<option value={profile.id}>{profile.name}</option>{/each}
                  </select>
                  <button type="button" onclick={() => { jumpIds = jumpIds.filter((_, i) => i !== index); }}>{i18n.t('common.delete')}</button>
                </div>
              {/each}
              {#if !jumpIds.length}<p>{i18n.t('groupDefaults.direct')}</p>{/if}
              <div class="actions"><button type="button" disabled={!profiles.some(profile => profile.kind === 'ssh')} onclick={() => { const first = profiles.find(profile => profile.kind === 'ssh'); if (first) jumpIds = [...jumpIds, first.id]; }}>{i18n.t('groupDefaults.addHop')}</button>
              <button type="button" onclick={() => { jumpIds = []; }}>{i18n.t('groupDefaults.direct')}</button></div>
            {/if}
          {/if}
        </section>
      {/each}
    </fieldset>
    </div>
    <footer class="group-defaults-footer">
      {#if !loaded && !loading}<button type="button" onclick={() => open(path)}>{i18n.t('groupDefaults.retry')}</button>{/if}
      <button type="button" disabled={saving} onclick={close}>{i18n.t('common.cancel')}</button>
      <button type="submit" class="btn-primary" disabled={!loaded || loading || saving}>{i18n.t('common.save')}</button>
    </footer>
  </form>
</dialog>

<style>
  dialog.group-defaults-dialog { box-sizing: border-box; width: min(660px, calc(100vw - 32px)); max-height: calc(100vh - 32px); max-height: calc(100dvh - 32px); padding: 0; overflow: hidden; color: var(--color-fg); background: var(--color-panel); }
  .group-defaults-form { box-sizing: border-box; display: flex; flex-direction: column; min-height: 0; max-height: inherit; padding: 0; margin: 0; overflow: hidden; }
  .group-defaults-body { min-height: 0; overflow-y: auto; overscroll-behavior: contain; padding: 24px 24px 0; }
  h2 { font-size: 20px; font-weight: 600; }
  .path { overflow-wrap: anywhere; margin: 6px 0 12px; }
  .hint { color: var(--color-fg-muted); font-size: 12px; margin: 6px 0 10px; overflow-wrap: anywhere; }
  .error { color: var(--color-danger); margin: 12px 0; }
  fieldset { border: 0; padding: 0; min-width: 0; }
  section { padding: 14px 0; border-bottom: 1px solid var(--color-border-soft); }
  label { display: flex; flex-direction: column; gap: 6px; margin-top: 8px; }
  .mode { flex-direction: row; justify-content: space-between; align-items: center; gap: 16px; }
  input, select { border: 1px solid var(--color-border); border-radius: 6px; padding: 8px; color: var(--color-fg); background: var(--color-panel-2); width: 100%; min-width: 0; }
  .mode select { width: auto; }
  .force { flex-direction: row; align-items: center; }
  .force input { width: auto; }
  .jump, .actions, footer { display: flex; gap: 8px; margin-top: 8px; }
  button { padding: 8px 12px; border: 1px solid var(--color-border); border-radius: 6px; white-space: nowrap; }
  button:disabled { opacity: .5; }
  .group-defaults-footer { flex-shrink: 0; justify-content: flex-end; margin: 0; padding: 20px 24px 24px; }
</style>
