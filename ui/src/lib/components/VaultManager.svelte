<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import type { TreeStore } from "../store.svelte";
  import type {
    CredentialDraft,
    CredentialInfo,
    CredentialUpdate,
    RevealedSecret,
    UsageItem,
  } from "../types";
  import type { ApiError } from "../types";
  import { KEYCHAIN_HINT, type VaultStore } from "../vault.svelte";
  import ConfirmDialog from "./ConfirmDialog.svelte";
  import Icon from "./Icon.svelte";
  import PasswordField from "./PasswordField.svelte";

  let {
    vault,
    tree,
    onclose,
    revealMs = 10_000,
    newToken = null,
  }: {
    vault: VaultStore;
    tree: TreeStore;
    onclose: () => void;
    /** How long a revealed secret stays visible. */
    revealMs?: number;
    /**
     * A request to start on the new-credential form. A token this manager has not seen yet
     * (also the one it is mounted with) selects "new"; the same token again, or null, does not.
     */
    newToken?: number | null;
  } = $props();

  type Kind = CredentialInfo["kind"];

  // `null`: nothing selected; "new": the create form; otherwise the id of the entry being edited
  let selected = $state<string | null>(null);
  let name = $state("");
  let kind = $state<Kind>("password");
  let username = $state("");
  let domain = $state("");
  let notes = $state("");
  // typed secrets: they live only in these fields and are cleared after a successful save
  let password = $state("");
  let privateKey = $state("");
  let passphrase = $state("");
  // edit only: remove the stored passphrase (ignored while a new passphrase is typed)
  let clearPassphrase = $state(false);

  let busy = $state(false);
  let nameError = $state<string | null>(null);
  let formError = $state<string | null>(null);

  // the revealed value is component state only and emptied on every hide path
  let revealed = $state<RevealedSecret | null>(null);
  let revealError = $state<string | null>(null);
  let revealTimer: ReturnType<typeof setTimeout> | undefined;

  let changing = $state(false);
  let oldPassword = $state("");
  let newPassword = $state("");
  let repeatPassword = $state("");
  let oldError = $state<string | null>(null);
  let newError = $state<string | null>(null);
  let repeatError = $state<string | null>(null);
  let changeError = $state<string | null>(null);
  let changed = $state(false);

  let autoError = $state<string | null>(null);

  let deleting = $state<{ id: string; name: string; usage: UsageItem[] } | null>(null);

  const editing = $derived(selected !== null && selected !== "new");
  const autoUnlock = $derived(vault.status?.autoUnlock ?? false);
  const unlocked = $derived(vault.status?.state === "unlocked");

  // bumped by every hide path: a reveal answer from an older generation is dropped
  let revealGeneration = 0;

  function hideReveal() {
    revealGeneration++;
    clearTimeout(revealTimer);
    revealTimer = undefined;
    revealed = null;
  }

  // locking hides the secret at once
  $effect(() => {
    if (!unlocked) hideReveal();
  });

  onDestroy(hideReveal);

  function close() {
    hideReveal();
    clearSecrets();
    onclose();
  }

  /** The vault locked behind our back: re-read the status so the view closes. */
  function handleError(err: ApiError) {
    if (err.code === "vault_locked") void vault.init();
  }

  function clearSecrets() {
    password = "";
    privateKey = "";
    passphrase = "";
    clearPassphrase = false;
  }

  function clearErrors() {
    nameError = null;
    formError = null;
    revealError = null;
  }

  function select(info: CredentialInfo) {
    hideReveal();
    clearSecrets();
    clearErrors();
    selected = info.id;
    name = info.name;
    kind = info.kind;
    username = info.username ?? "";
    domain = info.domain ?? "";
    notes = info.notes;
  }

  function startNew() {
    hideReveal();
    clearSecrets();
    clearErrors();
    selected = "new";
    name = "";
    kind = "password";
    username = "";
    domain = "";
    notes = "";
  }

  let seenToken: number | null = null;
  $effect(() => {
    const token = newToken;
    if (token === null || token === seenToken) return;
    seenToken = token;
    untrack(startNew);
  });

  function showError(err: ApiError) {
    handleError(err);
    if (err.code === "empty_name") nameError = err.message;
    else formError = err.message;
  }

  async function save(e: SubmitEvent) {
    e.preventDefault();
    if (busy || selected === null) return;
    busy = true;
    clearErrors();
    if (selected === "new") {
      // secret fields that do not belong to the kind are left out, as are empty optionals
      const base = {
        name,
        notes,
        ...(username ? { username } : {}),
        ...(domain ? { domain } : {}),
      };
      const draft: CredentialDraft =
        kind === "password"
          ? { ...base, kind, password }
          : { ...base, kind, privateKey, ...(passphrase ? { passphrase } : {}) };
      const { value, error } = await vault.createCredential(draft);
      busy = false;
      if (error) return showError(error);
      clearSecrets();
      select(value);
    } else {
      // only a secret field the user typed goes along
      const update: CredentialUpdate = { name, username, domain, notes };
      if (kind === "password") {
        if (password) update.password = password;
      } else {
        if (privateKey) update.privateKey = privateKey;
        if (passphrase) update.passphrase = passphrase;
        else if (clearPassphrase) update.clearPassphrase = true;
      }
      const { value, error } = await vault.updateCredential(selected, update);
      busy = false;
      if (error) return showError(error);
      clearSecrets();
      select(value);
    }
  }

  async function reveal() {
    if (!editing || selected === null) return;
    hideReveal();
    revealError = null;
    const id = selected;
    const generation = revealGeneration;
    const { value, error } = await vault.revealCredential(id);
    // closed, locked, saved, selection changed or a newer reveal started while waiting
    if (generation !== revealGeneration) return;
    if (error) {
      handleError(error);
      revealError = error.message;
      return;
    }
    // the selection may have changed or the vault locked while the call was running
    if (selected !== id || !unlocked) return;
    revealed = value;
    clearTimeout(revealTimer);
    revealTimer = setTimeout(hideReveal, revealMs);
  }

  async function askDelete() {
    if (!editing || selected === null) return;
    const id = selected;
    const { value, error } = await vault.credentialUsage(id);
    if (error) return showError(error);
    deleting = { id, name, usage: value };
  }

  async function confirmDelete() {
    const target = deleting;
    if (!target) return;
    deleting = null;
    const { value, error } = await vault.deleteCredential(target.id);
    if (error) {
      // the credential list is in the vault, but the nodes that used it are in connections.json
      if (error.code === "changed_on_disk") tree.markChangedOnDisk();
      return showError(error);
    }
    await tree.applySnapshot(value.snapshot);
    if (selected === target.id) {
      hideReveal();
      clearSecrets();
      selected = null;
    }
  }

  function deleteDetail(d: { usage: UsageItem[] }): string {
    if (d.usage.length === 0) return "This credential is not used by any connection or folder.";
    return `Used by: ${d.usage.map((u) => u.name).join(", ")}. They will be set back to inherit.`;
  }

  function openChange() {
    changing = true;
    changed = false;
  }

  function closeChange() {
    changing = false;
    oldPassword = "";
    newPassword = "";
    repeatPassword = "";
    oldError = null;
    newError = null;
    repeatError = null;
    changeError = null;
  }

  async function submitChange(e: SubmitEvent) {
    e.preventDefault();
    if (busy) return;
    oldError = newError = repeatError = changeError = null;
    // confirmation field only; strength and the current password are judged by Rust
    if (newPassword !== repeatPassword) {
      repeatError = "The passwords do not match.";
      return;
    }
    busy = true;
    const err = await vault.changePassword(oldPassword, newPassword);
    busy = false;
    if (!err) {
      closeChange();
      changed = true;
    } else if (err.code === "wrong_password") oldError = err.message;
    else if (err.code === "weak_password") newError = err.message;
    else {
      handleError(err);
      changeError = err.message;
    }
  }

  async function toggleAuto(e: Event & { currentTarget: HTMLInputElement }) {
    const box = e.currentTarget;
    autoError = null;
    const err = await vault.setAutoUnlock(box.checked);
    if (err) autoError = err.message;
    // show what Rust says, whatever the click did
    box.checked = vault.status?.autoUnlock ?? false;
  }

  const kindLabel = (k: Kind) => (k === "password" ? "Password" : "Private key");
  const revealedAny = $derived(
    revealed !== null &&
      (revealed.password !== undefined ||
        revealed.privateKey !== undefined ||
        revealed.passphrase !== undefined),
  );
