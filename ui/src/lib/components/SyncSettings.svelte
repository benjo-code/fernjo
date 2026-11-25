<script lang="ts">
  import { onDestroy, onMount, tick } from "svelte";
  import type { SyncStore } from "../sync.svelte";
  import { syncErrorText } from "../sync-errors";
  import { RESEED_CONFIRM, syncLine } from "../sync-line";
  import type { SyncConfirmation } from "../types";
  import ConfirmDialog from "./ConfirmDialog.svelte";
  import PasswordField from "./PasswordField.svelte";

  let {
    sync,
    onexport,
  }: {
    sync: SyncStore;
    /** "Export a backup first…" in the join confirmation: opens the import, export & backup dialog. */
    onexport?: () => void;
  } = $props();

  type Mode = "choose" | "setup" | "join";

  let mode = $state<Mode>("choose");
  let url = $state("");
  let secret = $state("");
  let caPem = $state("");
  let deviceName = $state("");
  let joinCode = $state("");
  let editing = $state(false);
  let newSecret = $state("");
  let copied = $state(false);
  let asking = $state<"reseed" | "leave" | null>(null);

  let urlInput = $state<HTMLInputElement>();
  let codeInput = $state<HTMLTextAreaElement>();
  let replaceCancel = $state<HTMLButtonElement>();
  let secretField = $state<PasswordField>();

  const status = $derived(sync.status);
  const paired = $derived(status !== null && status.state !== "off");
  const line = $derived(syncLine(status));
  const failure = $derived(sync.error ? syncErrorText(sync.error) : null);

  // an old error of the last visit must not greet the user
  onMount(() => {
    sync.error = null;
  });

  // the code and the typed values go away with the dialog
  onDestroy(() => {
    sync.hidePairingCode();
    sync.dropConfirmation();
    sync.error = null;
  });

  async function choose(next: Mode) {
    sync.error = null;
    mode = next;
    await tick();
    if (next === "setup") urlInput?.focus();
    else if (next === "join") codeInput?.focus();
  }

  function cancelJoin() {
    sync.dropConfirmation();
    joinCode = "";
  }

  function back() {
    sync.error = null;
    cancelJoin();
    mode = "choose";
  }

  async function submitSetup(e: SubmitEvent) {
    e.preventDefault();
    if (sync.busy) return;
    // the typed secret is emptied before the call, the call keeps its own copy
    const typed = secret;
    secret = "";
    const ok = await sync.setup(url, typed, caPem.trim() === "" ? null : caPem, deviceName);
    if (ok) {
      mode = "choose";
      url = "";
      caPem = "";
      deviceName = "";
    }
  }

  async function submitJoin(e: SubmitEvent) {
    e.preventDefault();
    if (sync.busy) return;
    const outcome = await sync.join(joinCode, deviceName, false);
    if (outcome === "confirm") {
      await tick();
      replaceCancel?.focus();
    } else if (outcome === "joined") finishJoin();
  }

  async function confirmJoin() {
    if (sync.busy) return;
    const outcome = await sync.join(joinCode, deviceName, true);
    if (outcome === "joined") finishJoin();
  }

  function finishJoin() {
    mode = "choose";
    joinCode = "";
    deviceName = "";
  }

  async function submitSecret(e: SubmitEvent) {
    e.preventDefault();
    if (sync.busy) return;
    const typed = newSecret;
    newSecret = "";
    if (await sync.editSecret(typed)) editing = false;
  }

  async function startEditing() {
    sync.error = null;
    editing = true;
    await tick();
    secretField?.focus();
  }

  async function copyCode() {
    if (sync.pairingCode === null) return;
    try {
      await navigator.clipboard.writeText(sync.pairingCode);
      copied = true;
    } catch {
      copied = false;
    }
  }

  async function togglePairingCode() {
    copied = false;
    if (sync.pairingCode !== null) sync.hidePairingCode();
    else await sync.showPairingCode();
  }

  async function confirmAsk() {
    const which = asking;
    asking = null;
    if (which === "reseed") await sync.reseed();
    else if (which === "leave") {
      if (await sync.leave()) {
        editing = false;
        newSecret = "";
        copied = false;
      }
    }
  }

  const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

  /** What a join replaces, from the counts Rust reported; the vault part follows what Rust knows. */
  function replaceText(c: SyncConfirmation): string {
    let text = `Joining replaces the connections on this device (${plural(c.connections, "connection", "connections")}, ${plural(c.folders, "folder", "folders")})`;
    if (c.hasVault) {
      text += ` and, if the sync group has a vault, this device's vault${
        c.vaultEntries === null ? "" : ` (${plural(c.vaultEntries, "entry", "entries")})`
      }`;
    }
    return `${text}. Open sessions will be closed. After joining you unlock the vault with the master password of your sync group (this device's own vault password no longer applies).`;
  }
