<script lang="ts">
  let {
    state,
    size = "tree",
  }: { state: "connecting" | "live" | "dropped"; size?: "tree" | "tab" } = $props();

  const label = $derived(
    state === "live" ? "Connected" : state === "connecting" ? "Connecting" : "Disconnected",
  );
</script>

<!-- One shape per state, so the state reads without colour: pulsing ring, filled dot, struck ring. -->
<svg
  class="lamp"
  class:tab={size === "tab"}
  data-state={state}
  viewBox="0 0 10 10"
  role="img"
  aria-label={label}
>
  {#if state === "live"}
    <circle class="dot" cx="5" cy="5" r="4.25" />
  {:else}
    <circle class="ring" cx="5" cy="5" r="3.75" />
    {#if state === "dropped"}
      <line class="strike" x1="2" y1="8" x2="8" y2="2" />
    {/if}
  {/if}
</svg>

<style>
  .lamp {
    display: block;
    flex: none;
    width: var(--lamp-size);
    height: var(--lamp-size);
    overflow: visible;
  }

  .lamp.tab {
    width: 10px;
    height: 10px;
  }

  .ring,
  .strike {
    fill: none;
    stroke-width: 1.5;
    stroke-linecap: round;
  }

  .dot {
    fill: var(--color-lamp-live);
  }

  [data-state="connecting"] {
    color: var(--color-lamp-connecting);
  }

  [data-state="dropped"] {
    color: var(--color-lamp-dropped);
  }

  .ring,
  .strike {
    stroke: currentColor;
  }

  [data-state="connecting"] .ring {
    animation: pulse var(--duration-pulse) var(--easing-pulse) infinite;
  }

  @keyframes pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.3;
    }
  }
</style>
