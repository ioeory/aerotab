<script lang="ts">
  import type { StoredProfile } from '../lib/types';
  import HostSelect from './HostSelect.svelte';
  import { profileChoices } from './HostChoiceList.svelte';
  let { profiles, value, placeholder, localLabel, onChange }: {
    profiles: StoredProfile[]; value: string; placeholder: string; localLabel: string; onChange: (id: string) => void;
  } = $props();
  const choices = $derived([{ id: '__local__', label: localLabel }, ...profileChoices(profiles, true)]);
</script>

<HostSelect {choices} {value} label={placeholder} {placeholder} {onChange} />
