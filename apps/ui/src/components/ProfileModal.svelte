<script lang="ts">
  import { X } from '@lucide/svelte';
  import type { RpcClient } from '../lib/rpc';
  import { uuidv4 } from '../lib/rpc';
  import type { StoredProfile, SshAuth, SshProfileSpec } from '../lib/types';
  import { i18n } from '../lib/i18n.svelte';
  import { BUILTIN_PROFILE_ICONS, formatTags, parseTagsInput } from '../lib/profileMeta';
  import ProfileIcon from './ProfileIcon.svelte';

  interface Props {
    rpc: RpcClient;
    onSaved: () => void;
    onError: (msg: string) => void;
  }
  let { rpc, onSaved, onError }: Props = $props();

  let dialog: HTMLDialogElement | null = null;
  let editing = $state<StoredProfile | null>(null);
  let name = $state('');
  let group = $state('');
  let tagsText = $state('');
  let favorite = $state(false);
  let iconKind = $state<'builtin' | 'emoji' | 'file' | 'data'>('builtin');
  let iconValue = $state('server');
  let host = $state('');
  let port = $state(22);
  let user = $state('');
  let authKind = $state<'password' | 'key'>('password');
  let password = $state('');
  let keyPath = $state('');
  let keyPassphrase = $state('');
  /** One bastion per line, in `user@host[:port]` form. Each hop reuses the
   * target profile's auth method (key or password). */
  let jumpsText = $state('');

  function formatJumps(jumps: SshProfileSpec[]): string {
    return jumps
      .map((j) => `${j.user}@${j.host}${j.port === 22 ? '' : ':' + j.port}`)
      .join('\n');
  }

  function parseJumps(text: string, auth: SshAuth): SshProfileSpec[] {
    return text
      .split('\n')
      .map((l) => l.trim())
      .filter((l) => l.length > 0)
      .map((line) => {
        const at = line.indexOf('@');
        if (at < 0) throw new Error(`jump host "${line}" missing user@`);
        const u = line.slice(0, at);
        const rest = line.slice(at + 1);
        const colon = rest.lastIndexOf(':');
        const h = colon >= 0 ? rest.slice(0, colon) : rest;
        const p = colon >= 0 ? Number(rest.slice(colon + 1)) || 22 : 22;
        return { host: h, port: p, user: u, auth, jump_via: [] };
      });
  }

  export function open(existing?: StoredProfile) {
    editing = existing ?? null;
    if (existing) {
      name = existing.name;
      group = existing.group ?? '';
      tagsText = formatTags(existing.tags);
      favorite = !!existing.favorite;
      iconKind = (existing.icon?.kind as typeof iconKind) ?? 'builtin';
      iconValue = existing.icon?.value ?? 'server';
      host = existing.ssh.host;
      port = existing.ssh.port;
      user = existing.ssh.user;
      jumpsText = formatJumps(existing.ssh.jump_via ?? []);
      if (typeof existing.ssh.auth === 'object' && 'Password' in existing.ssh.auth) {
        authKind = 'password';
        password = existing.ssh.auth.Password.secret;
        keyPath = '';
        keyPassphrase = '';
      } else if (typeof existing.ssh.auth === 'object' && 'PublicKey' in existing.ssh.auth) {
        authKind = 'key';
        keyPath = existing.ssh.auth.PublicKey.key_path;
        keyPassphrase = existing.ssh.auth.PublicKey.passphrase ?? '';
        password = '';
      } else {
        authKind = 'key';
        keyPath = '';
        keyPassphrase = '';
        password = '';
      }
    } else {
      name = '';
      group = '';
      tagsText = '';
      favorite = false;
      iconKind = 'builtin';
      iconValue = 'server';
      host = '';
      port = 22;
      user = '';
      authKind = 'password';
      password = '';
      keyPath = '';
      keyPassphrase = '';
      jumpsText = '';
    }
    dialog?.showModal();
  }

  function close() {
    dialog?.close();
  }

  async function submit(ev: Event) {
    ev.preventDefault();
    const auth: SshAuth =
      authKind === 'key'
        ? { PublicKey: { key_path: keyPath, passphrase: keyPassphrase || undefined } }
        : { Password: { secret: password } };
    let jump_via: SshProfileSpec[];
    try {
      jump_via = parseJumps(jumpsText, auth);
    } catch (e) {
      onError((e as Error).message);
      return;
    }
    const profile: StoredProfile = {
      schemaVersion: 1,
      id: editing?.id ?? uuidv4(),
      name: name || 'profile',
      group: group.trim() || null,
      tags: parseTagsInput(tagsText),
      favorite,
      icon: iconValue.trim() ? { kind: iconKind, value: iconValue.trim() } : null,
      kind: 'ssh',
      ssh: { host, port: Number(port) || 22, user, auth, jump_via },
    };
    try {
      await rpc.call('profile.upsert', profile);
      close();
      onSaved();
    } catch (e) {
      onError(i18n.t('profileModal.saveFailed', { message: (e as Error).message }));
    }
  }
