<script lang="ts">
  import type { Snippet } from "svelte";
  import type { TreeStore } from "../store.svelte";
  import FilterBox from "./FilterBox.svelte";
  import NoticeBar from "./NoticeBar.svelte";
  import TreeView from "./TreeView.svelte";

  let { store, children }: { store: TreeStore; children?: Snippet } = $props();
</script>

<aside class="sidebar" aria-label="Connections">
  <div class="head"><FilterBox {store} /></div>
  <NoticeBar {store} />
  <div class="tree-slot"><TreeView {store} /></div>
  {@render children?.()}
</aside>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--color-sidebar);
    border-right: 1px solid var(--color-border);
  }

  .head {
    padding: var(--space-3);
  }

  .tree-slot {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
    overflow-y: auto;
  }
</style>
