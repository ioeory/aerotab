<script lang="ts">
  import { FolderOpen } from '@lucide/svelte';
  import type { Snippet } from 'svelte';
  import type { RpcClient } from '../../lib/rpc';
  import { uuidv4 } from '../../lib/rpc';
  import type { StoredProfile, SshInheritField, ProfileGroupDefaults } from '../../lib/types';
  import { i18n } from '../../lib/i18n.svelte';
  import { profileToEditorDraft, profileFromEditorDraft, authFromDraft, resolveProfileEditorJumps, previewProfileInheritance } from '../../lib/profileCatalogHelpers';
  import { defaultPortForKind } from '../../lib/profileDefaults';
  import { formatJumpLinesForEdit, splitJumpLines, joinJumpLines, parseJumpLines, loadProfilesForJumps } from '../../lib/jumpProfiles';
  import { BUILTIN_PROFILE_ICONS, collectProfileTags, normalizeTags, parseProfileIconInput, suggestDuplicateProfileName } from '../../lib/profileMeta';
  import { pickIconFilePath, pickPrivateKeyPath } from '../../lib/localFiles';
  import { builtinIconTone, groupStyle, normalizeGroupKey, normalizeTagKey } from '../../lib/profileVisuals';
  import { profileVisualsStore } from '../../lib/profileVisualsStore.svelte';
  import { getProfileCatalog } from '../../lib/profileCatalog.svelte';
  import type { ProfileModalOpenOptions } from '../ProfileModal.svelte';
  import JumpChainEditor from '../JumpChainEditor.svelte';
  import ProfileIcon from '../ProfileIcon.svelte';
  import ProfileTag from '../ProfileTag.svelte';
  import ProfileTagEditor from '../ProfileTagEditor.svelte';
  import VisualColorPicker from '../VisualColorPicker.svelte';

  let { rpc, onSaved, onError, onOpenVault, onCancel, header, saving = $bindable(false) }: {
    rpc: RpcClient; onSaved: () => void; onError: (message: string) => void; onOpenVault?: () => void;
    onCancel: () => void; header?: Snippet; saving?: boolean;
  } = $props();
  let draft = $state(profileToEditorDraft());
  let editingId = $state<string | undefined>();
  let profiles = $state<StoredProfile[]>([]);
  const tunnels = $derived(profiles.filter(p => p.kind === 'ssh'));
  const knownTags = $derived(collectProfileTags(profiles));
  let jumps = $state<string[]>([]);
  let originalLines = '';
  let revision = $state(0);
  let originalJumps = profileToEditorDraft().jumps;
  let initialIconKind = 'builtin';
  let initialIconValue = 'server';
  let proxies = $state<Array<{ id: string; name: string }>>([]);
  let vaultUnlocked = $state(false);
  let vaultEntries = $state<Array<{ id: string; label: string; kind: string }>>([]);
  let iconKind = $state('builtin');
  let iconValue = $state('server');
  let iconGridExpanded = $state(false);
  let groupDefaults = $state<ProfileGroupDefaults>({});
  let inheritanceLoaded = $state(false);
  const inheritanceFields: SshInheritField[] = ['user', 'port', 'auth', 'proxy_id', 'jump'];
  const inheritanceLabels: Record<SshInheritField, string> = { user: 'profileModal.user', port: 'profileModal.port', auth: 'profileModal.authMethod', proxy_id: 'shell.proxies', jump: 'profileModal.proxyJump' };
  const inheritedPreview = $derived(previewProfileInheritance(profileFromEditorDraft(draft, editingId ?? 'preview'), groupDefaults));
  function toggleInheritance(field: SshInheritField, checked: boolean) {
    draft.inherit = checked ? [...new Set([...draft.inherit, field])] : draft.inherit.filter(value => value !== field);
  }
  function inherited(field: SshInheritField): boolean {
    return draft.inherit.includes(field) && !!inheritedPreview.sources[field];
  }
  function previewValue(field: SshInheritField): string {
    if (inheritedPreview.profile.kind !== 'ssh') return '';
    const ssh = inheritedPreview.profile.ssh;
    if (field === 'user') return ssh.user || '—';
    if (field === 'port') return String(ssh.port);
    if (field === 'auth') return typeof ssh.auth === 'string' ? i18n.t('ssh.agent') : 'VaultRef' in ssh.auth ? i18n.t('profileModal.vault') : 'PublicKey' in ssh.auth ? i18n.t('profileModal.publicKey') : i18n.t('profileModal.password');
    if (field === 'proxy_id') return proxies.find(proxy => proxy.id === ssh.proxy_id)?.name ?? ssh.proxy_id ?? i18n.t('groupDefaults.direct');
    return ssh.jump_profile_ids?.map(id => profiles.find(profile => profile.id === id)?.name ?? id).join(' → ') || (ssh.jump_via.length ? ssh.jump_via.map(hop => hop.host).join(' → ') : i18n.t('groupDefaults.direct'));
  }

  export function load(existing?: StoredProfile, options?: ProfileModalOpenOptions) {
    const current = ++revision;
    const source = options?.duplicateFrom ?? existing;
    groupDefaults = {};
    inheritanceLoaded = false;
    void rpc.call<{ value: ProfileGroupDefaults | null }>('settings.get', { key: 'profile.groupDefaults' }).then(result => {
      if (current !== revision) return;
      groupDefaults = result.value ?? {};
      inheritanceLoaded = true;
    }).catch(error => { if (current === revision) onError((error as Error).message); });
    draft = profileToEditorDraft(source, options?.group?.trim());
    draft.tags = normalizeTags(draft.tags);
    editingId = options?.duplicateFrom ? undefined : existing?.id;
    if (options?.duplicateFrom) {
      draft.name = suggestDuplicateProfileName(source!.name, options.existingNames ?? []);
      draft.favorite = false;
    }
    iconKind = source?.icon?.kind ?? 'builtin';
    iconValue = source?.icon?.value ?? 'server';
    iconGridExpanded = false;
    jumps = draft.jumpProfileIds.length ? draft.jumpProfileIds.map(id => `@${id}`) : splitJumpLines(formatJumpLinesForEdit(draft.jumps, profiles));
    originalLines = joinJumpLines(jumps);
    originalJumps = JSON.parse(JSON.stringify(draft.jumps));
    initialIconKind = iconKind;
    initialIconValue = iconValue;
    void rpc.call<StoredProfile[]>('profile.list').then(list => { if (current === revision) profiles = list; }).catch(() => {});
    void rpc.call<typeof proxies>('proxy.list').then(list => { if (current === revision) proxies = list; }).catch(() => {});
    vaultUnlocked = false;
    vaultEntries = [];
    void (async () => {
      try {
        const status = await rpc.call<{ initialized: boolean; unlocked: boolean }>('vault.status', {});
        if (current !== revision) return;
        vaultUnlocked = status.initialized && status.unlocked;
        if (vaultUnlocked) {
          const entries = await rpc.call<typeof vaultEntries>('vault.list', {});
          if (current === revision) vaultEntries = entries;
        }
      } catch { /* Vault is optional and never erases the loaded reference. */ }
    })();
  }

  function kindChanged() {
    draft.port = defaultPortForKind(draft.kind);
  }
  async function choosePath(icon: boolean) {
    try {
      const path = await (icon ? pickIconFilePath() : pickPrivateKeyPath());
      if (!path) return;
      if (icon) { iconKind = 'file'; iconValue = path; } else draft.auth.keyPath = path;
    } catch (error) { onError(i18n.t(icon ? 'profileModal.saveFailed' : 'profileModal.browsePrivateKeyFailed', { message: (error as Error).message })); }
  }
  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (saving) return;
    if (draft.kind === 'ssh' && !inherited('auth') && draft.auth.kind === 'vault' && !draft.auth.vaultEntryId.trim()) { onError(i18n.t('profileModal.vaultEntry')); return; }
    saving = true;
    try {
      if (draft.kind === 'ssh') {
        if (!inherited('jump') && joinJumpLines(jumps) !== originalLines) {
          const list = await loadProfilesForJumps(rpc);
          // Only explicit @ references become IDs. Legacy literals remain snapshots.
          const references = jumps.map(line => line.trim()).filter(Boolean);
          if (references.length && references.every(line => line.startsWith('@'))) {
            draft.jumpProfileIds = references.map(line => {
              const ref = line.slice(1).trim();
              const hit = list.find(p => p.id === ref) ?? list.find(p => p.name === ref);
              if (!hit || hit.kind !== 'ssh') throw Error(`Jump profile not found: ${ref}`);
              return hit.id;
            });
            draft.jumps = [];
          } else {
            draft.jumpProfileIds = [];
            draft.jumps = await resolveProfileEditorJumps(joinJumpLines(jumps), originalLines, originalJumps,
              async () => parseJumpLines(joinJumpLines(jumps), authFromDraft(draft.auth), list));
          }
        }
      }
      if (iconKind !== initialIconKind || iconValue !== initialIconValue || !editingId) draft.icon = iconKind === 'builtin' ? parseProfileIconInput(iconValue) : iconKind === 'selfhst' ? parseProfileIconInput(`selfhst:${iconValue}`) : iconValue.trim() ? { kind: iconKind, value: iconValue.trim() } : null;
      const profile = profileFromEditorDraft(draft, editingId ?? uuidv4());
      await getProfileCatalog(rpc).upsert(profile);
      onSaved();
    } catch (error) { onError(i18n.t('profileModal.saveFailed', { message: (error as Error).message })); }
    finally { saving = false; }
  }
