<script lang="ts">
  import type { SyncStore } from "../sync.svelte";
  import { syncLine } from "../sync-line";

  let {
    sync,
    onreseed,
    ondetails,
  }: {
    sync: SyncStore;
    /** "Re-seed…" next to a hub that is older than this device. */
    onreseed?: () => void;
    /** "Details" next to a sync problem, and a click on the plain line: opens Settings. */
    ondetails?: () => void;
  } = $props();

  const line = $derived(syncLine(sync.status));
</script>

{#if line}
  <div class="sync" class:attention={line.tone === "attention"}>
    <button type="button" class="text" title={line.title} onclick={() => ondetails?.()}>
      {line.text}
    </button>
    {#if line.action === "reseed"}
      <button type="button" class="act" onclick={() => onreseed?.()}>Re-seed…</button>
    {:else if line.action === "details"}
      <button type="button" class="act" onclick={() => ondetails?.()}>Details</button>
    {/if}
  </div>
{/if}

<style>
  .sync {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    padding: var(--space-1) var(--space-3);
    border-top: 1px solid var(--color-border);
    font-size: var(--text-sm);
    color: var(--color-text-muted);
  }

  .attention .text {
    color: var(--color-warning);
  }

  .text,
  .act {
    min-width: 0;
    height: 28px;
    padding: 0 var(--space-2);
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: transparent;
    color: inherit;
    font: inherit;
    cursor: default;
  }

  .text {
    flex: 0 1 auto;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .act {
    flex: none;
    border-color: var(--color-border-strong);
    color: var(--color-text);
  }

  .text:hover,
  .act:hover {
    background: var(--color-hover);
  }

  .text:focus-visible,
  .act:focus-visible {
    outline: 1px solid var(--color-focus);
    outline-offset: -1px;
  }
</style>
