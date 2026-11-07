<script lang="ts">
  import type { RpcClient } from '../lib/rpc';
  import { getHostMonitoring } from '../lib/hostMonitoring.svelte';
  import { monitoringMessage as m, monitoringNumber as number, monitoringBytes as bytes } from '../lib/monitoringMessages';
  let { rpc, profileId, onError }: { rpc: RpcClient; profileId: string | null | undefined; onError: (message: string) => void } = $props();
  const monitoring = $derived(getHostMonitoring(rpc));
  const snapshot = $derived.by(() => { void monitoring.revision; return monitoring.service.snapshot(profileId ?? ''); });
  const sample = $derived(snapshot.latest);
  const stats = $derived(sample?.stats);
  const network = $derived(sample?.network_rates ?? []);
  const validNetwork = $derived(network.length > 0 && network.every(r => r.rx_bytes_per_second != null && r.tx_bytes_per_second != null));
  $effect(() => { if (!profileId) return; return monitoring.service.subscribe(profileId, 5000); });
  $effect(() => { if (snapshot.error) onError(m('probeFailed')); });
  function uptime(seconds: number | null | undefined): string { return seconds == null ? '—' : `${Math.floor(seconds / 86400)}d ${Math.floor(seconds % 86400 / 3600)}h ${Math.floor(seconds % 3600 / 60)}m`; }
