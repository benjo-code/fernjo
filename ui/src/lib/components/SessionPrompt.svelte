<script lang="ts">
  import type { SessionStore, Tab } from "../sessions.svelte";
  import type { VaultStore } from "../vault.svelte";
  import HostKeyChangedPrompt from "./HostKeyChangedPrompt.svelte";
  import HostKeyPrompt from "./HostKeyPrompt.svelte";
  import HostKeySyncedPrompt from "./HostKeySyncedPrompt.svelte";
  import NeedsVaultPrompt from "./NeedsVaultPrompt.svelte";
  import SecretsPrompt from "./SecretsPrompt.svelte";

  let {
    tab,
    sessions,
    vault,
  }: {
    tab: Tab;
    sessions: SessionStore;
    vault: VaultStore;
  } = $props();

  const asked = $derived(tab.prompt);
</script>

<!-- A new prompt id remounts the variant, so nothing typed into the last one carries over. -->
{#if tab.state === "needsVault"}
  <NeedsVaultPrompt {tab} {sessions} {vault} />
{:else if asked}
  {#key asked.id}
    {#if asked.prompt.kind === "hostKey"}
      <HostKeyPrompt {tab} {sessions} prompt={asked.prompt} />
    {:else if asked.prompt.kind === "hostKeySynced"}
      <HostKeySyncedPrompt {tab} {sessions} prompt={asked.prompt} />
    {:else if asked.prompt.kind === "hostKeyChanged"}
      <HostKeyChangedPrompt {tab} {sessions} prompt={asked.prompt} />
    {:else if asked.prompt.kind === "certificate" || asked.prompt.kind === "certificateChanged"}
      <!-- RDP certificate prompts have no view yet: the connect keeps waiting for an answer. -->
    {:else}
      <SecretsPrompt {tab} {sessions} prompt={asked.prompt} />
    {/if}
  {/key}
{/if}
