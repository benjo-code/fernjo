<script lang="ts">
  import "@xterm/xterm/css/xterm.css";
  import { onMount, untrack } from "svelte";
  import type { SessionStore, Tab } from "../sessions.svelte";
  import type { TerminalSettings } from "../types";
  import ContextMenu, { type MenuItem } from "../components/ContextMenu.svelte";
  import { tauriApi } from "../api";
  import { TerminalController } from "./controller";
  import SearchBar from "./SearchBar.svelte";

  let {
    tab,
    sessions,
    settings,
    visible,
  }: { tab: Tab; sessions: SessionStore; settings: TerminalSettings; visible: boolean } = $props();

  let host = $state<HTMLElement>();
  let controller = $state.raw<TerminalController | null>(null);
  let searching = $state(false);
  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);

  // One controller per tab for the tab's whole life; hidden tabs stay mounted.
  onMount(() => {
    const el = host!;
    const id = untrack(() => tab.id);
    const c = new TerminalController(
      untrack(() => ({
        fontFamily: settings.fontFamily,
        fontSize: settings.fontSize,
        scrollback: settings.scrollback,
      })),
      {
        onData: (bytes) => sessions.input(id, bytes),
        onResize: (cols, rows) => sessions.resize(id, cols, rows),
        onAck: (bytes) => sessions.ack(id, bytes),
        openUrl: (url) => void tauriApi.openExternalUrl(url).catch(() => {}),
        onSearchRequest: () => (searching = true),
      },
    );
    controller = c;
    c.mount(el);
    c.setVisible(untrack(() => visible));
    // The first fit comes before the attach so the session starts at the real size.
    c.fit();
    sessions.attachTerminal(id, {
      write: (bytes) => c.write(bytes),
      size: () => ({ cols: c.cols, rows: c.rows }),
    });

    const observer = new ResizeObserver(() => c.fit());
    observer.observe(el);

    // The scheme (system preference or an explicit data-theme) changes the resolved tokens.
    const media = window.matchMedia?.("(prefers-color-scheme: dark)");
    const refresh = () => c.refreshTheme();
    media?.addEventListener?.("change", refresh);
    const mutations = new MutationObserver(refresh);
    mutations.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-theme", "class"],
    });

    return () => {
      observer.disconnect();
      mutations.disconnect();
      media?.removeEventListener?.("change", refresh);
      sessions.detachTerminal(id);
      c.dispose();
      controller = null;
    };
  });

  $effect(() => {
    const c = controller;
    if (!c) return;
    c.setVisible(visible);
    if (visible) c.focus();
  });

  $effect(() => {
    const c = controller;
    if (!c) return;
    c.setOptions({
      fontFamily: settings.fontFamily,
      fontSize: settings.fontSize,
      scrollback: settings.scrollback,
    });
  });

  function closeSearch() {
    searching = false;
    controller?.clearSearch();
    controller?.focus();
  }

  function oncontextmenu(e: MouseEvent) {
    const c = controller;
    if (!c) return;
    e.preventDefault();
    menu = {
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: "Copy", action: () => void c.copySelection().catch(() => {}) },
        { label: "Paste", action: () => void c.paste().catch(() => {}) },
        { label: "Search…", action: () => (searching = true) },
      ],
    };
  }
</script>

<div class="view" class:hidden={!visible} {oncontextmenu} role="presentation">
  <div class="term" bind:this={host}></div>
  {#if searching && controller}
    <SearchBar search={(t, d) => controller?.search(t, d) ?? false} onClose={closeSearch} />
  {/if}
</div>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => (menu = null)} />
{/if}

<style>
  .view {
    position: relative;
    width: 100%;
    height: 100%;
    background: var(--term-bg);
  }

  .view.hidden {
    display: none;
  }

  .term {
    position: absolute;
    inset: var(--space-3) 0 0 var(--space-4);
  }
</style>
