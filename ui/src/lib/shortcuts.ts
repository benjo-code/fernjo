export type AppAction =
  "nextTab" | "prevTab" | "closeTab" | "focusFilter" | "terminalSearch" | "copy" | "paste";

type Key = Pick<KeyboardEvent, "key" | "ctrlKey" | "shiftKey" | "altKey" | "metaKey">;

const CHORDS: Record<string, AppAction> = {
  w: "closeTab",
  k: "focusFilter",
  f: "terminalSearch",
  c: "copy",
  v: "paste",
};

/**
 * Which app action a key press is, or `null` when the terminal or the page gets it. Plain
 * Ctrl+letter and Alt+letter always belong to the shell. Chords are Ctrl+Shift+letter, or
 * Cmd+letter on macOS (where Ctrl+Shift+letter stays with the terminal).
 */
export function routeKey(e: Key, mac: boolean, inTerminal: boolean): AppAction | null {
  if (e.altKey) return null;
  if (e.key === "Tab" && e.ctrlKey && !e.metaKey) return e.shiftKey ? "prevTab" : "nextTab";
  const action = CHORDS[e.key.toLowerCase()];
  if (!action) return null;
  const isChord = mac
    ? e.metaKey && !e.ctrlKey && !e.shiftKey
    : e.ctrlKey && e.shiftKey && !e.metaKey;
  if (!isChord) return null;
  if ((action === "copy" || action === "paste") && !inTerminal) return null;
  return action;
}

type NavLike = { platform?: string; userAgentData?: { platform?: string } };

/** The one place that decides whether Cmd-style shortcuts apply; falls back to `false`. */
export function isMac(nav?: NavLike): boolean {
  const n: NavLike | undefined = nav ?? (typeof navigator === "undefined" ? undefined : navigator);
  const platform = n?.userAgentData?.platform || n?.platform || "";
  return /^mac/i.test(platform);
}