</script>
<aside class="system-panel" aria-label={m('system')}>
  <header><strong>{m('system')}</strong><span class:busy={snapshot.sampling}>{monitoring.service.isPaused() ? m('paused') : snapshot.sampling ? m('sampling') : m('lastSample')}</span></header>
  {#if !profileId}<p>{m('sshOnly')}</p>
  {:else}
    {#if snapshot.error}<p class="error" role="status">{m('probeFailed')}</p>{/if}
    {#if !sample}<p>{m('waiting')}</p>
    {:else}
      <h3>{stats?.hostname ?? profileId}</h3>
      <div class="summary">
        {#each [{ label: m('cpu'), value: stats?.cpu_percent }, { label: m('memory'), value: stats?.mem_percent }, { label: m('disk'), value: stats?.disk_percent }] as metric (metric.label)}
          <div><span>{metric.label}</span><strong>{number(metric.value, '%')}</strong>{#if metric.value != null}<meter min="0" max="100" value={metric.value} aria-label={metric.label}></meter>{/if}</div>
        {/each}
      </div>
      <dl>
        <dt>{m('network')} ↓ / ↑</dt><dd>{bytes(validNetwork ? network.reduce((n,r) => n + r.rx_bytes_per_second!,0) : null,true)} / {bytes(validNetwork ? network.reduce((n,r) => n + r.tx_bytes_per_second!,0) : null,true)}</dd>
        <dt>{m('memory')}</dt><dd>{bytes(stats?.mem_used_kb == null ? null : stats.mem_used_kb*1024)} / {bytes(stats?.mem_total_kb == null ? null : stats.mem_total_kb*1024)}</dd>
        <dt>{m('swap')}</dt><dd>{bytes(stats?.swap_used_kb == null ? null : stats.swap_used_kb*1024)} / {bytes(stats?.swap_total_kb == null ? null : stats.swap_total_kb*1024)}</dd>
        <dt>{m('load')}</dt><dd>{number(stats?.load1)} / {number(stats?.load5)} / {number(stats?.load15)}</dd>
        <dt>{m('uptime')}</dt><dd>{uptime(stats?.uptime_seconds)}</dd>
        <dt>{m('os')}</dt><dd>{stats?.os ?? m('unavailable')}</dd>
        <dt>{m('kernel')}</dt><dd>{stats?.kernel ?? m('unavailable')}</dd>
        <dt>{m('latency')}</dt><dd>{number(sample.probeMs,' ms')}</dd>
        <dt>{m('lastSample')}</dt><dd>{new Date(sample.at).toLocaleTimeString()}</dd>
      </dl>
      <details open><summary>{m('cores')}</summary>
        {#each stats?.cpu_cores ?? [] as core (core.name)}<div class="core"><span>{core.name}</span><meter min="0" max="100" value={core.percent} aria-label={core.name}></meter><strong>{number(core.percent,'%')}</strong></div>{:else}<p class="muted">{m('unavailable')}</p>{/each}
      </details>
      <details open><summary>{m('mounts')}</summary>
        {#each stats?.mounts ?? [] as mount, index (index)}<div class="detail-row"><strong>{mount.mount_point}</strong><small>{mount.device}</small><span>{bytes(mount.used_kb*1024)} / {bytes(mount.total_kb*1024)} · {number(mount.percent,'%')}</span><meter min="0" max="100" value={mount.percent} aria-label={mount.mount_point}></meter></div>{:else}<p class="muted">{m('unavailable')}</p>{/each}
      </details>
      <details open><summary>{m('interfaces')}</summary>
        {#each network as row (row.name)}<div class="detail-row"><strong>{row.name}</strong><span>{m('rx')}: {bytes(row.rx_bytes_per_second,true)} · {bytes(row.rx_bytes)}</span><span>{m('tx')}: {bytes(row.tx_bytes_per_second,true)} · {bytes(row.tx_bytes)}</span></div>{:else}<p class="muted">{m('unavailable')}</p>{/each}
      </details>
      <details><summary>{m('diskIo')}</summary>
        {#each sample.disk_rates as row (row.name)}<div class="detail-row"><strong>{row.name}</strong><span>{m('read')}: {bytes(row.read_bytes_per_second,true)} · {bytes(row.read_bytes)}</span><span>{m('write')}: {bytes(row.write_bytes_per_second,true)} · {bytes(row.write_bytes)}</span></div>{:else}<p class="muted">{m('unavailable')}</p>{/each}
      </details>
    {/if}
  {/if}
</aside>
<style>
  .system-panel{height:100%;width:100%;min-width:240px;overflow:auto;box-sizing:border-box;padding:14px;color:var(--fg,#e2e8f0);background:var(--surface,#ffffff05);border-left:1px solid var(--border,#ffffff15);font-size:12px}header{display:flex;justify-content:space-between;align-items:center;gap:10px;border-bottom:1px solid var(--border,#ffffff15);padding-bottom:12px}header span{opacity:.55;font-size:10px}.busy{color:var(--accent,#4f8cff)}h3{font-size:14px;overflow-wrap:anywhere}.summary{display:grid;grid-template-columns:repeat(3,1fr);gap:12px;margin:16px 0}.summary div{display:flex;flex-direction:column;gap:6px}.summary span{font-size:10px;opacity:.6}.summary strong{font-size:17px;font-weight:500;font-variant-numeric:tabular-nums}meter{width:100%;height:5px;accent-color:var(--accent,#4f8cff)}dl{display:grid;grid-template-columns:minmax(80px,.9fr) minmax(0,1.3fr);gap:9px 8px;margin:18px 0}dt{opacity:.6}dd{margin:0;text-align:right;overflow-wrap:anywhere;font-variant-numeric:tabular-nums}details{border-top:1px solid var(--border,#ffffff15);padding:12px 0}summary{cursor:pointer;font-weight:600;margin-bottom:10px}.core{display:grid;grid-template-columns:52px 1fr 45px;gap:8px;align-items:center;padding:4px 0;font-size:11px}.core strong{text-align:right;font-weight:400}.detail-row{display:flex;flex-direction:column;gap:5px;padding:9px 0;font-size:11px}.detail-row strong{font-weight:500}.detail-row small,.muted{opacity:.55}.error{color:var(--warning,#e7af63);line-height:1.5}summary:focus-visible{outline:2px solid var(--accent,#4f8cff)}
</style>
