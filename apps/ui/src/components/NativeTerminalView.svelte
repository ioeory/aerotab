<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import type { RpcClient } from '../lib/rpc';
  import {
    NativeTerminalController,
    type LinkModifier,
    type NativeRendererPref,
    type ThemeColors,
  } from '../lib/termgrid';

  interface Props {
    rpc: RpcClient;
    sessionId: string;
    fontFamily?: string;
    fontSize?: number;
    fontWeight?: number;
    fontWeightBold?: number;
    linePadding?: number;
    scrollback?: number;
    cursorBlink?: boolean;
    altIsMeta?: boolean;
    ligatures?: boolean;
    theme?: ThemeColors;
    renderer?: NativeRendererPref;
    translucent?: boolean;
    transferDetect?: boolean;
    linkModifier?: LinkModifier;
    copyOnSelect?: boolean;
    writeInput?: (data: string) => void | Promise<void>;
    visible?: boolean;
    onBell?: () => void;
    onTitle?: (title: string) => void;
    onExit?: () => void;
    onActivity?: () => void;
    onRaw?: (bytes: Uint8Array) => void;
    onTrzszDetected?: () => void;
    onZmodemHint?: () => void;
    onReady?: (ctl: NativeTerminalController) => void;
    onError?: (msg: string) => void;
  }

  let {
    rpc,
    sessionId,
    fontFamily = 'JetBrains Mono, Menlo, monospace',
    fontSize = 13,
    fontWeight = 400,
    fontWeightBold = 700,
    linePadding = 0,
    scrollback = 10_000,
    cursorBlink = true,
    altIsMeta = false,
    ligatures = false,
    theme,
    renderer = 'auto',
    translucent = false,
    transferDetect = false,
    linkModifier = 'none',
    copyOnSelect = false,
    writeInput,
    visible = true,
    onBell,
    onTitle,
    onExit,
    onActivity,
    onRaw,
    onTrzszDetected,
    onZmodemHint,
    onReady,
    onError,
  }: Props = $props();

  let canvas: HTMLCanvasElement | null = $state(null);
  let input: HTMLTextAreaElement | null = $state(null);
  let controller: NativeTerminalController | null = null;

  export function getController(): NativeTerminalController | null {
    return controller;
  }

  export function focus() {
    controller?.focus();
  }

  onMount(() => {
    if (!canvas) return;
    const ctl = new NativeTerminalController({
      sessionId,
      rpc,
      canvas,
      input,
      fontFamily,
      fontSize,
      fontWeight,
      fontWeightBold,
      linePadding,
      scrollback,
      cursorBlink,
      altIsMeta,
      ligatures,
      theme,
      renderer,
      translucent,
      transferDetect,
      linkModifier,
      copyOnSelect,
      writeInput,
      onBell,
      onTitle,
      onExit,
      onActivity,
      onRaw,
      onTrzszDetected,
      onZmodemHint,
      onClipboard: (bytes) => {
        const text = new TextDecoder().decode(bytes);
        void navigator.clipboard.writeText(text).catch(() => {});
      },
    });
    controller = ctl;
    void ctl.start().then(() => {
      onReady?.(ctl);
      if (visible) ctl.focus();
    }).catch((e: Error) => {
      onError?.(e.message ?? String(e));
    });
  });

  $effect(() => {
    controller?.setVisible(visible);
  });

  $effect(() => {
    if (theme) controller?.setTheme(theme);
  });

  $effect(() => {
    controller?.setFont({
      family: fontFamily,
      size: fontSize,
      weight: fontWeight,
      weightBold: fontWeightBold,
      linePadding,
    });
  });

  $effect(() => {
    controller?.setLinkModifier(linkModifier);
  });

  $effect(() => {
    controller?.setCopyOnSelect(copyOnSelect);
  });

  $effect(() => {
    controller?.setWriteInput(writeInput);
  });

  $effect(() => {
    controller?.setTransferDetect(transferDetect);
  });

  onDestroy(() => {
    void controller?.dispose();
    controller = null;
  });
</script>

<div class="native-term absolute inset-0 min-h-0 min-w-0 overflow-hidden">
  <canvas
    bind:this={canvas}
    class="block w-full h-full outline-none"
    tabindex="0"
  ></canvas>
  <!-- IME / composition target; visually hidden but focusable via controller.focus() -->
  <textarea
    bind:this={input}
    class="native-term-ime"
    autocomplete="off"
    autocapitalize="off"
    spellcheck="false"
    tabindex="0"
    aria-label="Terminal input"
  ></textarea>
</div>

<style>
  /*
   * Mirror xterm's helper textarea: focusable, in-flow stacking, no z-index:-1.
   * WebView2 drops key events to elements painted under the canvas (z-index < 0).
   * pointer-events:none so clicks hit the canvas; controller.focus() still works.
   */
  .native-term-ime {
    position: absolute;
    left: 0;
    top: 0;
    width: 20px;
    height: 20px;
    opacity: 0;
    border: 0;
    padding: 0;
    margin: 0;
    resize: none;
    overflow: hidden;
    caret-color: transparent;
    color: transparent;
    background: transparent;
    z-index: 1;
    pointer-events: none;
    outline: none;
  }
</style>
