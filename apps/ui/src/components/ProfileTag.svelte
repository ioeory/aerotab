<script lang="ts">
  import { tagStyle } from '../lib/profileVisuals';
  import { profileVisualsStore } from '../lib/profileVisualsStore.svelte';

  interface Props {
    tag: string;
    compact?: boolean;
  }

  let { tag, compact = false }: Props = $props();

  const style = $derived(tagStyle(tag, profileVisualsStore.overrides));
</script>

<span
  class="profile-tag {compact ? 'profile-tag--compact' : ''}"
  {style}
  title={tag}
>{tag}</span>

<style>
  .profile-tag {
    font-family: var(--font-sans, system-ui, sans-serif);
    font-size: var(--text-compact, 14px);
    line-height: 1.5;
    max-width: min(240px, 100%);
    min-width: 0;
  }
  .profile-tag--compact {
    font-size: var(--text-meta, 12px);
    line-height: 1.5;
  }
</style>
