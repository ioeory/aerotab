<script lang="ts">
  import { Boxes, Cloud, Cpu, Database, KeyRound, Router, Server, Terminal } from '@lucide/svelte';
  import type { ProfileIcon as ProfileIconData } from '../lib/types';

  interface Props {
    icon?: ProfileIconData | null;
    name?: string;
    size?: number;
  }

  let { icon = null, name = '', size = 14 }: Props = $props();

  const value = $derived((icon?.value ?? '').trim());
  const normalized = $derived(value.toLowerCase());
  const initial = $derived((name.trim().charAt(0) || '?').toUpperCase());
</script>

<span class="profile-icon" title={value || name}>
  {#if icon?.kind === 'emoji' && value}
    <span class="emoji" style="font-size:{Math.max(11, size)}px">{value.slice(0, 2)}</span>
  {:else if normalized === 'database'}
    <Database {size} />
  {:else if normalized === 'cloud'}
    <Cloud {size} />
  {:else if normalized === 'router'}
    <Router {size} />
  {:else if normalized === 'key'}
    <KeyRound {size} />
  {:else if normalized === 'terminal'}
    <Terminal {size} />
  {:else if normalized === 'cpu'}
    <Cpu {size} />
  {:else if normalized === 'cluster'}
    <Boxes {size} />
  {:else if normalized === 'server' || icon?.kind === 'builtin'}
    <Server {size} />
  {:else}
    <span class="initial" style="font-size:{Math.max(10, size - 2)}px">{initial}</span>
  {/if}
</span>

<style>
  .profile-icon {
    width: 24px;
    height: 24px;
    border-radius: var(--radius-md);
    display: inline-grid;
    place-items: center;
    flex: 0 0 auto;
    color: var(--color-accent);
    background: var(--color-panel-2);
    border: 1px solid var(--color-border-soft);
    overflow: hidden;
  }
  .emoji,
  .initial {
    line-height: 1;
    font-weight: 700;
    color: var(--color-fg);
  }
</style>