<script lang="ts">
  import { onMount } from 'svelte';
  import { newProxyResource, proxyConfig, networkError, type NetworkRpc, type NetworkProfile, type ProxyResource, type ProxyConfig, type VaultMeta } from '../lib/networkResources';
  import { networkMessages, networkLocale, type NetworkLocale } from '../lib/networkMessages';
  let { rpc, profiles, onError, locale = networkLocale() }: { rpc: NetworkRpc; profiles: NetworkProfile[]; onError: (message: string) => void; locale?: NetworkLocale } = $props();
  const m = $derived(networkMessages[locale]);
  let resources = $state<ProxyResource[]>([]);
  let draft = $state<ProxyResource | null>(null);
  let vault = $state<VaultMeta[]>([]);
  let busy = $state(false);
  let confirmDelete = $state(false);
  let error = $state('');
  const sshProfiles = $derived(profiles.filter(p => p.kind === 'ssh'));
  async function perform(action: () => Promise<void>) { if (busy) return; busy = true; error = ''; try { await action(); } catch (e) { error = networkError(e); onError(error); } finally { busy = false; } }
  async function refresh() { resources = await rpc.call<ProxyResource[]>('proxy.list'); }
  function edit(resource: ProxyResource) { draft = JSON.parse(JSON.stringify(resource)); confirmDelete = false; error = ''; }
  function changeKind(kind: ProxyConfig['kind']) { if (draft) draft.config = proxyConfig(kind, sshProfiles[0]?.id); }
  async function save() { await perform(async () => { if (!draft || !draft.name.trim()) throw Error(m.invalid); await rpc.call('proxy.save', { resource: draft }); await refresh(); confirmDelete = false; }); }
  async function remove() { await perform(async () => { if (!draft) return; await rpc.call('proxy.delete', { id: draft.id }); draft = null; await refresh(); }); }
  onMount(() => { void perform(refresh); });
</script>
<section class="network-workspace" aria-label={m.proxies}>
  <header><h1>{m.proxies}</h1><button disabled={busy} onclick={() => { draft = newProxyResource(); confirmDelete = false; }}>{m.add}</button><button disabled={busy} onclick={() => perform(refresh)}>{m.refresh}</button></header>
  <p class="warning">{m.plainWarning}</p>
  <div class="columns"><nav aria-label={m.proxies}>
    {#each resources as resource (resource.id)}<button class:selected={draft?.id === resource.id} disabled={busy} onclick={() => edit(resource)}><strong>{resource.name}</strong><small>{m[resource.config.kind]}</small></button>{:else}<p>{m.empty}</p>{/each}
  </nav><div class="editor">
    {#if draft}
      <form onsubmit={event => { event.preventDefault(); void save(); }}>
        <label>{m.name}<input required maxlength="256" bind:value={draft.name} disabled={busy} /></label>
        <label>{m.kind}<select value={draft.config.kind} disabled={busy} onchange={event => changeKind(event.currentTarget.value as ProxyConfig['kind'])}><option value="socks5">{m.socks5}</option><option value="http_connect">{m.http_connect}</option><option value="ssh_jump">{m.ssh_jump}</option></select></label>
        {#if draft.config.kind === 'ssh_jump'}
          <label>{m.profile}<select required bind:value={draft.config.profile_id} disabled={busy}><option value="">{m.missingProfile}</option>{#each sshProfiles as profile (profile.id)}<option value={profile.id}>{profile.name}</option>{/each}</select></label>
        {:else}
          <label>{m.host}<input required bind:value={draft.config.host} disabled={busy} /></label>
          <label>{m.port}<input required type="number" min="1" max="65535" bind:value={draft.config.port} disabled={busy} /></label>
          <label>{m.vault}<select bind:value={draft.config.vault_entry_id} disabled={busy}><option value={null}>{m.noAuth}</option>{#if draft.config.vault_entry_id && !vault.some(entry => entry.id === (draft?.config as { vault_entry_id?: string }).vault_entry_id)}<option value={draft.config.vault_entry_id}>{draft.config.vault_entry_id}</option>{/if}{#each vault.filter(entry => entry.kind === 'password') as entry (entry.id)}<option value={entry.id}>{entry.label} — {entry.username ?? ''}</option>{/each}</select></label>
          <button type="button" disabled={busy} onclick={() => perform(async () => { vault = await rpc.call<VaultMeta[]>('vault.list'); })}>{m.loadVault}</button>
          <p>{m.vaultWarning}</p>
        {/if}
        <div class="actions"><button type="submit" disabled={busy}>{m.save}</button><button type="button" disabled={busy} onclick={() => { draft = null; }}>{m.cancel}</button><button type="button" disabled={busy || !resources.some(r => r.id === draft?.id)} onclick={() => { confirmDelete = true; }}>{m.remove}</button></div>
      </form>
      {#if confirmDelete}<div role="alert" class="warning"><p>{m.deleteWarning}</p><button disabled={busy} onclick={remove}>{m.confirmDelete}</button><button disabled={busy} onclick={() => { confirmDelete = false; }}>{m.cancel}</button></div>{/if}
    {:else}<p>{m.select}</p>{/if}
  </div></div>
  {#if error}<p role="alert" class="error">{error}</p>{/if}{#if busy}<p role="status">{m.busy}</p>{/if}
</section>
<style>
  .network-workspace{padding:24px;overflow:auto;height:100%;color:var(--text-primary,inherit)}header,.actions{display:flex;align-items:center;gap:10px}h1{font-size:20px;margin-right:auto}.columns{display:grid;grid-template-columns:minmax(180px,28%) 1fr;gap:24px;margin-top:20px}nav{display:flex;flex-direction:column;gap:8px}nav button{text-align:left;display:flex;flex-direction:column;gap:4px}.selected{outline:1px solid var(--accent,#6aa7ff)}form{display:grid;gap:14px;max-width:650px}label{display:grid;gap:6px}input,select,button{font:inherit;color:inherit;background:var(--bg-secondary,rgba(127,127,127,.1));border:1px solid var(--border-color,rgba(127,127,127,.3));border-radius:6px;padding:8px 10px}button:disabled{opacity:.5}button:not(:disabled){cursor:pointer}.warning{padding:12px;border-left:3px solid #d3a44d;background:rgba(211,164,77,.08);font-size:13px;line-height:1.6}.error{color:#ef7777}small{opacity:.65}p{line-height:1.6}@media(max-width:650px){.columns{grid-template-columns:1fr}}
</style>
