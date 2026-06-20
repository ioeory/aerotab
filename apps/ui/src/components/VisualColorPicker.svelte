<script lang="ts">
  import { i18n } from '../lib/i18n.svelte';
  import { PRESET_VISUAL_COLORS, toneFromColor, visualStyle } from '../lib/profileVisuals';

  interface Props {
    value?: string | null;
    compact?: boolean;
    /** Inline single-row swatches for context menus. */
    menu?: boolean;
    onPick: (color: string | null) => void;
  }

  let { value = null, compact = false, menu = false, onPick }: Props = $props();
</script>

<div
  class="visual-color-picker {compact ? 'visual-color-picker--compact' : ''} {menu ? 'visual-color-picker--menu' : ''}"
  role="listbox"
  aria-label={i18n.t('profiles.visualColorPicker')}
  onclick={(e) => e.stopPropagation()}
>
  <div class="visual-color-picker-grid">
    {#each PRESET_VISUAL_COLORS as color (color)}
      {@const selected = value?.toLowerCase() === color.toLowerCase()}
      <button
        type="button"
        role="option"
        aria-selected={selected}
        class="visual-color-swatch {selected ? 'selected' : ''}"
        style={visualStyle(toneFromColor(color))}
        title={color}
        onclick={() => onPick(color)}
      >
        <span class="visual-color-swatch-core" style="background:{color}"></span>
      </button>
    {/each}
  </div>
  {#if value}
    <button type="button" class="visual-color-reset" onclick={() => onPick(null)}>
      {menu ? '×' : i18n.t('profiles.visualColorReset')}
    </button>
  {/if}
</div>

<style>
  .visual-color-picker {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 6px 8px;
  }
  .visual-color-picker--compact {
    padding: 4px 6px;
    gap: 4px;
  }
  .visual-color-picker-grid {
    display: grid;
    grid-template-columns: repeat(8, 1fr);
    gap: 4px;
  }
  .visual-color-picker--compact .visual-color-picker-grid {
    grid-template-columns: repeat(4, 1fr);
  }
  .visual-color-picker--menu {
    padding: 0;
    gap: 4px;
    flex-direction: row;
    align-items: center;
    flex-wrap: nowrap;
  }
  .visual-color-picker--menu .visual-color-picker-grid {
    grid-template-columns: repeat(8, 16px);
    gap: 3px;
    flex-shrink: 0;
  }
  .visual-color-picker--menu .visual-color-swatch {
    width: 16px;
    height: 16px;
    border-radius: 4px;
    padding: 1px;
  }
  .visual-color-picker--menu .visual-color-reset {
    width: 16px;
    height: 16px;
    display: grid;
    place-items: center;
    font-size: 13px;
    line-height: 1;
    padding: 0;
    flex-shrink: 0;
  }
  .visual-color-swatch {
    width: 100%;
    aspect-ratio: 1;
    min-width: 0;
    border-radius: 6px;
    border: 1px solid var(--profile-tone-border, var(--color-border-soft));
    background: var(--profile-tone-bg, var(--color-panel-2));
    padding: 2px;
    cursor: pointer;
    display: grid;
    place-items: center;
  }
  .visual-color-swatch.selected {
    outline: 2px solid var(--profile-tone-fg, var(--color-accent));
    outline-offset: 1px;
  }
  .visual-color-swatch-core {
    width: 100%;
    height: 100%;
    border-radius: 4px;
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.08);
  }
  .visual-color-reset {
    align-self: flex-start;
    font-size: 10.5px;
    color: var(--color-fg-muted);
    background: transparent;
    border: none;
    padding: 2px 0;
    cursor: pointer;
  }
  .visual-color-reset:hover {
    color: var(--color-accent);
    text-decoration: underline;
  }
</style>