</script>

<h3 class="section" data-section="sync">Sync</h3>

{#if status === null}
  <p class="note">Loading…</p>
{:else if !paired}
  {#if mode === "choose"}
    {#if status.error}
      <!-- a leave that could not remove the keychain item: Rust's message names the item -->
      <p class="warning-text" role="status" data-leave-note>{status.error.message}</p>
    {/if}
    <p class="note">
      Keep your connections and vault in step across your devices through a hub you run yourself.
    </p>
    <div class="actions start">
      <button type="button" class="btn" onclick={() => void choose("setup")}>
        Set up this device
      </button>
      <button type="button" class="btn" onclick={() => void choose("join")}>
        Join with a pairing code
      </button>
    </div>
  {:else if mode === "setup"}
    <form class="form" onsubmit={(e) => void submitSetup(e)}>
      <label class="field">
        <span>Hub URL</span>
        <input
          id="sync-url"
          bind:this={urlInput}
          type="text"
          placeholder="https://hub.example.lan"
          autocomplete="off"
          spellcheck="false"
          bind:value={url}
        />
      </label>
      <PasswordField id="sync-secret" label="Hub secret" bind:value={secret} />
      <label class="field">
        <span>CA certificate (optional, PEM)</span>
        <textarea
          id="sync-ca"
          rows="3"
          autocomplete="off"
          spellcheck="false"
          placeholder="-----BEGIN CERTIFICATE-----"
          bind:value={caPem}></textarea>
        <small class="hint">
          Only needed when the hub's certificate comes from your own CA, for example Caddy's local
          CA.
        </small>
      </label>
      <label class="field">
        <span>Device name</span>
        <input
          id="sync-device"
          type="text"
          placeholder="Gaming PC"
          autocomplete="off"
          bind:value={deviceName}
        />
      </label>
      {@render failureNote()}
      <div class="actions">
        <button type="button" class="btn" onclick={back}>Cancel</button>
        <button type="submit" class="btn primary" aria-disabled={sync.busy}>Set up</button>
      </div>
    </form>
  {:else if sync.confirmation}
    <div class="confirm" role="group" aria-label="Confirm joining">
      <p class="warning-text">{replaceText(sync.confirmation)}</p>
      {@render failureNote()}
      <div class="actions">
        <button type="button" class="btn" onclick={() => onexport?.()}>
          Export a backup first…
        </button>
        <button bind:this={replaceCancel} type="button" class="btn" onclick={cancelJoin}>
          Cancel
        </button>
        <button
          type="button"
          class="btn danger"
          aria-disabled={sync.busy}
          onclick={() => void confirmJoin()}
        >
          Replace and join
        </button>
      </div>
    </div>
  {:else}
    <form class="form" onsubmit={(e) => void submitJoin(e)}>
      <label class="field">
        <span>Pairing code</span>
        <textarea
          id="sync-code"
          bind:this={codeInput}
          rows="3"
          autocomplete="off"
          spellcheck="false"
          bind:value={joinCode}></textarea>
      </label>
      <label class="field">
        <span>Device name</span>
        <input
          id="sync-device"
          type="text"
          placeholder="Gaming PC"
          autocomplete="off"
          bind:value={deviceName}
        />
      </label>
      {@render failureNote()}
      <div class="actions">
        <button type="button" class="btn" onclick={back}>Cancel</button>
        <button type="submit" class="btn primary" aria-disabled={sync.busy}>Join</button>
      </div>
    </form>
  {/if}
{:else}
  <dl class="facts">
    <dt>Status</dt>
    <dd role="status">{line?.text ?? "Synced"}</dd>
    <dt>Device</dt>
    <dd>{status.deviceName ?? ""}</dd>
    <dt>Hub</dt>
    <dd class="mono">{status.hubUrl ?? ""}</dd>
  </dl>
  {#if status.state === "error" && status.error}
    <p class="error">{syncErrorText(status.error).text}</p>
  {:else if status.state === "rolled_back"}
    <p class="warning-text">
      The hub shows an older state than this device has seen. If you know the hub was restored on
      purpose, re-seed it from this device.
    </p>
  {/if}

  {@render failureNote()}

  <div class="actions start">
    <button type="button" class="btn" aria-disabled={sync.busy} onclick={() => void sync.syncNow()}>
      Sync now
    </button>
    <button
      type="button"
      class="btn"
      aria-expanded={sync.pairingCode !== null}
      onclick={() => void togglePairingCode()}
    >
      {sync.pairingCode !== null ? "Hide pairing code" : "Show pairing code"}
    </button>
    <button type="button" class="btn" onclick={() => void startEditing()}>Edit hub secret</button>
    {#if status.state === "rolled_back"}
      <button type="button" class="btn" onclick={() => (asking = "reseed")}>Re-seed hub…</button>
    {/if}
    <button type="button" class="btn danger" onclick={() => (asking = "leave")}>Leave sync…</button>
  </div>

  {#if sync.pairingCode !== null}
    <div class="reveal">
      <p class="note strong">
        Keep this code private: it contains the keys of your sync group. You need it to add your
        other devices.
      </p>
      <textarea
        class="code"
        readonly
        rows="4"
        aria-label="Pairing code"
        autocomplete="off"
        spellcheck="false"
        value={sync.pairingCode}></textarea>
      <div class="actions">
        <button type="button" class="btn" onclick={() => void copyCode()}>Copy</button>
        {#if copied}<span class="note" role="status">Copied</span>{/if}
      </div>
    </div>
  {/if}

  {#if editing}
    <form class="form" onsubmit={(e) => void submitSecret(e)}>
      <PasswordField
        bind:this={secretField}
        id="sync-new-secret"
        label="New hub secret"
        bind:value={newSecret}
      />
      <div class="actions">
        <button
          type="button"
          class="btn"
          onclick={() => {
            editing = false;
            newSecret = "";
          }}
        >
          Cancel
        </button>
        <button type="submit" class="btn primary" aria-disabled={sync.busy}>Save secret</button>
      </div>
    </form>
  {/if}
{/if}

{#snippet failureNote()}
  {#if failure}
    <p class="error" role="alert">
      {failure.text}{#if failure.code}
        <small class="code-text"> ({failure.code})</small>{/if}
    </p>
  {/if}
{/snippet}

{#if asking === "reseed"}
  <ConfirmDialog
    title={RESEED_CONFIRM.title}
    detail={RESEED_CONFIRM.detail}
    confirmLabel={RESEED_CONFIRM.confirmLabel}
    onConfirm={() => void confirmAsk()}
    onCancel={() => (asking = null)}
  />
{:else if asking === "leave"}
  <ConfirmDialog
    title="Leave sync?"
    detail="This device stops syncing and forgets the hub and the sync keys. Your local connections and vault stay as they are; other devices are not affected."
    confirmLabel="Leave sync"
    onConfirm={() => void confirmAsk()}
    onCancel={() => (asking = null)}
  />
{/if}

<style>
  .section {
    margin: var(--space-5) 0 0;
    font-size: var(--text-sm);
    font-weight: var(--weight-strong);
  }

  .form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    margin-top: var(--space-3);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }

  .field input,
  .field textarea,
  .reveal textarea {
    padding: 0 var(--space-3);
    background: var(--color-surface);
    border: 1px solid var(--color-border-input);
    border-radius: var(--radius-sm);
    color: var(--color-text);
    font: inherit;
  }

  .field input {
    height: 28px;
  }

  .field textarea,
  .reveal textarea {
    padding: var(--space-2) var(--space-3);
    resize: vertical;
    font-family: var(--font-mono);
  }

  .reveal textarea {
    width: 100%;
    box-sizing: border-box;
    user-select: text;
  }

  .field input:focus,
  .field textarea:focus,
  .reveal textarea:focus {
    outline: 1px solid var(--color-focus);
    outline-offset: -1px;
  }

  .hint {
    color: var(--color-text-muted);
  }

  .note {
    margin: var(--space-2) 0 0;
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }

  .strong {
    color: var(--color-text);
    font-weight: var(--weight-medium);
  }

  .warning-text {
    margin: var(--space-3) 0 0;
    color: var(--color-text);
    font-size: var(--text-sm);
    line-height: 1.3;
    overflow-wrap: anywhere;
  }

  .error {
    margin: var(--space-3) 0 0;
    color: var(--color-danger);
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
  }

  .code-text {
    color: var(--color-text-muted);
    font-family: var(--font-mono);
  }

  .facts {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: var(--space-2) var(--space-4);
    margin: var(--space-3) 0 0;
    font-size: var(--text-sm);
  }

  .facts dt {
    color: var(--color-text-muted);
  }

  .facts dd {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .mono {
    font-family: var(--font-mono);
  }

  .reveal {
    margin-top: var(--space-3);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-3);
    margin-top: var(--space-3);
  }

  .actions.start {
    justify-content: flex-start;
  }

  .danger {
    color: var(--color-danger);
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
