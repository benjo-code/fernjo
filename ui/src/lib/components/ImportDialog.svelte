<script lang="ts">
  import { tick } from "svelte";
  import type { ImportStore } from "../import.svelte";
  import { collectIds } from "../rows";
  import type { TreeStore } from "../store.svelte";
  import type { ImportKind } from "../types";
  import type { VaultStore } from "../vault.svelte";
  import ConfirmDialog from "./ConfirmDialog.svelte";
  import ImportReport from "./ImportReport.svelte";
  import PasswordField from "./PasswordField.svelte";

  let {
    store,
    vault,
    tree,
    onclose,
  }: {
    store: ImportStore;
    vault: VaultStore;
    /** Only read for the number of nodes a restore replaces. */
    tree: TreeStore;
    onclose: () => void;
  } = $props();

  const uid = $props.id();

  /** Errors that mean the plan in Rust is gone: the only way on is to start over. */
  const STALE = new Set(["no_pending_import", "not_inspected", "already_inspected"]);
  const MIN_PASSWORD = 8;

  let dialog = $state<HTMLElement>();
  let secretField: PasswordField | undefined = $state();
  let vaultField: PasswordField | undefined = $state();

  let secret = $state("");
  let backupOpen = $state(false);
  let backupPassword = $state("");
  let backupRepeat = $state("");
  let vaultPassword = $state("");
  let vaultRepeat = $state("");
  let vaultError = $state<string | null>(null);
  let vaultBusy = $state(false);
  let confirming = $state(false);
  /** The vault was created or unlocked in this dialog since the file was picked. */
  let vaultReady = $state(false);

  const step = $derived(store.step);
  const locked = $derived(store.busy);
  const backupOk = $derived(
    backupPassword.length >= MIN_PASSWORD && backupPassword === backupRepeat,
  );

  const secretLabel = $derived(
    store.kind === "backup"
      ? "Backup password"
      : store.kind === "mremoteng"
        ? "Custom key"
        : "Password",
  );
  const secretHint = $derived(
    store.kind === "mremoteng"
      ? "This file is encrypted with a custom key. Enter it to read the file."
      : "Enter the password this file was saved with.",
  );

  const dateOf = (iso: string) => iso.slice(0, 10);

  const nodes = (n: number) =>
    n === 1 ? "1 folder or connection" : `${n} folders and connections`;
  const creds = (n: number) => (n === 1 ? "1 credential" : `${n} credentials`);

  /**
   * What the dialog decides on: Rust's word at inspect time (`preview.vault`) beats the UI
   * store's copy, which another window of events may have left stale; once the form here
   * succeeded the vault is open.
   */
  const vaultState = $derived(
    vaultReady ? "unlocked" : (store.preview?.vault.state ?? vault.status?.state),
  );

  // the current credential count is only known while the vault is unlocked and its list loaded
  const credentialsKnown = $derived(
    vault.status?.state === "unlocked" && vault.credentialsState === "loaded",
  );

  const restoreText = $derived.by(() => {
    const b = store.preview?.backup;
    if (!b) return "";
    const now = nodes(collectIds(tree.children).all.size);
    const current = credentialsKnown ? `${now} and ${creds(vault.credentials.length)}` : now;
    const unknown =
      !credentialsKnown && vaultState !== "none"
        ? " The credentials in the vault are replaced too."
        : "";
    return `This replaces your current ${current} with the backup from ${dateOf(b.createdAt)}, which has ${nodes(b.folders + b.connections)} and ${creds(b.credentials)}.${unknown} Open sessions are closed.`;
  });

  /** The backup preview must unlock (or create) the vault before the confirmation can be exact. */
  const restoreNeedsVault = $derived(
    store.preview?.backup != null &&
      store.preview.needsVault &&
      (vaultState === "locked" || vaultState === "none"),
  );

  $effect(() => {
    const root = dialog;
    if (!root) return;
    const opener = document.activeElement as HTMLElement | null;
    root.focus();
    return () => {
      const active = document.activeElement;
      if (!active || active === document.body || root.contains(active)) opener?.focus?.();
    };
  });

  // A clicked button that the next step removes drops focus to <body>, where Escape and the Tab
  // trap would not reach the dialog: put it back on the dialog when it left.
  $effect(() => {
    void step;
    void confirming;
    void tick().then(() => {
      if (confirming || !dialog || dialog.contains(document.activeElement)) return;
      dialog.focus();
    });
  });

  // The field is empty when asked for again after a wrong secret and the clicked Continue
  // button still has focus: a new error (the step stays `key`) puts the cursor back in it.
  $effect(() => {
    void store.error;
    if (step === "key") void tick().then(() => secretField?.focus());
  });
  $effect(() => {
    if (step === "vault" || restoreNeedsVault) void tick().then(() => vaultField?.focus());
  });

  // While busy the buttons use aria-disabled (handlers ignore the click) instead of disabled:
  // a focused button that becomes disabled drops focus to <body> in WebView2 and the dialog
  // would lose its Escape handler and Tab trap.
  async function pick(kind: ImportKind) {
    if (locked) return;
    secret = "";
    vaultReady = false;
    await store.start(kind);
  }

  async function submitSecret(e: SubmitEvent) {
    e.preventDefault();
    if (locked || secret === "") return;
    // the typed secret is emptied before the call, the call keeps its own copy
    const typed = secret;
    secret = "";
    await store.submitSecret(typed);
  }

  async function apply(withoutPasswords: boolean) {
    if (locked) return;
    confirming = false;
    try {
      await store.apply({ withoutPasswords });
      // a restore that found connections.json changed on disk: offer the Reload bar
      if (store.kind === "backup" && store.error?.code === "changed_on_disk") {
        tree.changedOnDisk = true;
      }
    } catch {
      // the snapshot callback failed after Rust applied the plan; the step is already `done`
    }
  }

  /** `resume`: the apply was refused for the vault, so apply again once it is open. */
  async function submitVault(e: SubmitEvent, resume: boolean) {
    e.preventDefault();
    if (vaultBusy || locked) return;
    const none = vaultState === "none";
    if (none && vaultPassword !== vaultRepeat) {
      vaultError = "The passwords do not match.";
      return;
    }
    vaultBusy = true;
    const typed = vaultPassword;
    vaultPassword = "";
    vaultRepeat = "";
    const remember = vault.status?.autoUnlock ?? false;
    const err = none ? await vault.create(typed, false) : await vault.unlock(typed, remember);
    vaultBusy = false;
    if (err) {
      vaultError =
        err.code === "wrong_password"
          ? "Wrong master password. Check Caps Lock and try again."
          : err.code === "rolled_back"
            ? `${err.message} Unlock from the vault note to review it.`
            : err.message;
      await tick();
      vaultField?.focus();
      return;
    }
    vaultError = null;
    vaultReady = true;
    if (resume) await apply(false);
  }

  async function saveBackup(e: SubmitEvent) {
    e.preventDefault();
    if (locked || !backupOk) return;
    const typed = backupPassword;
    backupPassword = "";
    backupRepeat = "";
    await store.exportBackup(typed);
    if (store.exportResult) backupOpen = false;
  }

  async function exportConnections() {
    if (locked) return;
    await store.exportConnections();
  }

  function startOver() {
    store.reset();
    confirming = false;
    vaultReady = false;
  }

  function close() {
    // a running apply cannot be cancelled: closing would hide its result
    if (step === "applying") return;
    secret = "";
    backupPassword = "";
    backupRepeat = "";
    vaultPassword = "";
    vaultRepeat = "";
    // Always: after a failed apply the plan (with its decrypted passwords) is still in Rust, and
    // a pick or inspect that is still running would install one after the dialog is gone.
    void store.cancel();
    onclose();
  }

  function cancelPending() {
    secret = "";
    void store.cancel();
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close();
    } else if (e.key === "Tab" && dialog) {
      // keep focus inside the dialog
      e.preventDefault();
      const items = [
        ...dialog.querySelectorAll<HTMLElement>("button:not(:disabled), input:not(:disabled)"),
      ];
      if (items.length === 0) return;
      const index = items.indexOf(document.activeElement as HTMLElement);
      const next = e.shiftKey ? (index <= 0 ? items.length - 1 : index - 1) : index + 1;
      items[next % items.length]!.focus();
    }
  }
