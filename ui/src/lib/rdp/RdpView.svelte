<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { isMac, routeKey } from "../shortcuts";
  import type { SessionStore, Tab } from "../sessions.svelte";
  import { ClipboardSync, isPasteChord } from "./clipboard";
  import { displaySize, RdpController } from "./controller";
  import { scancodeFor } from "./keymap";

  let {
    tab,
    sessions,
    visible,
    locked = false,
  }: {
    tab: Tab;
    sessions: SessionStore;
    visible: boolean;
    /** An inline prompt covers the desktop: it keeps painting but takes no input or focus. */
    locked?: boolean;
  } = $props();

  // the box that gives the canvas its room; the canvas itself shows the desktop
  let box = $state<HTMLElement>();
  let canvas = $state<HTMLCanvasElement>();
  let controller = $state.raw<RdpController | null>(null);
  let room = $state({ width: 0, height: 0 });
  let desktop = $state({ width: 0, height: 0 });
  const shown = $derived(displaySize(room, desktop));

  /** The hint on entering full screen goes away on its own. */
  const HINT_MS = 3000;
  let hint = $state(false);

  // The clipboard sync exists only while the connection's clipboard setting is on, so with it off
  // the local clipboard is never read or written. A plain variable on purpose: the bridge below
  // reads it when the remote side copies something.
  let sync: ClipboardSync | null = null;
  let clipboardBlocked = $state(false);
  /** Key events wait here while a paste chord brings the clipboard over first. */
  let gate: Promise<void> | null = null;
  /** Bumped when keys are released, so a key event that still waits at the gate is dropped. */
  let epoch = 0;

  // One controller per tab for the tab's whole life; hidden tabs stay mounted.
  onMount(() => {
    const el = canvas!;
    const area = box!;
    const id = untrack(() => tab.id);
    const measure = () => {
      const r = area.getBoundingClientRect();
      return { width: Math.round(r.width), height: Math.round(r.height) };
    };
    const c = new RdpController({
      canvas: el,
      send: (events) => sessions.rdpInput(id, events),
      resize: (width, height) => sessions.resize(id, width, height),
      fixed: () => tab.fixedSize,
    });
    controller = c;
    // not passive: a wheel turn over the desktop must not scroll the page behind it
    const onwheel = (e: WheelEvent) => {
      e.preventDefault();
      c.wheel(e);
    };
    el.addEventListener("wheel", onwheel, { passive: false });
    const observer = new ResizeObserver(() => {
      const now = measure();
      room = now;
      c.boxChanged(now.width, now.height);
    });
    observer.observe(area);
    room = measure();
    sessions.attachRemote(id, {
      paint: (rect, pixels) => c.paint(rect, pixels),
      setDesktop: (width, height) => {
        c.setDesktop(width, height);
        desktop = { width, height };
      },
      clipboard: (text) => void sync?.onRemote(text),
      size: measure,
    });
    return () => {
      observer.disconnect();
      el.removeEventListener("wheel", onwheel);
      sessions.detachRemote(id);
      c.dispose();
      controller = null;
    };
  });

  // Keys and buttons that are down are released when the tab is left or the session is not live,
  // so none of them stays stuck on the server.
  $effect(() => {
    const c = controller;
    if (!c) return;
    if (!visible || tab.state !== "connected") {
      dropPending();
      c.releaseAll();
    }
  });

  $effect(() => {
    if (!tab.clipboard) {
      sync = null;
      clipboardBlocked = false;
      return;
    }
    const id = untrack(() => tab.id);
    sync = new ClipboardSync({
      read: () => navigator.clipboard.readText(),
      write: (text) => navigator.clipboard.writeText(text),
      send: (text) => sessions.rdpClipboard(id, text),
      onBlocked: () => (clipboardBlocked = true),
    });
    return () => {
      sync = null;
    };
  });

  // The local clipboard is offered to the remote one when the desktop gets the focus, and once
  // more when the session becomes live while it already has it.
  $effect(() => {
    if (tab.state === "connected" && tab.clipboard && canvas && document.activeElement === canvas) {
      void sync?.syncLocal();
    }
  });

  // The keyboard moves to the desktop when this tab becomes the one in front and nothing else
  // (an inline prompt) wants it.
  const wantsFocus = $derived(
    visible && (tab.state === "connecting" || tab.state === "connected") && tab.prompt === null,
  );

  $effect(() => {
    if (canvas && wantsFocus) canvas.focus();
  });

  // Entering full screen shows how to leave it, for a few seconds. The box changes size at once,
  // and the desktop follows after the debounce.
  $effect(() => {
    if (!(visible && sessions.remoteFullscreen)) {
      hint = false;
      return;
    }
    hint = true;
    const timer = setTimeout(() => (hint = false), HINT_MS);
    return () => clearTimeout(timer);
  });

  /** The app's own chords (full screen, tab switch) stay with the app; the rest is the remote's. */
  function ours(e: KeyboardEvent): boolean {
    return routeKey(e, isMac(), "remote") !== null;
  }

  function dropPending() {
    epoch++;
    gate = null;
  }

  /** Runs `fn` after whatever waits at the gate, so key events keep their order. */
  function ordered(fn: () => void) {
    const mine = epoch;
    const run = () => {
      if (mine === epoch) fn();
    };
    if (!gate) {
      run();
      return;
    }
    const next = gate.then(run);
    gate = next;
    void next.finally(() => {
      if (gate === next) gate = null;
    });
  }

  function onkeydown(e: KeyboardEvent) {
    if (ours(e) || !scancodeFor(e.code)) return;
    e.preventDefault();
    // The local clipboard goes over before a paste chord reaches the remote machine: the key
    // waits (at most the read timeout) so the remote side pastes what was copied here.
    if (sync && tab.state === "connected" && isPasteChord(e, isMac())) {
      const wait = sync.syncLocal();
      gate = gate ? gate.then(() => wait) : wait;
    }
    const code = e.code;
    ordered(() => void controller?.keyDown({ code }));
  }

  function onkeyup(e: KeyboardEvent) {
    if (ours(e) || !scancodeFor(e.code)) return;
    e.preventDefault();
    const code = e.code;
    ordered(() => void controller?.keyUp({ code }));
  }

  function onpointerdown(e: PointerEvent) {
    e.preventDefault();
    canvas?.focus();
    // the drag keeps reporting while the pointer is outside the canvas
    canvas?.setPointerCapture?.(e.pointerId);
    controller?.pointerButton(e, true);
  }

  function onpointerup(e: PointerEvent) {
    controller?.pointerButton(e, false);
  }
