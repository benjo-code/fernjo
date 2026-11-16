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
</script>

<!-- Covers the terminal area of its own tab only; the terminal stays mounted underneath. -->
<div class="wrap">
  <div bind:this={root} class="col" {role} aria-labelledby="{uid}-title" tabindex="-1" {onkeydown}>
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
