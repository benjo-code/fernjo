<script lang="ts">
  import type { UpdateStore } from "../update.svelte";
  import { automaticChecksNote, reasonText } from "../update-text";
  import { unsavedNote } from "../unsaved";

  let {
    update,
    onshow,
  }: {
    update: UpdateStore;
    /** "Show update": opens the update window (the caller closes Settings first). */
    onshow?: () => void;
  } = $props();

  const status = $derived(update.status);

  /** The last result, worded from the status the checks leave behind. */
  const result = $derived.by(() => {
    if (!status) return "";
    switch (status.state) {
      case "checking":
        return "Checking…";
      case "upToDate":
        return "You have the latest version.";
      case "available":
        if (!status.version) return "";
        return status.skipped
          ? `You skipped version ${status.version}.`
          : `Version ${status.version} is available.`;
      // while a command of the store is out, "Check now" waits: say what is going on
      case "downloading":
        return status.version ? `Downloading version ${status.version}…` : "";
      case "ready":
        return status.version ? `Version ${status.version} is ready to install.` : "";
      case "installing":
        return status.version ? `Installing version ${status.version}…` : "";
      case "failed":
        return status.error ? reasonText(status.error) : "The update failed.";
      default:
        return "";
    }
  });
  const checksNote = $derived(status ? automaticChecksNote(status.unsavedReason) : null);
  const failed = $derived(status?.state === "failed");
  const showable = $derived(
    status !== null &&
      ["available", "downloading", "ready", "installing", "failed"].includes(status.state),
  );

  async function toggle(e: Event) {
    const box = e.currentTarget as HTMLInputElement;
    const ok = await update.setEnabled(box.checked);
    // the browser already flipped the box; a failed save puts the stored value back
    if (!ok) box.checked = update.status?.checkEnabled ?? true;
  }
</script>

<h3 class="section" data-section="updates">Updates</h3>

{#if status === null}
  <p class="note">Loading…</p>
{:else}
  <p class="note installed">Installed version {status.currentVersion}</p>
  <label class="choice">
    <input
      type="checkbox"
      aria-label="Check for updates automatically"
      checked={status.checkEnabled}
      onchange={(e) => void toggle(e)}
    />
    <span>Check for updates automatically</span>
  </label>
  <p class="note">
    A check is one request to the release feed on GitHub, a minute after start and then once a day.
    Nothing is installed without your click.
  </p>
  {#if checksNote}
    <p class="note">{checksNote}</p>
  {/if}
  {#if status.unsavedReason}
    <p class="note">{unsavedNote(status.unsavedReason)}</p>
  {/if}
  <div class="actions">
    <button
      type="button"
      class="btn"
      aria-disabled={update.busy || status.state === "checking"}
      onclick={() => void update.check()}
    >
      Check now
    </button>
    {#if showable}
      <button type="button" class="btn" onclick={() => onshow?.()}>Show update</button>
    {/if}
  </div>
  {#if result}
    <p class:error={failed} class:note={!failed} role={failed ? "alert" : "status"}>{result}</p>
  {/if}
  {#if update.error}
    <p class="error" role="alert">{update.error.message}</p>
  {/if}
{/if}

<style>
  .section {
    margin: var(--space-5) 0 0;
    font-size: var(--text-sm);
    font-weight: var(--weight-strong);
  }

  .note,
  .error {
    margin: var(--space-2) 0 0;
    font-size: var(--text-sm);
  }

  .note {
    color: var(--color-text-muted);
  }

  .installed {
    margin-top: var(--space-3);
  }

  .error {
    color: var(--color-danger);
    overflow-wrap: anywhere;
  }

  .choice {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-3);
    font-size: var(--text-sm);
    cursor: pointer;
  }

  .choice input {
    margin: 0;
    accent-color: var(--color-accent);
  }

  .choice input:focus-visible {
    outline: 1px solid var(--color-focus);
    outline-offset: 2px;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-3);
    margin-top: var(--space-3);
  }

  .btn[aria-disabled="true"] {
    opacity: 0.6;
  }
</style>
