<script lang="ts">
  // Terminal — full Tabby-parity options.
  // Persists to two settings keys:
  //   - `terminal` : rendering / xterm options + bell + restoreTabs + altIsMeta
  //                  + scrollOnInput + word separators + link modifier + bracketed paste
  //                  + paste warnings + flatten / trim + middle-click paste
  //   - `behavior` : copyOnSelect + rmbPaste (legacy keys retained for compat)
  //
  // M3 introduces many fields; defaults preserve current behaviour.

  import { onMount, onDestroy } from 'svelte';
  import type { RpcClient } from '../../../lib/rpc';
  import { settingsCoord } from '../../../lib/settingsStore.svelte';
  import { i18n } from '../../../lib/i18n.svelte';

  interface Props {
    rpc: RpcClient;
    onError: (msg: string) => void;
  }
  let { rpc, onError }: Props = $props();

  type Renderer = 'dom' | 'canvas' | 'webgl';
  type BellMode = 'off' | 'visual' | 'audible';
  type RightClick = 'menu' | 'paste' | 'select-word';
  type LinkModifier = 'none' | 'ctrl' | 'alt' | 'meta';

  // terminal
  let scrollback = $state(2000);
  let cursorBlink = $state(true);
  let renderer = $state<Renderer>('canvas');
  let altIsMeta = $state(false);
  let scrollOnInput = $state(true);
  let bell = $state<BellMode>('off');
  let wordSeparator = $state(' ()[]{}\'",;:');
  let linkModifier = $state<LinkModifier>('none');
  let copyWithFormatting = $state(false);
  let bracketedPaste = $state(true);
  let pasteMultilineWarn = $state(true);
  let pasteFlattenNewlines = $state(false);
  let pasteTrimWhitespace = $state(false);
  let experimentalTransferDetection = $state(false);
  let autoOpenTerminal = $state(true);
  let restoreTabs = $state(true);

  // behaviour
  let copyOnSelect = $state(false);
  let rightClickAction = $state<RightClick>('menu');
  let middleClickPaste = $state(false);

  function markDirty() { settingsCoord.markDirty(); }

  async function load() {
    try {
      const t = await rpc.call<{ value: unknown }>('settings.get', { key: 'terminal' });
      if (t.value && typeof t.value === 'object') {
        const v = t.value as Record<string, unknown>;
        if (typeof v.scrollback === 'number') scrollback = v.scrollback;
        if (typeof v.cursorBlink === 'boolean') cursorBlink = v.cursorBlink;
        if (v.renderer === 'dom' || v.renderer === 'canvas' || v.renderer === 'webgl') renderer = v.renderer;
        if (typeof v.altIsMeta === 'boolean') altIsMeta = v.altIsMeta;
        if (typeof v.scrollOnInput === 'boolean') scrollOnInput = v.scrollOnInput;
        if (v.bell === 'off' || v.bell === 'visual' || v.bell === 'audible') bell = v.bell;
        if (typeof v.wordSeparator === 'string') wordSeparator = v.wordSeparator;
        if (v.linkModifier === 'none' || v.linkModifier === 'ctrl'
            || v.linkModifier === 'alt' || v.linkModifier === 'meta') linkModifier = v.linkModifier;
        if (typeof v.copyWithFormatting === 'boolean') copyWithFormatting = v.copyWithFormatting;
        if (typeof v.bracketedPaste === 'boolean') bracketedPaste = v.bracketedPaste;
        if (typeof v.pasteMultilineWarn === 'boolean') pasteMultilineWarn = v.pasteMultilineWarn;
        if (typeof v.pasteFlattenNewlines === 'boolean') pasteFlattenNewlines = v.pasteFlattenNewlines;
        if (typeof v.pasteTrimWhitespace === 'boolean') pasteTrimWhitespace = v.pasteTrimWhitespace;
        if (typeof v.experimentalTransferDetection === 'boolean') experimentalTransferDetection = v.experimentalTransferDetection;
        if (typeof v.autoOpenTerminal === 'boolean') autoOpenTerminal = v.autoOpenTerminal;
        if (typeof v.restoreTabs === 'boolean') restoreTabs = v.restoreTabs;
      }
      const b = await rpc.call<{ value: unknown }>('settings.get', { key: 'behavior' });
      if (b.value && typeof b.value === 'object') {
        const v = b.value as Record<string, unknown>;
        if (typeof v.copyOnSelect === 'boolean') copyOnSelect = v.copyOnSelect;
        if (v.rightClickAction === 'menu' || v.rightClickAction === 'paste'
            || v.rightClickAction === 'select-word') {
          rightClickAction = v.rightClickAction;
        } else if (typeof v.rmbPaste === 'boolean') {
          // legacy migration: rmbPaste=true -> 'paste'
          rightClickAction = v.rmbPaste ? 'paste' : 'menu';
        }
        if (typeof v.middleClickPaste === 'boolean') middleClickPaste = v.middleClickPaste;
      }
    } catch (e) {
      onError(`terminal load: ${(e as Error).message}`);
    }
  }

  async function save() {
    await rpc.call('settings.set', {
      key: 'terminal',
      value: {
        scrollback, cursorBlink, renderer, altIsMeta, scrollOnInput, bell,
        wordSeparator, linkModifier, copyWithFormatting, bracketedPaste,
        pasteMultilineWarn, pasteFlattenNewlines, pasteTrimWhitespace,
        experimentalTransferDetection,
        autoOpenTerminal, restoreTabs,
      },
    });
    await rpc.call('settings.set', {
      key: 'behavior',
      value: {
        copyOnSelect,
        rightClickAction,
        // Retain legacy rmbPaste flag for backward compatibility with older code paths.
        rmbPaste: rightClickAction === 'paste',
        middleClickPaste,
      },
    });
  }

  onMount(() => {
    settingsCoord.registerSaver('terminal', save);
    void load();
  });
  onDestroy(() => settingsCoord.unregisterSaver('terminal'));
