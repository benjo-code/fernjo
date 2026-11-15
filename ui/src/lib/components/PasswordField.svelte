<script lang="ts">
  import Icon from "./Icon.svelte";

  let {
    id,
    label,
    value = $bindable(""),
    error = null,
    placeholder = "",
    onedit,
  }: {
    id: string;
    label: string;
    /** Lives only in the input; the owner clears it after a successful submit. */
    value?: string;
    /** Shown in the danger state under the field and announced to screen readers. */
    error?: string | null;
    placeholder?: string;
    onedit?: () => void;
  } = $props();

  let shown = $state(false);
  let input: HTMLInputElement | undefined = $state();
  const errorId = $derived(`${id}-error`);

  export function focus() {
    input?.focus();
  }
</script>

<div class="field">
  <label for={id}>{label}</label>
  <div class="box" class:invalid={error !== null}>
    <input
      bind:this={input}
      {id}
      type={shown ? "text" : "password"}
      {placeholder}
      autocomplete="off"
      spellcheck="false"
      aria-invalid={error !== null ? "true" : undefined}
      aria-describedby={error !== null ? errorId : undefined}
      bind:value
      oninput={() => onedit?.()}
    />
    <button
      type="button"
      class="toggle"
      aria-label={`Show ${label.toLowerCase()}`}
      aria-pressed={shown}
      onclick={() => (shown = !shown)}
    >
      <Icon name={shown ? "eye-off" : "eye"} size={14} />
    </button>
  </div>
  {#if error !== null}
    <p class="error" id={errorId} role="alert">
      <Icon name="alert" size={14} />
      <span>{error}</span>
    </p>
  {/if}
</div>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  label {
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }

  .box {
    display: flex;
    align-items: center;
    height: 28px;
    background: var(--color-surface);
    border: 1px solid var(--color-border-input);
    border-radius: var(--radius-sm);
  }

  .box:focus-within {
    outline: 1px solid var(--color-focus);
    outline-offset: -1px;
  }

  .box.invalid {
    border-color: var(--color-danger);
    box-shadow: 0 0 0 1px var(--color-danger);
  }

  input {
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0 var(--space-3);
    border: 0;
    background: transparent;
    color: var(--color-text);
    font: inherit;
    outline: none;
  }

  .toggle {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 100%;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--color-text-muted);
    cursor: default;
  }

  .toggle:hover,
  .toggle[aria-pressed="true"] {
    color: var(--color-text);
  }

  .error {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    margin: 0;
    color: var(--color-danger);
    font-size: var(--text-sm);
    line-height: 1.3;
  }

  .error span {
    min-width: 0;
    overflow-wrap: anywhere;
  }
</style>
