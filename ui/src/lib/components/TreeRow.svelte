<script lang="ts">
  import type { DropPosition } from "../drop-target";
  import type { Row } from "../rows";
  import Icon from "./Icon.svelte";

  let {
    row,
    selected,
    tabbable,
    renaming,
    drop = null,
    dragging = false,
    onrename,
    oncancel,
  }: {
    row: Row;
    selected: boolean;
    tabbable: boolean;
    renaming: boolean;
    /** Where the dragged node would land relative to this row, if it is the drop target. */
    drop?: DropPosition | null;
    /** This row is the one being dragged. */
    dragging?: boolean;
    onrename: (value: string) => void;
    oncancel: () => void;
  } = $props();

  let input = $state<HTMLInputElement>();
  let finished = false;

  $effect(() => {
    if (renaming && input) {
      finished = false;
      input.focus();
      input.select();
    }
  });

  function commit(value: string) {
    if (finished) return;
    finished = true;
    onrename(value);
  }

  function cancel() {
    if (finished) return;
    finished = true;
    oncancel();
  }

  function onkeydown(e: KeyboardEvent) {
    // the tree must not see keys typed into the editor
    e.stopPropagation();
    if (e.key === "Enter") {
      e.preventDefault();
      commit(e.currentTarget instanceof HTMLInputElement ? e.currentTarget.value : "");
    } else if (e.key === "Escape") {
      e.preventDefault();
      cancel();
    }
  }
</script>

<div
  class="row"
  class:selected
  class:folder={row.node.type === "folder"}
  class:dragging
  class:drop-before={drop === "before"}
  class:drop-after={drop === "after"}
  class:drop-inside={drop === "inside"}
  role="treeitem"
  aria-level={row.depth + 1}
  aria-selected={selected}
  aria-expanded={row.node.type === "folder" ? row.expanded : undefined}
  data-node-id={row.node.id}
  tabindex={tabbable ? 0 : -1}
  title={row.node.name}
  style:--depth={row.depth}
>
  {#if row.node.type === "folder"}
    <span class="chevron" class:open={row.expanded} class:empty={!row.hasChildren}>
      {#if row.hasChildren}<Icon name="folder-chevron" size={14} />{/if}
    </span>
  {:else}
    <span class="chevron"></span>
    <span class="protocol"><Icon name={row.node.protocol} /></span>
  {/if}
  {#if renaming}
    <input
      bind:this={input}
      class="rename"
      type="text"
      aria-label="Name"
      value={row.node.name}
      spellcheck="false"
      autocomplete="off"
      {onkeydown}
      onblur={(e) => commit(e.currentTarget.value)}
    />
  {:else}
    <span class="name">{row.node.name}</span>
  {/if}
</div>

<style>
  .row {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: var(--row-height-tree);
    padding: 0 var(--space-3) 0 calc(var(--space-3) + var(--tree-indent) * var(--depth));
    color: var(--color-text);
    white-space: nowrap;
    cursor: default;
    user-select: none;
  }

  .row:hover {
    background: var(--color-hover);
  }

  .row.selected {
    background: var(--color-selection);
    font-weight: var(--weight-strong);
  }

  .row.selected::before {
    content: "";
    position: absolute;
    top: 0;
    bottom: 0;
    left: 0;
    width: 3px;
    background: var(--color-selection-bar);
  }

  .row.dragging {
    opacity: 0.55;
  }

  .row.drop-before::after,
  .row.drop-after::after {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    height: 2px;
    background: var(--color-selection-bar);
    pointer-events: none;
  }

  .row.drop-before::after {
    top: 0;
  }

  .row.drop-after::after {
    bottom: 0;
  }

  .row.drop-inside {
    background: var(--color-hover);
    outline: 1px solid var(--color-border-strong);
    outline-offset: -1px;
  }

  .folder {
    font-weight: var(--weight-strong);
  }

  .chevron {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: var(--row-height-tree);
    color: var(--color-text-muted);
  }

  .chevron:not(.open):not(.empty) :global(.icon) {
    transform: rotate(-90deg);
  }

  .protocol {
    display: flex;
    flex: none;
    color: var(--color-ssh);
  }

  .name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .rename {
    flex: 1;
    min-width: 0;
    height: 20px;
    margin: 0 calc(var(--space-2) * -1) 0 0;
    padding: 0 var(--space-2);
    border: 1px solid var(--color-border-input);
    border-radius: var(--radius-sm);
    background: var(--color-surface);
    color: var(--color-text);
    font: inherit;
    font-weight: var(--weight-regular);
    outline: none;
    user-select: text;
  }
</style>
