<script lang="ts">
  type Name =
    | "folder-chevron"
    | "ssh"
    | "rdp"
    | "search"
    | "alert"
    | "reset"
    | "lock"
    | "unlock"
    | "eye"
    | "eye-off";

  let { name, size = 16 }: { name: Name; size?: number } = $props();
</script>

<svg
  class="icon"
  width={size}
  height={size}
  viewBox="0 0 16 16"
  fill="none"
  stroke="currentColor"
  stroke-width="1.5"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  focusable="false"
>
  {#if name === "folder-chevron"}
    <path d="M3.5 6 8 10.5 12.5 6" />
  {:else if name === "ssh"}
    <rect x="2" y="3" width="12" height="10" rx="1.5" />
    <path d="M5 7l2 1.5L5 10M8.5 10.5H11" />
  {:else if name === "rdp"}
    <rect x="2" y="2.5" width="12" height="8.5" rx="1.5" />
    <path d="M8 11v2.5M5.5 13.5h5" />
  {:else if name === "search"}
    <circle cx="7" cy="7" r="4" />
    <path d="M10 10l3.5 3.5" />
  {:else if name === "alert"}
    <circle cx="8" cy="8" r="6" />
    <path d="M8 5v3.5M8 11h.01" />
  {:else if name === "reset"}
    <path d="M3.5 8a4.5 4.5 0 1 0 1.4-3.3M3 2.5v3h3" />
  {:else if name === "lock"}
    <rect x="3.5" y="7" width="9" height="6.5" rx="1.5" />
    <path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2" />
  {:else if name === "unlock"}
    <rect x="3.5" y="7" width="9" height="6.5" rx="1.5" />
    <path d="M5.5 7V5a2.5 2.5 0 0 1 4.9-.7" />
  {:else if name === "eye"}
    <path d="M1.5 8S4 3.5 8 3.5 14.5 8 14.5 8 12 12.5 8 12.5 1.5 8 1.5 8Z" />
    <circle cx="8" cy="8" r="2" />
  {:else if name === "eye-off"}
    <path d="M1.5 8S4 3.5 8 3.5 14.5 8 14.5 8 12 12.5 8 12.5 1.5 8 1.5 8Z" />
    <path d="M3 13 13 3" />
  {/if}
</svg>

<style>
  .icon {
    display: block;
    flex: none;
  }
</style>
