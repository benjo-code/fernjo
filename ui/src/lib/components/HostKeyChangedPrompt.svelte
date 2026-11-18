<script lang="ts">
  import { untrack } from "svelte";
  import type { SessionStore, Tab } from "../sessions.svelte";
  import type { Prompt } from "../types";
  import PromptFrame from "./PromptFrame.svelte";

  let {
    tab,
    sessions,
    prompt,
  }: {
    tab: Tab;
    sessions: SessionStore;
    prompt: Extract<Prompt, { kind: "hostKeyChanged" }>;
  } = $props();

  let cancelButton: HTMLButtonElement | undefined = $state();
  let replaceButton: HTMLButtonElement | undefined = $state();

  // The safe choice has the initial focus.
  $effect(() => {
    untrack(() => cancelButton)?.focus();
  });

  function decide(decision: "replace" | "reject") {
    sessions.answer(tab.id, { kind: "hostKeyChanged", decision });
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      decide("reject");
    } else if (e.key === "Tab") {
      // focus stays on the two buttons
      e.preventDefault();
      const buttons = [cancelButton, replaceButton];
      const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
      const next = e.shiftKey ? index - 1 + buttons.length : index + 1;
      buttons[next % buttons.length]?.focus();
    }
  }
</script>

<PromptFrame title="The host key has changed" icon="alert" role="alertdialog" {onkeydown}>
  <p class="body">
    The key {prompt.host}:{prompt.port} presented ({prompt.keyType}) differs from the one stored for
    it. This can mean the server was reinstalled, or that someone is intercepting the connection.
    Continue only if you know why it changed.
  </p>
  <div class="group">
    <span class="label">Stored fingerprint{prompt.old.length === 1 ? "" : "s"}</span>
    {#each prompt.old as fingerprint, i (i)}
      <code class="fingerprint">{fingerprint}</code>
    {/each}
  </div>
  <div class="group">
    <span class="label">Fingerprint now offered</span>
    <code class="fingerprint">{prompt.new}</code>
  </div>
  <div class="actions">
    <button bind:this={cancelButton} type="button" class="btn" onclick={() => decide("reject")}>
      Cancel
    </button>
    <button
      bind:this={replaceButton}
      type="button"
      class="btn danger"
      onclick={() => decide("replace")}
    >
      Replace key and connect
    </button>
  </div>
</PromptFrame>

<style>
  .body {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--text-sm);
    line-height: 1.3;
  }

  .group {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .label {
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }

  .fingerprint {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
    user-select: text;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
  }

  .danger {
    color: var(--color-danger);
  }
</style>
