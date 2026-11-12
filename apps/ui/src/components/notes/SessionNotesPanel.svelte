<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import type { RpcClient } from '../../lib/rpc';
  import { getProfileCatalog } from '../../lib/profileCatalog.svelte';
  import { getNotesStore } from '../../lib/notes.svelte';
  import { nm } from '../../lib/notesMessages';
  import NotesWorkspace from './NotesWorkspace.svelte';
  let { rpc, profileId, onError }: { rpc: RpcClient; profileId: string | null; onError: (message: string) => void } = $props();
  const catalog = $derived(getProfileCatalog(rpc)), store = $derived(getNotesStore(rpc));
  const profile = $derived(catalog.profiles.find(p => p.id === profileId));
  let remark = $derived(profileId ? store.profileRemarks[profileId] : undefined);
  $effect(() => {
    const current = profile;
    untrack(() => { if (current && !store.profileRemarks[current.id]?.dirty) {
      const value = current.note ?? ''; store.profileRemarks[current.id] = { value, base: value, dirty: false };
    } });
  });
  onMount(() => { void catalog.refresh().catch(error => onError(String(error))); });
  async function saveRemark(): Promise<void> {
    const id = profileId; if (!id) return; const draft = store.profileRemarks[id]; if (!draft) return;
    const value = draft.value;
    await catalog.invalidate(); const latest = catalog.profiles.find(p => p.id === id);
    if (!latest || (latest.note ?? '') !== draft.base) throw Error(nm('conflict'));
    await catalog.upsert({ ...latest, note: value });
    draft.base = value; draft.dirty = draft.value !== value;
  }
</script>
<div class="panel">
  <NotesWorkspace {rpc} profiles={catalog.profiles} {onError} {profileId} compact listFooter={hostRemark} />
</div>
{#snippet hostRemark()}
  <details class="remark"><summary>{nm('profileNote')}{remark?.dirty ? ' •' : ''}</summary>
    {#if profile && remark}
      <label>{profile.name}<textarea aria-label={nm('profileNote')} value={remark.value} oninput={event => { remark!.value = event.currentTarget.value; remark!.dirty = remark!.value !== remark!.base; }}></textarea></label>
      <button onclick={() => saveRemark().catch(error => onError(String(error)))}>{nm('saveProfile')}</button>
    {:else}<p>{nm('missingHost')}</p>{/if}
  </details>
{/snippet}
<style>
  .panel { height:100%; min-height:0; display:flex; flex-direction:column; } .panel :global(.workspace) { flex:1; }
  .remark { border-top:1px solid var(--color-border-soft); padding:6px 10px; flex-shrink:0; max-height:40%; overflow:auto; font-size:12px; } .remark p { color:var(--color-fg-subtle); margin:6px 0; }
  summary { cursor:pointer; color:var(--color-fg-muted); } label { display:flex; flex-direction:column; gap:6px; margin:8px 0; }
  textarea,button { color:inherit; background:var(--color-panel-2); border:1px solid var(--color-border); border-radius:5px; padding:6px; } textarea { min-height:70px; resize:vertical; }
</style>
