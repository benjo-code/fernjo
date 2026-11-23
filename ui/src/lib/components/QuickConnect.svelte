<script lang="ts">
  import { toApiError } from "../api";
  import type { SessionStore } from "../sessions.svelte";
  import { isMac, shortcutFor, shortcutKeys } from "../shortcuts";
  import Icon from "./Icon.svelte";

  let { sessions, mac = isMac() }: { sessions: SessionStore; mac?: boolean } = $props();

  const hint = $derived(`Quick connect (${shortcutKeys(shortcutFor("quickConnect")!, mac)})`);

  let input = $state<HTMLInputElement>();
  let text = $state("");
  let error = $state<string | null>(null);
  let busy = false;

  /** Puts the caret in the field with the text selected (the Ctrl+Shift+N shortcut). */
  export function focus() {
    input?.focus();
    input?.select();
  }

  async function submit() {
    const value = text.trim();
    if (value === "" || busy) return;
    busy = true;
    try {
      await sessions.openQuick(value);
      text = "";
      error = null;
    } catch (e) {
      error = toApiError(e).message;
    } finally {
      busy = false;
    }
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      void submit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      text = "";
      error = null;
      input?.blur();
    }
  }
</script>

<div class="quick">
  <label class="field">
    <span class="icon"><Icon name="ssh" size={14} /></span>
    <input
      bind:this={input}
      type="text"
      placeholder="user@host[:port]"
      aria-label="Quick connect"
      aria-invalid={error !== null}
      aria-describedby={error !== null ? "quick-connect-error" : undefined}
      title={hint}
      autocomplete="off"
      spellcheck="false"
      bind:value={text}
      oninput={() => (error = null)}
      {onkeydown}
    />
  </label>
  {#if error !== null}
    <p id="quick-connect-error" class="error" role="alert">{error}</p>
  {/if}
</div>

<style>
  .quick {
    padding: 0 var(--space-3) var(--space-3);
  }

  .field {
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

  .field:focus-within {
    outline: 1px solid var(--color-focus);
    outline-offset: -1px;
  }

  .error {
    margin: var(--space-2) 0 0;
    font-size: var(--text-sm);
    color: var(--color-danger);
  }
</style>
