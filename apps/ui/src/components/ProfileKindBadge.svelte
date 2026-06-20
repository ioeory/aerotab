<script lang="ts">
  import { profileKindLabel, profileKindTone, type ProfileKind, visualStyle } from '../lib/profileVisuals';
  import { profileVisualsStore } from '../lib/profileVisualsStore.svelte';

  interface Props {
    kind: ProfileKind;
    compact?: boolean;
  }

  let { kind, compact = false }: Props = $props();

  const visible = $derived(kind !== 'ssh' || profileVisualsStore.showSshKindBadge);
  const label = $derived(profileKindLabel(kind));
  const style = $derived(visualStyle(profileKindTone(kind)));
</script>

{#if visible}
  <span class="profile-kind-badge {compact ? 'profile-kind-badge--compact' : ''}" {style}>{label}</span>
{/if}
