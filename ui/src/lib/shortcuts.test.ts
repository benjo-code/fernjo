import { describe, expect, test } from "vitest";
import {
  isMac,
  routeKey,
  SHORTCUTS,
  shortcutEvent,
  shortcutFor,
  shortcutKeys,
  type AppAction,
} from "./shortcuts";

const k = (
  key: string,
  mods: Partial<Record<"ctrl" | "shift" | "alt" | "meta", boolean>> = {},
) => ({
  key,
  ctrlKey: !!mods.ctrl,
  shiftKey: !!mods.shift,
  altKey: !!mods.alt,
  metaKey: !!mods.meta,
});

describe("routeKey", () => {
  test("plain_ctrl_letters_and_alt_letters_reach_the_terminal", () => {
    for (const mac of [false, true]) {
      expect(routeKey(k("c", { ctrl: true }), mac, "terminal")).toBeNull();
      expect(routeKey(k("w", { ctrl: true }), mac, "terminal")).toBeNull();
      expect(routeKey(k("b", { ctrl: true }), mac, "terminal")).toBeNull();
      expect(routeKey(k("b", { alt: true }), mac, "terminal")).toBeNull();
    }
  });

  test("escape_is_never_an_app_shortcut", () => {
    for (const mac of [false, true]) {
      expect(routeKey(k("Escape"), mac, "terminal")).toBeNull();
      expect(routeKey(k("Escape", { ctrl: true, shift: true }), mac, "none")).toBeNull();
    }
  });

  test("ctrl_tab_and_ctrl_shift_tab_switch_tabs_everywhere", () => {
    for (const mac of [false, true]) {
      for (const inTerminal of ["none", "terminal", "remote"] as const) {
        expect(routeKey(k("Tab", { ctrl: true }), mac, inTerminal)).toBe("nextTab");
        expect(routeKey(k("Tab", { ctrl: true, shift: true }), mac, inTerminal)).toBe("prevTab");
      }
    }
  });

  test("ctrl_shift_chords_map_to_actions", () => {
    const chord = (key: string) => routeKey(k(key, { ctrl: true, shift: true }), false, "terminal");
    expect(chord("W")).toBe("closeTab");
    expect(chord("K")).toBe("focusFilter");
    expect(chord("F")).toBe("terminalSearch");
    expect(chord("C")).toBe("copy");
    expect(chord("V")).toBe("paste");
  });

  test("cmd_chords_only_on_mac", () => {
    expect(routeKey(k("w", { meta: true }), true, "terminal")).toBe("closeTab");
    expect(routeKey(k("k", { meta: true }), true, "terminal")).toBe("focusFilter");
    expect(routeKey(k("f", { meta: true }), true, "terminal")).toBe("terminalSearch");
    expect(routeKey(k("c", { meta: true }), true, "terminal")).toBe("copy");
    expect(routeKey(k("v", { meta: true }), true, "terminal")).toBe("paste");
    expect(routeKey(k("w", { meta: true }), false, "terminal")).toBeNull();
    expect(routeKey(k("k", { meta: true }), false, "none")).toBeNull();
  });

  test("ctrl_shift_w_is_not_close_on_mac", () => {
    expect(routeKey(k("W", { ctrl: true, shift: true }), true, "terminal")).toBeNull();
  });

  test("copy_and_paste_only_inside_the_terminal", () => {
    expect(routeKey(k("C", { ctrl: true, shift: true }), false, "none")).toBeNull();
    expect(routeKey(k("V", { ctrl: true, shift: true }), false, "none")).toBeNull();
    expect(routeKey(k("c", { meta: true }), true, "none")).toBeNull();
    expect(routeKey(k("v", { meta: true }), true, "none")).toBeNull();
    // the other chords work outside the terminal too
    expect(routeKey(k("W", { ctrl: true, shift: true }), false, "none")).toBe("closeTab");
  });
});

