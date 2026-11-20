<script lang="ts">
  import { tick } from "svelte";
  import type { VaultStore } from "../vault.svelte";
  import Icon from "./Icon.svelte";
  import PasswordField from "./PasswordField.svelte";

  let {
    vault,
    open = $bindable(false),
  }: {
    vault: VaultStore;
    /** True while the form is expanded, so the sidebar can give the tree some room back. */
    open?: boolean;
  } = $props();

  const KEYCHAIN_HINT = "Keeps the key in the OS keychain so the vault unlocks at start.";
  const WRONG_PASSWORD = "Wrong master password. Check Caps Lock and try again.";

  const phase = $derived(vault.status?.state ?? null);

  let region: HTMLElement | undefined = $state();
  let unlockField: PasswordField | undefined = $state();
  let createField: PasswordField | undefined = $state();
  let confirmField: PasswordField | undefined = $state();

  // The master password lives only in these bound inputs and is cleared after a success.
  let password = $state("");
  let confirm = $state("");
  // `null` until the user touches the checkbox; until then it follows what Rust reports
  let choice = $state<boolean | null>(null);
  const autoUnlock = $derived(vault.status?.autoUnlock ?? false);
  const remember = $derived(choice ?? autoUnlock);
  // The note is compact (state text and one button) until the user asks for the form, so it
  // does not squeeze the tree and the properties panel in the common locked / no-vault states.
  let unlocking = $state(false);
  let creating = $state(false);
  let busy = $state(false);
  let unlockError = $state<string | null>(null);
  // True while the last password unlock failed with `rolled_back`: the file is older than the
  // last version this device opened, and the user may choose to open it anyway.
  let olderOffered = $state(false);
  let createError = $state<string | null>(null);
  let confirmError = $state<string | null>(null);

  /** Expands the form and focuses the password field. The chip and the note's button call this. */
  export async function focusForm() {
    if (phase === "none") creating = true;
    else if (phase === "locked") unlocking = true;
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
      unlocking = false;
    } else {
      unlockError = err.code === "wrong_password" ? WRONG_PASSWORD : err.message;
      olderOffered = err.code === "rolled_back";
      // keep focus inside the form (a clicked submit button must not strand it on <body>)
      await tick();
      unlockField?.focus();
    }
  }

  /** Takes the older vault file; the password stays in the field from the failed attempt. */
  async function openOlder() {
    if (busy) return;
    busy = true;
    const err = await vault.unlockOlder(password, remember);
    busy = false;
    if (!err) {
      password = "";
      unlocking = false;
      olderOffered = false;
    } else {
      unlockError = err.code === "wrong_password" ? WRONG_PASSWORD : err.message;
      olderOffered = err.code === "rolled_back";
      await tick();
      unlockField?.focus();
    }
  }

  async function submitCreate(e: SubmitEvent) {
    e.preventDefault();
    if (busy) return;
    if (password !== confirm) {
      confirmError = "The passwords do not match.";
      await tick();
      confirmField?.focus();
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
      await tick();
      createField?.focus();
    }
  }

  function cancelUnlock() {
    unlocking = false;
    password = "";
    unlockError = null;
    olderOffered = false;
  }

  function cancelCreate() {
    creating = false;
    password = "";
    confirm = "";
    createError = null;
    confirmError = null;
  }

  $effect(() => {
    open = (phase === "locked" && unlocking) || (phase === "none" && creating);
  });

  const KEEP_KEY_LINE =
    "Automatic unlock did not work this time (the vault file may be incomplete or damaged). Your stored key was kept. Enter the master password.";
  // The neutral line never claims that a key is stored; unknown codes (newer Rust) get it too.
  const NEUTRAL_LINE =
    "The OS keychain could not be read, so automatic unlock was skipped. Enter the master password.";
  // `keychain_unavailable` shows no line on purpose: on a machine without a keychain service it
  // is reported at every start, also to users who never turned automatic unlock on.
  const ISSUE_LINES: Record<string, string | null> = {
    key_not_accepted: KEEP_KEY_LINE,
    unavailable: KEEP_KEY_LINE,
    keychain_unavailable: null,
    rolled_back:
      "Automatic unlock did not open this vault: the file is older than the last version this device opened. Enter the master password to review.",
  };
  const issue = $derived(vault.status?.autoUnlockIssue ?? null);
  const issueLine = $derived(
    issue
      ? Object.prototype.hasOwnProperty.call(ISSUE_LINES, issue)
        ? ISSUE_LINES[issue]
        : NEUTRAL_LINE
      : null,
  );

  const backupPath = $derived(vault.status?.path ? `${vault.status.path}.bak` : null);
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
      {#if phase === "locked" && !unlocking}
        <button type="button" class="btn small" onclick={() => void focusForm()}>Unlock</button>
      {:else if phase === "none" && !creating}
        <button type="button" class="btn small" onclick={() => void focusForm()}>
          Create vault
        </button>
      {/if}
    </div>

    {#if phase === "locked"}
      <p class="body">Connections that use a stored credential need the vault unlocked.</p>
      {#if issueLine}
        <p class="body" role="status">{issueLine}</p>
      {/if}
      {#if unlocking}
        <form onsubmit={submitUnlock}>
          <PasswordField
            bind:this={unlockField}
            id="vault-unlock-password"
            label="Master password"
            bind:value={password}
            error={unlockError}
            onedit={() => {
              unlockError = null;
              olderOffered = false;
            }}
          />
          <label class="check">
            <input
              type="checkbox"
              checked={remember}
              onchange={(e) => (choice = e.currentTarget.checked)}
            />
            <span>Unlock automatically on this device</span>
          </label>
          <p class="hint">
            {KEYCHAIN_HINT}{remember || autoUnlock ? " Lock then lasts until the next start." : ""}
          </p>
          <div class="actions">
            <button type="submit" class="btn small" aria-disabled={busy}>Unlock</button>
            {#if olderOffered}
              <button
                type="button"
                class="btn small"
                aria-disabled={busy}
                onclick={() => void openOlder()}
              >
                Open the older vault
              </button>
            {/if}
            <button type="button" class="btn small" onclick={cancelUnlock}>Cancel</button>
          </div>
        </form>
      {/if}
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
            bind:this={confirmField}
            id="vault-create-confirm"
            label="Repeat master password"
            bind:value={confirm}
            error={confirmError}
            onedit={() => (confirmError = null)}
          />
          <p class="body">There is no way to recover a forgotten master password.</p>
          <label class="check">
            <input
              type="checkbox"
              checked={remember}
              onchange={(e) => (choice = e.currentTarget.checked)}
            />
            <span>Unlock automatically on this device</span>
          </label>
          <p class="hint">{KEYCHAIN_HINT}</p>
          <div class="actions">
            <button type="submit" class="btn small primary" aria-disabled={busy}>
              Create vault
            </button>
            <button type="button" class="btn small" onclick={cancelCreate}>Cancel</button>
          </div>
        </form>
      {/if}
    {:else}
      <p class="body">{vault.status?.reason ?? "The vault file cannot be used."}</p>
      {#if vault.status?.path}
        <p class="path">{vault.status.path}</p>
      {/if}
      <p class="body">
        The file is left untouched. To restore an earlier version, close fernjo and copy
        vault.fernjo.bak over vault.fernjo.
      </p>
      {#if backupPath}
        <p class="path">{backupPath}</p>
      {/if}
    {/if}
  </div>
{/if}

<style>
  .note {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0 var(--space-3) var(--space-3);
    padding: var(--space-3) var(--space-4);
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
    flex: 1;
    min-width: 0;
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
    flex-wrap: wrap;
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

  .btn[aria-disabled="true"] {
    opacity: 0.6;
  }
</style>