</script>

<form onsubmit={submit} use:validation aria-busy={saving}>
  {@render header?.()}
  {#key revision}
  <details class="section" open>
    <summary>{i18n.t('profileModal.sectionIdentity')}</summary>
    <div class="fields">
      <label for="pm-name">{i18n.t('profileModal.name')}</label><input id="pm-name" class="input" bind:value={draft.name} required placeholder="prod web 01" />
      <label for="pm-group">{i18n.t('profileModal.group')}</label>
      <div class="group-input-row"><input id="pm-group" class="input" bind:value={draft.group} placeholder={i18n.t('profileModal.groupPlaceholder')} />
        {#if draft.group.trim()}<span class="group-color-preview" style={groupStyle(draft.group, profileVisualsStore.overrides)} aria-hidden="true"></span>{/if}
      </div>
      {#if draft.group.trim()}<div class="color-row"><span>{i18n.t('profileModal.groupColor')}</span><VisualColorPicker compact value={profileVisualsStore.groupColors[normalizeGroupKey(draft.group)] ?? null} onPick={(color) => { void profileVisualsStore.setGroupColor(rpc, draft.group.trim(), color); }} /></div>{/if}
      <label class="favorite-row"><input type="checkbox" bind:checked={draft.favorite} />{i18n.t('profileModal.favorite')}</label>
      <div id="pm-tags-label">{i18n.t('profileModal.tags')}</div>
      <div role="group" aria-labelledby="pm-tags-label"><ProfileTagEditor {knownTags} selected={draft.tags} onSelectedChange={(tags) => { draft.tags = tags; }} /></div>
      {#if draft.tags.length}<div class="color-row"><span>{i18n.t('profileModal.tagColors')}</span>
        {#each draft.tags as tag (tag)}<div class="color-row"><ProfileTag {tag} compact /><VisualColorPicker compact value={profileVisualsStore.tagColors[normalizeTagKey(tag)] ?? null} onPick={(color) => { void profileVisualsStore.setTagColor(rpc, tag, color); }} /></div>{/each}
      </div>{/if}
    </div>
  </details>
  {#if draft.kind === 'ssh'}
    <details class="section" open>
      <summary>{i18n.t('groupDefaults.title')}</summary>
      <div class="fields">
        <p>{i18n.t('groupDefaults.hostHint')}</p>
        {#each inheritanceFields as field (field)}
          <label class="inherit-row"><input type="checkbox" checked={draft.inherit.includes(field)} onchange={event => toggleInheritance(field, event.currentTarget.checked)} />{i18n.t('groupDefaults.inheritField', { field: i18n.t(inheritanceLabels[field]) })}</label>
          {#if draft.inherit.includes(field)}<p>{i18n.t('groupDefaults.effectiveValue')}: {inheritanceLoaded ? previewValue(field) : '…'} · {i18n.t('groupDefaults.source')}: {inheritanceLoaded ? inheritedPreview.sources[field] ?? i18n.t('groupDefaults.fallback') : '…'}</p>{/if}
        {/each}
      </div>
    </details>
  {/if}
  <details class="section" open>
    <summary>{i18n.t('profileModal.sectionConnection')}</summary>
    <div class="fields">
      <label for="pm-kind">{i18n.t('profileModal.kind')}</label>
      <select id="pm-kind" class="input" bind:value={draft.kind} onchange={kindChanged}><option value="ssh">{i18n.t('profileModal.kindSsh')}</option><option value="rdp">{i18n.t('profileModal.kindRdp')}</option><option value="vnc">{i18n.t('profileModal.kindVnc')}</option></select>
      <div class="endpoint"><div><label for="pm-host">{i18n.t('profileModal.host')}</label><input id="pm-host" class="input" bind:value={draft.host} required placeholder="example.com" /></div><div><label for="pm-port">{i18n.t('profileModal.port')}</label><input id="pm-port" class="input" bind:value={draft.port} disabled={draft.kind === 'ssh' && inherited('port')} type="number" min="1" max="65535" /></div></div>
      {#if draft.kind === 'ssh'}<label for="pm-user">{i18n.t('profileModal.user')}</label><input id="pm-user" class="input" bind:value={draft.user} disabled={inherited('user')} required={!inheritedPreview.sources.user} placeholder="root" />{/if}
    </div>
  </details>
  {#if draft.kind === 'ssh'}
    <details class="section" open>
      <summary>{i18n.t('profileModal.authMethod')}</summary>
      <fieldset class="fields" disabled={inherited('auth')}>
        <label for="pm-auth">{i18n.t('profileModal.authMethod')}</label>
        <select id="pm-auth" class="input" bind:value={draft.auth.kind}><option value="password">{i18n.t('profileModal.password')}</option><option value="key">{i18n.t('profileModal.publicKey')}</option><option value="vault">{i18n.t('profileModal.vault')}</option><option value="agent">{i18n.t('ssh.agent')}</option></select>
        {#if draft.auth.kind === 'password'}<label for="pm-pw">{i18n.t('profileModal.passwordStoredLocally')}</label><input id="pm-pw" class="input" type="password" bind:value={draft.auth.password} />
        {:else if draft.auth.kind === 'key'}
          <label for="pm-keypath">{i18n.t('profileModal.privateKeyPath')}</label><div class="path-field"><input id="pm-keypath" class="input" bind:value={draft.auth.keyPath} placeholder="~/.ssh/id_ed25519" /><button type="button" class="btn-secondary" onclick={() => { void choosePath(false); }}><FolderOpen size={16} />{i18n.t('profileModal.browsePrivateKey')}</button></div>
          <label for="pm-keypass">{i18n.t('profileModal.keyPassphrase')}</label><input id="pm-keypass" class="input" type="password" bind:value={draft.auth.keyPassphrase} />
        {:else if draft.auth.kind === 'vault'}
          {#if !vaultUnlocked}<div><p>{i18n.t('profileModal.vaultLockedHint')}</p>{#if onOpenVault}<button type="button" class="btn-secondary" onclick={onOpenVault}>{i18n.t('profileModal.vaultUnlock')}</button>{/if}</div>
          {:else}
            <label for="pm-vault-entry">{i18n.t('profileModal.vaultEntry')}</label><select id="pm-vault-entry" class="input" bind:value={draft.auth.vaultEntryId}><option value="">—</option>{#if draft.auth.vaultEntryId && !vaultEntries.some(entry => entry.id === draft.auth.vaultEntryId)}<option value={draft.auth.vaultEntryId}>{draft.auth.vaultEntryId}</option>{/if}{#each vaultEntries as entry (entry.id)}<option value={entry.id}>{entry.label} ({entry.kind})</option>{/each}</select>
            <label for="pm-vault-pass">{i18n.t('profileModal.vaultPassphraseEntry')}</label><select id="pm-vault-pass" class="input" bind:value={draft.auth.vaultPassphraseEntryId}><option value="">—</option>{#if draft.auth.vaultPassphraseEntryId && !vaultEntries.some(entry => entry.id === draft.auth.vaultPassphraseEntryId)}<option value={draft.auth.vaultPassphraseEntryId}>{draft.auth.vaultPassphraseEntryId}</option>{/if}{#each vaultEntries.filter(entry => entry.kind === 'password') as entry (entry.id)}<option value={entry.id}>{entry.label}</option>{/each}</select>
          {/if}
        {/if}
      </fieldset>
    </details>
    <label for="pm-proxy">{i18n.t('shell.proxies')}</label><select id="pm-proxy" class="input" disabled={inherited('proxy_id')} bind:value={draft.proxyId}><option value="">—</option>{#if draft.proxyId && !proxies.some(proxy => proxy.id === draft.proxyId)}<option value={draft.proxyId}>{draft.proxyId}</option>{/if}{#each proxies as proxy (proxy.id)}<option value={proxy.id}>{proxy.name}</option>{/each}</select>
    <details class="section"><summary>{i18n.t('profileModal.proxyJump')}</summary><div class="fields">{#if inherited('jump')}<p>{previewValue('jump')}</p>{:else}<JumpChainEditor profiles={tunnels} excludeProfileId={editingId} bind:jumpChainLines={jumps} {onError} />{/if}</div></details>
  {:else}
    <details class="section"><summary>{i18n.t('profileModal.sshTunnel')}</summary><div class="fields">
      <label for="pm-tunnel">{i18n.t('profileModal.sshTunnel')}</label><select id="pm-tunnel" class="input" bind:value={draft.remoteSshProfileId}><option value="">(direct)</option>{#if draft.remoteSshProfileId && !tunnels.some(profile => profile.id === draft.remoteSshProfileId)}<option value={draft.remoteSshProfileId}>{draft.remoteSshProfileId}</option>{/if}{#each tunnels as profile (profile.id)}<option value={profile.id}>{profile.name}</option>{/each}</select>
      <label for="pm-lport">{i18n.t('profileModal.localBindPort')}</label><input id="pm-lport" class="input" bind:value={draft.localBindPort} type="number" min="1" max="65535" placeholder="auto" />
    </div></details>
  {/if}
  <details class="section"><summary>{i18n.t('profileModal.icon')}</summary><div class="fields">
    <div class="color-row"><ProfileIcon icon={{ kind: iconKind, value: iconValue }} name={draft.name} kind={draft.kind} />
      <label for="pm-icon-kind">{i18n.t('profileModal.icon')}</label><select id="pm-icon-kind" class="input" bind:value={iconKind}><option value="builtin">{i18n.t('profileModal.builtin')}</option><option value="emoji">{i18n.t('profileModal.emoji')}</option><option value="file">{i18n.t('profileModal.filePath')}</option><option value="data">{i18n.t('profileModal.dataUri')}</option><option value="remote">Remote URL</option><option value="selfhst">selfh.st</option></select>
    </div>
    {#if iconKind === 'builtin'}<button type="button" class="btn-secondary" aria-expanded={iconGridExpanded} onclick={() => { iconGridExpanded = !iconGridExpanded; }}>{i18n.t(iconGridExpanded ? 'profileModal.collapseIconGrid' : 'profileModal.expandIconGrid')}</button>
      {#if iconGridExpanded}<div class="profile-icon-grid" role="listbox" aria-label={i18n.t('profileModal.builtinIcon')}>{#each BUILTIN_PROFILE_ICONS as icon (icon)}<button type="button" role="option" aria-selected={iconValue === icon} class="profile-icon-grid-btn" class:selected={iconValue === icon} style={`--profile-tone-fg:${builtinIconTone(icon).fg};--profile-tone-bg:${builtinIconTone(icon).bg};--profile-tone-border:${builtinIconTone(icon).border};`} onclick={() => { iconValue = icon; }}><ProfileIcon icon={{ kind: 'builtin', value: icon }} name={icon} kind={draft.kind} size={16} /><span>{icon}</span></button>{/each}</div>{/if}
    {:else}<label for="pm-icon-value">{i18n.t('profileModal.iconValue')}</label><div class="path-field"><input id="pm-icon-value" class="input" bind:value={iconValue} placeholder={iconKind === 'remote' ? 'https://host/a.svg|https://host/b.png' : 'path, URL, name, or data URI'} /><button type="button" class="btn-secondary" onclick={() => { void choosePath(true); }}>{i18n.t('profileModal.chooseIconFile')}</button></div>{/if}
  </div></details>
  <details class="section"><summary>{i18n.t('profileModal.sectionNote')}</summary><div class="fields"><label for="pm-note">{i18n.t('profileModal.note')}</label><textarea id="pm-note" class="input" bind:value={draft.note} rows="4" placeholder={i18n.t('profileModal.notePlaceholder')}></textarea></div></details>
  {/key}
  <footer><button type="button" class="btn-secondary" disabled={saving} onclick={onCancel}>{i18n.t('common.cancel')}</button><button type="submit" class="btn-primary" disabled={saving}>{i18n.t('common.save')}</button></footer>
</form>

<script lang="ts" module>
  // Kept local to the form; invalid does not bubble and must be observed in capture phase.
  function validation(form: HTMLFormElement) {
    const reveal = (event: Event) => {
      if (!(event.target instanceof HTMLElement)) return;
      let parent = event.target.parentElement;
      while (parent) { if (parent instanceof HTMLDetailsElement) parent.open = true; parent = parent.parentElement; }
    };
    form.addEventListener('invalid', reveal, true);
    return { destroy: () => form.removeEventListener('invalid', reveal, true) };
  }
</script>

<style>
  form { min-height: 100%; padding: 0 24px; font-size: 16px; line-height: 1.5; }
  .section { border: 1px solid var(--color-border-soft); border-radius: 8px; margin: 0 0 16px; min-width: 0; }
  summary { padding: 14px 16px; font-weight: 600; cursor: pointer; }
  summary:focus-visible { outline: 2px solid var(--color-accent); outline-offset: -2px; }
  .fields { padding: 0 16px 18px; display: flex; flex-direction: column; gap: 8px; min-width: 0; }
  label { color: var(--color-fg); }
  fieldset { border: 0; margin: 0; }
  fieldset:disabled { opacity: 0.55; }
  .inherit-row { display: flex; align-items: center; gap: 8px; }
  .input { min-width: 0; font-size: 16px; min-height: 40px; }
  button { min-height: 40px; font-size: 16px; }
  p { font-size: 14px; }
  .endpoint { display: grid; grid-template-columns: minmax(0, 1fr) 110px; gap: 12px; }
  .endpoint > div { display: flex; flex-direction: column; gap: 8px; }
  .color-row, .group-input-row, .path-field, .favorite-row { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .color-row { flex-wrap: wrap; font-size: 14px; }
  .group-input-row .input, .path-field .input { flex: 1; min-width: 0; }
  .path-field button { display: flex; gap: 6px; align-items: center; }
  .group-color-preview { width: 22px; height: 22px; flex-shrink: 0; border-radius: 6px; border: 1px solid var(--profile-tone-border); background: var(--profile-tone-bg); box-shadow: inset 3px 0 0 var(--profile-tone-fg); }
  footer { display: flex; justify-content: flex-end; gap: 8px; position: sticky; bottom: 0; z-index: 2; background: var(--color-panel); padding: 16px 24px; margin: 24px -24px 0; border-top: 1px solid var(--color-border-soft); }
  @media (max-width: 520px) { form { padding: 0 16px; } .path-field { flex-wrap: wrap; } }
</style>