describe("routeKey on other layouts", () => {
  const withCode = (key: string, code: string, ctrl = true, shift = true) => ({
    ...k(key, { ctrl, shift }),
    code,
  });

  test("ctrl_shift_w_on_a_non_latin_layout_closes_the_tab", () => {
    expect(routeKey(withCode("ц", "KeyW"), false, "none")).toBe("closeTab");
    expect(routeKey(withCode("Ц", "KeyW"), false, "terminal")).toBe("closeTab");
  });

  test("a_non_latin_layout_does_not_make_plain_ctrl_letters_chords", () => {
    expect(routeKey(withCode("ц", "KeyW", true, false), false, "terminal")).toBeNull();
    expect(
      routeKey({ ...k("ц", { alt: true, ctrl: true, shift: true }), code: "KeyW" }, false, "none"),
    ).toBeNull();
  });

  test("a_non_letter_key_with_a_letter_code_is_not_a_chord_on_the_plain_key", () => {
    // key is a Latin letter: it wins over the physical code (Dvorak: code KeyW types ",")
    expect(routeKey(withCode("a", "KeyW"), false, "none")).toBeNull();
    expect(routeKey(withCode("k", "KeyF"), false, "none")).toBe("focusFilter");
    // neither a Latin key nor a letter code
    expect(routeKey(withCode("1", "Digit1"), false, "none")).toBeNull();
    expect(routeKey(withCode("ц", "Semicolon"), false, "none")).toBeNull();
  });
});

describe("routeKey in a remote desktop", () => {
  test("remote_focus_forwards_everything_except_fullscreen_and_tab_chords", () => {
    for (const mac of [false, true]) {
      expect(routeKey(k("F11", { ctrl: true, shift: true }), mac, "remote")).toBe("fullscreen");
      expect(routeKey(k("Tab", { ctrl: true }), mac, "remote")).toBe("nextTab");
      expect(routeKey(k("Tab", { ctrl: true, shift: true }), mac, "remote")).toBe("prevTab");
    }
    // Ctrl+Shift+W and Ctrl+Shift+K reach the remote machine, so does Cmd+W on a Mac
    expect(routeKey(k("W", { ctrl: true, shift: true }), false, "remote")).toBeNull();
    expect(routeKey(k("K", { ctrl: true, shift: true }), false, "remote")).toBeNull();
    expect(routeKey(k("w", { meta: true }), true, "remote")).toBeNull();
    expect(routeKey(k("c", { ctrl: true, shift: true }), false, "remote")).toBeNull();
    expect(routeKey(k("v", { ctrl: true, shift: true }), false, "remote")).toBeNull();
    expect(routeKey(k("Escape"), false, "remote")).toBeNull();
  });

  test("terminal_focus_is_unchanged", () => {
    expect(routeKey(k("W", { ctrl: true, shift: true }), false, "terminal")).toBe("closeTab");
    expect(routeKey(k("C", { ctrl: true, shift: true }), false, "terminal")).toBe("copy");
    expect(routeKey(k("c", { ctrl: true }), false, "terminal")).toBeNull();
  });

  test("fullscreen_needs_ctrl_and_shift_and_nothing_else", () => {
    expect(routeKey(k("F11"), false, "none")).toBeNull();
    expect(routeKey(k("F11", { ctrl: true }), false, "none")).toBeNull();
    expect(routeKey(k("F11", { shift: true }), false, "none")).toBeNull();
    expect(routeKey(k("F11", { ctrl: true, shift: true, alt: true }), false, "none")).toBeNull();
    expect(routeKey(k("F11", { ctrl: true, shift: true, meta: true }), false, "none")).toBeNull();
    expect(routeKey(k("F11", { ctrl: true, shift: true }), false, "none")).toBe("fullscreen");
    expect(routeKey(k("F11", { ctrl: true, shift: true }), false, "terminal")).toBe("fullscreen");
  });
});

describe("isMac", () => {
  test("isMac_reads_userAgentData_then_platform_and_defaults_to_false", () => {
    expect(isMac({ userAgentData: { platform: "macOS" }, platform: "Win32" })).toBe(true);
    expect(isMac({ platform: "MacIntel" })).toBe(true);
    expect(isMac({ platform: "Win32" })).toBe(false);
    expect(isMac({})).toBe(false);
    expect(isMac(undefined)).toBe(false);
  });
});

