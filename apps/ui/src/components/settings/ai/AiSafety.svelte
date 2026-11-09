<script lang="ts">
  import {i18n} from '../../../lib/i18n.svelte';
  const t=(key:string)=>i18n.t(`aiSettings.${key}`);
</script>
<div class="safety">
  <p class="hint">{t('safetyHint')}</p>
  {#each ['observer','confirm','auto'] as mode}
    <article><span class="mode-marker" aria-hidden="true"></span><div><h3>{t(mode)}</h3><p>{t(`${mode}Body`)}</p></div></article>
  {/each}
  <p class="privacy">{i18n.t('ai.privacy')}</p>
</div>
<style>
  .hint,.privacy{color:var(--color-fg-muted);font-size:12px;line-height:1.7}article{display:flex;gap:14px;padding:18px;border:1px solid var(--color-border);border-radius:10px;margin:12px 0;background:var(--color-bg-soft)}h3{margin:0 0 6px;font-size:13px}article p{margin:0;color:var(--color-fg-muted);font-size:12px;line-height:1.7}.mode-marker{width:7px;height:7px;margin-top:5px;border:2px solid var(--color-accent);border-radius:50%;flex-shrink:0}.privacy{padding-top:12px;border-top:1px solid var(--color-border)}
  .hint,.privacy,article p{font-size:14px;color:var(--color-fg)}h3{font-size:15px}article{border:0;border-bottom:1px solid var(--color-border-soft);border-radius:0;background:transparent;padding:18px 0}
</style>
