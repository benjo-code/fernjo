<script lang="ts">
  import { onMount, untrack } from "svelte";
  import type { SessionStore, Tab } from "../sessions.svelte";
  import { RdpController } from "./controller";

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

  // One controller per tab for the tab's whole life; hidden tabs stay mounted.
  onMount(() => {
    const el = canvas!;
    const room = box!;
    const id = untrack(() => tab.id);
    const c = new RdpController({
      canvas: el,
      send: (events) => sessions.rdpInput(id, events),
    });
    controller = c;
    // not passive: a wheel turn over the desktop must not scroll the page behind it
    const onwheel = (e: WheelEvent) => {
      e.preventDefault();
      c.wheel(e);
    };
    el.addEventListener("wheel", onwheel, { passive: false });
    sessions.attachRemote(id, {
      paint: (rect, pixels) => c.paint(rect, pixels),
      setDesktop: (width, height) => c.setDesktop(width, height),
      clipboard: () => {},
      size: () => {
        const r = room.getBoundingClientRect();
        return { width: Math.round(r.width), height: Math.round(r.height) };
      },
    });
    return () => {
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

  function onkeydown(e: KeyboardEvent) {
    if (controller?.keyDown(e)) e.preventDefault();
  }

  function onkeyup(e: KeyboardEvent) {
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
      tabindex="0"
      aria-label={`Remote desktop of ${tab.title}`}
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

  /* a smaller desktop is shown as it is; a larger one is scaled down, aspect ratio kept */
  .desktop {
    max-width: 100%;
    max-height: 100%;
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
</style>
