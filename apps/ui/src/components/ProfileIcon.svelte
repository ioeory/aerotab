<script lang="ts">
  import { Boxes, Cloud, Cpu, Database, Globe2, KeyRound, LockKeyhole, Monitor, Router, Server, Terminal } from '@lucide/svelte';
  import type { ProfileIcon as ProfileIconData } from '../lib/types';
  import { selfhstIconUrl } from '../lib/profileMeta';

  interface Props {
    icon?: ProfileIconData | null;
    name?: string;
    size?: number;
  }

  let { icon = null, name = '', size = 14 }: Props = $props();

  const value = $derived((icon?.value ?? '').trim());
  const normalized = $derived(value.toLowerCase());
  const initial = $derived((name.trim().charAt(0) || '?').toUpperCase());
  let remoteIndex = $state(0);
  const selfhstUrl = $derived(icon?.kind === 'selfhst' && value ? selfhstIconUrl(value) : '');
  const remoteUrls = $derived(icon?.kind === 'remote' ? value.split(/[|,\n]/).map((part) => part.trim()).filter(Boolean) : []);
  const remoteUrl = $derived(remoteUrls[Math.min(remoteIndex, Math.max(0, remoteUrls.length - 1))] ?? '');
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
  {:else if normalized === 'desktop'}
    <Monitor {size} />
  {:else if normalized === 'globe'}
    <Globe2 {size} />
  {:else if normalized === 'lock'}
    <LockKeyhole {size} />
  {:else if selfhstUrl}
    <img src={selfhstUrl} alt="" class="custom-icon" loading="lazy" referrerpolicy="no-referrer" />
  {:else if remoteUrl}
    <img
      src={remoteUrl}
      alt=""
      class="custom-icon"
      loading="lazy"
      referrerpolicy="no-referrer"
      onerror={() => { if (remoteIndex < remoteUrls.length - 1) remoteIndex += 1; }}
    />
  {:else if (icon?.kind === 'file' || icon?.kind === 'data') && value}
    <img src={value} alt="" class="custom-icon" loading="lazy" referrerpolicy="no-referrer" />
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
  .custom-icon {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .emoji,
  .initial {
    line-height: 1;
    font-weight: 700;
    color: var(--color-fg);
  }
</style>