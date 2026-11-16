<script lang="ts">
  import type { SessionStore } from "../sessions.svelte";
  import TerminalView from "../terminal/TerminalView.svelte";
  import type { TerminalSettings } from "../types";
  import type { VaultStore } from "../vault.svelte";
  import SessionPrompt from "./SessionPrompt.svelte";

  let {
    sessions,
    vault,
    settings,
    hidden = false,
  }: {
    sessions: SessionStore;
    /** For the inline unlock prompt of a tab that waits for the vault. */
    vault: VaultStore;
    settings: TerminalSettings;
    /** Another view (the vault manager) covers the area; the terminals stay mounted behind it. */
    hidden?: boolean;
  } = $props();
</script>

<div class="area" class:hidden>
  {#each sessions.tabs as tab (tab.id)}
    <div class="pane" class:inactive={hidden || tab.id !== sessions.activeId}>
      <TerminalView
        {tab}
        {sessions}
        {settings}
        visible={!hidden && tab.id === sessions.activeId}
        locked={tab.state === "needsVault" || tab.prompt !== null}
      />
      <!-- Only the tab in front shows its prompt, so it can take the keyboard when it appears (a
           prompt of another tab waits, and typed text is not kept across a tab switch). -->
      {#if !hidden && tab.id === sessions.activeId}
        <SessionPrompt {tab} {sessions} {vault} />
      {/if}
    </div>
  {/each}
</div>

<style>
  .area {
    position: relative;
    flex: 1 1 0;
    min-height: 0;
    background: var(--term-bg);
  }

  .area.hidden {
    display: none;
  }

  .pane {
    position: absolute;
    inset: 0;
  }

  /* an empty box would still catch the pointer above the active terminal */
  .pane.inactive {
    display: none;
  }
</style>
