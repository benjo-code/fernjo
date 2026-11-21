// Mapping a pointer position on the (possibly scaled) canvas to desktop pixels, and the browser's
// button numbers to RDP buttons.

export interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface DesktopSize {
  width: number;
  height: number;
}

/**
 * The desktop pixel under a client position: the canvas box is scaled to the desktop, the result
 * is rounded and clamped to `0..size-1`, so a pointer on the border or a pointer captured outside
 * the canvas still lands on a real pixel. An empty box (a hidden tab) maps to the origin.
 */
export function toDesktop(
  clientX: number,
  clientY: number,
  box: Box,
  desktop: DesktopSize,
): { x: number; y: number } {
  const axis = (client: number, start: number, extent: number, size: number) => {
    if (!(extent > 0) || !Number.isFinite(client)) return 0;
    const scaled = Math.round(((client - start) / extent) * size);
    return Math.min(Math.max(scaled, 0), Math.max(size - 1, 0));
  };
  return {
    x: axis(clientX, box.left, box.width, desktop.width),
    y: axis(clientY, box.top, box.height, desktop.height),
  };
}

export type RdpButton = "left" | "middle" | "right" | "x1" | "x2";

/** `PointerEvent.button`: 0 left, 1 middle, 2 right, 3 back, 4 forward. */
export function buttonOf(button: number): RdpButton | null {
  switch (button) {
    case 0:
      return "left";
    case 1:
      return "middle";
    case 2:
      return "right";
    case 3:
      return "x1";
    case 4:
      return "x2";
    default:
      return null;
  }
}
