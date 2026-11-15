<script lang="ts">
  import { tick } from "svelte";
  import type { VaultStore } from "../vault.svelte";
  import Icon from "./Icon.svelte";
  import PasswordField from "./PasswordField.svelte";

  let { vault }: { vault: VaultStore } = $props();

  const WRONG_PASSWORD = "Wrong master password. Check Caps Lock and try again.";

  const phase = $derived(vault.status?.state ?? null);

  let region: HTMLElement | undefined = $state();
  let unlockField: PasswordField | undefined = $state();
  let createField: PasswordField | undefined = $state();

  // The master password lives only in these bound inputs and is cleared after a success.
  let password = $state("");
  let confirm = $state("");
  // svelte-ignore state_referenced_locally
  let remember = $state(vault.status?.autoUnlock ?? false);
  let creating = $state(false);
  let busy = $state(false);
  let unlockError = $state<string | null>(null);
  let createError = $state<string | null>(null);
  let confirmError = $state<string | null>(null);

  /** Brings the form to the user: the chip calls this. */
  export async function focusForm() {
    if (phase === "none") creating = true;
    await tick();
    if (phase === "locked") unlockField?.focus();
    else if (phase === "none") createField?.focus();
    else region?.focus();
  }

  async function submitUnlock(e: SubmitEvent) {
    e.preventDefault();
    if (busy) return;
    busy = true;
    const err = await vault.unlock(password, remember);
    busy = false;
    if (!err) {
      password = "";
    } else {
      unlockError = err.code === "wrong_password" ? WRONG_PASSWORD : err.message;
    }
  }

  async function submitCreate(e: SubmitEvent) {
    e.preventDefault();
    if (busy) return;
    if (password !== confirm) {
      confirmError = "The passwords do not match.";
      return;
    }
    busy = true;
    const err = await vault.create(password, remember);
    busy = false;
    if (!err) {
      password = "";
      confirm = "";
      creating = false;
    } else {
      createError = err.message;
    }
  }

  function cancelCreate() {
    creating = false;
    password = "";
    confirm = "";
    createError = null;
    confirmError = null;
  }
</script>

{#if phase !== null && phase !== "unlocked"}
  <div
    class="note"
    role="region"
    aria-labelledby="vault-note-title"
    tabindex="-1"
    bind:this={region}
  >
    <div class="head">
      <Icon name={phase === "unusable" ? "alert" : "lock"} size={16} />
      <h2 id="vault-note-title">
        {phase === "none" ? "No vault yet" : phase === "locked" ? "Vault locked" : "Vault unusable"}
      </h2>
    </div>

    {#if phase === "locked"}
      <p class="body">Connections with stored credentials ask for it.</p>
      <form onsubmit={submitUnlock}>
        <PasswordField
          bind:this={unlockField}
          id="vault-unlock-password"
          label="Master password"
          bind:value={password}
          error={unlockError}
          onedit={() => (unlockError = null)}
        />
        <label class="check">
          <input type="checkbox" bind:checked={remember} />
          <span>Unlock automatically on this device</span>
        </label>
        {#if remember}
          <p class="hint">Lock then lasts until the next start.</p>
        {/if}
        <div class="actions">
          <button type="submit" class="btn small" disabled={busy}>Unlock</button>
        </div>
      </form>
    {:else if phase === "none"}
      <p class="body">
        Credentials are stored in an encrypted vault protected by a master password.
      </p>
      {#if creating}
        <form onsubmit={submitCreate}>
          <PasswordField
            bind:this={createField}
            id="vault-create-password"
            label="Master password"
            bind:value={password}
            error={createError}
            onedit={() => {
              createError = null;
              confirmError = null;
            }}
          />
          <PasswordField
            id="vault-create-confirm"
            label="Repeat master password"
            bind:value={confirm}
            error={confirmError}
            onedit={() => (confirmError = null)}
          />
          <p class="body">There is no way to recover a forgotten master password.</p>
          <label class="check">
            <input type="checkbox" bind:checked={remember} />
            <span>Unlock automatically on this device</span>
          </label>
          <div class="actions">
            <button type="submit" class="btn small primary" disabled={busy}>Create vault</button>
            <button type="button" class="btn small" onclick={cancelCreate}>Cancel</button>
          </div>
        </form>
      {:else}
        <div class="actions">
          <button type="button" class="btn small" onclick={() => void focusForm()}>
            Create vault
          </button>
        </div>
      {/if}
    {:else}
      <p class="body">The vault file cannot be used: {vault.status?.reason ?? "unknown reason"}.</p>
      {#if vault.status?.path}
        <p class="path">{vault.status.path}</p>
      {/if}
      <p class="body">The file is left untouched.</p>
    {/if}
  </div>
{/if}

<style>
  .note {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    margin: 0 var(--space-3) var(--space-3);
    padding: var(--space-4);
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
  }

  .note:focus {
    outline: 1px solid var(--color-focus);
    outline-offset: -1px;
  }

  .head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  h2 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: var(--weight-strong);
  }

  .body,
  .hint {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--text-sm);
    line-height: 1.3;
  }

  .path {
    margin: 0;
    color: var(--color-text);
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
  }

  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
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

  .small {
    height: 26px;
    padding: 0 var(--space-4);
    font-size: var(--text-sm);
  }

  .primary {
    background: var(--color-accent);
    border-color: var(--color-accent);
    color: var(--color-accent-text);
  }

  .primary:hover {
    background: var(--color-accent);
  }

  .btn:disabled {
    opacity: 0.6;
  }
</style>
