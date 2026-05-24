<script lang="ts">
  import { X, RefreshCw } from '@lucide/svelte';
  import type { RpcClient } from '../lib/rpc';
  import type { SerialFlow, SerialParity, SerialStopBits, SerialProfileSpec, SessionMeta } from '../lib/types';
  import { tabs } from '../lib/tabs.svelte';

  interface Props {
    rpc: RpcClient;
    onError: (msg: string) => void;
  }
  let { rpc, onError }: Props = $props();

  let dialog: HTMLDialogElement | null = null;
  let ports = $state<string[]>([]);
  let port = $state('');
  let baud = $state(115200);
  let dataBits = $state(8);
  let parity = $state<SerialParity>('None');
  let stopBits = $state<SerialStopBits>('One');
  let flow = $state<SerialFlow>('None');
  let loading = $state(false);

  const baudOptions = [9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600];

  async function refreshPorts() {
    loading = true;
    try {
      ports = await rpc.call<string[]>('serial.listPorts');
      if (!port && ports[0]) port = ports[0];
    } catch (e) {
      onError(`serial: ${(e as Error).message}`);
    } finally {
      loading = false;
    }
  }

  export async function open() {
    await refreshPorts();
    dialog?.showModal();
  }

  function close() { dialog?.close(); }

  async function submit(ev: Event) {
    ev.preventDefault();
    if (!port) {
      onError('select a port first');
      return;
    }
    const profile: SerialProfileSpec = {
      port,
      baud: Number(baud) || 115200,
      data_bits: Number(dataBits) || 8,
      parity, stop_bits: stopBits, flow_control: flow,
    };
    try {
      const meta = await rpc.call<SessionMeta>('session.openSerial', {
        title: `${port} @ ${baud}`,
        profile,
      });
      tabs.add(meta);
      close();
    } catch (e) {
      onError(`serial open: ${(e as Error).message}`);
    }
  }
</script>

<dialog bind:this={dialog} class="min-w-[420px]">
  <form onsubmit={submit} class="p-5">
    <div class="flex items-center justify-between mb-3">
      <h2 class="text-[14px] font-semibold text-[var(--color-accent)]">New serial connection</h2>
      <button type="button" onclick={close} aria-label="Close"
              class="p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]">
        <X size={14} />
      </button>
    </div>

    <label for="sm-port" class="block text-[11px] text-[var(--color-fg-muted)] mb-1">Port</label>
    <div class="flex gap-2">
      <select id="sm-port" bind:value={port} class="input flex-1">
        {#if ports.length === 0}
          <option value="">(no ports detected)</option>
        {/if}
        {#each ports as p}<option value={p}>{p}</option>{/each}
      </select>
      <button type="button" onclick={refreshPorts} class="btn-secondary" title="Refresh port list" aria-label="Refresh">
        <RefreshCw size={12} class={loading ? 'animate-spin' : ''} />
      </button>
    </div>

    <div class="flex gap-3 mt-2">
      <div class="flex-1">
        <label for="sm-baud" class="block text-[11px] text-[var(--color-fg-muted)] mb-1">Baud</label>
        <select id="sm-baud" bind:value={baud} class="input">
          {#each baudOptions as b}<option value={b}>{b}</option>{/each}
        </select>
      </div>
      <div class="flex-1">
        <label for="sm-data" class="block text-[11px] text-[var(--color-fg-muted)] mb-1">Data bits</label>
        <select id="sm-data" bind:value={dataBits} class="input">
          {#each [5,6,7,8] as d}<option value={d}>{d}</option>{/each}
        </select>
      </div>
    </div>

    <div class="flex gap-3 mt-2">
      <div class="flex-1">
        <label for="sm-parity" class="block text-[11px] text-[var(--color-fg-muted)] mb-1">Parity</label>
        <select id="sm-parity" bind:value={parity} class="input">
          <option value="None">None</option>
          <option value="Even">Even</option>
          <option value="Odd">Odd</option>
        </select>
      </div>
      <div class="flex-1">
        <label for="sm-stop" class="block text-[11px] text-[var(--color-fg-muted)] mb-1">Stop bits</label>
        <select id="sm-stop" bind:value={stopBits} class="input">
          <option value="One">1</option>
          <option value="Two">2</option>
        </select>
      </div>
      <div class="flex-1">
        <label for="sm-flow" class="block text-[11px] text-[var(--color-fg-muted)] mb-1">Flow</label>
        <select id="sm-flow" bind:value={flow} class="input">
          <option value="None">None</option>
          <option value="Software">XON/XOFF</option>
          <option value="Hardware">RTS/CTS</option>
        </select>
      </div>
    </div>

    <div class="flex justify-end gap-2 mt-5">
      <button type="button" onclick={close} class="btn-secondary">Cancel</button>
      <button type="submit" class="btn-primary">Connect</button>
    </div>
  </form>
</dialog>

