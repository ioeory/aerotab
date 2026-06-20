<script lang="ts">
  import { Boxes, Cloud, Cpu, Database, Globe2, KeyRound, LockKeyhole, Monitor, Router, Server, Terminal } from '@lucide/svelte';
  import type { ProfileIcon as ProfileIconData } from '../lib/types';
  import { selfhstIconUrl } from '../lib/profileMeta';
  import { profileIconStyle, type ProfileKind } from '../lib/profileVisuals';

  interface Props {
    icon?: ProfileIconData | null;
    name?: string;
    kind?: ProfileKind;
    size?: number;
  }

  let { icon = null, name = '', kind, size = 14 }: Props = $props();

  const value = $derived((icon?.value ?? '').trim());
  const normalized = $derived(value.toLowerCase());
  const initial = $derived((name.trim().charAt(0) || '?').toUpperCase());
  const toneStyle = $derived(profileIconStyle(icon, name, kind));
  let remoteIndex = $state(0);
  const selfhstUrl = $derived(icon?.kind === 'selfhst' && value ? selfhstIconUrl(value) : '');
  const remoteUrls = $derived(icon?.kind === 'remote' ? value.split(/[|,\n]/).map((part) => part.trim()).filter(Boolean) : []);
  const remoteUrl = $derived(remoteUrls[Math.min(remoteIndex, Math.max(0, remoteUrls.length - 1))] ?? '');
  const isImageIcon = $derived(
    !!selfhstUrl
      || !!remoteUrl
      || ((icon?.kind === 'file' || icon?.kind === 'data') && !!value),
  );
</script>

<span
  class="profile-icon"
  class:profile-icon--image={isImageIcon}
  style={toneStyle}
  title={value || name}
>
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
    color: var(--profile-tone-fg, var(--color-accent));
    background: var(--profile-tone-bg, var(--color-panel-2));
    border: 1px solid var(--profile-tone-border, var(--color-border-soft));
    overflow: hidden;
    box-shadow: inset 0 1px 0 color-mix(in srgb, var(--profile-tone-fg, #fff) 8%, transparent);
  }
  .profile-icon--image {
    background: var(--color-panel-2);
    border-color: var(--color-border-soft);
    box-shadow: none;
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
    color: var(--profile-tone-fg, var(--color-fg));
  }
  .initial {
    font-family: var(--font-mono);
    letter-spacing: -0.02em;
  }
</style>