</script>

<div class="settings-section">
  <h2>Terminal</h2>

  <div>
    <div class="section-h">Rendering</div>
    <label for="tm-renderer" class="lbl">Frontend</label>
    <select id="tm-renderer" bind:value={renderer} onchange={markDirty} class="select">
      <option value="dom">DOM (most compatible)</option>
      <option value="canvas">Canvas (recommended)</option>
      <option value="webgl">WebGL (GPU-accelerated)</option>
    </select>
    <div class="help">Change takes effect on next pane open.</div>
  </div>

  <div>
    <div class="section-h">Scrollback &amp; cursor</div>
    <div class="grid grid-cols-2 gap-3">
      <div>
        <label for="tm-scrollback" class="lbl">Scrollback lines</label>
        <input
          id="tm-scrollback" type="number" min="100" max="100000" step="500"
          bind:value={scrollback} oninput={markDirty} class="input"
        />
      </div>
    </div>
    <label class="row mt-3">
      <input type="checkbox" bind:checked={cursorBlink} onchange={markDirty} />
      Blinking cursor
    </label>
    <label class="row mt-2">
      <input type="checkbox" bind:checked={scrollOnInput} onchange={markDirty} />
      Scroll to bottom on user input
    </label>
  </div>

  <div>
    <div class="section-h">Keyboard</div>
    <label class="row">
      <input type="checkbox" bind:checked={altIsMeta} onchange={markDirty} />
      Treat Alt as Meta key
    </label>
  </div>

  <div>
    <div class="section-h">Bell</div>
    <label for="tm-bell" class="lbl">Bell mode</label>
    <select id="tm-bell" bind:value={bell} onchange={markDirty} class="select">
      <option value="off">Off</option>
      <option value="visual">Visual</option>
      <option value="audible">Audible</option>
    </select>
  </div>

  <div>
    <div class="section-h">Mouse</div>
    <label for="tm-right-click" class="lbl">Right-click action</label>
    <select
      id="tm-right-click"
      bind:value={rightClickAction}
      onchange={markDirty}
      class="select"
    >
      <option value="menu">Show context menu</option>
      <option value="paste">Paste</option>
      <option value="select-word">Select word</option>
    </select>
    <label class="row mt-3">
      <input type="checkbox" bind:checked={middleClickPaste} onchange={markDirty} />
      Middle-click pastes selection
    </label>
    <label class="row mt-2">
      <input type="checkbox" bind:checked={copyOnSelect} onchange={markDirty} />
      Copy on selection
    </label>
  </div>

  <div>
    <div class="section-h">Words &amp; links</div>
    <label for="tm-word-sep" class="lbl">Word separator characters</label>
    <input
      id="tm-word-sep"
      bind:value={wordSeparator}
      oninput={markDirty}
      class="input"
    />
    <label for="tm-link-mod" class="lbl">Hold this key to follow links</label>
    <select id="tm-link-mod" bind:value={linkModifier} onchange={markDirty} class="select">
      <option value="none">None (always clickable)</option>
      <option value="ctrl">Ctrl</option>
      <option value="alt">Alt</option>
      <option value="meta">Meta / Cmd</option>
    </select>
  </div>

  <div>
    <div class="section-h">Copy / paste</div>
    <label class="row">
      <input type="checkbox" bind:checked={copyWithFormatting} onchange={markDirty} />
      Copy with formatting (HTML)
    </label>
    <label class="row mt-2">
      <input type="checkbox" bind:checked={bracketedPaste} onchange={markDirty} />
      Use bracketed paste mode
    </label>
    <label class="row mt-2">
      <input type="checkbox" bind:checked={pasteMultilineWarn} onchange={markDirty} />
      Warn before pasting multi-line text
    </label>
    <label class="row mt-2">
      <input type="checkbox" bind:checked={pasteFlattenNewlines} onchange={markDirty} />
      Flatten line breaks on paste
    </label>
    <label class="row mt-2">
      <input type="checkbox" bind:checked={pasteTrimWhitespace} onchange={markDirty} />
      Trim trailing whitespace on paste
    </label>
  </div>

  <div>
    <div class="section-h">{i18n.t('settings.terminal.experimentalTransfers')}</div>
    <label class="row">
      <input type="checkbox" bind:checked={experimentalTransferDetection} onchange={markDirty} />
      {i18n.t('settings.terminal.detectTransfers')}
    </label>
    <div class="help">{i18n.t('settings.terminal.detectTransfersHelp')}</div>
  </div>

  <div>
    <div class="section-h">Startup</div>
    <label class="row">
      <input type="checkbox" bind:checked={autoOpenTerminal} onchange={markDirty} />
      Auto-open a terminal on launch
    </label>
    <label class="row mt-2">
      <input type="checkbox" bind:checked={restoreTabs} onchange={markDirty} />
      Restore previous tabs &amp; panes (requires M9 session-restore backend)
    </label>
  </div>
</div>