</script>

<section class="manager" aria-labelledby="vault-manager-title">
  <header class="head">
    <div class="title">
      <Icon name="unlock" size={16} />
      <h2 id="vault-manager-title">Vault</h2>
    </div>
    <div class="head-actions">
      <label class="check">
        <input id="vault-auto-unlock" type="checkbox" checked={autoUnlock} onchange={toggleAuto} />
        <span>Unlock automatically on this device</span>
      </label>
      <button type="button" class="btn small" onclick={() => void vault.lock()}>Lock</button>
      <button type="button" class="btn small" onclick={openChange}>Change password</button>
      <button type="button" class="btn small" onclick={close}>Close</button>
    </div>
    <p class="hint line">
      {KEYCHAIN_HINT}{autoUnlock ? " Lock then lasts until the next start." : ""}
    </p>
    {#if autoError}
      <p class="error line" role="alert">
        <Icon name="alert" size={14} /><span>{autoError}</span>
      </p>
    {/if}
    {#if changed}
      <p class="hint line" role="status">Master password changed.</p>
    {/if}
  </header>

  {#if changing}
    <form class="change" onsubmit={submitChange}>
      <PasswordField
        id="vault-change-old"
        label="Current master password"
        bind:value={oldPassword}
        error={oldError}
        onedit={() => (oldError = null)}
      />
      <PasswordField
        id="vault-change-new"
        label="New master password"
        bind:value={newPassword}
        error={newError}
        onedit={() => (newError = repeatError = null)}
      />
      <PasswordField
        id="vault-change-repeat"
        label="Repeat new master password"
        bind:value={repeatPassword}
        error={repeatError}
        onedit={() => (repeatError = null)}
      />
      <p class="hint">
        Older copies of the vault file, such as backups and sync history, still open with your
        previous password.
      </p>
      {#if changeError}
        <p class="error" role="alert"><Icon name="alert" size={14} /><span>{changeError}</span></p>
      {/if}
      <div class="actions">
        <button type="submit" class="btn small" disabled={busy}>Change</button>
        <button type="button" class="btn small" onclick={closeChange}>Cancel</button>
      </div>
    </form>
  {/if}

  <div class="body">
    <div class="list">
      <div class="list-head">
        <div class="list-title">
          <h3>Credentials</h3>
          <span class="muted">
            {vault.credentials.length === 1
              ? "1 credential"
              : `${vault.credentials.length} credentials`}
          </span>
        </div>
        <button type="button" class="btn small" onclick={startNew}>+ New</button>
      </div>
      {#if vault.credentials.length === 0}
        <p class="muted pad">
          No credentials yet. Create one with + New and pick it on a connection's Credential
          property.
        </p>
      {:else}
        <ul>
          {#each vault.credentials as info (info.id)}
            <li>
              <button
                type="button"
                class="row"
                class:selected={selected === info.id}
                data-credential={info.id}
                aria-current={selected === info.id ? "true" : undefined}
                onclick={() => select(info)}
              >
                <span class="row-name">{info.name}</span>
                <span class="row-meta">
                  {kindLabel(info.kind)}{info.username ? ` · ${info.username}` : ""}
                </span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>

    <div class="detail">
      {#if selected === null}
        <p class="muted pad">Select a credential or create a new one.</p>
      {:else}
        <form onsubmit={save}>
          <div class="field">
            <label for="cred-name">Name</label>
            <input
              id="cred-name"
              type="text"
              autocomplete="off"
              spellcheck="false"
              class:invalid={nameError !== null}
              aria-invalid={nameError !== null ? "true" : undefined}
              aria-describedby={nameError !== null ? "cred-name-error" : undefined}
              bind:value={name}
              oninput={() => (nameError = null)}
            />
            {#if nameError !== null}
              <p class="error" id="cred-name-error" role="alert">
                <Icon name="alert" size={14} /><span>{nameError}</span>
              </p>
            {/if}
          </div>
          <div class="field">
            <label for="cred-kind">Type</label>
            <select id="cred-kind" bind:value={kind} disabled={editing}>
              <option value="password">Password</option>
              <option value="privateKey">Private key</option>
            </select>
          </div>
          <div class="field">
            <label for="cred-username">Username</label>
            <input
              id="cred-username"
              type="text"
              autocomplete="off"
              spellcheck="false"
              bind:value={username}
            />
          </div>
          <div class="field">
            <label for="cred-domain">Domain</label>
            <input
              id="cred-domain"
              type="text"
              autocomplete="off"
              spellcheck="false"
              bind:value={domain}
            />
          </div>

          {#if kind === "password"}
            <PasswordField
              id="cred-password"
              label="Password"
              placeholder={editing ? "unchanged" : ""}
              bind:value={password}
            />
          {:else}
            <div class="field">
              <label for="cred-private-key">Private key</label>
              <textarea
                id="cred-private-key"
                rows="5"
                autocomplete="off"
                spellcheck="false"
                placeholder={editing ? "unchanged" : ""}
                bind:value={privateKey}></textarea>
            </div>
            <PasswordField
              id="cred-passphrase"
              label="Passphrase"
              placeholder={editing ? "unchanged" : "optional"}
              bind:value={passphrase}
              onedit={() => {
                if (passphrase) clearPassphrase = false;
              }}
            />
            {#if editing}
              <label class="check" for="cred-clear-passphrase">
                <input
                  id="cred-clear-passphrase"
                  type="checkbox"
                  bind:checked={clearPassphrase}
                  disabled={passphrase !== ""}
                />
                <span>Remove stored passphrase</span>
              </label>
            {/if}
          {/if}

          <div class="field">
            <label for="cred-notes">Notes</label>
            <textarea id="cred-notes" rows="3" bind:value={notes}></textarea>
          </div>

          {#if revealedAny && revealed}
            <div class="revealed" role="group" aria-label="Revealed secret">
              {#if revealed.password !== undefined}
                <label for="cred-revealed-password">Stored password</label>
                <input id="cred-revealed-password" type="text" readonly value={revealed.password} />
              {/if}
              {#if revealed.privateKey !== undefined}
                <span class="label">Stored private key</span>
                <pre>{revealed.privateKey}</pre>
              {/if}
              {#if revealed.passphrase !== undefined}
                <label for="cred-revealed-passphrase">Stored passphrase</label>
                <input
                  id="cred-revealed-passphrase"
                  type="text"
                  readonly
                  value={revealed.passphrase}
                />
              {/if}
              <p class="hint">Hidden again after {Math.round(revealMs / 1000)} seconds.</p>
            </div>
          {/if}
          {#if revealError}
            <p class="error" role="alert">
              <Icon name="alert" size={14} /><span>{revealError}</span>
            </p>
          {/if}
          {#if formError}
            <p class="error" role="alert">
              <Icon name="alert" size={14} /><span>{formError}</span>
            </p>
          {/if}

          <div class="actions">
            <button type="submit" class="btn primary" disabled={busy}>Save</button>
            {#if editing}
              <button type="button" class="btn" onclick={() => void reveal()}>Reveal</button>
              <button type="button" class="btn danger" onclick={() => void askDelete()}>
                Delete
              </button>
            {/if}
          </div>
        </form>
      {/if}
    </div>
  </div>
</section>

{#if deleting}
  <ConfirmDialog
    title={`Delete ${deleting.name}?`}
    detail={deleteDetail(deleting)}
    confirmLabel="Delete"
    onConfirm={() => void confirmDelete()}
    onCancel={() => (deleting = null)}
  />
{/if}

<style>
  .manager {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    width: 100%;
    height: 100%;
    min-width: 0;
    padding: var(--space-5);
    overflow: auto;
    box-sizing: border-box;
  }

  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .title {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  h2 {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: var(--weight-strong);
  }

  .head-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
  }

  .line {
    flex-basis: 100%;
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

  .hint,
  .muted {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--text-sm);
    line-height: 1.3;
  }

  .pad {
    padding: var(--space-4);
  }

  .error {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    margin: 0;
    color: var(--color-danger);
    font-size: var(--text-sm);
    line-height: 1.3;
  }

  .error span {
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .change {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    max-width: 360px;
    padding: var(--space-4);
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
  }

  .body {
    display: grid;
    grid-template-columns: 260px minmax(0, 1fr);
    gap: var(--space-5);
    min-height: 0;
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }

  .list-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .list-title {
    display: flex;
    flex-direction: column;
  }

  h3 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: var(--weight-strong);
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    background: var(--color-surface);
  }

  .row {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    width: 100%;
    padding: var(--space-3) var(--space-4);
    border: 0;
    background: transparent;
    color: var(--color-text);
    font: inherit;
    text-align: left;
    cursor: default;
  }

  .row:hover {
    background: var(--color-hover);
  }

  .row.selected {
    background: var(--color-selection);
    box-shadow: inset 2px 0 0 var(--color-selection-bar);
  }

  .row:focus-visible {
    outline: 1px solid var(--color-focus);
    outline-offset: -1px;
  }

  .row-name {
    font-weight: var(--weight-strong);
    overflow-wrap: anywhere;
  }

  .row-meta {
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }

  .detail {
    min-width: 0;
    max-width: 480px;
  }

  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .field,
  .revealed {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  label,
  .label {
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }

  .check span,
  .check {
    color: var(--color-text);
  }

  input[type="text"],
  select,
  textarea {
    width: 100%;
    min-width: 0;
    box-sizing: border-box;
    padding: 0 var(--space-3);
    border: 1px solid var(--color-border-input);
    border-radius: var(--radius-sm);
    background: var(--color-surface);
    color: var(--color-text);
    font: inherit;
  }

  input[type="text"],
  select {
    height: 28px;
  }

  textarea {
    padding: var(--space-3);
    resize: vertical;
  }

  input[type="text"]:focus,
  select:focus,
  textarea:focus {
    outline: 1px solid var(--color-focus);
    outline-offset: -1px;
  }

  input.invalid {
    border-color: var(--color-danger);
    box-shadow: 0 0 0 1px var(--color-danger);
  }

  select:disabled {
    opacity: 0.6;
  }

  .revealed {
    padding: var(--space-4);
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
  }

  pre {
    margin: 0;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .revealed input {
    font-family: var(--font-mono);
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

  .danger {
    color: var(--color-danger);
  }

  .btn:disabled {
    opacity: 0.6;
  }
</style>