</script>

<dialog bind:this={dialog} class="min-w-[420px]">
  <form onsubmit={submit} class="p-5">
    <div class="flex items-center justify-between mb-3">
      <h2 class="text-[14px] font-semibold text-[var(--color-accent)]">
        {editing ? i18n.t('profileModal.editTitle') : i18n.t('profileModal.newTitle')}
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

    <label for="pm-name" class="block text-[11px] text-[var(--color-fg-muted)] mb-1 mt-2">{i18n.t('profileModal.name')}</label>
    <input id="pm-name" bind:value={name} required placeholder="prod web 01" class="input" />

    <div class="flex gap-3 mt-2">
      <div class="flex-1">
        <label for="pm-group" class="block text-[11px] text-[var(--color-fg-muted)] mb-1">{i18n.t('profileModal.group')}</label>
        <input id="pm-group" bind:value={group} placeholder="prod / customer-a" class="input" />
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

    <label for="pm-user" class="block text-[11px] text-[var(--color-fg-muted)] mb-1 mt-2">{i18n.t('profileModal.user')}</label>
    <input id="pm-user" bind:value={user} required placeholder="root" class="input" />

    <label for="pm-auth" class="block text-[11px] text-[var(--color-fg-muted)] mb-1 mt-2">{i18n.t('profileModal.authMethod')}</label>
    <select id="pm-auth" bind:value={authKind} class="input">
      <option value="password">{i18n.t('profileModal.password')}</option>
      <option value="key">{i18n.t('profileModal.publicKey')}</option>
    </select>

    {#if authKind === 'password'}
      <label for="pm-pw" class="block text-[11px] text-[var(--color-fg-muted)] mb-1 mt-2">
        {i18n.t('profileModal.passwordStoredLocally')}
      </label>
      <input id="pm-pw" type="password" bind:value={password} class="input" />
    {:else}
      <label for="pm-keypath" class="block text-[11px] text-[var(--color-fg-muted)] mb-1 mt-2">{i18n.t('profileModal.privateKeyPath')}</label>
      <input id="pm-keypath" bind:value={keyPath} placeholder="~/.ssh/id_ed25519" class="input" />
      <label for="pm-keypass" class="block text-[11px] text-[var(--color-fg-muted)] mb-1 mt-2">
        {i18n.t('profileModal.keyPassphrase')}
      </label>
      <input id="pm-keypass" type="password" bind:value={keyPassphrase} class="input" />
    {/if}

    <label for="pm-jumps" class="block text-[11px] text-[var(--color-fg-muted)] mb-1 mt-2">
      {i18n.t('profileModal.proxyJump')}
    </label>
    <textarea
      id="pm-jumps"
      bind:value={jumpsText}
      rows="2"
      placeholder="jumpuser@bastion.example.com&#10;deep@inner.gw:2222"
      class="input font-mono text-[11.5px]"
    ></textarea>

    <div class="flex justify-end gap-2 mt-5">
      <button type="button" onclick={close} class="btn-secondary">{i18n.t('common.cancel')}</button>
      <button type="submit" class="btn-primary">{i18n.t('common.save')}</button>
    </div>
  </form>
</dialog>

<style>
  .input {
    width: 100%;
    background: var(--color-bg);
    color: var(--color-fg);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    padding: 6px 10px;
    font: inherit;
    outline: none;
  }
  .input:focus { border-color: var(--color-accent); }
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
  .btn-primary {
    background: var(--color-accent); color: var(--color-bg);
    border: none; padding: 6px 14px; border-radius: var(--radius-sm);
    font-weight: 600; cursor: pointer;
  }
  .btn-primary:hover { filter: brightness(1.08); }
  .btn-secondary {
    background: var(--color-panel-2); color: var(--color-fg);
    border: 1px solid var(--color-border); padding: 6px 14px;
    border-radius: var(--radius-sm); cursor: pointer;
  }
  .btn-secondary:hover { background: var(--color-border); }
</style>
