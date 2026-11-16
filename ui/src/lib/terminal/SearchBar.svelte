<script lang="ts">
  import { onMount } from "svelte";

  let {
    search,
    onClose,
  }: {
    search: (term: string, dir: "next" | "previous") => boolean;
    onClose: () => void;
  } = $props();

  let term = $state("");
  let found = $state(true);
  let input = $state<HTMLInputElement>();

  onMount(() => input?.focus());

  function run(dir: "next" | "previous") {
    found = search(term, dir) || term === "";
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onClose();
    } else if (e.key === "Enter") {
      e.preventDefault();
      run(e.shiftKey ? "previous" : "next");
    }
  }
</script>

<div class="bar" role="search">
  <input
    bind:this={input}
    type="text"
    placeholder="Search terminal"
    aria-label="Search terminal"
    autocomplete="off"
    spellcheck="false"
    value={term}
    oninput={(e) => {
      term = e.currentTarget.value;
      run("next");
    }}
    {onkeydown}
  />
  <span class="status" aria-live="polite">{found ? "" : "No matches"}</span>
  <button type="button" aria-label="Previous match" onclick={() => run("previous")}>Prev</button>
  <button type="button" aria-label="Next match" onclick={() => run("next")}>Next</button>
  <button type="button" aria-label="Close search" onclick={onClose}>Close</button>
</div>

<style>
  .bar {
    position: absolute;
    top: var(--space-3);
    right: var(--space-5);
    z-index: 2;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--color-surface-raised);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    box-shadow: 0 2px 8px var(--color-shadow);
    font-family: var(--font-ui);
    font-size: 13px;
  }

  input {
    width: 200px;
    height: 24px;
    padding: 0 var(--space-3);
    background: var(--color-surface);
    color: var(--color-text);
    border: 1px solid var(--color-border-input);
    border-radius: var(--radius-sm);
    font: inherit;
  }

  input:focus-visible,
  button:focus-visible {
    outline: 2px solid var(--color-focus);
    outline-offset: 1px;
  }

  .status {
    min-width: 5.5em;
    color: var(--color-danger);
  }

  button {
    height: 24px;
    padding: 0 var(--space-3);
    background: transparent;
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    font: inherit;
    cursor: pointer;
  }

  button:hover {
    background: var(--color-hover);
  }
</style>
