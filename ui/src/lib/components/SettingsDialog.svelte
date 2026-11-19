<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { toApiError } from "../api";
  import type { TerminalSettingsStore } from "../terminal-settings.svelte";
  import type { ThemeStore } from "../theme.svelte";
  import type { Api, DataFolderInfo, Theme } from "../types";
  import { unsavedNote } from "../unsaved";
  import Icon from "./Icon.svelte";

  let {
    api,
    terminal,
    theme,
    onclose,
    onchange,
  }: {
    api: Api;
    /** Holds the terminal settings the open terminals follow. */
    terminal: TerminalSettingsStore;
    /** Holds the theme choice; picking one applies it at once. */
    theme: ThemeStore;
    onclose: () => void;
    /** A folder was chosen (the info carries the pending restart). */
    onchange: (info: DataFolderInfo) => void;
  } = $props();

  const uid = $props.id();

  let dialog = $state<HTMLElement>();
  let info = $state<DataFolderInfo | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let restarting = $state(false);
  // existing files are only reported for a choice made in this dialog
  let chosen = $state(false);

  const sourceText = $derived(
    info?.source === "env"
      ? "Set by FERNJO_DATA_DIR"
      : info?.source === "settings"
        ? "Chosen in settings"
        : "Default location",
  );

  const existingText = $derived(
    `This folder already contains ${info?.existingFiles.join(", ")}. fernjo will use them as they are; nothing was copied.`,
  );

  let fontFamily = $state("");
  let fontSize = $state<number | null>(null);
  let scrollback = $state<number | null>(null);
  let terminalError = $state<string | null>(null);
  let saving = $state(false);
  let saved = $state(false);

  let terminalLoadError = $state(false);
  // fields the user already edited are never overwritten by a load that arrives later
  const touched = new SvelteSet<string>();

  function fillTerminal() {
    const t = terminal.info;
    if (!t) return;
    if (!touched.has("fontFamily")) fontFamily = t.fontFamily ?? "";
    if (!touched.has("fontSize")) fontSize = t.fontSize;
    if (!touched.has("scrollback")) scrollback = t.scrollback;
  }

  function edited(e: Event) {
    const name = (e.target as HTMLElement).dataset.field;
    if (name) touched.add(name);
    saved = false;
  }

  let themeError = $state<string | null>(null);
  let themeGroup = $state<HTMLElement>();

  async function pickTheme(next: Theme) {
    themeError = null;
    const err = await theme.set(next);
    if (err) {
      themeError = "Could not save the theme";
      // the browser already ticked the clicked radio; put the tick back on the stored choice
      for (const r of themeGroup?.querySelectorAll<HTMLInputElement>("input") ?? []) {
        r.checked = r.value === (theme.info?.theme ?? "system");
      }
    }
  }

  function submitOnEnter(e: KeyboardEvent) {
    // Enter that confirms an IME candidate must not submit
    if (e.key === "Enter" && !e.isComposing) {
      e.preventDefault();
      void saveTerminal();
    }
  }

  const TERMINAL_ERRORS: Record<string, string> = {
    invalid_font_size: "Font size must be between 8 and 32.",
    invalid_scrollback: "Scrollback must be between 1000 and 100000 lines.",
  };

  /**
   * What Rust can even parse: a whole number within the wire type (`u16` / `u32`). Anything else is
   * sent as 0 so that Rust answers with its coded error instead of a deserialisation message.
   * Whether a number is a sensible size is Rust's call.
   */
  const toWire = (n: number | null, max: number) =>
    n !== null && Number.isInteger(n) && n >= 0 && n <= max ? n : 0;

  async function saveTerminal() {
    // without loaded settings the fields hold placeholders, not the stored values
    if (saving || !terminal.info) return;
    saving = true;
    saved = false;
    terminalError = null;
    const err = await terminal.save({
      fontFamily: fontFamily.trim() === "" ? null : fontFamily.trim(),
      fontSize: toWire(fontSize, 65535),
      scrollback: toWire(scrollback, 4294967295),
    });
    saving = false;
    if (err) terminalError = TERMINAL_ERRORS[err.code] ?? "Could not save the terminal settings";
    else saved = true;
  }

  onMount(() => {
    fillTerminal();
    void terminal.load().then((ok) => {
      terminalLoadError = !ok && !terminal.info;
      fillTerminal();
    });
    void theme.load();
    void (async () => {
      try {
        info = await api.getDataFolder();
      } catch (e) {
        error = toApiError(e).message;
      }
    })();
  });

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

  // While busy the buttons use aria-disabled (handlers ignore the click) instead of disabled:
  // WebView2 drops focus to <body> when the focused button becomes disabled, which would kill
  // the Escape handler and the Tab trap on the dialog.
  async function choose() {
    if (busy) return;
    busy = true;
    error = null;
    try {
      const next = await api.chooseDataFolder();
      if (next) {
        info = next;
        chosen = true;
        onchange(next);
      }
    } catch (e) {
      error = toApiError(e).message;
    } finally {
      busy = false;
    }
  }

  async function restart() {
    if (restarting) return;
    restarting = true;
    error = null;
    try {
      await api.restartApp();
    } catch (e) {
      restarting = false;
      error = toApiError(e).message;
    }
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onclose();
    } else if (e.key === "Tab" && dialog) {
      // keep focus inside the dialog
      e.preventDefault();
      const items = [
        ...dialog.querySelectorAll<HTMLElement>("button:not(:disabled), input:not(:disabled)"),
      ];
      if (items.length === 0) return;
      const index = items.indexOf(document.activeElement as HTMLElement);
      const next = e.shiftKey ? (index <= 0 ? items.length - 1 : index - 1) : index + 1;
      items[next % items.length].focus();
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
    <h2 class="title" id="{uid}-title">Settings</h2>

    <h3 class="section">Appearance</h3>
    <div bind:this={themeGroup} class="choices" role="radiogroup" aria-label="Theme">
      {#each [["system", "System"], ["light", "Light"], ["dark", "Dark"]] as [value, label] (value)}
        <label class="choice">
          <input
            type="radio"
            name="{uid}-theme"
            {value}
            checked={(theme.info?.theme ?? "system") === value}
            onchange={() => void pickTheme(value as Theme)}
          />
          <span>{label}</span>
        </label>
      {/each}
    </div>
    {#if theme.info?.unsavedReason}
      <p class="note">{unsavedNote(theme.info.unsavedReason)}</p>
    {/if}
    {#if themeError}
      <p class="error" role="alert">{themeError}</p>
    {/if}

    <h3 class="section">Terminal</h3>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="fields" oninput={edited} onkeydown={submitOnEnter}>
      <label class="field wide">
        <span>Font family</span>
        <input
          type="text"
          aria-label="Font family"
          data-field="fontFamily"
          placeholder="Default monospace"
          autocomplete="off"
          spellcheck="false"
          bind:value={fontFamily}
        />
      </label>
      <label class="field">
        <span>Font size</span>
        <input
          type="number"
          aria-label="Font size"
          data-field="fontSize"
          min="8"
          max="32"
          step="1"
          bind:value={fontSize}
        />
      </label>
      <label class="field">
        <span>Scrollback</span>
        <input
          type="number"
          aria-label="Scrollback"
          data-field="scrollback"
          min="1000"
          max="100000"
          step="1000"
          bind:value={scrollback}
        />
      </label>
    </div>
    {#if terminal.info?.unsavedReason}
      <p class="note">{unsavedNote(terminal.info.unsavedReason)}</p>
    {/if}
    {#if terminalLoadError}
      <p class="error" role="alert">Could not load the terminal settings</p>
    {/if}
    {#if terminalError}
      <p class="error" role="alert">{terminalError}</p>
    {:else if saved}
      <p class="note" role="status">Saved</p>
    {/if}
    <div class="actions">
      <button
        type="button"
        class="btn"
        aria-disabled={saving || !terminal.info}
        onclick={() => void saveTerminal()}>Save</button
      >
    </div>

    <h3 class="section">Data folder</h3>

    {#if info}
      <p class="path">{info.path}</p>
      <p class="note">{sourceText}</p>
      {#if !info.changeable && info.source !== "env"}
        <p class="note">Cannot be changed here.</p>
      {/if}
      {#if info.restartRequired}
        <p class="note strong">Restart to use the new folder</p>
      {/if}
      {#if chosen && info.existingFiles.length > 0}
        <p class="warning">
          <Icon name="alert" size={14} />
          <span>{existingText}</span>
        </p>
      {/if}
    {:else if !error}
      <p class="note">Loading…</p>
    {/if}

    {#if error}
      <p class="error" role="alert">{error}</p>
    {/if}

    <div class="actions">
      <button
        type="button"
        class="btn"
        class:primary={!info?.restartRequired}
        disabled={!info || !info.changeable}
        aria-disabled={busy}
        onclick={() => void choose()}>Choose folder…</button
      >
      {#if info?.restartRequired}
        <button
          type="button"
          class="btn primary"
          aria-disabled={restarting}
          onclick={() => void restart()}>Restart now</button
        >
      {/if}
    </div>

    <div class="actions footer">
      <button type="button" class="btn" onclick={onclose}>Close</button>
    </div>
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
    /* short windows: the dialog scrolls inside, so Close and Save stay reachable */
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

  .section {
    margin: var(--space-5) 0 0;
    font-size: var(--text-sm);
    font-weight: var(--weight-strong);
  }

  .title + .section {
    margin-top: var(--space-4);
  }

  .fields {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    margin-top: var(--space-3);
  }

  .field {
    display: flex;
    flex: 1 1 120px;
    flex-direction: column;
    gap: var(--space-2);
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }

  .field.wide {
    flex-basis: 100%;
  }

  .field input {
    height: 28px;
    padding: 0 var(--space-3);
    background: var(--color-surface);
    border: 1px solid var(--color-border-input);
    border-radius: var(--radius-sm);
    color: var(--color-text);
    font: inherit;
  }

  .field input:focus {
    outline: 1px solid var(--color-focus);
    outline-offset: -1px;
  }

  .choices {
    display: flex;
    gap: var(--space-5);
    margin-top: var(--space-3);
  }

  .choice {
    display: flex;
    align-items: center;
    gap: var(--space-2);
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

  .path {
    margin: var(--space-3) 0 0;
    padding: var(--space-3);
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
    user-select: text;
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

  .warning {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    margin: var(--space-3) 0 0;
    color: var(--color-warning);
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

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-3);
    margin-top: var(--space-3);
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

  .btn:disabled,
  .btn[aria-disabled="true"] {
    opacity: 0.6;
  }
</style>
