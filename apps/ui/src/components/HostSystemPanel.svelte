<script lang="ts">
  import { untrack } from 'svelte';
  import { Cpu, HardDrive, Network, Activity } from '@lucide/svelte';
  import type { RpcClient } from '../lib/rpc';
  import type { MountUsage } from '../lib/monitoringTypes';
  import { getHostMonitoring } from '../lib/hostMonitoring.svelte';
  import { capacityTone, isVirtualMount, networkSummary, networkState } from '../lib/systemDetails';
  import SystemMetricCard from './SystemMetricCard.svelte';
  import { monitoringMessage as m, monitoringNumber as number, monitoringBytes as bytes } from '../lib/monitoringMessages';
  let { rpc, profileId, onError }: { rpc: RpcClient; profileId: string | null | undefined; onError: (message: string) => void } = $props();
  const monitoring = $derived(getHostMonitoring(rpc));
  const snapshot = $derived.by(() => { void monitoring.revision; return monitoring.service.snapshot(profileId ?? ''); });
  const sample = $derived(snapshot.latest);
  const stats = $derived(sample?.stats);
  const network = $derived(sample?.network_rates ?? []);
  const throughput = $derived(networkState(snapshot));
  const networkStatus = $derived(m(({ waitingFirst: 'waiting', waitingSecond: 'networkWaitingSecond', interfacesUnavailable: 'networkInterfacesUnavailable', noEligibleInterfaces: 'networkNoEligible', incomplete: 'networkIncomplete', invalidCounters: 'networkInvalidCounters', probeFailed: 'probeFailed', ready: 'networkReady' } as const)[throughput.status]));
  const physicalMounts = $derived((stats?.mounts ?? []).filter(row => !isVirtualMount(row)));
  const virtualMounts = $derived((stats?.mounts ?? []).filter(isVirtualMount));
  const recent = $derived(snapshot.history.slice(-60));
  const metrics = $derived([
    { kind: 'cpu' as const, label: m('cpu'), percent: stats?.cpu_percent, subtitle: stats?.cpu_cores?.length ? `${stats.cpu_cores.length} ${m('coreCount')}` : m('unavailable'), points: recent.map(s => ({ at: s.monotonicAt ?? s.at, value: s.stats.cpu_percent, breakBefore: s.breakBefore })) },
    { kind: 'memory' as const, label: m('memory'), percent: stats?.mem_percent, subtitle: `${kb(stats?.mem_used_kb)} / ${kb(stats?.mem_total_kb)}`, points: recent.map(s => ({ at: s.monotonicAt ?? s.at, value: s.stats.mem_percent, breakBefore: s.breakBefore })) },
    { kind: 'disk' as const, label: m('disk'), percent: stats?.disk_percent, subtitle: `${kb(stats?.disk_used_kb)} / ${kb(stats?.disk_total_kb)} · /`, points: recent.map(s => ({ at: s.monotonicAt ?? s.at, value: s.stats.disk_percent, breakBefore: s.breakBefore })) },
  ]);
  const networkPoints = $derived(recent.map(s => { const rate = networkSummary(s.network_rates); return { at: s.monotonicAt ?? s.at, value: rate.rx != null && rate.tx != null ? rate.rx + rate.tx : null, breakBefore: s.breakBefore }; }));
  const info = $derived([
    { label: m('load'), value: `${number(stats?.load1)} / ${number(stats?.load5)} / ${number(stats?.load15)}` },
    { label: m('uptime'), value: uptime(stats?.uptime_seconds) },
    { label: m('os'), value: stats?.os ?? m('unavailable') },
    { label: m('kernel'), value: stats?.kernel ?? m('unavailable') },
    { label: m('swap'), value: `${kb(stats?.swap_used_kb)} / ${kb(stats?.swap_total_kb)}` },
    { label: m('latency'), value: number(sample?.probeMs, ' ms') },
  ]);
  $effect(() => { if (!profileId) return; return monitoring.service.subscribe(profileId, 5000); });
  // Error reporting reads/writes reactive diagnostics and status; those must
  // not become dependencies of this effect and recursively report the failure.
  $effect(() => { if (snapshot.error) untrack(() => onError(m('probeFailed'))); });
  function kb(value: number | null | undefined): string { return bytes(value == null ? null : value * 1024); }
  function uptime(seconds: number | null | undefined): string { return seconds == null ? '—' : `${Math.floor(seconds / 86400)}d ${Math.floor(seconds % 86400 / 3600)}h ${Math.floor(seconds % 3600 / 60)}m`; }
  function bounded(value: number): number { return Number.isFinite(value) ? Math.max(0, Math.min(100, value)) : 0; }
