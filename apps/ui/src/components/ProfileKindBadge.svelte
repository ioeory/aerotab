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

<style>
  .profile-kind-badge {
    min-height: 22px;
    padding: 1px 5px;
    font-size: var(--text-compact);
    line-height: var(--leading-ui, 1.4);
    letter-spacing: 0.025em;
    white-space: nowrap;
  }

  .profile-kind-badge--compact {
    padding: 0 4px;
  }
</style>
