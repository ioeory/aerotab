<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { i18n } from '../lib/i18n.svelte';
  import { portal } from '../lib/portal';
  import ProfileTagEditor from './ProfileTagEditor.svelte';

  interface Props {
    x: number;
    y: number;
    selected: string[];
    knownTags: string[];
    onSave: (tags: string[]) => void;
    onClose: () => void;
  }

  let { x, y, selected: initialSelected, knownTags, onSave, onClose }: Props = $props();

  let panel: HTMLDivElement | null = $state(null);
  let selected = $state([...initialSelected]);
  let tagEditor: ProfileTagEditor | null = $state(null);
  let left = $state(x);
  let top = $state(y);

  onMount(() => {
    void tick().then(() => {
      if (!panel) return;
      const pad = 8;
      const w = panel.offsetWidth;
      const h = panel.offsetHeight;
      let nx = x;
      let ny = y;
      if (nx + w + pad > window.innerWidth) nx = Math.max(pad, window.innerWidth - w - pad);
      if (ny + h + pad > window.innerHeight) ny = Math.max(pad, window.innerHeight - h - pad);
      left = Math.max(pad, nx);
      top = Math.max(pad, ny);
      tagEditor?.focusAddInput();
    });
  });

  function save() {
    onSave(selected);
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      onClose();
    }
  }
</script>

<div use:portal class="profile-tag-picker-backdrop" onclick={onClose} onkeydown={onKeydown} role="presentation">
  <div
    bind:this={panel}
    class="profile-tag-picker-panel panel"
    style="left: {left}px; top: {top}px;"
    role="dialog"
    aria-modal="true"
    aria-label={i18n.t('sidebar.editTags')}
    onclick={(e) => e.stopPropagation()}
    onkeydown={onKeydown}
  >
    <div class="profile-tag-picker-title">{i18n.t('sidebar.editTags')}</div>
    <ProfileTagEditor
      bind:this={tagEditor}
      compact
      {knownTags}
      {selected}
      onSelectedChange={(tags) => { selected = tags; }}
    />
    <div class="profile-tag-picker-actions">
      <button type="button" class="btn-secondary" onclick={onClose}>{i18n.t('common.cancel')}</button>
      <button type="button" class="btn-primary" onclick={save}>{i18n.t('common.save')}</button>
    </div>
  </div>
</div>

<style>
  .profile-tag-picker-backdrop {
    position: fixed;
    inset: 0;
    z-index: 58;
  }
  .profile-tag-picker-panel {
    position: fixed;
    z-index: 59;
    width: min(300px, calc(100vw - 24px));
    padding: 10px 12px 12px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-lg);
    background: var(--color-panel);
  }
  .profile-tag-picker-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--color-fg);
    margin-bottom: 8px;
  }
  .profile-tag-picker-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 10px;
  }
</style>
