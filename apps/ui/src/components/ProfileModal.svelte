<script lang="ts">
  import { FolderOpen, X } from '@lucide/svelte';
  import { pickPrivateKeyPath } from '../lib/localFiles';
  import type { RpcClient } from '../lib/rpc';
  import { uuidv4 } from '../lib/rpc';
  import type { RemoteDesktopSpec, StoredProfile, SshAuth, SshProfileSpec } from '../lib/types';
  import { i18n } from '../lib/i18n.svelte';
  import { appendJumpProfileLines, loadProfilesForJumps, parseJumpLines } from '../lib/jumpProfiles';
  import { defaultPortForKind } from '../lib/profileDefaults';
  import {
    BUILTIN_PROFILE_ICONS,
    formatTags,
    parseTagsInput,
    profileEndpointLabel,
    suggestDuplicateProfileName,
  } from '../lib/profileMeta';
  import { notifyProfilesChanged } from '../lib/profileEvents';
  import ProfileIcon from './ProfileIcon.svelte';

  interface Props {
    rpc: RpcClient;
    onSaved: () => void;
    onClosed?: () => void;
    onError: (msg: string) => void;
  }
  let { rpc, onSaved, onClosed, onError }: Props = $props();

  let dialog: HTMLDialogElement | null = null;
  let editing = $state<StoredProfile | null>(null);
  let cloning = $state(false);
  let profileKind = $state<'ssh' | 'rdp' | 'vnc'>('ssh');
  let remoteSshProfileId = $state('');
  let localBindPort = $state<number | ''>('');
  let tunnelProfiles = $state<StoredProfile[]>([]);
  let name = $state('');
  let group = $state('');
  let tagsText = $state('');
  let favorite = $state(false);
  let iconKind = $state<'builtin' | 'emoji' | 'file' | 'data'>('builtin');
  let iconValue = $state('server');
  let host = $state('');
  let port = $state(22);
  let user = $state('');
  let authKind = $state<'password' | 'key' | 'vault'>('password');
  let password = $state('');
  let keyPath = $state('');
  let keyPassphrase = $state('');
  let vaultEntryId = $state('');
  let vaultPassphraseEntryId = $state('');
  let vaultEntries = $state<Array<{ id: string; label: string; kind: string }>>([]);
  let vaultUnlocked = $state(false);
  /** One bastion per line, in `user@host[:port]` form. Each hop reuses the
   * target profile's auth method (key or password). */
  let jumpsText = $state('');
  let jumpPickerIds = $state<Set<string>>(new Set());

  const jumpPickProfiles = $derived(
    tunnelProfiles.filter((p) => p.kind === 'ssh' && p.id !== editing?.id),
  );

  function formatJumps(jumps: SshProfileSpec[]): string {
    return jumps
      .map((j) => `${j.user}@${j.host}${j.port === 22 ? '' : ':' + j.port}`)
      .join('\n');
  }

  function loadRemoteFields(spec: RemoteDesktopSpec) {
    host = spec.host;
    port = spec.port;
    remoteSshProfileId = spec.ssh_profile_id ?? '';
    localBindPort = spec.local_bind_port ?? '';
    user = '';
    password = '';
    keyPath = '';
    keyPassphrase = '';
    jumpsText = '';
  }

  async function refreshVaultEntries() {
    try {
      const st = await rpc.call<{ initialized: boolean; unlocked: boolean }>('vault.status', {});
      vaultUnlocked = st.initialized && st.unlocked;
      if (!vaultUnlocked) {
        vaultEntries = [];
        return;
      }
      vaultEntries = await rpc.call<Array<{ id: string; label: string; kind: string }>>('vault.list', {});
    } catch {
      vaultUnlocked = false;
      vaultEntries = [];
    }
  }

  export interface ProfileModalOpenOptions {
    /** Pre-fill Group when creating a new SSH profile (slash-separated path). */
    group?: string;
    /** Pre-fill fields from an existing profile; saves as a new profile (new id on submit). */
    duplicateFrom?: StoredProfile;
    /** Existing profile names for duplicate name suggestion. */
    existingNames?: string[];
  }

  function loadFieldsFromProfile(existing: StoredProfile) {
    profileKind = existing.kind;
    name = existing.name;
    group = existing.group ?? '';
    tagsText = formatTags(existing.tags);
    favorite = !!existing.favorite;
    iconKind = (existing.icon?.kind as typeof iconKind) ?? 'builtin';
    iconValue = existing.icon?.value ?? 'server';
    if (existing.kind === 'ssh') {
      host = existing.ssh.host;
      port = existing.ssh.port;
      user = existing.ssh.user;
      jumpsText = formatJumps(existing.ssh.jump_via ?? []);
      remoteSshProfileId = '';
      localBindPort = '';
      if (typeof existing.ssh.auth === 'object' && 'Password' in existing.ssh.auth) {
        authKind = 'password';
        password = existing.ssh.auth.Password.secret;
        keyPath = '';
        keyPassphrase = '';
        vaultEntryId = '';
        vaultPassphraseEntryId = '';
      } else if (typeof existing.ssh.auth === 'object' && 'PublicKey' in existing.ssh.auth) {
        authKind = 'key';
        keyPath = existing.ssh.auth.PublicKey.key_path;
        keyPassphrase = existing.ssh.auth.PublicKey.passphrase ?? '';
        password = '';
        vaultEntryId = '';
        vaultPassphraseEntryId = '';
      } else if (typeof existing.ssh.auth === 'object' && 'VaultRef' in existing.ssh.auth) {
        authKind = 'vault';
        vaultEntryId = existing.ssh.auth.VaultRef.entry_id;
        vaultPassphraseEntryId = existing.ssh.auth.VaultRef.passphrase_entry_id ?? '';
        password = '';
        keyPath = '';
        keyPassphrase = '';
      } else {
        authKind = 'key';
        keyPath = '';
        keyPassphrase = '';
        password = '';
        vaultEntryId = '';
        vaultPassphraseEntryId = '';
      }
    } else {
      loadRemoteFields(existing.kind === 'rdp' ? existing.rdp : existing.vnc);
      authKind = 'password';
    }
  }

  function onProfileKindChange() {
    port = defaultPortForKind(profileKind);
    if (profileKind !== 'ssh') {
      user = '';
      password = '';
      keyPath = '';
      keyPassphrase = '';
      vaultEntryId = '';
      vaultPassphraseEntryId = '';
      jumpsText = '';
      authKind = 'password';
    }
  }

  function resetNewProfileFields(groupDefault = '') {
    profileKind = 'ssh';
    name = '';
    group = groupDefault;
    tagsText = '';
    favorite = false;
    iconKind = 'builtin';
    iconValue = 'server';
    host = '';
    port = defaultPortForKind('ssh');
    user = '';
    authKind = 'password';
    password = '';
    keyPath = '';
    keyPassphrase = '';
    vaultEntryId = '';
    vaultPassphraseEntryId = '';
    jumpsText = '';
    remoteSshProfileId = '';
    localBindPort = '';
  }

  export function open(existing?: StoredProfile, options?: ProfileModalOpenOptions) {
    cloning = false;
    editing = existing ?? null;
    jumpPickerIds = new Set();
    void refreshVaultEntries();
    void rpc.call<StoredProfile[]>('profile.list')
      .then((list) => { tunnelProfiles = list.filter((p) => p.kind === 'ssh'); })
      .catch(() => { tunnelProfiles = []; });
    if (options?.duplicateFrom) {
      editing = null;
      cloning = true;
      loadFieldsFromProfile(options.duplicateFrom);
      name = suggestDuplicateProfileName(
        options.duplicateFrom.name,
        options.existingNames ?? [],
      );
      favorite = false;
    } else if (existing) {
      loadFieldsFromProfile(existing);
    } else {
      resetNewProfileFields(options?.group?.trim() ?? '');
    }
    dialog?.showModal();
  }

  function close() {
    dialog?.close();
  }

  function toggleJumpPicker(id: string) {
    const next = new Set(jumpPickerIds);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    jumpPickerIds = next;
  }

  function appendJumpPickerSelection() {
    const picked = jumpPickProfiles.filter((p) => jumpPickerIds.has(p.id));
    if (picked.length === 0) return;
    jumpsText = appendJumpProfileLines(jumpsText, picked);
    jumpPickerIds = new Set();
  }

  async function browsePrivateKeyPath() {
    try {
      const path = await pickPrivateKeyPath();
      if (path) keyPath = path;
    } catch (e) {
      onError(i18n.t('profileModal.browsePrivateKeyFailed', { message: (e as Error).message }));
    }
  }

  async function submit(ev: Event) {
    ev.preventDefault();
    const base = {
      schemaVersion: 1,
      id: editing?.id ?? uuidv4(),
      name: name || 'profile',
      group: group.trim() || null,
      tags: parseTagsInput(tagsText),
      favorite,
      icon: iconValue.trim() ? { kind: iconKind, value: iconValue.trim() } : null,
    } as const;
    let profile: StoredProfile;
    if (profileKind === 'ssh') {
      let auth: SshAuth;
      if (authKind === 'vault') {
        if (!vaultEntryId.trim()) {
          onError(i18n.t('profileModal.vaultEntry'));
          return;
        }
        auth = {
          VaultRef: {
            entry_id: vaultEntryId.trim(),
            passphrase_entry_id: vaultPassphraseEntryId.trim() || undefined,
          },
        };
      } else if (authKind === 'key') {
        auth = { PublicKey: { key_path: keyPath, passphrase: keyPassphrase || undefined } };
      } else {
        auth = { Password: { secret: password } };
      }
      let jump_via: SshProfileSpec[];
      try {
        const profiles = await loadProfilesForJumps(rpc);
        jump_via = parseJumpLines(jumpsText, auth, profiles);
      } catch (e) {
        onError((e as Error).message);
        return;
      }
      profile = {
        ...base,
        kind: 'ssh',
        ssh: { host, port: Number(port) || 22, user, auth, jump_via },
      };
    } else {
      const remote: RemoteDesktopSpec = {
        host,
        port: Number(port) || (profileKind === 'rdp' ? 3389 : 5900),
        ssh_profile_id: remoteSshProfileId.trim() || null,
        local_bind_port: localBindPort === '' ? undefined : Number(localBindPort) || undefined,
      };
      profile = profileKind === 'rdp'
        ? { ...base, kind: 'rdp', rdp: remote }
        : { ...base, kind: 'vnc', vnc: remote };
    }
    try {
      await rpc.call('profile.upsert', profile);
      notifyProfilesChanged({ profileId: profile.id, group: profile.group });
      onSaved();
      close();
    } catch (e) {
      onError(i18n.t('profileModal.saveFailed', { message: (e as Error).message }));
    }
  }
