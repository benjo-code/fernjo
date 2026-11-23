<script lang="ts">
  let {
    orientation,
    value,
    min,
    max,
    label,
    onchange,
    onreset,
    invert = false,
  }: {
    /** `vertical` is a vertical bar that resizes horizontally, and the other way round. */
    orientation: "vertical" | "horizontal";
    value: number;
    min: number;
    max: number;
    label: string;
    onchange: (px: number) => void;
    onreset: () => void;
    /** The thing grows when the bar moves up or left (a panel below or right of the bar). */
    invert?: boolean;
  } = $props();

  let dragging = $state(false);
  let startPos = 0;
  let startValue = 0;

  const clamp = (n: number) => Math.round(Math.min(max, Math.max(min, n)));
  const pos = (e: MouseEvent) => (orientation === "vertical" ? e.clientX : e.clientY);

  function onkeydown(e: KeyboardEvent) {
    const step = e.shiftKey ? 64 : 16;
    const [more, less] =
      orientation === "vertical" ? ["ArrowRight", "ArrowLeft"] : ["ArrowDown", "ArrowUp"];
    // a panel that grows upwards grows with ArrowUp
    const [grow, shrink] = invert ? [less, more] : [more, less];
    let next: number;
    if (e.key === grow) next = value + step;
    else if (e.key === shrink) next = value - step;
    else if (e.key === "Home") next = min;
    else if (e.key === "End") next = max;
    else return;
    e.preventDefault();
    onchange(clamp(next));
  }

  function onpointerdown(e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    dragging = true;
    startPos = pos(e);
    startValue = value;
    try {
      (e.currentTarget as Element).setPointerCapture?.(e.pointerId);
    } catch {
      // a synthetic pointer has nothing to capture
    }
  }

  function onpointermove(e: PointerEvent) {
    if (!dragging) return;
    const delta = pos(e) - startPos;
    onchange(clamp(startValue + (invert ? -delta : delta)));
  }

  function end() {
    dragging = false;
  }
</script>

<!-- a separator is focusable and operable by keyboard here, so the interactive-role lint does not apply -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
  class="splitter {orientation}"
  class:dragging
  role="separator"
  aria-orientation={orientation}
  aria-valuenow={value}
  aria-valuemin={min}
  aria-valuemax={max}
  aria-label={label}
  tabindex="0"
  {onkeydown}
  {onpointerdown}
  {onpointermove}
  onpointerup={end}
  onpointercancel={end}
  ondblclick={onreset}
></div>

<style>
  .splitter {
    position: absolute;
    z-index: 2;
    background: transparent;
  }

  .vertical {
    top: 0;
    /* the whole 6 px hit area hangs outside the sidebar, clear of the tree's scrollbar */
    right: -6px;
    bottom: 0;
    width: 6px;
    cursor: col-resize;
  }

  .horizontal {
    top: -3px;
    right: 0;
    left: 0;
    height: 6px;
    cursor: row-resize;
  }

  .splitter:hover,
  .splitter.dragging {
    background: var(--color-focus);
    opacity: 0.35;
  }

  .splitter:focus-visible {
    box-shadow: inset 0 0 0 1px var(--color-focus);
    outline: none;
  }
</style>
