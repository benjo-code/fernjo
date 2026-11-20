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

/**
 * One line of the shortcut list the UI shows (Settings, the filter tooltip). An entry with an
 * `action` is a key chord that `routeKey` maps; the others are tree gestures that the tree handles
 * itself and only carry display text. `shortcutEvent` builds the event `routeKey` sees for a
 * chord, so a test can prove that the list and `routeKey` agree.
 */
export type Shortcut = {
  label: string;
} & (
  | { action: AppAction; key: string; shift?: boolean }
  | { action?: undefined; text: (mac: boolean) => string }
);

/** Ctrl on every platform, plus Shift for the previous tab. */
const TAB_KEY = "Tab";

export const SHORTCUTS: readonly Shortcut[] = [
  { label: "Filter connections", action: "focusFilter", key: "K" },
  { label: "Next tab", action: "nextTab", key: TAB_KEY },
  { label: "Previous tab", action: "prevTab", key: TAB_KEY, shift: true },
  { label: "Close tab", action: "closeTab", key: "W" },
  { label: "Search in the terminal", action: "terminalSearch", key: "F" },
  { label: "Copy in the terminal", action: "copy", key: "C" },
  { label: "Paste in the terminal", action: "paste", key: "V" },
  { label: "Open a connection", text: () => "Double-click or Enter" },
  { label: "Select several", text: (mac) => `${mac ? "Cmd" : "Ctrl"}+click` },
  { label: "Select a range", text: () => "Shift+click" },
  { label: "Rename", text: () => "F2" },
  { label: "Delete selected", text: () => "Delete" },
];

/** The list entry for an app action, or `undefined` when none is listed. */
export function shortcutFor(action: AppAction): Shortcut | undefined {
  return SHORTCUTS.find((s) => s.action === action);
}

/** The keys as shown to the user on this platform, for example `Ctrl+Shift+K` or `Cmd+K`. */
export function shortcutKeys(s: Shortcut, mac: boolean): string {
  if (!s.action) return s.text(mac);
  if (s.key === TAB_KEY) return s.shift ? "Ctrl+Shift+Tab" : "Ctrl+Tab";
  return mac ? `Cmd+${s.key}` : `Ctrl+Shift+${s.key}`;
}

/** The key event `routeKey` receives for a chord entry; `null` for the gestures without one. */
export function shortcutEvent(s: Shortcut, mac: boolean): Key | null {
  if (!s.action) return null;
  const tab = s.key === TAB_KEY;
  return {
    key: s.key,
    ctrlKey: tab || !mac,
    shiftKey: tab ? !!s.shift : !mac,
    altKey: false,
    metaKey: !tab && mac,
  };
}

type NavLike = { platform?: string; userAgentData?: { platform?: string } };

/** The one place that decides whether Cmd-style shortcuts apply; falls back to `false`. */
export function isMac(nav?: NavLike): boolean {
  const n: NavLike | undefined = nav ?? (typeof navigator === "undefined" ? undefined : navigator);
  const platform = n?.userAgentData?.platform || n?.platform || "";
  return /^mac/i.test(platform);
}