</script>

<dialog bind:this={dialog} class="min-w-[420px]" onclose={() => onClosed?.()}>
  <form onsubmit={submit} class="p-5">
    <div class="flex items-center justify-between mb-3">
      <h2 class="text-[14px] font-semibold text-[var(--color-accent)]">
        {editing
          ? i18n.t('profileModal.editTitle')
          : cloning
            ? i18n.t('profileModal.cloneTitle')
            : i18n.t('profileModal.newTitle')}
      </h2>
      <button
        type="button"
        onclick={close}
        class="p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]"
        aria-label={i18n.t('common.close')}
      >
        <X size={14} />
      </button>
    </div>

    <label for="pm-kind" class="block text-[11px] text-[var(--color-fg-muted)] mb-1 mt-2">{i18n.t('profileModal.kind')}</label>
    <select id="pm-kind" bind:value={profileKind} onchange={onProfileKindChange} class="input">
      <option value="ssh">{i18n.t('profileModal.kindSsh')}</option>
      <option value="rdp">{i18n.t('profileModal.kindRdp')}</option>
      <option value="vnc">{i18n.t('profileModal.kindVnc')}</option>
    </select>

    <label for="pm-name" class="block text-[11px] text-[var(--color-fg-muted)] mb-1 mt-2">{i18n.t('profileModal.name')}</label>
    <input id="pm-name" bind:value={name} required placeholder="prod web 01" class="input" />

    <div class="flex gap-3 mt-2">
      <div class="flex-1">
        <label for="pm-group" class="block text-[11px] text-[var(--color-fg-muted)] mb-1">{i18n.t('profileModal.group')}</label>
        <input id="pm-group" bind:value={group} placeholder={i18n.t('profileModal.groupPlaceholder')} class="input" />
      </div>
      <label class="favorite-row">
        <input type="checkbox" bind:checked={favorite} />
        <span>{i18n.t('profileModal.favorite')}</span>
      </label>
    </div>

    <label for="pm-tags" class="block text-[11px] text-[var(--color-fg-muted)] mb-1 mt-2">{i18n.t('profileModal.tags')}</label>
    <input id="pm-tags" bind:value={tagsText} placeholder="prod, db, singapore" class="input" />

    <div class="flex gap-3 mt-2 items-end">
      <div>
        <label for="pm-icon-kind" class="block text-[11px] text-[var(--color-fg-muted)] mb-1">{i18n.t('profileModal.icon')}</label>
        <div class="flex items-center gap-2">
          <ProfileIcon icon={{ kind: iconKind, value: iconValue }} {name} />
          <select id="pm-icon-kind" bind:value={iconKind} class="input min-w-[112px]">
            <option value="builtin">{i18n.t('profileModal.builtin')}</option>
            <option value="emoji">{i18n.t('profileModal.emoji')}</option>
            <option value="file">{i18n.t('profileModal.filePath')}</option>
            <option value="data">{i18n.t('profileModal.dataUri')}</option>
          </select>
        </div>
      </div>
      <div class="flex-1">
        {#if iconKind === 'builtin'}
          <label for="pm-icon-value" class="block text-[11px] text-[var(--color-fg-muted)] mb-1">{i18n.t('profileModal.builtinIcon')}</label>
          <select id="pm-icon-value" bind:value={iconValue} class="input">
            {#each BUILTIN_PROFILE_ICONS as icon (icon)}
              <option value={icon}>{icon}</option>
            {/each}
          </select>
        {:else}
          <label for="pm-icon-value" class="block text-[11px] text-[var(--color-fg-muted)] mb-1">{i18n.t('profileModal.iconValue')}</label>
          <input id="pm-icon-value" bind:value={iconValue} placeholder={iconKind === 'emoji' ? 'emoji or short text' : 'path or data URI'} class="input" />
        {/if}
      </div>
    </div>

    <div class="flex gap-3 mt-2">
      <div class="flex-1">
        <label for="pm-host" class="block text-[11px] text-[var(--color-fg-muted)] mb-1">{i18n.t('profileModal.host')}</label>
        <input id="pm-host" bind:value={host} required placeholder="example.com" class="input" />
      </div>
      <div style="max-width:110px">
        <label for="pm-port" class="block text-[11px] text-[var(--color-fg-muted)] mb-1">{i18n.t('profileModal.port')}</label>
        <input id="pm-port" bind:value={port} type="number" min="1" max="65535" class="input" />
      </div>
    </div>

    {#if profileKind === 'ssh'}
      <label for="pm-user" class="block text-[11px] text-[var(--color-fg-muted)] mb-1 mt-2">{i18n.t('profileModal.user')}</label>
      <input id="pm-user" bind:value={user} required placeholder="root" class="input" />

      <label for="pm-auth" class="block text-[11px] text-[var(--color-fg-muted)] mb-1 mt-2">{i18n.t('profileModal.authMethod')}</label>
      <select id="pm-auth" bind:value={authKind} class="input">
        <option value="password">{i18n.t('profileModal.password')}</option>
        <option value="key">{i18n.t('profileModal.publicKey')}</option>
        <option value="vault">{i18n.t('profileModal.vault')}</option>
      </select>

      {#if authKind === 'vault'}
        {#if !vaultUnlocked}
          <p class="text-[10.5px] text-[var(--color-fg-muted)] mt-2">{i18n.t('profileModal.vaultLockedHint')}</p>
        {:else}
          <label for="pm-vault-entry" class="block text-[11px] text-[var(--color-fg-muted)] mb-1 mt-2">
            {i18n.t('profileModal.vaultEntry')}
          </label>
          <select id="pm-vault-entry" bind:value={vaultEntryId} class="input">
            <option value="">—</option>
            {#each vaultEntries as ve (ve.id)}
              <option value={ve.id}>{ve.label} ({ve.kind})</option>
            {/each}
          </select>
          <label for="pm-vault-pass" class="block text-[11px] text-[var(--color-fg-muted)] mb-1 mt-2">
            {i18n.t('profileModal.vaultPassphraseEntry')}
          </label>
          <select id="pm-vault-pass" bind:value={vaultPassphraseEntryId} class="input">
            <option value="">—</option>
            {#each vaultEntries.filter((e) => e.kind === 'password') as ve (ve.id)}
              <option value={ve.id}>{ve.label}</option>
            {/each}
          </select>
        {/if}
      {:else if authKind === 'password'}
        <label for="pm-pw" class="block text-[11px] text-[var(--color-fg-muted)] mb-1 mt-2">
          {i18n.t('profileModal.passwordStoredLocally')}
        </label>
        <input id="pm-pw" type="password" bind:value={password} class="input" />
      {:else}
        <label for="pm-keypath" class="block text-[11px] text-[var(--color-fg-muted)] mb-1 mt-2">{i18n.t('profileModal.privateKeyPath')}</label>
        <div class="path-field">
          <input
            id="pm-keypath"
            bind:value={keyPath}
            placeholder="~/.ssh/id_ed25519"
            class="input path-field-input"
          />
          <button
            type="button"
            class="btn-secondary path-field-btn"
            title={i18n.t('profileModal.browsePrivateKey')}
            aria-label={i18n.t('profileModal.browsePrivateKey')}
            onclick={() => { void browsePrivateKeyPath(); }}
          >
            <FolderOpen size={14} />
            <span>{i18n.t('profileModal.browsePrivateKey')}</span>
          </button>
        </div>
        <label for="pm-keypass" class="block text-[11px] text-[var(--color-fg-muted)] mb-1 mt-2">
          {i18n.t('profileModal.keyPassphrase')}
        </label>
        <input id="pm-keypass" type="password" bind:value={keyPassphrase} class="input" />
      {/if}

      <label for="pm-jumps" class="block text-[11px] text-[var(--color-fg-muted)] mb-1 mt-2">
        {i18n.t('profileModal.proxyJump')}
      </label>
      {#if jumpPickProfiles.length > 0}
        <div class="jump-picker mt-2 border border-[var(--color-border-soft)] rounded-md overflow-hidden">
          <div class="px-2 py-1.5 text-[10.5px] text-[var(--color-fg-muted)] bg-[var(--color-panel-2)] border-b border-[var(--color-border-soft)]">
            {i18n.t('profileModal.jumpPickFromList')}
          </div>
          <div class="jump-picker-list max-h-[140px] overflow-y-auto px-1 py-1">
            {#each jumpPickProfiles as jp (jp.id)}
              <label class="jump-picker-row flex items-start gap-2 px-2 py-1 rounded hover:bg-[var(--color-panel-2)] cursor-pointer text-[11.5px]">
                <input
                  type="checkbox"
                  class="mt-0.5 shrink-0"
                  checked={jumpPickerIds.has(jp.id)}
                  onchange={() => toggleJumpPicker(jp.id)}
                />
                <span class="min-w-0 flex-1">
                  <span class="block truncate text-[var(--color-fg)]">{jp.name}</span>
                  <span class="block truncate text-[10px] text-[var(--color-fg-muted)]">{profileEndpointLabel(jp)}</span>
                </span>
              </label>
            {/each}
          </div>
          <div class="px-2 py-1.5 border-t border-[var(--color-border-soft)] flex justify-end">
            <button
              type="button"
              class="btn-secondary text-[11px] py-0.5 px-2"
              disabled={jumpPickerIds.size === 0}
              onclick={appendJumpPickerSelection}
            >
              {i18n.t('profileModal.jumpAddSelected')}
            </button>
          </div>
        </div>
      {/if}
      <textarea
        id="pm-jumps"
        bind:value={jumpsText}
        rows="2"
        placeholder="jumpuser@bastion.example.com&#10;@prod-bastion"
        class="input font-mono text-[11.5px] mt-2"
      ></textarea>
      <p class="text-[10.5px] text-[var(--color-fg-muted)] mt-1">{i18n.t('profileModal.proxyJumpProfileRef')}</p>
    {:else}
      <label for="pm-tunnel" class="block text-[11px] text-[var(--color-fg-muted)] mb-1 mt-2">{i18n.t('profileModal.sshTunnel')}</label>
      <select id="pm-tunnel" bind:value={remoteSshProfileId} class="input">
        <option value="">(direct)</option>
        {#each tunnelProfiles as tp (tp.id)}
          <option value={tp.id}>{tp.name}</option>
        {/each}
      </select>
      <label for="pm-lport" class="block text-[11px] text-[var(--color-fg-muted)] mb-1 mt-2">{i18n.t('profileModal.localBindPort')}</label>
      <input id="pm-lport" bind:value={localBindPort} type="number" min="1" max="65535" class="input" placeholder="auto" />
    {/if}

    <div class="flex justify-end gap-2 mt-5">
      <button type="button" onclick={close} class="btn-secondary">{i18n.t('common.cancel')}</button>
      <button type="submit" class="btn-primary">{i18n.t('common.save')}</button>
    </div>
  </form>
</dialog>

<style>
  .favorite-row {
    min-width: 112px;
    display: flex;
    align-items: center;
    gap: 6px;
    align-self: end;
    min-height: 31px;
    font-size: 12px;
    color: var(--color-fg-muted);
  }
  .path-field {
    display: flex;
    align-items: stretch;
    gap: 8px;
  }
  .path-field-input {
    flex: 1;
    min-width: 0;
  }
  .path-field-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
    flex-shrink: 0;
  }
</style>
