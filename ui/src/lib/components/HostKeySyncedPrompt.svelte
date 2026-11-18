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
    prompt: Extract<Prompt, { kind: "hostKeySynced" }>;
  } = $props();

  let root: HTMLElement | undefined = $state();

  // The keyboard lands on the dialog, not on a button: Enter typed into the terminal a moment ago
  // must not confirm a key. Tab then reaches "Confirm and connect" first.
  $effect(() => {
    untrack(() => root)?.focus();
  });

  function decide(decision: "once" | "always" | "reject") {
    sessions.answer(tab.id, { kind: "hostKey", decision });
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      decide("reject");
    }
  }
</script>

<PromptFrame bind:root title="Confirm this host key?" icon="ssh" {onkeydown}>
  <p class="body">
    This host key was added by another device or file. This device has not confirmed it. If you did
    not add this host yourself on one of your devices, cancel.
  </p>
  <dl>
    <dt>Host</dt>
    <dd>{prompt.host}</dd>
    <dt>Port</dt>
    <dd>{prompt.port}</dd>
    <dt>Key type</dt>
    <dd>{prompt.keyType}</dd>
    <dt>Fingerprint</dt>
    <dd><code class="fingerprint">{prompt.fingerprint}</code></dd>
  </dl>
  {#if prompt.previous.length > 0}
    <div class="previous">
      <span>This device previously trusted</span>
      {#each prompt.previous as fingerprint, i (i)}
        <code class="fingerprint">{fingerprint}</code>
      {/each}
    </div>
  {/if}
  <div class="actions">
    <button type="button" class="btn primary" onclick={() => decide("always")}>
      Confirm and connect
    </button>
    <button type="button" class="btn" onclick={() => decide("once")}>Connect once</button>
    <button type="button" class="btn" onclick={() => decide("reject")}>Cancel</button>
  </div>
</PromptFrame>

<style>
  .body {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--text-sm);
    line-height: 1.3;
  }

  dl {
    display: grid;
    grid-template-columns: 84px minmax(0, 1fr);
    gap: var(--space-2) var(--space-3);
    margin: 0;
  }

  dt {
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }

  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .fingerprint {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    user-select: text;
  }

  .previous {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
  }

  .primary {
    background: var(--color-accent);
    border-color: var(--color-accent);
    color: var(--color-accent-text);
  }

  .primary:hover {
    background: var(--color-accent);
  }
</style>
