<script lang="ts">
  import type { TreeStore } from "../store.svelte";
  import type { Notice } from "../types";
  import Icon from "./Icon.svelte";

  let {
    store,
    restartPending = false,
    onRestart,
  }: {
    store: TreeStore;
    /** A new data folder was chosen; the bar stays until the app restarts. */
    restartPending?: boolean;
    onRestart?: () => void;
  } = $props();

  const baseName = (path: string) => path.split(/[\\/]/).pop() ?? path;

  function noticeText(notice: Notice): string {
    switch (notice.kind) {
      case "restoredFromBackup":
        return notice.badFile
          ? `The connections file could not be read, so the backup was restored. The damaged file was kept as ${baseName(notice.badFile)}.`
          : "The connections file was missing, so the backup was restored.";
      case "syncConflicts":
        return `Sync conflicts found next to your data: ${notice.files.join(", ")}. Resolve them in your sync tool; fernjo does not merge them.`;
      case "settingsUnreadable":
        return `The settings file could not be used, so the default data folder is in effect. ${notice.message}`;
      case "deviceFolderInsideDataDir":
        return `The device-local folder (${notice.path}) lies inside your data folder, so it is synced with your data. Choose a data folder that does not contain it.`;
    }
  }
</script>

{#if store.loadError}
  <div class="bar error" role="alert">
    <Icon name="alert" size={14} />
    <span class="text">{store.loadError}</span>
  </div>
{/if}

{#if restartPending}
  <div class="bar restart" role="status">
    <span class="text">Restart fernjo to use the new data folder.</span>
    <button type="button" class="dismiss" onclick={() => onRestart?.()}>Restart now</button>
  </div>
{/if}

{#if store.changedOnDisk}
  <div class="bar error" role="alert">
    <Icon name="alert" size={14} />
    <span class="text"
      >connections.json was changed by another program or device. Your last edit was not saved.</span
    >
    <button type="button" class="dismiss" onclick={() => void store.reload()}>Reload</button>
  </div>
{/if}

{#if store.error}
  <div class="bar error" role="alert">
    <Icon name="alert" size={14} />
    <span class="text">{store.error}</span>
    <button type="button" class="dismiss" onclick={() => store.dismissError()}>Dismiss</button>
  </div>
{/if}

{#each store.notices as notice, index (index)}
  <div class="bar" role="status">
    <span class="text">{noticeText(notice)}</span>
    <button type="button" class="dismiss" onclick={() => store.dismissNotice(index)}>Dismiss</button
    >
  </div>
{/each}

<style>
  .bar {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    margin: var(--space-3) var(--space-3) 0;
    padding: var(--space-3);
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-text-muted);
    font-size: var(--text-sm);
    line-height: 1.3;
  }

  .error {
    color: var(--color-danger);
  }

  .text {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .dismiss {
    flex: none;
    padding: 0 var(--space-2);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text);
    font: inherit;
    cursor: default;
  }

  .dismiss:hover {
    background: var(--color-hover);
  }
</style>
