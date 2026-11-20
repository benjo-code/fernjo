<script lang="ts">
  import type { Snippet } from "svelte";
  import type { Source } from "../types";
  import Icon from "./Icon.svelte";

  let {
    name,
    label,
    source = null,
    hasValue = false,
    error = null,
    wrapAux = false,
    onreset,
    children,
  }: {
    /** Stable key used for `data-field`. */
    name: string;
    label: string;
    /** Where the effective value comes from; leave out for fields that are never inherited. */
    source?: Source | null;
    /** An effective value exists (so a built-in default can be named as such). */
    hasValue?: boolean;
    error?: string | null;
    /**
     * The aux column drops under the value when the line is too narrow for both, instead of
     * squeezing the value (a dropdown that must stay readable).
     */
    wrapAux?: boolean;
    onreset?: () => void;
    /** Receives the id to put on the field so the label points at it. */
    children: Snippet<[string]>;
  } = $props();

  const id = $props.id();
</script>

<div class="prop" class:wrap={wrapAux} data-field={name}>
  <label class="key" for={id}>{label}</label>
  <div class="val">{@render children(id)}</div>
  <div class="aux">
    {#if source?.kind === "local" && onreset}
      <button type="button" class="reset" onclick={onreset}>
        <Icon name="reset" size={12} />Inherit
      </button>
    {:else if source?.kind === "folder"}
      <span class="mark">from {source.name}</span>
    {:else if source?.kind === "default" && hasValue}
      <span class="mark">default</span>
    {/if}
  </div>
  {#if error}
    <p class="error" role="alert"><Icon name="alert" size={12} /><span>{error}</span></p>
  {/if}
</div>

<style>
  .prop {
    display: grid;
    grid-template-columns: 84px minmax(0, 1fr) auto;
    align-items: center;
    column-gap: var(--space-2);
    min-height: var(--row-height-tree);
    padding: 1px 0;
  }

  /* flex-wrap instead of the grid: the value keeps at least 100px, the aux column wraps below it */
  .prop.wrap {
    display: flex;
    flex-wrap: wrap;
    row-gap: var(--space-1);
  }

  .wrap .key {
    flex: none;
    width: 84px;
  }

  .wrap .val {
    flex: 1 1 100px;
  }

  .wrap .aux {
    flex: none;
    margin-left: auto;
  }

  .wrap .error {
    flex: 0 0 100%;
    box-sizing: border-box;
    padding-left: 92px;
  }

  .key {
    color: var(--color-text-muted);
    font-size: var(--text-sm);
    line-height: 1.3;
  }

  .val {
    min-width: 0;
  }

  .aux {
    display: flex;
    justify-content: flex-end;
    min-width: 0;
  }

  .mark {
    max-width: 96px;
    overflow: hidden;
    color: var(--color-text-muted);
    font-size: var(--text-sm);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .reset {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    height: 20px;
    padding: 0 var(--space-2);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text);
    font: inherit;
    font-size: var(--text-sm);
    cursor: default;
  }

  .reset:hover {
    background: var(--color-hover);
  }

  .error {
    display: flex;
    grid-column: 2 / 4;
    align-items: flex-start;
    gap: var(--space-2);
    margin: var(--space-1) 0 0;
    color: var(--color-danger);
    font-size: var(--text-sm);
    line-height: 1.3;
  }
</style>