</script>

<div class="view" class:hidden={!visible} inert={locked} role="presentation">
  <div class="box" bind:this={box}>
    <canvas
      bind:this={canvas}
      class="desktop"
      data-remote-desktop
      tabindex="0"
      aria-label={`Remote desktop of ${tab.title}`}
      style:width={shown ? `${shown.width}px` : null}
      style:height={shown ? `${shown.height}px` : null}
      {onkeydown}
      {onkeyup}
      {onpointerdown}
      {onpointerup}
      onpointermove={(e) => controller?.pointerMove(e)}
      onfocus={() => {
        if (tab.state === "connected") void sync?.syncLocal();
      }}
      onblur={() => {
        dropPending();
        controller?.blur();
      }}
      oncontextmenu={(e) => e.preventDefault()}
    ></canvas>
    {#if tab.state === "connecting"}
      <div class="status" role="status">Connecting to {tab.title}…</div>
    {/if}
    {#if clipboardBlocked}
      <div class="note" role="status">The clipboard is blocked by the web view</div>
    {/if}
    {#if hint}
      <div class="hint" role="status">Press Ctrl+Shift+F11 to leave full screen</div>
    {/if}
  </div>
</div>

<style>
  .view {
    position: absolute;
    inset: 0;
    background: var(--term-bg);
  }

  .view.hidden {
    display: none;
  }

  .box {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
  }

  /* the whole desktop is shown, scaled to the box with its aspect ratio kept (inline size) */
  .desktop {
    outline: none;
    cursor: default;
    touch-action: none;
  }

  .status {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--color-text-muted);
    font-size: var(--text-sm);
    pointer-events: none;
  }

  .note {
    position: absolute;
    bottom: var(--space-4);
    left: 50%;
    transform: translateX(-50%);
    padding: var(--space-2) var(--space-4);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    background: var(--color-sidebar);
    color: var(--color-text);
    font-size: var(--text-sm);
    pointer-events: none;
  }

  .hint {
    position: absolute;
    top: var(--space-4);
    left: 50%;
    transform: translateX(-50%);
    padding: var(--space-2) var(--space-4);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    background: var(--color-sidebar);
    color: var(--color-text);
    font-size: var(--text-sm);
    pointer-events: none;
  }
</style>
