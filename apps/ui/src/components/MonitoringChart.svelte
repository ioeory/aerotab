<script lang="ts">
  import type { MonitoringSample } from '../lib/monitoringTypes';
  import { monitoringMessage as m, monitoringNumber, monitoringBytes } from '../lib/monitoringMessages';
  let { samples, metric, label }: { samples: MonitoringSample[]; metric: 'cpu' | 'memory' | 'disk' | 'network'; label: string } = $props();
  const width = 360, height = 92;
  const values = $derived(samples.map(sample => ({ at: sample.at, value: metric === 'cpu' ? sample.stats.cpu_percent : metric === 'memory' ? sample.stats.mem_percent : metric === 'disk' ? sample.stats.disk_percent : sample.network_rates.length && sample.network_rates.every(r => r.rx_bytes_per_second != null && r.tx_bytes_per_second != null) ? sample.network_rates.reduce((sum, r) => sum + r.rx_bytes_per_second! + r.tx_bytes_per_second!, 0) : null })));
  const ceiling = $derived(metric === 'network' ? Math.max(1, ...values.map(v => v.value ?? 0)) : 100);
  const start = $derived(samples[0]?.at ?? 0);
  const end = $derived(samples.at(-1)?.at ?? start);
  const segments = $derived.by(() => {
    const paths: string[] = []; let path = '';
    for (const point of values) {
      if (point.value == null || !Number.isFinite(point.value)) { if (path) paths.push(path); path = ''; continue; }
      const x = end > start ? (point.at - start) / (end - start) * width : width / 2;
      const y = height - Math.min(1, Math.max(0, point.value / ceiling)) * (height - 4) - 2;
      path += `${path ? ' L' : 'M'}${x.toFixed(1)},${y.toFixed(1)}`;
    }
    if (path) paths.push(path); return paths;
  });
</script>
<figure>
  <figcaption><span>{label}</span><strong>{metric === 'network' ? monitoringBytes(values.at(-1)?.value, true) : monitoringNumber(values.at(-1)?.value, '%')}</strong></figcaption>
  {#if values.some(v => v.value != null)}
    <svg viewBox={`-2 -2 ${width + 4} ${height + 4}`} role="img" aria-label={`${label}: ${samples.length}`}>
      <path class="grid" d={`M0,${height/2}H${width} M0,${height}H${width}`} />
      {#each segments as path, index (index)}<path d={path} class="line" />{/each}
      {#if values.length === 1 && values[0]!.value != null}<circle cx={width/2} cy={height - Math.min(1, values[0]!.value / ceiling) * (height - 4) - 2} r="2" />{/if}
    </svg>
    <div class="axis"><span>{new Date(start).toLocaleTimeString()}</span><span>{new Date(end).toLocaleTimeString()}</span></div>
  {:else}<div class="no-data">{m('unavailable')}</div>{/if}
</figure>
<style>
  figure{margin:0;min-width:0}figcaption,.axis{display:flex;justify-content:space-between;gap:8px}figcaption{font-size:11px;margin-bottom:8px}strong{font-variant-numeric:tabular-nums;font-weight:500}svg{width:100%;height:92px;display:block;overflow:visible}.grid{stroke:currentColor;opacity:.1;fill:none;stroke-width:.6}.line{stroke:var(--accent,#4f8cff);fill:none;stroke-width:1.6;vector-effect:non-scaling-stroke}circle{fill:var(--accent,#4f8cff)}.axis{font-size:9px;opacity:.55;font-variant-numeric:tabular-nums;margin-top:4px}.no-data{height:110px;display:grid;place-items:center;font-size:11px;opacity:.55}
</style>
