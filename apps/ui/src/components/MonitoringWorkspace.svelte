<script lang="ts">
  import type { RpcClient } from '../lib/rpc';
  import type { StoredProfile } from '../lib/types';
  import type { MonitoringSample, MonitoringPanel } from '../lib/monitoringTypes';
  import { getHostMonitoring } from '../lib/hostMonitoring.svelte';
  import { monitoringMessage as m, monitoringNumber as number, monitoringBytes as bytes } from '../lib/monitoringMessages';
  import MonitoringHostPicker from './MonitoringHostPicker.svelte';
  import MonitoringChart from './MonitoringChart.svelte';
  import HostSystemPanel from './HostSystemPanel.svelte';
  let { rpc, profiles, onError }: { rpc: RpcClient; profiles: StoredProfile[]; onError: (message: string) => void } = $props();
  const monitoring = $derived(getHostMonitoring(rpc));
  const panel = $derived(monitoring.panels.find(item => item.id === monitoring.selectedPanelId));
  const running = $derived(!!panel && monitoring.running.includes(panel.id));
  let picker = $state<'new' | 'edit' | null>(null);
  let nameMode = $state<'new' | 'rename' | null>(null);
  let draftName = $state('');
  let confirmDelete = $state(false);
  let detailProfileId = $state<string | null>(null);
  let clock = $state(Date.now());
  $effect(() => { monitoring.setProfiles(profiles); });
  $effect(() => { const timer = setInterval(() => { clock = Date.now(); }, 1000); return () => clearInterval(timer); });
  $effect(() => { if (monitoring.persistenceError) onError(m('persistenceError')); });
  const rows = $derived.by(() => {
    void monitoring.revision;
    if (!panel) return [];
    return panel.profileIds.map(id => ({ id, profile: profiles.find(p => p.id === id), snapshot: monitoring.service.snapshot(id) }))
      .filter(row => `${row.profile?.name ?? row.id} ${row.profile?.kind === 'ssh' ? row.profile.ssh.host : ''}`.toLowerCase().includes(panel.search.toLowerCase()))
      .sort((a,b) => panel.sort === 'cpu' ? (b.snapshot.latest?.stats.cpu_percent ?? -1) - (a.snapshot.latest?.stats.cpu_percent ?? -1) : panel.sort === 'memory' ? (b.snapshot.latest?.stats.mem_percent ?? -1) - (a.snapshot.latest?.stats.mem_percent ?? -1) : (a.profile?.name ?? a.id).localeCompare(b.profile?.name ?? b.id));
  });
  function update(patch: Partial<MonitoringPanel>): void { if (panel) monitoring.update(panel.id, patch); }
  function history(samples: MonitoringSample[]): MonitoringSample[] { return samples.filter(s => s.at >= clock - (panel?.rangeMinutes ?? 15) * 60000); }
  function newPanel(): void { draftName = `${m('title')} ${monitoring.panels.length + 1}`; nameMode = 'new'; }
  function nameConfirm(): void {
    if (!draftName.trim()) return;
    if (nameMode === 'new') { nameMode = null; picker = 'new'; }
    else { update({ name: draftName.trim() }); nameMode = null; }
  }
  function picked(ids: string[]): void {
    if (picker === 'new') monitoring.create(draftName.trim(), ids); else update({ profileIds: ids });
    picker = null;
  }
  function sumNetwork(sample: MonitoringSample | undefined): number | null {
    if (!sample?.network_rates.length || sample.network_rates.some(r => r.rx_bytes_per_second == null || r.tx_bytes_per_second == null)) return null;
    return sample.network_rates.reduce((total, row) => total + row.rx_bytes_per_second! + row.tx_bytes_per_second!, 0);
  }
  function status(hasSample: boolean, error: boolean): string {
    void monitoring.revision;
    return monitoring.service.isPaused() ? m('paused') : error ? m('probeFailed') : running ? hasSample ? m('sampling') : m('waiting') : hasSample ? m('stopped') : m('notStarted');
  }
