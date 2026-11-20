<script lang="ts">
  import type { SessionStore } from "../sessions.svelte";
  import TerminalView from "../terminal/TerminalView.svelte";
  import type { TerminalSettings } from "../types";
  import type { VaultStore } from "../vault.svelte";
  import Icon from "./Icon.svelte";
  import Lamp from "./Lamp.svelte";
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
      <!-- The reason a tab dropped (the tab itself only shows the mark and its title). It sits above
           the terminal's box instead of over it, so the terminal never loses lines under the bar:
           the box shrinks when the bar appears and grows when it goes, and the ResizeObserver in
           TerminalView refits either way. -->
      {#if !hidden && tab.id === sessions.activeId && tab.state === "disconnected"}
        <div class="dropped" role="status">
          <Lamp state="dropped" />
          <span class="reason">{tab.detail ?? "Disconnected"}</span>
          <button
            type="button"
            class="reconnect"
            aria-label={`Reconnect ${tab.title}`}
            onclick={() => sessions.reconnect(tab.id)}
          >
            <Icon name="reconnect" size={12} />
            Reconnect
          </button>
        </div>
      {/if}
      <div class="body">
        <TerminalView
          {tab}
          {sessions}
          {settings}
          visible={!hidden && tab.id === sessions.activeId}
          locked={tab.state === "needsVault" || tab.prompt !== null}
        />
      </div>
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
    display: flex;
    flex-direction: column;
  }

  /* the terminal's own box: what is left under the bar */
  .body {
    position: relative;
    flex: 1 1 0;
    min-height: 0;
  }

  .dropped {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
    background: var(--color-sidebar);
    border-bottom: 1px solid var(--color-border);
    color: var(--color-text);
    font-size: var(--text-sm);
  }

  .reason {
    flex: 1 1 0;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* outlined, not yellow: the tab's own icon button is the one yellow key (One Key Rule) */
  .reconnect {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--space-2);
    height: 20px;
    padding: 0 var(--space-3) 0 var(--space-2);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text);
    font: inherit;
    font-size: var(--text-xs);
    cursor: default;
  }

  .reconnect:hover {
    background: var(--color-hover);
  }

  /* an empty box would still catch the pointer above the active terminal */
  .pane.inactive {
    display: none;
  }
</style>