</script>

{#snippet vaultForm(resume: boolean, withoutOption: boolean)}
  {@const none = vaultState === "none"}
  <form onsubmit={(e) => void submitVault(e, resume)}>
    <PasswordField
      bind:this={vaultField}
      id="import-vault-password"
      label="Master password"
      bind:value={vaultPassword}
      error={vaultError}
      onedit={() => (vaultError = null)}
    />
    {#if none}
      <PasswordField
        id="import-vault-repeat"
        label="Repeat master password"
        bind:value={vaultRepeat}
        onedit={() => (vaultError = null)}
      />
      <p class="note">There is no way to recover a forgotten master password.</p>
    {/if}
    <div class="actions">
      <button type="button" class="btn" onclick={cancelPending}>Cancel</button>
      {#if withoutOption}
        <button type="button" class="btn" aria-disabled={locked} onclick={() => void apply(true)}
          >Import without passwords</button
        >
      {/if}
      <button type="submit" class="btn primary" aria-disabled={vaultBusy || locked}
        >{none ? "Create vault" : "Unlock"}{resume
          ? store.kind === "backup"
            ? " and restore"
            : " and import"
          : ""}</button
      >
    </div>
  </form>
{/snippet}

<div class="overlay">
  <div
    bind:this={dialog}
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-labelledby="{uid}-title"
    tabindex="-1"
    {onkeydown}
  >
    <h2 class="title" id="{uid}-title">Import, export &amp; backup</h2>

    {#if step === "key"}
      <p class="file">{store.fileName}</p>
      <p class="note">{secretHint}</p>
      <form onsubmit={submitSecret}>
        <PasswordField
          bind:this={secretField}
          id="import-secret"
          label={secretLabel}
          bind:value={secret}
          error={store.error?.message ?? null}
          onedit={() => {}}
        />
        <div class="actions">
          <button type="button" class="btn" onclick={cancelPending}>Cancel</button>
          <button type="submit" class="btn primary" aria-disabled={locked}>Continue</button>
        </div>
      </form>
    {:else if step === "preview" && store.preview}
      <p class="file">{store.fileName}</p>
      {#if store.preview.report}
        <ImportReport report={store.preview.report} />
        <div class="actions">
          <button type="button" class="btn" onclick={cancelPending}>Cancel</button>
          {#if store.preview.needsVault}
            <button
              type="button"
              class="btn"
              aria-disabled={locked}
              onclick={() => void apply(true)}>Import without passwords</button
            >
          {/if}
          <button
            type="button"
            class="btn primary"
            aria-disabled={locked}
            onclick={() => void apply(false)}>Import</button
          >
        </div>
      {:else if store.preview.backup}
        {@const b = store.preview.backup}
        <p class="note">
          Backup from {dateOf(b.createdAt)} ({b.app}): {b.folders}
          {b.folders === 1 ? "folder" : "folders"}, {b.connections}
          {b.connections === 1 ? "connection" : "connections"}, {b.credentials}
          {b.credentials === 1 ? "credential" : "credentials"}.
        </p>
        {#if restoreNeedsVault}
          <p class="note">
            {vaultState === "none"
              ? "The backup holds credentials. Create the vault they are restored into."
              : "A restore replaces the vault, so unlock it first."}
          </p>
          {@render vaultForm(false, false)}
        {:else}
          <div class="actions">
            <button type="button" class="btn" onclick={cancelPending}>Cancel</button>
            <button
              type="button"
              class="btn danger"
              aria-disabled={locked}
              onclick={() => (confirming = true)}>Restore…</button
            >
          </div>
        {/if}
      {/if}
    {:else if step === "vault"}
      <p class="file">{store.fileName}</p>
      {#if store.kind === "backup"}
        <p class="note">
          {vaultState === "none"
            ? "The backup holds credentials. Create the vault they are restored into."
            : "A restore replaces the vault, so it must be unlocked first."}
        </p>
      {:else}
        <p class="note">
          {vaultState === "none"
            ? "Imported passwords are stored in the vault. Create one with a master password, or import without passwords."
            : "Imported passwords are stored in the vault. Unlock it, or import without passwords."}
        </p>
      {/if}
      {@render vaultForm(true, store.kind !== "backup")}
    {:else if step === "applying"}
      <p class="note" role="status">Working…</p>
    {:else}
      {#if step === "done"}
        <div class="result" role="status">
          {#if store.exportResult}
            <p class="file">Saved {store.exportResult.fileName}</p>
            {#if store.exportResult.droppedCredentials > 0}
              <p class="note">
                {store.exportResult.droppedCredentials}
                {store.exportResult.droppedCredentials === 1
                  ? "credential reference was"
                  : "credential references were"} left out.
              </p>
            {/if}
          {:else if store.result}
            {#if store.result.restored}
              {@const r = store.result.restored}
              <p class="file">
                Restored {r.folders + r.connections} folders and connections and {r.credentials}
                credentials from the backup of {dateOf(r.createdAt)}.
              </p>
            {:else}
              <p class="file">Imported {store.fileName}</p>
            {/if}
            {#if store.result.report}
              <ImportReport report={store.result.report} />
            {/if}
          {/if}
        </div>
      {:else if step === "error" && store.error && store.error.code !== "password_too_short"}
        <p class="error" role="alert">{store.error.message}</p>
        {#if STALE.has(store.error.code)}
          <div class="actions start">
            <button type="button" class="btn" onclick={startOver}>Start over</button>
          </div>
        {/if}
      {/if}

      <h3 class="section">Import</h3>
      <div class="actions start">
        <button
          type="button"
          class="btn"
          aria-disabled={locked}
          onclick={() => void pick("mremoteng")}>mRemoteNG file…</button
        >
        <button
          type="button"
          class="btn"
          aria-disabled={locked}
          onclick={() => void pick("connections")}>fernjo connections file…</button
        >
      </div>

      <h3 class="section">Export</h3>
      <div class="actions start">
        <button
          type="button"
          class="btn"
          aria-disabled={locked}
          onclick={() => void exportConnections()}>Connections (JSON)</button
        >
        <button
          type="button"
          class="btn"
          aria-expanded={backupOpen}
          aria-disabled={locked}
          onclick={() => (backupOpen = !backupOpen)}>Full backup…</button
        >
      </div>
      <p class="note">
        Connections (JSON) leaves credentials out. A full backup includes the vault and is encrypted
        with its own password.
      </p>
      {#if backupOpen}
        <form class="backup" onsubmit={saveBackup}>
          <PasswordField
            id="backup-password"
            label="Backup password"
            bind:value={backupPassword}
            error={step === "error" && store.error?.code === "password_too_short"
              ? store.error.message
              : null}
          />
          <PasswordField
            id="backup-repeat"
            label="Repeat backup password"
            bind:value={backupRepeat}
          />
          <p class="note">At least {MIN_PASSWORD} characters. There is no way to recover it.</p>
          <div class="actions">
            <button type="submit" class="btn primary" aria-disabled={locked || !backupOk}
              >Save backup…</button
            >
          </div>
        </form>
      {/if}

      <h3 class="section">Restore</h3>
      <div class="actions start">
        <button type="button" class="btn" aria-disabled={locked} onclick={() => void pick("backup")}
          >Backup file…</button
        >
      </div>
      <p class="note">Replaces everything in fernjo with the contents of a backup.</p>

      <div class="actions footer">
        <button type="button" class="btn" onclick={close}>Close</button>
      </div>
    {/if}
  </div>
</div>

{#if confirming}
  <ConfirmDialog
    title="Restore this backup?"
    detail={restoreText}
    confirmLabel="Restore"
    onConfirm={() => void apply(false)}
    onCancel={() => (confirming = false)}
  />
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 30;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--color-overlay);
  }

  .dialog {
    display: flex;
    flex-direction: column;
    width: 480px;
    max-width: calc(100vw - var(--space-6));
    /* short windows: the dialog scrolls inside, so Close stays reachable */
    max-height: calc(100vh - var(--space-6));
    overflow-y: auto;
    padding: var(--space-5);
    background: var(--color-surface-raised);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-raised);
    outline: none;
  }

  .title {
    margin: 0 0 var(--space-3);
    font-size: var(--text-md);
    font-weight: var(--weight-strong);
  }

  .section {
    margin: var(--space-5) 0 0;
    font-size: var(--text-sm);
    font-weight: var(--weight-strong);
  }

  .file {
    margin: 0 0 var(--space-2);
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    overflow-wrap: anywhere;
  }

  .note {
    margin: var(--space-2) 0 0;
    color: var(--color-text-muted);
    font-size: var(--text-sm);
    line-height: 1.3;
  }

  .error {
    margin: 0 0 var(--space-2);
    color: var(--color-danger);
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
  }

  .result {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
  }

  .result .file,
  .result .note {
    margin: 0;
  }

  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    margin-top: var(--space-3);
  }

  .backup {
    padding: var(--space-3);
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: var(--space-3);
    margin-top: var(--space-3);
  }

  .actions.start {
    justify-content: flex-start;
  }

  .footer {
    margin-top: var(--space-5);
    padding-top: var(--space-4);
    border-top: 1px solid var(--color-border);
  }

  .primary {
    background: var(--color-accent);
    border-color: var(--color-accent);
    color: var(--color-accent-text);
  }

  .primary:hover {
    background: var(--color-accent);
  }

  .danger {
    color: var(--color-danger);
  }

  .btn[aria-disabled="true"] {
    opacity: 0.6;
  }
</style>