</script>
<section class="monitoring-workspace" aria-label={m('title')}>
  <header class="workspace-header"><div><h1>{m('title')}</h1><p>{m('keepRunning')}</p></div><button class="primary" onclick={newPanel}>＋ {m('newPanel')}</button></header>
  <nav aria-label={m('title')} class="panels">
    {#each monitoring.panels as item (item.id)}<button class:active={panel?.id === item.id} aria-current={panel?.id === item.id ? 'page' : undefined} onclick={() => monitoring.select(item.id)}><span class:live={monitoring.running.includes(item.id)} class="dot"></span>{item.name}<small>{item.profileIds.length}</small></button>{/each}
  </nav>
  {#if monitoring.persistenceError}<p class="error" role="alert">{m('persistenceError')} <button onclick={() => { if (panel) monitoring.update(panel.id, {}); }}>{m('retry')}</button></p>{/if}
  {#if panel}
    <div class="panel-heading"><h2>{panel.name}</h2><span>{panel.profileIds.length} {m('hosts')}</span><div class="spacer"></div><button onclick={() => { draftName = panel.name; nameMode = 'rename'; }}>{m('rename')}</button><button onclick={() => { confirmDelete = true; }}>{m('delete')}</button></div>
    {#if confirmDelete}<div class="confirm" role="alert"><span>{m('deleteConfirm')}</span><button onclick={() => { monitoring.remove(panel.id); confirmDelete = false; }}>{m('confirm')}</button><button onclick={() => { confirmDelete = false; }}>{m('cancel')}</button></div>{/if}
    <div class="toolbar">
      <button class:primary={!running} onclick={() => running ? monitoring.stop(panel.id) : monitoring.start(panel.id)} disabled={!panel.profileIds.length}>{running ? '■ ' + m('stop') : '▶ ' + m('start')}</button>
      <button onclick={() => { picker = 'edit'; }}>{m('editHosts')}</button>
      <label>{m('interval')}<select value={panel.intervalMs} onchange={(event) => update({ intervalMs: Number(event.currentTarget.value) })}>{#each [1000,2000,5000,10000,30000,60000] as interval}<option value={interval}>{interval/1000} {m('seconds')}</option>{/each}</select></label>
      <label>{m('range')}<select value={panel.rangeMinutes} onchange={(event) => update({ rangeMinutes: Number(event.currentTarget.value) })}>{#each [5,15,30,60] as range}<option value={range}>{range} {m('minutes')}</option>{/each}</select></label>
      <label>{m('sort')}<select value={panel.sort} onchange={(event) => update({ sort: event.currentTarget.value as MonitoringPanel['sort'] })}><option value="name">{m('name')}</option><option value="cpu">{m('cpu')}</option><option value="memory">{m('memory')}</option></select></label>
      <div class="spacer"></div><button aria-pressed={panel.view === 'list'} onclick={() => update({ view: 'list' })}>{m('list')}</button><button aria-pressed={panel.view === 'chart'} onclick={() => update({ view: 'chart' })}>{m('chart')}</button>
      <input class="search" aria-label={m('search')} placeholder={m('search')} value={panel.search} oninput={(event) => update({ search: event.currentTarget.value })} />
    </div>
    <div class="content">
      {#if panel.view === 'list'}
        <div class="table-scroll"><table><thead><tr><th>{m('name')}</th><th>{m('cpu')}</th><th>{m('memory')}</th><th>{m('disk')}</th><th>{m('network')} ↓↑</th><th>{m('latency')}</th><th>{m('lastSample')}</th><th></th></tr></thead><tbody>
          {#each rows as row (row.id)}
            <tr><td><button class="host-name" onclick={() => { detailProfileId = row.id; }}>{row.profile?.name ?? row.id}</button><small class:error={!row.profile || !!row.snapshot.error}>{!row.profile ? m('missingProfile') : status(!!row.snapshot.latest, !!row.snapshot.error)}</small></td><td>{number(row.snapshot.latest?.stats.cpu_percent,'%')}</td><td>{number(row.snapshot.latest?.stats.mem_percent,'%')}</td><td>{number(row.snapshot.latest?.stats.disk_percent,'%')}</td><td>{bytes(sumNetwork(row.snapshot.latest),true)}</td><td>{number(row.snapshot.latest?.probeMs,' ms')}</td><td>{row.snapshot.latest ? new Date(row.snapshot.latest.at).toLocaleTimeString() : '—'}</td><td><button aria-label={`${m('removeHost')}: ${row.profile?.name ?? row.id}`} onclick={() => update({ profileIds: panel.profileIds.filter(id => id !== row.id) })}>×</button></td></tr>
          {/each}
        </tbody></table></div>
      {:else}
        <div class="host-charts">{#each rows as row (row.id)}
          {@const samples = history(row.snapshot.history)}
          <article><header><div><button class="host-name" onclick={() => { detailProfileId = row.id; }}>{row.profile?.name ?? row.id}</button><small class:error={!!row.snapshot.error}>{status(!!row.snapshot.latest, !!row.snapshot.error)}</small></div><button aria-label={`${m('removeHost')}: ${row.profile?.name ?? row.id}`} onclick={() => update({ profileIds: panel.profileIds.filter(id => id !== row.id) })}>×</button></header>
            <div class="charts"><MonitoringChart {samples} metric="cpu" label={m('cpuHistory')} /><MonitoringChart {samples} metric="memory" label={m('memoryHistory')} /><MonitoringChart {samples} metric="disk" label={m('diskHistory')} /><MonitoringChart {samples} metric="network" label={m('networkHistory')} /></div>
            <p class="actual-range">{m('actualRange')}: {samples.length ? `${new Date(samples[0]!.at).toLocaleTimeString()} – ${new Date(samples.at(-1)!.at).toLocaleTimeString()}` : m('waiting')}</p>
          </article>
        {/each}</div>
      {/if}
      {#if !rows.length}<div class="empty">{panel.profileIds.length ? m('noResults') : m('noHosts')}<button onclick={() => { picker = 'edit'; }}>{m('addHosts')}</button></div>{/if}
    </div>
  {:else}<div class="empty"><h2>{m('title')}</h2><p>{m('empty')}</p><button class="primary" onclick={newPanel}>{m('newPanel')}</button></div>{/if}
  <footer>{m('historyHint')}</footer>
  {#if nameMode}<div class="name-overlay"><form class="name-form" onsubmit={(event) => { event.preventDefault(); nameConfirm(); }}><label>{m('panelName')}<input aria-label={m('panelName')} bind:value={draftName} maxlength="120" required /></label><button type="button" onclick={() => { nameMode = null; }}>{m('cancel')}</button><button class="primary" type="submit" disabled={!draftName.trim()}>{m('confirm')}</button></form></div>{/if}
  {#if picker}<MonitoringHostPicker {profiles} selected={picker === 'edit' ? panel?.profileIds ?? [] : []} onConfirm={picked} onClose={() => { picker = null; }} />{/if}
  {#if detailProfileId}<div class="system-detail"><button class="close-detail" onclick={() => { detailProfileId = null; }}>{m('cancel')} ×</button><HostSystemPanel {rpc} profileId={detailProfileId} {onError} /></div>{/if}
</section>
<style>
  .monitoring-workspace{position:relative;height:100%;min-width:0;display:flex;flex-direction:column;color:var(--fg,#e2e8f0);background:transparent;font-size:12px;overflow:hidden}.workspace-header{display:flex;align-items:center;justify-content:space-between;padding:24px 24px 14px;gap:20px}h1{font-size:22px;font-weight:600;letter-spacing:-.5px;margin:0}h2{font-size:15px;font-weight:600;margin:0}.workspace-header p{margin:7px 0 0;opacity:.55;font-size:11px}button,input,select{font:inherit;color:inherit;background:var(--surface,#ffffff08);border:1px solid var(--border,#ffffff18);border-radius:6px;padding:7px 10px}button{cursor:pointer}button:disabled{cursor:default;opacity:.4}button:hover:not(:disabled){background:var(--hover,#ffffff12)}button:focus-visible,input:focus-visible,select:focus-visible{outline:2px solid var(--accent,#4f8cff);outline-offset:2px}.primary{background:var(--accent,#4f8cff)!important;color:white}.panels{display:flex;flex-shrink:0;gap:6px;padding:0 24px 12px;overflow-x:auto;border-bottom:1px solid var(--border,#ffffff12)}.panels button{display:flex;align-items:center;gap:8px;white-space:nowrap;background:transparent;border-color:transparent}.panels button.active{border-color:var(--accent,#4f8cff);background:var(--surface,#ffffff08)}.panels small{opacity:.5}.dot{width:6px;height:6px;border-radius:50%;background:currentColor;opacity:.25}.dot.live{background:var(--success,#40b983);opacity:1}.panel-heading{padding:16px 24px 10px;display:flex;align-items:center;gap:12px}.panel-heading>span{opacity:.5;font-size:11px}.panel-heading button{font-size:11px}.spacer{flex:1}.toolbar{display:flex;flex-wrap:wrap;align-items:center;gap:8px;padding:0 24px 16px;border-bottom:1px solid var(--border,#ffffff12)}.toolbar label{display:flex;align-items:center;gap:6px;font-size:10px;color:var(--muted,#8893a6)}select{font-size:11px;color:var(--fg,#e2e8f0)}option{background:var(--bg,#171b24)}.toolbar button{font-size:11px}.toolbar [aria-pressed=true]{border-color:var(--accent,#4f8cff)}.search{width:160px;font-size:11px}.content{flex:1;min-height:0;overflow:auto;padding:0 24px}.table-scroll{overflow:auto}table{width:100%;border-collapse:collapse;font-variant-numeric:tabular-nums;text-align:right}th{padding:14px 12px;font-size:10px;font-weight:500;opacity:.6;white-space:nowrap}td{padding:14px 12px;border-top:1px solid var(--border,#ffffff12);white-space:nowrap;font-size:12px}td:first-child,th:first-child{text-align:left;max-width:250px}.host-name{font-size:13px;font-weight:600;border:0;padding:0;background:transparent!important;text-align:left;overflow-wrap:anywhere;white-space:normal}td small,article small{display:block;font-size:10px;opacity:.55;margin-top:5px;max-width:300px;white-space:normal}footer{padding:12px 24px;font-size:10px;opacity:.55;border-top:1px solid var(--border,#ffffff12)}.empty{display:flex;flex:1;flex-direction:column;align-items:center;justify-content:center;gap:18px;min-height:180px;opacity:.8}.host-charts{display:grid;grid-template-columns:repeat(auto-fit,minmax(min(100%,420px),1fr));gap:16px;padding:18px 0}article{border:1px solid var(--border,#ffffff18);border-radius:10px;padding:16px;background:var(--surface,#ffffff03);overflow:hidden}article header{display:flex;justify-content:space-between;align-items:flex-start;margin-bottom:22px;gap:10px}.charts{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:24px 20px}.actual-range{opacity:.5;font-size:9px;margin:16px 0 0}.error{color:var(--warning,#e7af63);padding:0 24px}.confirm{display:flex;align-items:center;justify-content:flex-end;gap:10px;padding:10px 24px;background:#c0701220}.confirm span{margin-right:auto}.name-overlay{position:absolute;inset:0;background:#0006;z-index:65;display:grid;place-items:center}.name-form{padding:24px;background:var(--bg,#171b24);border:1px solid var(--border,#384152);border-radius:10px;max-width:360px;width:90%;display:flex;gap:10px;flex-wrap:wrap;justify-content:flex-end}.name-form label{display:flex;flex-direction:column;gap:10px;width:100%;margin-bottom:8px}.system-detail{position:absolute;right:0;top:0;bottom:0;width:min(400px,100%);z-index:60;background:var(--bg,#171b24);box-shadow:-15px 0 40px #0003;display:flex;flex-direction:column}.system-detail :global(.system-panel){flex:1;min-height:0}.close-detail{align-self:flex-end;margin:8px 12px}@media(max-width:700px){.workspace-header,.panel-heading{padding-left:14px;padding-right:14px}.content{padding:0 14px}.toolbar{padding:0 14px 14px}.toolbar .spacer{display:none}.workspace-header p{max-width:210px}.charts{gap:20px 12px}}
</style>
