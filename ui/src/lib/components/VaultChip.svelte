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
      ? "Vault: unlocked"
      : phase === "locked"
        ? "Vault: locked"
        : phase === "none"
          ? "Vault: none"
          : "Vault unusable",
  );
  const label = $derived(
    phase === "unlocked"
      ? "Vault: unlocked, open the credentials"
      : phase === "locked"
        ? "Vault: locked, unlock"
        : phase === "none"
          ? "Vault: none, create one"
          : "Vault unusable, show details",
  );

  const title = $derived(
    phase === "unlocked" ? "Credential vault, open the credentials" : "Credential vault",
  );

  function onclick() {
    if (phase === "unlocked") onOpenManager();
    else onFocusUnlock();
  }
</script>

{#if phase === null && vault.unavailable}
  <span class="chip unavailable">
    <Icon name="alert" size={14} /><span>Vault unavailable</span>
  </span>
{:else if phase !== null}
  <button type="button" class="chip" aria-label={label} {title} {onclick}>
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

  .unavailable {
    color: var(--color-text-muted);
  }

  button.chip:hover {
    background: var(--color-hover);
  }
</style>
