<script lang="ts">
  import { Plus } from '@lucide/svelte';
  import { i18n } from '../lib/i18n.svelte';
  import { normalizeTags, toggleProfileTag } from '../lib/profileMeta';
  import ProfileTag from './ProfileTag.svelte';

  interface Props {
    selected: string[];
    knownTags?: string[];
    compact?: boolean;
    onSelectedChange: (tags: string[]) => void;
  }

  let {
    selected,
    knownTags = [],
    compact = false,
    onSelectedChange,
  }: Props = $props();

  let newTag = $state('');
  let addInput: HTMLInputElement | null = $state(null);

  const selectedKeys = $derived(new Set(selected.map((t) => t.toLowerCase())));
  const sortedKnown = $derived(
    [...knownTags].sort((a, b) => a.localeCompare(b, undefined, { sensitivity: 'base' })),
  );

  function isSelected(tag: string): boolean {
    return selectedKeys.has(tag.toLowerCase());
  }

  function toggle(tag: string) {
    onSelectedChange(toggleProfileTag(selected, tag));
  }

  function addTag(raw?: string) {
    const trimmed = (raw ?? newTag).trim();
    if (!trimmed) return;
    onSelectedChange(normalizeTags([...selected, trimmed]));
    newTag = '';
  }

  export function focusAddInput() {
    addInput?.focus();
  }
</script>

<div class="profile-tag-editor {compact ? 'profile-tag-editor--compact' : ''}">
  {#if sortedKnown.length > 0}
    <div class="profile-tag-editor-label">{i18n.t('profileTags.known')}</div>
    <div class="profile-tag-editor-grid" role="listbox" aria-label={i18n.t('profileTags.known')}>
      {#each sortedKnown as tag (tag.toLowerCase())}
        <button
          type="button"
          role="option"
          aria-selected={isSelected(tag)}
          class="profile-tag-option {isSelected(tag) ? 'profile-tag-option--selected' : ''}"
          onclick={() => toggle(tag)}
        >
          <ProfileTag {tag} compact />
        </button>
      {/each}
    </div>
  {/if}

  <div class="profile-tag-editor-add">
    <input
      bind:this={addInput}
      bind:value={newTag}
      type="text"
      class="input profile-tag-editor-input"
      placeholder={i18n.t('profileTags.addPlaceholder')}
      aria-label={i18n.t('profileTags.addPlaceholder')}
      onkeydown={(e) => {
        if (e.key === 'Enter') {
          e.preventDefault();
          addTag();
        }
      }}
    />
    <button
      type="button"
      class="profile-tag-editor-add-btn"
      title={i18n.t('profileTags.add')}
      aria-label={i18n.t('profileTags.add')}
      onclick={() => addTag()}
    >
      <Plus size={12} />
    </button>
  </div>

  {#if selected.length > 0}
    <div class="profile-tag-editor-label">{i18n.t('profileTags.selected')}</div>
    <div class="profile-tag-editor-selected">
      {#each selected as tag (tag.toLowerCase())}
        <button type="button" class="profile-tag-option profile-tag-option--selected" onclick={() => toggle(tag)}>
          <ProfileTag {tag} compact />
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .profile-tag-editor {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }
  .profile-tag-editor--compact {
    gap: 6px;
  }
  .profile-tag-editor-label {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--color-fg-muted);
  }
  .profile-tag-editor-grid,
  .profile-tag-editor-selected {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    max-height: 140px;
    overflow-y: auto;
  }
  .profile-tag-editor--compact .profile-tag-editor-grid {
    max-height: 120px;
  }
  .profile-tag-option {
    display: inline-flex;
    border: none;
    background: transparent;
    padding: 1px;
    border-radius: 999px;
    cursor: pointer;
    opacity: 0.72;
  }
  .profile-tag-option:hover {
    opacity: 1;
  }
  .profile-tag-option--selected {
    opacity: 1;
    outline: 2px solid var(--color-accent);
    outline-offset: 1px;
  }
  .profile-tag-editor-add {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .profile-tag-editor-input {
    flex: 1;
    min-width: 0;
    font-size: 11.5px;
    padding: 4px 8px;
  }
  .profile-tag-editor-add-btn {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: 4px;
    border: 1px solid var(--color-border-soft);
    background: var(--color-panel-2);
    color: var(--color-fg-muted);
    cursor: pointer;
    flex-shrink: 0;
  }
  .profile-tag-editor-add-btn:hover {
    color: var(--color-accent);
    border-color: color-mix(in srgb, var(--color-accent) 40%, var(--color-border-soft));
  }
</style>
