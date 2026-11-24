// The activity heartbeat for the vault auto-lock. Rust owns the idle clock; this only tells it
// that the user is still there. The listeners are passive and run in the capture phase on the
// window, so a view that stops propagation (the terminal, the remote desktop) still counts.

const EVENTS = ["keydown", "pointerdown", "pointermove", "wheel"] as const;

/**
 * Calls `send` at most once per `minGapMs` while input arrives on `target`. Returns the function
 * that removes the listeners.
 */
export function startActivityHeartbeat(
  send: () => void,
  target: Window,
  now: () => number,
  minGapMs = 5000,
): () => void {
  let last: number | null = null;
  const onActivity = () => {
    const t = now();
    if (last !== null && t - last < minGapMs) return;
    last = t;
    send();
  };
  const options = { capture: true, passive: true } as const;
  for (const name of EVENTS) target.addEventListener(name, onActivity, options);
  return () => {
    for (const name of EVENTS) target.removeEventListener(name, onActivity, options);
  };
}
