<script lang="ts">
  import { onMount } from 'svelte';
  import { newForwardingRule, networkError, type NetworkRpc, type NetworkProfile, type ForwardingRule, type ForwardingInstance } from '../lib/networkResources';
  import { networkMessages, networkLocale, type NetworkLocale } from '../lib/networkMessages';
  let { rpc, profiles, onError, locale = networkLocale() }: { rpc: NetworkRpc; profiles: NetworkProfile[]; onError: (message: string) => void; locale?: NetworkLocale } = $props();
  const m = $derived(networkMessages[locale]);
  let rules = $state<ForwardingRule[]>([]);
  let instances = $state<ForwardingInstance[]>([]);
  let draft = $state<ForwardingRule | null>(null);
  let busy = $state(false);
  let error = $state('');
  let confirmDelete = $state(false);
  const sshProfiles = $derived(profiles.filter(p => p.kind === 'ssh'));
  const instance = $derived(instances.find(item => item.rule_id === draft?.id));
  async function perform(action: () => Promise<void>) { if (busy) return; busy = true; error = ''; try { await action(); } catch (e) { error = networkError(e); onError(error); } finally { busy = false; } }
  async function refresh() { const [nextRules, nextInstances] = await Promise.all([rpc.call<ForwardingRule[]>('forwarding.list'), rpc.call<ForwardingInstance[]>('forwarding.running')]); rules = nextRules; instances = nextInstances; }
  function edit(rule: ForwardingRule) { draft = JSON.parse(JSON.stringify(rule)); confirmDelete = false; error = ''; }
  async function save() { await perform(async () => { if (!draft || !draft.name.trim() || !draft.profile_id) throw Error(m.invalid); await rpc.call('forwarding.save', { rule: draft }); await refresh(); }); }
  async function toggle() { await perform(async () => { if (!draft) return; await rpc.call(instance ? 'forwarding.stop' : 'forwarding.start', { id: draft.id }); await refresh(); }); }
  async function remove() { await perform(async () => { if (!draft) return; await rpc.call('forwarding.delete', { id: draft.id }); draft = null; await refresh(); }); }
  onMount(() => { void perform(refresh); });
</script>
<section class="network-workspace" aria-label={m.forwarding}>
  <header><h1>{m.forwarding}</h1><button disabled={busy} onclick={() => { draft = newForwardingRule(sshProfiles[0]?.id); confirmDelete = false; }}>{m.add}</button><button disabled={busy} onclick={() => perform(refresh)}>{m.refresh}</button></header>
  <p class="notice">{m.rulesWarning}</p>
  <div class="columns"><nav aria-label={m.forwarding}>
    {#each rules as rule (rule.id)}<button class:selected={draft?.id === rule.id} disabled={busy} onclick={() => edit(rule)}><strong>{rule.name}</strong><small>{m[rule.kind]} · {instances.some(item => item.rule_id === rule.id) ? m.running : m.stopped}</small></button>{:else}<p>{m.empty}</p>{/each}
  </nav><div>
    {#if draft}
      <form onsubmit={event => { event.preventDefault(); void save(); }}>
        <label>{m.name}<input required maxlength="256" bind:value={draft.name} disabled={busy} /></label>
        <label>{m.profile}<select required bind:value={draft.profile_id} disabled={busy}><option value="">{m.missingProfile}</option>{#each sshProfiles as profile (profile.id)}<option value={profile.id}>{profile.name}</option>{/each}</select></label>
        <label>{m.kind}<select bind:value={draft.kind} disabled={busy}><option value="local">{m.local}</option><option value="remote">{m.remote}</option><option value="dynamic">{m.dynamic}</option></select></label>
        <div class="pair"><label>{m.bindHost}<input required bind:value={draft.bind_host} disabled={busy} /></label><label>{m.bindPort}<input required type="number" min="0" max="65535" bind:value={draft.bind_port} disabled={busy} /></label></div>
        {#if !['127.0.0.1', '::1'].includes(draft.bind_host)}<p class="notice">{m.nonLoopback}</p>{/if}
        {#if draft.kind !== 'dynamic'}<div class="pair"><label>{m.targetHost}<input required bind:value={draft.target_host} disabled={busy} /></label><label>{m.targetPort}<input required type="number" min="1" max="65535" bind:value={draft.target_port} disabled={busy} /></label></div>{/if}
        <div class="actions"><button type="submit" disabled={busy}>{m.save}</button><button type="button" disabled={busy || !rules.some(r => r.id === draft?.id)} onclick={toggle}>{instance ? m.stop : m.start}</button><button type="button" disabled={busy || !!instance || !rules.some(r => r.id === draft?.id)} onclick={() => { confirmDelete = true; }}>{m.remove}</button><button type="button" disabled={busy} onclick={() => { draft = null; }}>{m.cancel}</button></div>
      </form>
      {#if instance}<p role="status">{m.runtime}: {instance.tunnel.bind_host}:{instance.tunnel.bind_port} · {instance.tunnel.status}</p>{/if}
      {#if confirmDelete}<div role="alert" class="notice"><p>{m.deleteWarning}</p><button disabled={busy} onclick={remove}>{m.confirmDelete}</button><button disabled={busy} onclick={() => { confirmDelete = false; }}>{m.cancel}</button></div>{/if}
    {:else}<p>{m.select}</p>{/if}
  </div></div>
  {#if error}<p role="alert" class="error">{error}</p>{/if}{#if busy}<p role="status">{m.busy}</p>{/if}
</section>
<style>
  .network-workspace{padding:24px;overflow:auto;height:100%;color:var(--text-primary,inherit)}header,.actions{display:flex;align-items:center;gap:10px}h1{font-size:20px;margin-right:auto}.columns{display:grid;grid-template-columns:minmax(180px,28%) 1fr;gap:24px;margin-top:20px}nav{display:flex;flex-direction:column;gap:8px}nav button{text-align:left;display:flex;flex-direction:column;gap:4px}.selected{outline:1px solid var(--accent,#6aa7ff)}form{display:grid;gap:14px;max-width:650px}label{display:grid;gap:6px}.pair{display:grid;grid-template-columns:1fr 1fr;gap:12px}input,select,button{font:inherit;color:inherit;background:var(--bg-secondary,rgba(127,127,127,.1));border:1px solid var(--border-color,rgba(127,127,127,.3));border-radius:6px;padding:8px 10px;min-width:0}button:disabled{opacity:.5}button:not(:disabled){cursor:pointer}.notice{padding:12px;border-left:3px solid #d3a44d;background:rgba(211,164,77,.08);font-size:13px;line-height:1.6}.error{color:#ef7777}small{opacity:.65}p{line-height:1.6}@media(max-width:650px){.columns,.pair{grid-template-columns:1fr}}
</style>
