<script lang="ts">
  import type { TreeStore } from "../store.svelte";
  import { isMac, shortcutFor, shortcutKeys } from "../shortcuts";
  import Icon from "./Icon.svelte";

  let { store, mac = isMac() }: { store: TreeStore; mac?: boolean } = $props();

  const hint = $derived(`Filter connections (${shortcutKeys(shortcutFor("focusFilter")!, mac)})`);

  let input = $state<HTMLInputElement>();

  /** Puts the caret in the field with the text selected, ready to be typed over. */
  export function focus() {
    input?.focus();
    input?.select();
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && store.filter !== "") {
      e.preventDefault();
      store.setFilter("");
    }
  }
</script>

<label class="filter">
  <span class="icon"><Icon name="search" size={14} /></span>
  <input
    bind:this={input}
    type="text"
    placeholder="Filter"
    aria-label="Filter connections"
    title={hint}
    autocomplete="off"
    spellcheck="false"
    value={store.filter}
    oninput={(e) => store.setFilter(e.currentTarget.value)}
    {onkeydown}
  />
</label>

<style>
  .filter {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    height: 28px;
    padding: 0 var(--space-3);
    background: var(--color-surface);
    border: 1px solid var(--color-border-input);
    border-radius: var(--radius-sm);
    color: var(--color-text-muted);
  }

  .icon {
    display: flex;
  }

  input {
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--color-text);
    font: inherit;
    outline: none;
  }

  input::placeholder {
    color: var(--color-text-muted);
    opacity: 1;
  }

  .filter:focus-within {
    outline: 1px solid var(--color-focus);
    outline-offset: -1px;
  }
</style>
