<script lang="ts" module>
  export interface MenuItem {
    label: string;
    action: () => void;
  }
</script>

<script lang="ts">
  let { x, y, items, onClose }: { x: number; y: number; items: MenuItem[]; onClose: () => void } =
    $props();

  let el = $state<HTMLElement>();
  let adjusted = $state<{ x: number; y: number } | null>(null);

  const left = $derived(adjusted?.x ?? x);
  const top = $derived(adjusted?.y ?? y);

  const menuItems = () => [...(el?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? [])];

  $effect(() => {
    const menu = el;
    if (!menu) return;
    // keep the menu inside the window
    const rect = menu.getBoundingClientRect();
    adjusted = {
      x: Math.max(0, Math.min(x, window.innerWidth - rect.width)),
      y: Math.max(0, Math.min(y, window.innerHeight - rect.height)),
    };
    const opener = document.activeElement as HTMLElement | null;
    menuItems()[0]?.focus();
    return () => {
      // give focus back, unless an action already moved it somewhere useful
      const active = document.activeElement;
      if (!active || active === document.body || menu.contains(active)) opener?.focus?.();
    };
  });

  function onpointerdown(e: Event) {
    if (!el?.contains(e.target as Node)) onClose();
  }

  function onkeydown(e: KeyboardEvent) {
    const all = menuItems();
    const index = all.indexOf(document.activeElement as HTMLElement);
    switch (e.key) {
      case "Escape":
      case "Tab":
        e.preventDefault();
        e.stopPropagation();
        onClose();
        break;
      case "ArrowDown":
        e.preventDefault();
        all[(index + 1) % all.length]?.focus();
        break;
      case "ArrowUp":
        e.preventDefault();
        all[(index - 1 + all.length) % all.length]?.focus();
        break;
      case "Home":
        e.preventDefault();
        all[0]?.focus();
        break;
      case "End":
        e.preventDefault();
        all[all.length - 1]?.focus();
        break;
      case "Enter":
      case " ":
        // handled here so the browser's own button click does not fire a second time
        e.preventDefault();
        (document.activeElement as HTMLElement | null)?.click();
        break;
    }
  }
</script>

<svelte:window {onpointerdown} onblur={onClose} onresize={onClose} />

<div
  bind:this={el}
  class="menu"
  role="menu"
  tabindex="-1"
  style:left="{left}px"
  style:top="{top}px"
  {onkeydown}
>
  {#each items as item (item.label)}
    <button
      type="button"
      class="item"
      role="menuitem"
      tabindex="-1"
      onclick={() => {
        item.action();
        onClose();
      }}
    >
      {item.label}
    </button>
  {/each}
</div>

<style>
  .menu {
    position: fixed;
    z-index: 20;
    min-width: 168px;
    padding: var(--space-2);
    background: var(--color-surface-raised);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-raised);
    outline: none;
  }

  .item {
    display: block;
    width: 100%;
    height: 28px;
    padding: 0 var(--space-3);
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text);
    font: inherit;
    text-align: left;
    cursor: default;
  }

  .item:hover {
    background: var(--color-hover);
  }
</style>