</script>
<aside class="system-panel" aria-label={m('system')}>
  <div class="panel-content">
    <header><strong>{m('system')}</strong><span class:busy={snapshot.sampling}>{monitoring.service.isPaused() ? m('paused') : snapshot.sampling ? m('sampling') : ''}</span></header>
    {#if !profileId}<p class="empty">{m('sshOnly')}</p>
    {:else}
      {#if snapshot.error}<p class="error" role="status">{m('probeFailed')}{snapshot.failureStage ? ` ${m(({ profile: 'failureProfile', stats: 'failureStats', processing: 'failureProcessing', timeout: 'failureTimeout' } as const)[snapshot.failureStage])}` : ''}{sample ? ` ${m('stale')}` : ''}</p>{/if}
      {#if !sample}<p class="empty">{m('waiting')}</p>
      {:else}
        <div class="host-line"><h3 title={stats?.hostname ?? profileId}>{stats?.hostname ?? profileId}</h3><time datetime={new Date(sample.at).toISOString()} title={m('lastSample')}>{new Date(sample.at).toLocaleTimeString()}</time></div>
        <div class="metric-grid">
          {#each metrics as metric (metric.kind)}<SystemMetricCard {...metric} value={number(metric.percent, '%')} />{/each}
          <SystemMetricCard kind="network" label={m('network')} value={bytes(throughput.rx != null && throughput.tx != null ? throughput.rx + throughput.tx : null, true)} subtitle={throughput.status === 'ready' ? `↓ ${bytes(throughput.rx, true)} · ↑ ${bytes(throughput.tx, true)}` : networkStatus} trendStatus={networkStatus} points={networkPoints} />
        </div>
        <p class="scope" role="status">{networkStatus} · {throughput.readyCount}/{throughput.eligibleCount} {m('interfaces')}</p>
        <p class="scope">{m('networkScope')}</p>
        <dl class="info-grid">
          {#each info as item (item.label)}<div><dt>{item.label}</dt><dd title={item.value}>{item.value}</dd></div>{/each}
        </dl>
        <details class="section" open>
          <summary><Cpu size={15} /><span>{m('cores')}</span><small>{number(stats?.cpu_percent, '%')}</small></summary>
          <div class="core-grid">
            {#each stats?.cpu_cores ?? [] as core (core.name)}
              <div class="core"><span title={core.name}>{core.name.replace(/^cpu/, 'C')}</span><div class="bar" role="meter" aria-label={core.name} aria-valuemin={0} aria-valuemax={100} aria-valuenow={core.percent}><i style={`width:${bounded(core.percent)}%`}></i></div><span>{number(core.percent, '%')}</span></div>
            {:else}<p class="muted">{m('unavailable')}</p>{/each}
          </div>
        </details>
        <details class="section" open>
          <summary><HardDrive size={15} /><span>{m('mounts')}</span><small>{physicalMounts.length}</small></summary>
          {#each physicalMounts as mount, index (index)}{@render mountRow(mount)}{:else}<p class="muted">{m('unavailable')}</p>{/each}
          {#if virtualMounts.length}<details class="virtual"><summary>{m('virtualMounts')} <small>{virtualMounts.length}</small></summary>{#each virtualMounts as mount, index (index)}{@render mountRow(mount)}{/each}</details>{/if}
        </details>
        <details class="section" open>
          <summary><Network size={15} /><span>{m('interfaces')}</span><small>{network.length}</small></summary>
          {#each network as row (row.name)}
            <div class="interface"><strong title={row.name}>{row.name}</strong><div><span>↓ {bytes(row.rx_bytes)} · ↑ {bytes(row.tx_bytes)}</span><small>↓ {bytes(row.rx_bytes_per_second, true)} · ↑ {bytes(row.tx_bytes_per_second, true)}</small></div></div>
          {:else}<p class="muted">{m('networkInterfacesUnavailable')}</p>{/each}
        </details>
        <details class="section">
          <summary><Activity size={15} /><span>{m('diskIo')}</span></summary>
          {#each sample.disk_rates as row (row.name)}<div class="interface"><strong>{row.name}</strong><div><span>{m('read')}: {bytes(row.read_bytes_per_second, true)}</span><small>{m('write')}: {bytes(row.write_bytes_per_second, true)}</small></div></div>{:else}<p class="muted">{m('unavailable')}</p>{/each}
        </details>
      {/if}
    {/if}
  </div>
</aside>
{#snippet mountRow(mount: MountUsage)}
  <div class="mount" data-tone={capacityTone(mount.percent)}>
    <div><strong title={`${mount.mount_point} · ${mount.device}`}>{mount.mount_point}</strong><span>{kb(mount.used_kb)} / {kb(mount.total_kb)}</span></div>
    <div class="capacity"><div class="bar" role="meter" aria-label={mount.mount_point} aria-valuemin={0} aria-valuemax={100} aria-valuenow={mount.percent}><i style={`width:${bounded(mount.percent)}%`}></i></div><small>{number(mount.percent, '%')}</small></div>
  </div>
{/snippet}
<style>
  .system-panel{height:100%;width:100%;min-width:0;overflow:auto;box-sizing:border-box;padding:12px;color:var(--color-fg);background:var(--color-bg);border-left:1px solid var(--color-border-soft);font-size:12px;line-height:1.45;container-type:inline-size;font-variant-numeric:tabular-nums}.panel-content{min-width:0}header,.host-line{display:flex;align-items:center;justify-content:space-between;gap:8px}header{padding-bottom:10px;border-bottom:1px solid var(--color-border-soft)}header strong{font-size:13px;font-weight:600}header span{font-size:11px;color:var(--color-fg-muted)}.busy{color:var(--color-accent-text)}.host-line{margin:12px 0}h3{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:13px;font-weight:500;margin:0}time{font-size:11px;color:var(--color-fg-muted);flex-shrink:0}.metric-grid,.info-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:8px}.scope{font-size:11px;color:var(--color-fg-muted);line-height:1.45;margin:8px 2px 12px}.info-grid{margin:0 0 12px}.info-grid>div{min-width:0;border:1px solid var(--color-border-soft);border-radius:9px;background:var(--color-panel);padding:9px 11px}dt{color:var(--color-fg-muted);font-size:12px}dd{margin:3px 0 0;font-size:13px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.section{border:1px solid var(--color-border-soft);border-radius:10px;background:var(--color-panel);padding:11px 12px;margin-top:10px}summary{display:flex;align-items:center;gap:7px;cursor:pointer;list-style:none;min-height:20px;font-size:12px;font-weight:600}summary::-webkit-details-marker{display:none}summary::after{content:'›';font-size:16px;color:var(--color-fg-muted);margin-left:2px}details[open]>summary::after{transform:rotate(90deg)}summary :global(svg){color:var(--color-accent);flex-shrink:0}summary>span{flex:1}summary small{color:var(--color-fg-muted);font-size:11px;font-weight:400}.section>summary{margin-bottom:0}.section[open]>summary{margin-bottom:10px}.core-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:8px 14px}.core{display:grid;grid-template-columns:30px minmax(12px,1fr) 40px;align-items:center;gap:7px;font-size:11px;color:var(--color-fg-muted)}.core>span:first-child{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.core>span:last-child{text-align:right}.bar{height:4px;border-radius:3px;background:var(--color-panel-2);overflow:hidden}.bar i{display:block;height:100%;border-radius:inherit;background:var(--color-info)}.mount{margin:12px 0}.mount>div:first-child{display:flex;justify-content:space-between;align-items:baseline;gap:10px}.mount strong,.interface strong{min-width:0;font-size:12px;font-weight:500;overflow-wrap:anywhere}.mount strong{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.mount span{font-size:11px;color:var(--color-fg-muted);flex-shrink:0}.capacity{display:grid;grid-template-columns:minmax(0,1fr) 42px;gap:8px;align-items:center;margin-top:6px}.capacity small{text-align:right;font-size:11px;color:var(--color-fg-muted)}[data-tone='warning'] .bar i{background:var(--color-warning)}[data-tone='danger'] .bar i{background:var(--color-danger)}.virtual{border-top:1px solid var(--color-border-soft);padding-top:9px;margin-top:10px}.virtual summary{font-weight:400;color:var(--color-fg-muted);font-size:11px}.interface{display:flex;justify-content:space-between;align-items:baseline;gap:10px;padding:8px 0}.interface>div{min-width:0;text-align:right;color:var(--color-fg-muted);font-size:11px;overflow-wrap:anywhere}.interface small{display:block;font-size:11px;margin-top:4px}.muted,.empty{color:var(--color-fg-muted);font-size:12px}.error{border:1px solid var(--color-warning);border-radius:8px;padding:9px;color:var(--color-warning);font-size:12px}summary:focus-visible{outline:2px solid var(--color-focus-ring);outline-offset:3px;border-radius:3px}
  @container (max-width: 339px){.metric-grid,.info-grid{grid-template-columns:minmax(0,1fr)}.core-grid{gap:8px 10px}.core{grid-template-columns:24px minmax(10px,1fr) 34px;gap:4px}.mount>div:first-child{gap:5px}.interface{gap:8px}}
</style>
