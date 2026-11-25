<script lang="ts">
  import { tick } from "svelte";
  import type { SessionStore, Tab } from "../sessions.svelte";
  import type { Protocol } from "../types";
  import ConfirmDialog from "./ConfirmDialog.svelte";
  import Icon from "./Icon.svelte";
  import Lamp from "./Lamp.svelte";

  let {
    sessions,
    protocolOf = () => "ssh",
    onreconnectall = () => {},
  }: {
    sessions: SessionStore;
    /** The protocol of the connection behind a tab (decides the icon). */
    protocolOf?: (nodeId: string) => Protocol;
    /** Connects the restored tabs; the app supplies the vault question. */
    onreconnectall?: () => void | Promise<void>;
  } = $props();

  const DRAG_THRESHOLD = 4;

  let strip = $state<HTMLElement>();

  // --- reorder by pointer drag ---
  let press: { id: string; x: number; y: number } | null = null;
  let justDragged = false;
  let dragView = $state<{ dragId: string | null; beforeId: string | null }>({
    dragId: null,
    beforeId: null,
  });

  function caption(tab: Tab): string | null {
    // A dropped tab has no caption: its reason is the bar above the terminal (SessionArea) and the
    // tooltip, so the title keeps the room.
    // a pending prompt is state "connecting" in Rust; the caption tells that the tab waits for the user
    if (tab.prompt) {
      const kind = tab.prompt.prompt.kind;
      return kind === "hostKey" || kind === "hostKeySynced" || kind === "hostKeyChanged"
        ? "Confirm host key"
        : "Needs input";
    }
    switch (tab.state) {
      case "connecting":
        return "Connecting…";
      case "needsVault":
        return "Unlock vault";
      default:
        return null;
    }
  }

  function markFor(tab: Tab): "connecting" | "live" | "dropped" | null {
    if (tab.state === "connecting") return "connecting";
    if (tab.state === "connected") return "live";
    // a restored tab never connected in this run: no lamp, it is not a lost connection
    if (tab.state === "disconnected") return tab.restored ? null : "dropped";
    return null;
  }

  function onpointerdown(e: PointerEvent, tab: Tab) {
    if (e.button !== 0) {
      cancelDrag();
      return;
    }
    press = { id: tab.id, x: e.clientX, y: e.clientY };
  }

  /** The tab the dragged one would go before: the first other tab whose middle is right of the pointer. */
  function beforeIdAt(x: number, dragId: string): string | null {
    const others = [...(strip?.querySelectorAll<HTMLElement>("[data-tab-id]") ?? [])].filter(
      (el) => el.dataset.tabId !== dragId,
    );
    for (const el of others) {
      const rect = el.getBoundingClientRect();
      if (rect.left + rect.width / 2 > x) return el.dataset.tabId!;
    }
    return null;
  }

  function onwindowpointermove(e: PointerEvent) {
    if (!press) return;
    if ((e.buttons & 1) === 0) {
      // the release happened where we could not see it
      cancelDrag();
      return;
    }
    if (dragView.dragId === null) {
      if (Math.hypot(e.clientX - press.x, e.clientY - press.y) < DRAG_THRESHOLD) return;
      dragView = { dragId: press.id, beforeId: null };
    }
    dragView = { dragId: press.id, beforeId: beforeIdAt(e.clientX, press.id) };
  }

  function onwindowpointerup() {
    const { dragId, beforeId } = dragView;
    press = null;
    dragView = { dragId: null, beforeId: null };
    if (dragId === null) return;
    sessions.reorder(dragId, beforeId);
    // the click that follows a drag must not activate a tab
    justDragged = true;
    setTimeout(() => (justDragged = false), 0);
  }

  function cancelDrag() {
    press = null;
    dragView = { dragId: null, beforeId: null };
  }

  function onwindowkeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && press) cancelDrag();
  }

  function onclick(tab: Tab) {
    if (justDragged) {
      justDragged = false;
      return;
    }
    sessions.activate(tab.id);
  }

  async function onkeydown(e: KeyboardEvent) {
    if (e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    if (e.key === "ArrowRight") sessions.next();
    else sessions.previous();
    await tick();
    strip?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')?.focus();
  }

  /** A vertical wheel scrolls the (scrollbar-less) strip sideways. */
  function onwheel(e: WheelEvent) {
    if (!strip || e.deltaX !== 0 || e.deltaY === 0) return;
    strip.scrollLeft += e.deltaY;
    e.preventDefault();
  }

  // the tab in front must not hide beyond the strip's edge
  $effect(() => {
    const id = sessions.activeId;
    if (id === null || !strip) return;
    [...strip.querySelectorAll<HTMLElement>("[data-tab-id]")]
      .find((el) => el.dataset.tabId === id)
      ?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
  });

  const last = $derived(sessions.tabs[sessions.tabs.length - 1]?.id ?? null);
</script>

<svelte:window
  onpointermove={onwindowpointermove}
  onpointerup={onwindowpointerup}
  onpointercancel={cancelDrag}
  onkeydown={onwindowkeydown}
/>

<div class="bar">
  <div
    bind:this={strip}
    class="strip"
    class:dragging={dragView.dragId !== null}
    role="tablist"
    aria-label="Sessions"
    tabindex="-1"
    {onkeydown}
    {onwheel}
  >
    {#each sessions.tabs as tab (tab.id)}
      {@const active = tab.id === sessions.activeId}
      {@const mark = markFor(tab)}
      {@const text = caption(tab)}
      <div
        class="tab"
        class:active
        class:dragged={dragView.dragId === tab.id}
        class:drop-before={dragView.dragId !== null && dragView.beforeId === tab.id}
        class:drop-after={dragView.dragId !== null && dragView.beforeId === null && tab.id === last}
        role="presentation"
        data-tab-id={tab.id}
      >
        <button
          type="button"
          role="tab"
          class="main"
          aria-selected={active}
          tabindex={active ? 0 : -1}
          title={tab.detail ? `${tab.title}: ${tab.detail}` : tab.title}
          onclick={() => onclick(tab)}
          onpointerdown={(e) => onpointerdown(e, tab)}
        >
          {#if mark}
            <Lamp state={mark} size="tab" />
          {:else if tab.state === "disconnected"}
            <span class="mark"></span>
          {:else}
            <span class="mark"><Icon name="lock" size={12} /></span>
          {/if}
          <span class="protocol">
            <Icon name={tab.nodeId === null ? tab.protocol : protocolOf(tab.nodeId)} />
          </span>
          <span class="title">{tab.title}</span>
          {#if tab.protocol === "rdp" && tab.clipboard}
            {@const shared = `Clipboard shared with ${tab.host ?? tab.title}`}
            <span class="clip" title={shared} aria-label={shared} role="img">
              <Icon name="clipboard" size={12} />
            </span>
          {/if}
          {#if text}<span class="caption">{text}</span>{/if}
        </button>
        {#if tab.state === "disconnected"}
          <button
            type="button"
            class="reconnect"
            aria-label={`Reconnect ${tab.title}`}
            title="Reconnect"
            onclick={() => sessions.reconnect(tab.id)}
          >
            <Icon name="reconnect" size={12} />
          </button>
        {/if}
        <button
          type="button"
          class="close"
          aria-label={`Close ${tab.title}`}
          title="Close"
          onclick={() => sessions.requestClose(tab.id)}
        >
          <Icon name="close" size={12} />
        </button>
      </div>
    {/each}
  </div>
  {#if sessions.restoredDisconnected > 0}
    <div class="all">
      <button
        type="button"
        class="reconnect-all"
        aria-label="Reconnect all restored tabs"
        title="Connect every restored tab"
        onclick={() => void onreconnectall()}
      >
        <Icon name="reconnect" size={12} />
        Reconnect all
      </button>
    </div>
  {/if}
</div>

{#if sessions.closeRequest}
  <ConfirmDialog
    title={`Close “${sessions.closeRequest.title}”?`}
    detail="The session ends."
    confirmLabel="Close"
    onConfirm={() => sessions.confirmClose()}
    onCancel={() => sessions.cancelClose()}
  />
{/if}

<style>
  .bar {
    display: flex;
    flex: none;
    min-width: 0;
  }

  .all {
    display: flex;
    flex: none;
    align-items: center;
    padding: 0 var(--space-3);
    background: var(--color-sidebar);
    border-bottom: 1px solid var(--color-border);
  }

  .reconnect-all {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 22px;
    padding: 0 var(--space-3) 0 var(--space-2);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text);
    font: inherit;
    font-size: var(--text-xs);
    cursor: default;
  }

  .reconnect-all:hover {
    background: var(--color-hover);
  }

  .reconnect-all:focus-visible {
    outline: 1px solid var(--color-focus);
    outline-offset: 2px;
  }

  .strip {
    display: flex;
    flex: 1 1 0;
    min-width: 0;
    align-items: stretch;
    height: 36px;
    overflow-x: auto;
    overflow-y: hidden;
    background: var(--color-sidebar);
    border-bottom: 1px solid var(--color-border);
    outline: none;
    user-select: none;
  }

  /* the strip scrolls by wheel and by activating a tab; a 10 px bar would eat a third of its height */
  .strip::-webkit-scrollbar {
    height: 0;
  }

  .strip.dragging {
    cursor: grabbing;
  }

  .tab {
    position: relative;
    display: flex;
    flex: 0 1 260px;
    align-items: center;
    gap: var(--space-2);
    min-width: 150px;
    max-width: 260px;
    padding-right: var(--space-2);
    border-right: 1px solid var(--color-border);
    color: var(--color-tab-text);
  }

  .tab:hover {
    background: var(--color-hover);
  }

  .tab.active {
    margin-bottom: -1px;
    background: var(--term-bg);
    color: var(--color-text);
  }

  .tab.active::after {
    content: "";
    position: absolute;
    right: 0;
    bottom: 1px;
    left: 0;
    height: 2px;
    background: var(--color-text);
  }

  .tab.dragged {
    opacity: 0.55;
  }

  .tab.drop-before::before,
  .tab.drop-after::before {
    content: "";
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    background: var(--color-selection-bar);
    pointer-events: none;
    z-index: 1;
  }

  .tab.drop-before::before {
    left: -1px;
  }

  .tab.drop-after::before {
    right: -1px;
  }

  .main {
    display: flex;
    flex: 1 1 0;
    align-items: center;
    gap: var(--space-3);
    min-width: 0;
    height: 100%;
    padding: 0 0 0 var(--space-3);
    overflow: hidden;
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: default;
  }

  .mark {
    display: flex;
    flex: none;
    width: 10px;
    justify-content: center;
    color: var(--color-text-muted);
  }

  .protocol {
    display: flex;
    flex: none;
    color: var(--color-ssh);
  }

  .clip {
    display: flex;
    flex: none;
    color: var(--color-text-muted);
  }

  .title {
    flex: 0 1 auto;
    min-width: 24px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .caption {
    flex: 1 1 0;
    min-width: 0;
    overflow: hidden;
    color: var(--color-text-muted);
    font-size: var(--text-xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .reconnect {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    padding: 0;
    border: 0;
    border-radius: var(--radius-sm);
    background: var(--color-accent);
    color: var(--color-accent-text);
    cursor: default;
  }

  .close {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    padding: 0;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: inherit;
    cursor: default;
  }

  .close:hover {
    background: var(--color-selection);
    color: var(--color-text);
  }
</style>