describe("SHORTCUTS", () => {
  test("every_entry_with_a_chord_routes_to_its_action_on_both_platforms", () => {
    const chords = SHORTCUTS.filter((s) => s.action);
    expect(chords.map((s) => s.action).sort()).toEqual(
      [
        "closeTab",
        "copy",
        "focusFilter",
        "fullscreen",
        "nextTab",
        "paste",
        "prevTab",
        "quickConnect",
        "terminalSearch",
      ].sort(),
    );
    for (const mac of [false, true]) {
      for (const s of chords) {
        const event = shortcutEvent(s, mac);
        expect(event).not.toBeNull();
        expect(routeKey(event!, mac, "terminal"), `${s.label} (mac=${mac})`).toBe(s.action);
      }
    }
  });

  test("the_other_platforms_chord_does_not_route_to_the_action", () => {
    // the tab switch and full screen are one chord everywhere
    const everywhere: (AppAction | undefined)[] = ["nextTab", "prevTab", "fullscreen"];
    for (const s of SHORTCUTS.filter((x) => x.action && !everywhere.includes(x.action))) {
      expect(routeKey(shortcutEvent(s, true)!, false, "terminal")).toBeNull();
      expect(routeKey(shortcutEvent(s, false)!, true, "terminal")).toBeNull();
    }
  });

  test("labels_name_the_chord_the_platform_really_uses", () => {
    const keys = (action: AppAction, mac: boolean) => shortcutKeys(shortcutFor(action)!, mac);
    expect(keys("focusFilter", false)).toBe("Ctrl+Shift+K");
    expect(keys("focusFilter", true)).toBe("Cmd+K");
    expect(keys("closeTab", false)).toBe("Ctrl+Shift+W");
    expect(keys("closeTab", true)).toBe("Cmd+W");
    expect(keys("terminalSearch", false)).toBe("Ctrl+Shift+F");
    expect(keys("terminalSearch", true)).toBe("Cmd+F");
    expect(keys("nextTab", false)).toBe("Ctrl+Tab");
    expect(keys("nextTab", true)).toBe("Ctrl+Tab");
    expect(keys("prevTab", false)).toBe("Ctrl+Shift+Tab");
    expect(keys("prevTab", true)).toBe("Ctrl+Shift+Tab");
    expect(keys("fullscreen", false)).toBe("Ctrl+Shift+F11");
    expect(keys("fullscreen", true)).toBe("Ctrl+Shift+F11");
  });

  test("mouse_and_tree_gestures_have_text_but_no_chord", () => {
    const plain = SHORTCUTS.filter((s) => !s.action);
    expect(plain.map((s) => shortcutKeys(s, false))).toEqual([
      "Double-click or Enter",
      "Ctrl+click",
      "Shift+click",
      "F2",
      "Delete",
    ]);
    expect(shortcutKeys(plain[1], true)).toBe("Cmd+click");
    for (const s of plain) expect(shortcutEvent(s, false)).toBeNull();
  });
});

describe("quick connect chord", () => {
  test("ctrl_shift_n_is_quick_connect_and_plain_ctrl_n_is_not", () => {
    expect(routeKey(k("N", { ctrl: true, shift: true }), false, "terminal")).toBe("quickConnect");
    expect(routeKey(k("n", { meta: true }), true, "none")).toBe("quickConnect");
    expect(routeKey(k("n", { ctrl: true }), false, "terminal")).toBeNull();
    expect(routeKey(k("n", { ctrl: true }), true, "terminal")).toBeNull();
    expect(routeKey(k("N", { ctrl: true, shift: true }), false, "remote")).toBeNull();
    expect(shortcutKeys(shortcutFor("quickConnect")!, false)).toBe("Ctrl+Shift+N");
    expect(shortcutKeys(shortcutFor("quickConnect")!, true)).toBe("Cmd+N");
  });
});
