<script lang="ts">
  import { tick } from "svelte";

  let {
    id,
    value,
    label,
    placeholder = "",
    title,
    inputmode,
    onCommit,
    onedit,
  }: {
    id: string;
    /** The committed value. The field follows it whenever it changes. */
    value: string;
    label: string;
    placeholder?: string;
    title?: string;
    inputmode?: "numeric" | "text";
    /** Resolves to `false` if the value was not accepted; the field then shows `value` again. */
    onCommit: (text: string) => Promise<boolean>;
    onedit?: () => void;
  } = $props();

  // follows `value`, but can be typed over until the next commit or revert
  let draft = $derived(value);
  let inflight = false;

  async function commit() {
    if (inflight || draft === value) return;
    inflight = true;
    try {
      await onCommit(draft);
      await tick();
      // on success `value` has moved on, otherwise this restores the old value
      draft = value;
    } finally {
      inflight = false;
    }
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      void commit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      draft = value;
    }
  }
</script>

<input
  {id}
  type="text"
  {inputmode}
  {placeholder}
  {title}
  aria-label={label}
  autocomplete="off"
  spellcheck="false"
  bind:value={draft}
  oninput={() => onedit?.()}
  onblur={() => void commit()}
  {onkeydown}
/>

<style>
  input {
    width: 100%;
    min-width: 0;
    height: 22px;
    padding: 0 6px;
    border: 1px solid var(--color-border-input);
    border-radius: var(--radius-sm);
    background: var(--color-surface);
    color: var(--color-text);
    font: inherit;
  }

  input::placeholder {
    color: var(--color-text-muted);
    opacity: 1;
  }
</style>
