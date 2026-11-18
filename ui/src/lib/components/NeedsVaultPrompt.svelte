<script lang="ts">
  import { tick, untrack } from "svelte";
  import type { SessionStore, Tab } from "../sessions.svelte";
  import type { VaultStore } from "../vault.svelte";
  import PasswordField from "./PasswordField.svelte";
  import PromptFrame from "./PromptFrame.svelte";

  let {
    tab,
    sessions,
    vault,
  }: {
    tab: Tab;
    sessions: SessionStore;
    vault: VaultStore;
  } = $props();

  const WRONG_PASSWORD = "Wrong master password. Check Caps Lock and try again.";
  // The choice to open an older vault file lives in the vault note, not in this prompt.
  const ROLLED_BACK_HINT = " Unlock from the vault note to review it.";

  let field: PasswordField | undefined = $state();
  let password = $state("");
  // `null` until the user touches the checkbox; until then it follows what Rust reports
  let choice = $state<boolean | null>(null);
  const remember = $derived(choice ?? vault.status?.autoUnlock ?? false);
  let busy = $state(false);
  let error = $state<string | null>(null);

  $effect(() => {
    untrack(() => field)?.focus();
  });

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (busy) return;
    busy = true;
    // the typed password is emptied before the call, the call keeps its own copy
    const typed = password;
    password = "";
    const err = await vault.unlock(typed, remember);
    busy = false;
    if (!err) {
      sessions.resume(tab.id, false);
    } else {
      error =
        err.code === "wrong_password"
          ? WRONG_PASSWORD
          : err.code === "rolled_back"
            ? err.message + ROLLED_BACK_HINT
            : err.message;
      await tick();
      field?.focus();
    }
  }

  function cancel() {
    password = "";
    sessions.resume(tab.id, true);
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      cancel();
    }
  }
</script>

<PromptFrame title="{tab.title} needs the vault" icon="lock" {onkeydown}>
  <p class="body">
    The vault is locked and this connection uses a stored credential. Unlock it to connect; other
    tabs keep running.
  </p>
  <form onsubmit={submit}>
    <PasswordField
      bind:this={field}
      id="session-vault-password"
      label="Master password"
      bind:value={password}
      {error}
      onedit={() => (error = null)}
    />
    <label class="check">
      <input
        type="checkbox"
        checked={remember}
        onchange={(e) => (choice = e.currentTarget.checked)}
      />
      <span>Unlock automatically on this device</span>
    </label>
    <div class="actions">
      <button type="submit" class="btn primary" aria-disabled={busy}>Unlock and connect</button>
      <button type="button" class="btn" onclick={cancel}>Cancel</button>
    </div>
  </form>
</PromptFrame>

<style>
  .body {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--text-sm);
    line-height: 1.3;
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .check {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    font-size: var(--text-sm);
  }

  .check input {
    margin: 0;
    accent-color: var(--color-text);
  }

  .actions {
    display: flex;
    gap: var(--space-3);
  }

  .primary {
    background: var(--color-accent);
    border-color: var(--color-accent);
    color: var(--color-accent-text);
  }

  .primary:hover {
    background: var(--color-accent);
  }

  .btn[aria-disabled="true"] {
    opacity: 0.6;
  }
</style>
