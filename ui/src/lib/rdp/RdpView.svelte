<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { isMac, routeKey } from "../shortcuts";
  import type { SessionStore, Tab } from "../sessions.svelte";
  import { displaySize, RdpController } from "./controller";

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
      clipboard: () => {},
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
    if (!visible || tab.state !== "connected") c.releaseAll();
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

  function onkeydown(e: KeyboardEvent) {
    if (ours(e)) return;
    if (controller?.keyDown(e)) e.preventDefault();
  }

  function onkeyup(e: KeyboardEvent) {
    if (ours(e)) return;
    if (controller?.keyUp(e)) e.preventDefault();
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
      onblur={() => controller?.blur()}
      oncontextmenu={(e) => e.preventDefault()}
    ></canvas>
    {#if tab.state === "connecting"}
      <div class="status" role="status">Connecting to {tab.title}…</div>
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
