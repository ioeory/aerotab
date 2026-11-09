<script lang="ts">
  import { onMount, tick } from 'svelte';
  import type { RpcClient } from '../lib/rpc';
  import { i18n } from '../lib/i18n.svelte';
  import { selectApproval } from '../lib/mcpApprovalQueue';
  let { rpc }: { rpc: RpcClient } = $props();
  interface Plan { id: string; digest: string; script_name: string; command: string; scriptDigest: string; targets: {profile_id:string;target:string}[]; concurrency:number }
  let plans = $state<Plan[]>([]);
  let current = $state<Plan | null>(null);
  let error = $state('');
  let busy = $state(false);
  let dialog = $state<HTMLDialogElement | null>(null);
  $effect(() => {
    if (current) void tick().then(() => { if (dialog && !dialog.open) dialog.showModal(); });
  });
  onMount(() => {
    let stopped = false;
    let loading = false;
    async function refresh() {
      if (loading) return;
      loading = true;
      try { const pending = await rpc.call<Plan[]>('mcp.batchPending', {}); if (!stopped) { plans = pending; if (!busy) current = selectApproval(current, pending); } }
      catch { /* Older/headless endpoints have no local approval UI. Fail closed. */ }
      finally { loading = false; }
    }
    void refresh();
    const timer = setInterval(() => { void refresh(); }, 1500);
    return () => { stopped = true; clearInterval(timer); };
  });
  async function decide(plan:Plan, approved:boolean) {
    if (busy) return;
    busy = true; error = '';
    try { await rpc.call('mcp.batchApprove', { id:plan.id, digest:plan.digest, approved }); plans = plans.filter(p => p.id !== plan.id); current = selectApproval(null, plans); }
    catch { error = i18n.t('mcp.batch.stale'); }
    finally { busy = false; }
  }
</script>

{#if current}
  {@const plan = current}
  <div class="approval-backdrop">
    <dialog bind:this={dialog} aria-label={i18n.t('mcp.batch.title')} class="approval-dialog" oncancel={(event) => { event.preventDefault(); void decide(plan,false); }}>
      <h2>{i18n.t('mcp.batch.title')}</h2>
      <p>{i18n.t('mcp.batch.warning')}</p>
      <h3>{plan.script_name}</h3>
      <pre>{plan.command}</pre>
      <p>{i18n.t('mcp.batch.digest')}: <code>{plan.scriptDigest}</code></p>
      <p>{i18n.t('mcp.batch.targets')} ({plan.targets.length}) · {i18n.t('mcp.batch.concurrency')}: {plan.concurrency}</p>
      <ul>{#each plan.targets as target (target.profile_id)}<li>{target.target} <small>{target.profile_id}</small></li>{/each}</ul>
      <p>{i18n.t('mcp.batch.expiry')}</p>
      {#if error}<p role="alert">{error}</p>{/if}
      <div class="actions">
        <button type="button" class="btn-secondary" disabled={busy} onclick={() => decide(plan,false)}>{i18n.t('mcp.batch.reject')}</button>
        <button type="button" class="btn-primary" disabled={busy} onclick={() => decide(plan,true)}>{i18n.t('mcp.batch.approve')}</button>
      </div>
    </dialog>
  </div>
{/if}

<style>
  .approval-backdrop { position:fixed; inset:0; z-index:100; display:grid; place-items:center; background:#0009; }
  .approval-dialog { width:min(760px,90vw); max-height:85vh; overflow:auto; padding:24px; border:1px solid var(--color-border); border-radius:10px; background:var(--color-panel); color:var(--color-fg); }
  h2 { font-size:18px; margin-bottom:12px; } h3 { margin-top:16px; }
  p,li { margin:8px 0; font-size:13px; } small { color:var(--color-fg-muted); }
  pre { white-space:pre-wrap; overflow-wrap:anywhere; padding:12px; background:var(--color-bg-soft); max-height:240px; overflow:auto; }
  code { overflow-wrap:anywhere; } .actions { display:flex; justify-content:flex-end; gap:12px; margin-top:20px; }
</style>
