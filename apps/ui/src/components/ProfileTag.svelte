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
