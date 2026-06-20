<script lang="ts">
  import { FolderOpen, X } from '@lucide/svelte';
  import { pickIconFilePath, pickPrivateKeyPath } from '../lib/localFiles';
  import type { RpcClient } from '../lib/rpc';
  import { uuidv4 } from '../lib/rpc';
  import type { RemoteDesktopSpec, StoredProfile, SshAuth, SshProfileSpec } from '../lib/types';
  import { i18n } from '../lib/i18n.svelte';
  import {
    formatJumpLinesForEdit,
    joinJumpLines,
    loadProfilesForJumps,
    parseJumpLines,
    splitJumpLines,
  } from '../lib/jumpProfiles';
  import { defaultPortForKind } from '../lib/profileDefaults';
  import {
    BUILTIN_PROFILE_ICONS,
    collectProfileTags,
    normalizeTags,
    parseProfileIconInput,
    suggestDuplicateProfileName,
  } from '../lib/profileMeta';
  import { notifyProfilesChanged } from '../lib/profileEvents';
  import { builtinIconTone, groupStyle, normalizeGroupKey, normalizeTagKey } from '../lib/profileVisuals';
  import { profileVisualsStore } from '../lib/profileVisualsStore.svelte';
  import JumpChainEditor from './JumpChainEditor.svelte';
  import ProfileIcon from './ProfileIcon.svelte';
  import ProfileTag from './ProfileTag.svelte';
  import ProfileTagEditor from './ProfileTagEditor.svelte';
  import VisualColorPicker from './VisualColorPicker.svelte';

  interface Props {
    rpc: RpcClient;
    onSaved: () => void;
    onClosed?: () => void;
    onError: (msg: string) => void;
    onOpenVault?: () => void;
  }
  let { rpc, onSaved, onClosed, onError, onOpenVault }: Props = $props();

  let dialog: HTMLDialogElement | null = null;
  let editing = $state<StoredProfile | null>(null);
  let cloning = $state(false);
  let profileKind = $state<'ssh' | 'rdp' | 'vnc'>('ssh');
  let remoteSshProfileId = $state('');
  let localBindPort = $state<number | ''>('');
  let tunnelProfiles = $state<StoredProfile[]>([]);
  let catalogProfiles = $state<StoredProfile[]>([]);
  let name = $state('');
  let group = $state('');
  let selectedTags = $state<string[]>([]);
  const knownTags = $derived(collectProfileTags(catalogProfiles));
  let note = $state('');
  let favorite = $state(false);
  let iconKind = $state<'builtin' | 'emoji' | 'file' | 'data' | 'remote' | 'selfhst'>('builtin');
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
  /** Ordered ProxyJump lines (`@ProfileName` or `user@host[:port]`). */
  let jumpChainLines = $state<string[]>([]);
  let iconGridExpanded = $state(false);

  function loadRemoteFields(spec: RemoteDesktopSpec) {
    host = spec.host;
    port = spec.port;
    remoteSshProfileId = spec.ssh_profile_id ?? '';
    localBindPort = spec.local_bind_port ?? '';
    user = '';
    password = '';
    keyPath = '';
    keyPassphrase = '';
    jumpChainLines = [];
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

  function loadFieldsFromProfile(existing: StoredProfile, profiles: StoredProfile[] = tunnelProfiles) {
    profileKind = existing.kind;
    name = existing.name;
    group = existing.group ?? '';
    selectedTags = normalizeTags(existing.tags);
    note = existing.note ?? '';
    favorite = !!existing.favorite;
    iconKind = (existing.icon?.kind as typeof iconKind) ?? 'builtin';
    iconValue = existing.icon?.value ?? 'server';
    if (existing.kind === 'ssh') {
      host = existing.ssh.host;
      port = existing.ssh.port;
      user = existing.ssh.user;
      jumpChainLines = splitJumpLines(formatJumpLinesForEdit(existing.ssh.jump_via ?? [], profiles));
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
      jumpChainLines = [];
      authKind = 'password';
    }
  }

  function resetNewProfileFields(groupDefault = '') {
    profileKind = 'ssh';
    name = '';
    group = groupDefault;
    selectedTags = [];
    note = '';
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
    jumpChainLines = [];
    remoteSshProfileId = '';
    localBindPort = '';
    iconGridExpanded = false;
  }

  export function open(existing?: StoredProfile, options?: ProfileModalOpenOptions) {
    cloning = false;
    editing = existing ?? null;
    void refreshVaultEntries();
    void rpc.call<StoredProfile[]>('profile.list')
      .then((list) => {
        catalogProfiles = list;
        tunnelProfiles = list.filter((p) => p.kind === 'ssh');
        if (existing && !options?.duplicateFrom) {
          loadFieldsFromProfile(existing, tunnelProfiles);
        }
      })
      .catch(() => { tunnelProfiles = []; catalogProfiles = []; });
    if (options?.duplicateFrom) {
      editing = null;
      cloning = true;
      loadFieldsFromProfile(options.duplicateFrom, tunnelProfiles);
      name = suggestDuplicateProfileName(
        options.duplicateFrom.name,
        options.existingNames ?? [],
      );
      favorite = false;
    } else if (existing) {
      loadFieldsFromProfile(existing, tunnelProfiles);
    } else {
      resetNewProfileFields(options?.group?.trim() ?? '');
    }
    iconGridExpanded = false;
    dialog?.showModal();
  }

  function close() {
    dialog?.close();
  }

  async function chooseIconFile() {
    try {
      const path = await pickIconFilePath();
      if (!path) return;
      iconKind = 'file';
      iconValue = path;
    } catch (e) {
      onError(`icon picker: ${(e as Error).message}`);
    }
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
      tags: selectedTags,
      note: note.trim() || null,
      favorite,
      icon: iconKind === 'builtin' ? parseProfileIconInput(iconValue) : iconKind === 'selfhst' ? parseProfileIconInput(`selfhst:${iconValue}`) : (iconValue.trim() ? { kind: iconKind, value: iconValue.trim() } : null),
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
        jump_via = parseJumpLines(joinJumpLines(jumpChainLines), auth, profiles);
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

<dialog
  bind:this={dialog}
  class="min-w-[min(480px,96vw)] max-w-[min(560px,96vw)]"
  aria-labelledby="profile-modal-title"
  onclose={() => onClosed?.()}
>
  <form onsubmit={submit} class="p-5">
    <div class="flex items-center justify-between mb-3">
      <h2 id="profile-modal-title" class="text-[14px] font-semibold text-[var(--color-accent)]">
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

    <fieldset class="profile-modal-section">
      <legend>{i18n.t('profileModal.sectionConnection')}</legend>

      <label for="pm-kind" class="block text-[11px] text-[var(--color-fg-muted)] mb-1">{i18n.t('profileModal.kind')}</label>
      <select id="pm-kind" bind:value={profileKind} onchange={onProfileKindChange} class="input">
        <option value="ssh">{i18n.t('profileModal.kindSsh')}</option>
        <option value="rdp">{i18n.t('profileModal.kindRdp')}</option>
        <option value="vnc">{i18n.t('profileModal.kindVnc')}</option>
      </select>

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
            <div class="vault-locked-row mt-2">
              <p class="text-[10.5px] text-[var(--color-fg-muted)]">{i18n.t('profileModal.vaultLockedHint')}</p>
              {#if onOpenVault}
                <button type="button" class="btn-secondary text-[11px] py-0.5 px-2" onclick={() => onOpenVault?.()}>
                  {i18n.t('profileModal.vaultUnlock')}
                </button>
              {/if}
            </div>
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

        <div class="block text-[11px] text-[var(--color-fg-muted)] mb-1 mt-2">
          {i18n.t('profileModal.proxyJump')}
        </div>
        {#key editing?.id ?? 'new'}
          <JumpChainEditor
            profiles={tunnelProfiles}
            excludeProfileId={editing?.id}
            bind:jumpChainLines
            onError={onError}
          />
        {/key}
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
    </fieldset>

    <fieldset class="profile-modal-section">
      <legend>{i18n.t('profileModal.sectionIdentity')}</legend>

      <label for="pm-name" class="block text-[11px] text-[var(--color-fg-muted)] mb-1">{i18n.t('profileModal.name')}</label>
      <input id="pm-name" bind:value={name} required placeholder="prod web 01" class="input" />

      <div class="flex gap-3 mt-2">
        <div class="flex-1">
          <label for="pm-group" class="block text-[11px] text-[var(--color-fg-muted)] mb-1">{i18n.t('profileModal.group')}</label>
          <div class="group-input-row">
            <input id="pm-group" bind:value={group} placeholder={i18n.t('profileModal.groupPlaceholder')} class="input flex-1 min-w-0" />
            {#if group.trim()}
              <span
                class="group-color-preview"
                style={groupStyle(group.trim(), profileVisualsStore.overrides)}
                title={group.trim()}
                aria-hidden="true"
              ></span>
            {/if}
          </div>
          {#if group.trim()}
            <div class="profile-modal-color-row mt-1">
              <span class="text-[10px] text-[var(--color-fg-muted)]">{i18n.t('profileModal.groupColor')}</span>
              <VisualColorPicker
                compact
                value={profileVisualsStore.groupColors[normalizeGroupKey(group)] ?? null}
                onPick={(color) => {
                  void profileVisualsStore.setGroupColor(rpc, group.trim(), color);
                }}
              />
            </div>
          {/if}
        </div>
        <label class="favorite-row">
          <input type="checkbox" bind:checked={favorite} />
          <span>{i18n.t('profileModal.favorite')}</span>
        </label>
      </div>

      <label class="block text-[11px] text-[var(--color-fg-muted)] mb-1 mt-2">{i18n.t('profileModal.tags')}</label>
      <div class="profile-modal-tag-editor">
        <ProfileTagEditor
          {knownTags}
          selected={selectedTags}
          onSelectedChange={(tags) => { selectedTags = tags; }}
        />
      </div>
      {#if selectedTags.length > 0}
        <div class="profile-modal-tag-colors mt-1">
          <span class="text-[10px] text-[var(--color-fg-muted)]">{i18n.t('profileModal.tagColors')}</span>
          {#each selectedTags as tag (tag)}
            <div class="profile-modal-color-row">
              <ProfileTag {tag} compact />
              <VisualColorPicker
                compact
                value={profileVisualsStore.tagColors[normalizeTagKey(tag)] ?? null}
                onPick={(color) => {
                  void profileVisualsStore.setTagColor(rpc, tag, color);
                }}
              />
            </div>
          {/each}
        </div>
      {/if}

      <div class="mt-2">
        <label for="pm-icon-kind" class="block text-[11px] text-[var(--color-fg-muted)] mb-1">{i18n.t('profileModal.icon')}</label>
        <div class="flex items-center gap-2 flex-wrap">
          <ProfileIcon icon={{ kind: iconKind, value: iconValue }} {name} kind={profileKind} />
          <select id="pm-icon-kind" bind:value={iconKind} class="input min-w-[112px]">
            <option value="builtin">{i18n.t('profileModal.builtin')}</option>
            <option value="emoji">{i18n.t('profileModal.emoji')}</option>
            <option value="file">{i18n.t('profileModal.filePath')}</option>
            <option value="data">{i18n.t('profileModal.dataUri')}</option>
          </select>
          {#if iconKind === 'builtin'}
            <button
              type="button"
              class="btn-secondary text-[11px] py-0.5 px-2"
              onclick={() => { iconGridExpanded = !iconGridExpanded; }}
            >
              {iconGridExpanded ? i18n.t('profileModal.collapseIconGrid') : i18n.t('profileModal.expandIconGrid')}
            </button>
          {/if}
        </div>
        {#if iconKind === 'builtin'}
          {#if iconGridExpanded}
            <div class="profile-icon-grid mt-2" role="listbox" aria-label={i18n.t('profileModal.builtinIcon')}>
              {#each BUILTIN_PROFILE_ICONS as icon (icon)}
                <button
                  type="button"
                  role="option"
                  aria-selected={iconValue === icon}
                  class="profile-icon-grid-btn {iconValue === icon ? 'selected' : ''}"
                  style={`--profile-tone-fg:${builtinIconTone(icon).fg};--profile-tone-bg:${builtinIconTone(icon).bg};--profile-tone-border:${builtinIconTone(icon).border};`}
                  onclick={() => { iconValue = icon; }}
                >
                  <ProfileIcon icon={{ kind: 'builtin', value: icon }} name={icon} kind={profileKind} size={14} />
                  <span>{icon}</span>
                </button>
              {/each}
            </div>
          {/if}
        {:else}
          <label for="pm-icon-value" class="block text-[11px] text-[var(--color-fg-muted)] mb-1 mt-2">{i18n.t('profileModal.iconValue')}</label>
          <div class="flex gap-2">
            <input id="pm-icon-value" bind:value={iconValue} placeholder={iconKind === 'emoji' ? 'emoji or short text' : iconKind === 'remote' ? 'https://host/a.svg|https://host/b.png' : 'path, URL, name, or data URI'} class="input flex-1 min-w-0" />
            <button type="button" class="btn-secondary shrink-0 px-2" onclick={() => { void chooseIconFile(); }}>{i18n.t('profileModal.chooseIconFile')}</button>
          </div>
        {/if}
      </div>
    </fieldset>

    <fieldset class="profile-modal-section">
      <legend>{i18n.t('profileModal.sectionNote')}</legend>
      <label for="pm-note" class="block text-[11px] text-[var(--color-fg-muted)] mb-1">{i18n.t('profileModal.note')}</label>
      <textarea id="pm-note" bind:value={note} rows="3" placeholder={i18n.t('profileModal.notePlaceholder')} class="input resize-y min-h-[68px]"></textarea>
    </fieldset>

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
  .group-input-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .group-color-preview {
    width: 22px;
    height: 22px;
    border-radius: 6px;
    flex-shrink: 0;
    border: 1px solid var(--profile-tone-border, var(--color-border-soft));
    background: var(--profile-tone-bg, var(--color-panel-2));
    box-shadow: inset 3px 0 0 var(--profile-tone-fg, var(--color-accent));
  }
  .profile-modal-color-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .profile-modal-tag-colors {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .profile-modal-section {
    border: 1px solid var(--color-border-soft);
    border-radius: var(--radius-md, 6px);
    padding: 10px 12px 12px;
    margin: 0 0 10px;
    min-width: 0;
  }
  .profile-modal-section legend {
    padding: 0 4px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--color-fg-muted);
  }
  .vault-locked-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }
</style>
