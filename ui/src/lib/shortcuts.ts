export type AppAction =
  | "nextTab"
  | "prevTab"
  | "closeTab"
  | "focusFilter"
  | "terminalSearch"
  | "copy"
  | "paste"
  | "fullscreen";

/** Where the keyboard focus is: a terminal, a remote desktop, or anything else. */
export type FocusKind = "none" | "terminal" | "remote";

type Key = Pick<KeyboardEvent, "key" | "ctrlKey" | "shiftKey" | "altKey" | "metaKey"> & {
  /** The physical key (`KeyW`); used for a chord only when `key` is not a Latin letter. */
  code?: string;
};

/**
 * The letter a chord is bound to: `key` when it is a Latin letter (so Dvorak and AZERTY follow
 * what the user types), else the physical key from `code` (a Cyrillic or Greek layout still
 * reaches Ctrl+Shift+W). `null` for everything else.
 */
function chordLetter(e: Key): string | null {
  const key = e.key.toLowerCase();
  if (/^[a-z]$/.test(key)) return key;
  const m = /^Key([A-Z])$/.exec(e.code ?? "");
  return m ? m[1].toLowerCase() : null;
}

const CHORDS: Record<string, AppAction> = {
  w: "closeTab",
  k: "focusFilter",
  f: "terminalSearch",
  c: "copy",
  v: "paste",
};

/** Ctrl+Shift+F11 on every platform. */
const FULLSCREEN_KEY = "F11";

/**
 * Which app action a key press is, or `null` when the terminal, the remote desktop or the page
 * gets it. Plain Ctrl+letter and Alt+letter always belong to the shell. Chords are
 * Ctrl+Shift+letter, or Cmd+letter on macOS (where Ctrl+Shift+letter stays with the terminal).
 * Full screen (Ctrl+Shift+F11) and the tab switch (Ctrl+Tab, Ctrl+Shift+Tab) work from every
 * focus; in a remote desktop nothing else is taken, so a chord like Ctrl+Shift+W reaches the
 * remote machine.
 */
export function routeKey(e: Key, mac: boolean, focus: FocusKind): AppAction | null {
  if (e.altKey) return null;
  if (e.key === FULLSCREEN_KEY && e.ctrlKey && e.shiftKey && !e.metaKey) return "fullscreen";
  if (e.key === "Tab" && e.ctrlKey && !e.metaKey) return e.shiftKey ? "prevTab" : "nextTab";
  if (focus === "remote") return null;
  const letter = chordLetter(e);
  const action = letter ? CHORDS[letter] : undefined;
  if (!action) return null;
  const isChord = mac
    ? e.metaKey && !e.ctrlKey && !e.shiftKey
    : e.ctrlKey && e.shiftKey && !e.metaKey;
  if (!isChord) return null;
  if ((action === "copy" || action === "paste") && focus !== "terminal") return null;
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
  { label: "Remote desktop full screen", action: "fullscreen", key: FULLSCREEN_KEY, shift: true },
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
  if (s.key === FULLSCREEN_KEY) return "Ctrl+Shift+F11";
  return mac ? `Cmd+${s.key}` : `Ctrl+Shift+${s.key}`;
}

/** The key event `routeKey` receives for a chord entry; `null` for the gestures without one. */
export function shortcutEvent(s: Shortcut, mac: boolean): Key | null {
  if (!s.action) return null;
  // the tab switch and full screen are the same chord on every platform
  const same = s.key === TAB_KEY || s.key === FULLSCREEN_KEY;
  return {
    key: s.key,
    code: same ? undefined : `Key${s.key}`,
    ctrlKey: same || !mac,
    shiftKey: same ? !!s.shift : !mac,
    altKey: false,
    metaKey: !same && mac,
  };
}

type NavLike = { platform?: string; userAgentData?: { platform?: string } };

/** The one place that decides whether Cmd-style shortcuts apply; falls back to `false`. */
export function isMac(nav?: NavLike): boolean {
  const n: NavLike | undefined = nav ?? (typeof navigator === "undefined" ? undefined : navigator);
  const platform = n?.userAgentData?.platform || n?.platform || "";
  return /^mac/i.test(platform);
}
