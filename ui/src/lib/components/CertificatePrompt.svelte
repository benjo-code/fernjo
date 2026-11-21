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
    prompt: Extract<Prompt, { kind: "certificate" }>;
  } = $props();

  let root: HTMLElement | undefined = $state();

  // The keyboard lands on the dialog, not on a button: Enter typed into the desktop a moment ago
  // must not trust a certificate. Tab then reaches "Trust always" first.
  $effect(() => {
    untrack(() => root)?.focus();
  });

  // The answer has the same shape as a host key answer; Rust maps it to the certificate decision.
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

<PromptFrame bind:root title="Trust this server certificate?" icon="rdp" {onkeydown}>
  <p class="body">
    fernjo has not connected to this server before. Check that the fingerprint is the one you
    expect.
  </p>
  <dl>
    <dt>Host</dt>
    <dd>{prompt.host}</dd>
    <dt>Port</dt>
    <dd>{prompt.port}</dd>
    <dt>Fingerprint</dt>
    <dd><code class="fingerprint">{prompt.fingerprint}</code></dd>
  </dl>
  <div class="actions">
    <button type="button" class="btn primary" onclick={() => decide("always")}>
      Trust always
    </button>
    <button type="button" class="btn" onclick={() => decide("once")}>Trust once</button>
    <button type="button" class="btn" onclick={() => decide("reject")}>Cancel</button>
  </div>
  <p class="choice-note">
    Trust always remembers this certificate on this device. Trust once asks again next time.
  </p>
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

  .choice-note {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--text-sm);
    line-height: 1.3;
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
