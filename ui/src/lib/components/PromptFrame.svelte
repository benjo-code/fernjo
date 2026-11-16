<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";

  let {
    title,
    icon,
    role = "dialog",
    root = $bindable(),
    onkeydown,
    children,
  }: {
    title: string;
    icon: "lock" | "alert" | "ssh";
    role?: "dialog" | "alertdialog";
    /** The dialog element, for the owner's initial focus. */
    root?: HTMLElement;
    onkeydown?: (e: KeyboardEvent) => void;
    children: Snippet;
  } = $props();

  const uid = $props.id();

  const FOCUSABLE = "button:not([disabled]), input:not([disabled]), a[href]";

  // The handlers sit on the covering area, so Escape and the Tab rules keep working after a click
  // on its empty part. Tab never leaves the prompt: the ends of the column wrap around.
  function keydown(e: KeyboardEvent) {
    onkeydown?.(e);
    if (e.defaultPrevented || e.key !== "Tab" || !root) return;
    const items = [...root.querySelectorAll<HTMLElement>(FOCUSABLE)];
    if (items.length === 0) return;
    const at = items.indexOf(document.activeElement as HTMLElement);
    if (e.shiftKey && at <= 0) {
      e.preventDefault();
      items[items.length - 1]!.focus();
    } else if (!e.shiftKey && at === items.length - 1) {
      e.preventDefault();
      items[0]!.focus();
    }
  }

  // A click on the empty area must not drop the focus on <body>.
  function mousedown(e: MouseEvent) {
    if (root && !root.contains(e.target as Node)) {
      e.preventDefault();
      root.focus();
    }
  }
</script>

<!-- Covers the terminal area of its own tab only; the terminal stays mounted underneath. -->
<div class="wrap" role="presentation" onkeydown={keydown} onmousedown={mousedown}>
  <div bind:this={root} class="col" {role} aria-labelledby="{uid}-title" tabindex="-1">
    <div class="head">
      <Icon name={icon} size={16} />
      <h2 id="{uid}-title">{title}</h2>
    </div>
    {@render children()}
  </div>
</div>

<style>
  .wrap {
    position: absolute;
    inset: 0;
    z-index: 5;
    overflow: auto;
    background: var(--color-bg);
  }

  .col {
    display: flex;
    flex-direction: column;
    gap: 14px;
    box-sizing: border-box;
    width: 420px;
    max-width: 100%;
    margin: 88px auto 0;
    padding: 0 var(--space-5);
    outline: none;
  }

  .head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  h2 {
    margin: 0;
    min-width: 0;
    font-size: var(--text-lg);
    font-weight: var(--weight-strong);
    overflow-wrap: anywhere;
  }
</style>
