<script lang="ts">
  import type { UpdateStore } from "../update.svelte";
  import { reasonText, stageText } from "../update-text";

  let {
    update,
    onclose,
  }: {
    update: UpdateStore;
    /** "Later", "Close", Escape, or a skip that was saved. Never called while installing. */
    onclose: () => void;
  } = $props();

  const uid = $props.id();

  let dialog = $state<HTMLElement>();

  const status = $derived(update.status);
  const phase = $derived(status?.state ?? "idle");
  const installing = $derived(phase === "installing");
  const pageOnly = $derived(status?.install?.kind === "releasePage");
  const failedInstall = $derived(
    phase === "failed" &&
      status?.version != null &&
      status.install?.kind === "selfUpdate" &&
      ["download_refused", "signature_invalid", "install_failed"].includes(status.error ?? ""),
  );

  const title = $derived(
    phase === "failed"
      ? "Update failed"
      : phase === "downloading"
        ? "Downloading update"
        : phase === "ready"
          ? "Update ready"
          : installing
            ? "Installing update"
            : phase === "available"
              ? "Update available"
              : "Updates",
  );

  const stage = $derived(status ? stageText(status) : "");
  // the bar shows while there is something to download or wait for
  const showBar = $derived(phase === "downloading" || installing);

  // without both numbers the bar is a separate element without value, so it is truly indeterminate
  const determinate = $derived(!installing && status?.received != null && status.total != null);

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

  // A state change removes the button that had the focus (the click on "Update and restart" ends
  // in a state without it). Take the focus back to the dialog, or Escape and the Tab trap, which
  // sit on the dialog, would stop working and Tab would leave the modal.
  $effect(() => {
    void phase;
    void update.busy;
    const root = dialog;
    if (root && !root.contains(document.activeElement)) root.focus();
  });

  async function skip() {
    if (await update.skip()) onclose();
  }

  async function retry() {
    if (failedInstall) await update.install();
    else await update.check();
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      // the installer is about to take over: there is nothing to go back to
      if (!installing) onclose();
    } else if (e.key === "Tab" && dialog) {
      // keep focus inside the dialog
      e.preventDefault();
      const items = [...dialog.querySelectorAll<HTMLElement>("button:not(:disabled)")];
      if (items.length === 0) return;
      const index = items.indexOf(document.activeElement as HTMLElement);
      const next = e.shiftKey ? (index <= 0 ? items.length - 1 : index - 1) : index + 1;
      items[next % items.length]!.focus();
    }
  }
</script>

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
    <h2 class="title" id="{uid}-title">{title}</h2>

    {#if status && status.version !== null}
      <p class="versions">
        <span class="version">fernjo {status.version}</span>
        <span class="muted">(you have {status.currentVersion})</span>
      </p>
    {/if}

    {#if phase === "available" && status?.skipped}
      <p class="note">You skipped this version earlier.</p>
    {/if}

    {#if phase === "available" && status?.install?.kind === "releasePage"}
      <p class="note">{reasonText(status.install.reason)}</p>
    {/if}

    {#if phase === "failed" && status?.error}
      <p class="error" role="alert">{reasonText(status.error)}</p>
    {/if}

    {#if showBar && status}
      {#if determinate}
        <progress
          class="bar"
          aria-label="Update progress"
          value={status.received ?? 0}
          max={status.total ?? 1}
        ></progress>
      {:else}
        <progress class="bar" aria-label="Update progress"></progress>
      {/if}
    {/if}
    {#if stage}
      <p class="stage" role="status">{stage}</p>
    {/if}

    {#if (phase === "available" || phase === "ready") && status?.notes}
      <h3 class="section">What's new</h3>
      <!-- release notes are text only: never rendered as markup -->
      <pre class="notes">{status.notes}</pre>
    {/if}

    {#if update.error}
      <p class="error" role="alert">{update.error.message}</p>
    {/if}

    {#if phase === "idle" || phase === "upToDate" || phase === "checking"}
      <p class="note">No update is waiting.</p>
    {/if}

    {#if !installing}
      <div class="actions">
        {#if phase === "available"}
          {#if pageOnly}
            <button
              type="button"
              class="btn primary"
              aria-disabled={update.busy}
              onclick={() => void update.openReleasePage()}
            >
              Open release page
            </button>
          {:else}
            <button
              type="button"
              class="btn primary"
              aria-disabled={update.busy}
              onclick={() => void update.install()}
            >
              Update and restart
            </button>
          {/if}
          <button type="button" class="btn" aria-disabled={update.busy} onclick={() => void skip()}>
            Skip this version
          </button>
          <button type="button" class="btn" onclick={onclose}>Later</button>
        {:else if phase === "ready"}
          <button
            type="button"
            class="btn primary"
            aria-disabled={update.busy}
            onclick={() => void update.install()}
          >
            Restart to update
          </button>
          <button type="button" class="btn" onclick={onclose}>Later</button>
        {:else if phase === "downloading"}
          <button type="button" class="btn" onclick={onclose}>Later</button>
        {:else if phase === "failed"}
          <button
            type="button"
            class="btn primary"
            aria-disabled={update.busy}
            onclick={() => void retry()}
          >
            Retry
          </button>
          <button
            type="button"
            class="btn"
            aria-disabled={update.busy}
            onclick={() => void update.openReleasePage()}
          >
            Open release page
          </button>
          <button type="button" class="btn" onclick={onclose}>Close</button>
        {:else}
          <button type="button" class="btn" onclick={onclose}>Close</button>
        {/if}
      </div>
    {/if}
  </div>
</div>

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
    width: 440px;
    max-width: calc(100vw - var(--space-6));
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
    margin: 0;
    font-size: var(--text-md);
    font-weight: var(--weight-strong);
  }

  .versions {
    margin: var(--space-3) 0 0;
    font-size: var(--text-sm);
  }

  .version {
    font-weight: var(--weight-medium);
  }

  .muted,
  .note {
    color: var(--color-text-muted);
  }

  .note {
    margin: var(--space-2) 0 0;
    font-size: var(--text-sm);
  }

  .section {
    margin: var(--space-4) 0 0;
    font-size: var(--text-sm);
    font-weight: var(--weight-strong);
  }

  .notes {
    max-height: 200px;
    margin: var(--space-2) 0 0;
    padding: var(--space-3);
    overflow: auto;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    font: inherit;
    font-size: var(--text-sm);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }

  .bar {
    display: block;
    width: 100%;
    height: 8px;
    margin: var(--space-4) 0 0;
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    background: var(--color-surface);
    accent-color: var(--color-accent);
    appearance: none;
    overflow: hidden;
  }

  .bar::-webkit-progress-bar {
    background: var(--color-surface);
  }

  .bar::-webkit-progress-value {
    background: var(--color-accent);
  }

  .bar::-moz-progress-bar {
    background: var(--color-accent);
  }

  .stage {
    margin: var(--space-2) 0 0;
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }

  .error {
    margin: var(--space-3) 0 0;
    color: var(--color-danger);
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: var(--space-3);
    margin-top: var(--space-5);
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
