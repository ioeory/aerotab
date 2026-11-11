<script lang="ts">
  import { Cpu, MemoryStick, HardDrive, Network } from '@lucide/svelte';
  import { sparklinePaths } from '../lib/systemDetails';
  import { monitoringMessage as m, monitoringNumber as number } from '../lib/monitoringMessages';
  let { label, value, percent, subtitle, kind, trendStatus, points = [] }: {
    label: string; value: string; percent?: number | null; subtitle: string;
    kind: 'cpu' | 'memory' | 'disk' | 'network'; trendStatus?: string;
    points?: { at: number; value: number | null | undefined; breakBefore?: boolean }[];
  } = $props();
  const Icon = $derived(({ cpu: Cpu, memory: MemoryStick, disk: HardDrive, network: Network })[kind]);
  const valid = $derived(percent != null && Number.isFinite(percent));
  const ceiling = $derived(kind === 'network' ? Math.max(1, ...points.map(p => p.value ?? 0)) : 100);
  const paths = $derived(sparklinePaths(points, ceiling));
</script>
<article class="metric" class:network={kind === 'network'} style={`--metric-color:var(--color-${kind === 'cpu' ? 'info' : kind === 'memory' ? 'success' : kind === 'disk' ? 'warning' : 'accent'})`}>
  <div class="metric-label"><Icon size={15} strokeWidth={1.7} /><span>{label}</span></div>
  <div class="reading"><strong title={value}>{value}</strong>
    {#if kind !== 'network'}
      <svg class="ring" viewBox="0 0 64 64" aria-hidden="true">
        <circle class="track" cx="32" cy="32" r="26" />
        {#if valid}<circle class="fill" cx="32" cy="32" r="26" pathLength="100" stroke-dasharray={`${Math.max(0, Math.min(100, percent!))} 100`} />{/if}
        <text x="32" y="33">{number(percent, '%')}</text>
      </svg>
    {/if}
  </div>
  <p title={subtitle}>{subtitle}</p>
  <div class="trend" title={m('actualRange')}>
    {#if paths.length}
      <svg viewBox="0 0 200 34" preserveAspectRatio="none" role="img" aria-label={`${label} · ${m('actualRange')}`}>
        {#each paths as path, index (index)}<path d={path} />{/each}
      </svg>
    {:else}<span>{trendStatus ?? m('trendWaiting')}</span>{/if}
  </div>
</article>
<style>
  .metric{position:relative;min-width:0;padding:12px;border:1px solid var(--color-border-soft);border-radius:10px;background:var(--color-panel);overflow:hidden}.metric-label{display:flex;align-items:center;gap:7px;color:var(--color-fg-muted);font-size:12px}.metric-label :global(svg){color:var(--metric-color);flex-shrink:0}.reading{display:flex;align-items:center;min-height:54px;padding-right:65px}strong{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:23px;line-height:1.2;letter-spacing:-.7px;font-weight:600;font-variant-numeric:tabular-nums}.ring{position:absolute;right:9px;top:22px;width:64px;height:64px}.ring circle{fill:none;stroke-width:7}.track{stroke:var(--color-panel-2)}.fill{stroke:var(--metric-color);stroke-linecap:round;transform:rotate(-90deg);transform-origin:center}.ring text{fill:var(--color-fg);text-anchor:middle;dominant-baseline:middle;font-size:11px;font-weight:500}p{position:relative;margin:0;color:var(--color-fg-muted);font-size:12px;line-height:1.45;min-height:18px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.network .reading{padding-right:0}.network strong{font-size:21px}.trend{height:32px;margin-top:10px;display:flex;align-items:flex-end}.trend svg{width:100%;height:32px;overflow:visible}.trend path{fill:none;stroke:var(--metric-color);stroke-width:2;vector-effect:non-scaling-stroke;stroke-linecap:round;stroke-linejoin:round}.trend span{font-size:11px;color:var(--color-fg-muted)}
</style>
