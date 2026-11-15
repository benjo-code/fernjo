<script lang="ts">
  import type { VaultStore } from "../vault.svelte";
  import Icon from "./Icon.svelte";

  let {
    vault,
    onOpenManager,
    onFocusUnlock,
  }: {
    vault: VaultStore;
    /** Unlocked: open the credential manager. */
    onOpenManager: () => void;
    /** Anything else: bring the lock note's form into view and focus it. */
    onFocusUnlock: () => void;
  } = $props();

  const phase = $derived(vault.status?.state ?? null);
  const text = $derived(
    phase === "unlocked"
      ? "Unlocked"
      : phase === "locked"
        ? "Locked"
        : phase === "none"
          ? "No vault"
          : "Vault unusable",
  );
  const label = $derived(
    phase === "unlocked"
      ? "Vault unlocked, open credentials"
      : phase === "locked"
        ? "Vault locked, unlock"
        : phase === "none"
          ? "No vault yet, create one"
          : "Vault unusable, show details",
  );

  function onclick() {
    if (phase === "unlocked") onOpenManager();
    else onFocusUnlock();
  }
</script>

{#if phase !== null}
  <button type="button" class="chip" aria-label={label} {onclick}>
    <Icon
      name={phase === "unlocked" ? "unlock" : phase === "unusable" ? "alert" : "lock"}
      size={14}
    />
    <span>{text}</span>
  </button>
{/if}

<style>
  .chip {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--space-3);
    height: 28px;
    padding: 0 var(--space-3);
    background: transparent;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-text);
    font: inherit;
    font-size: var(--text-sm);
    cursor: default;
  }

  .chip:hover {
    background: var(--color-hover);
  }
</style>
