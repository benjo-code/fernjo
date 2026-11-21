import { expect, test } from "vitest";
import { mappedCodes, scancodeFor } from "./keymap";

test("letters, digits, arrows and numpad enter map to set 1", () => {
  expect(scancodeFor("KeyA")).toEqual({ scancode: 0x1e, extended: false });
  expect(scancodeFor("KeyZ")).toEqual({ scancode: 0x2c, extended: false });
  expect(scancodeFor("Digit1")).toEqual({ scancode: 0x02, extended: false });
  expect(scancodeFor("Digit0")).toEqual({ scancode: 0x0b, extended: false });
  expect(scancodeFor("Enter")).toEqual({ scancode: 0x1c, extended: false });
  expect(scancodeFor("Space")).toEqual({ scancode: 0x39, extended: false });
  expect(scancodeFor("F12")).toEqual({ scancode: 0x58, extended: false });
  expect(scancodeFor("ArrowUp")).toEqual({ scancode: 0x48, extended: true });
  expect(scancodeFor("NumpadEnter")).toEqual({ scancode: 0x1c, extended: true });
  expect(scancodeFor("ControlRight")).toEqual({ scancode: 0x1d, extended: true });
  expect(scancodeFor("ControlLeft")).toEqual({ scancode: 0x1d, extended: false });
  expect(scancodeFor("Delete")).toEqual({ scancode: 0x53, extended: true });
  expect(scancodeFor("Numpad7")).toEqual({ scancode: 0x47, extended: false });
  expect(scancodeFor("Home")).toEqual({ scancode: 0x47, extended: true });
  expect(scancodeFor("MetaLeft")).toEqual({ scancode: 0x5b, extended: true });
});

test("the physical key decides, not the layout (KeyY stays the Y position)", () => {
  expect(scancodeFor("KeyY")).toEqual({ scancode: 0x15, extended: false });
});

test("an unknown code is null", () => {
  for (const code of ["", "Pause", "MediaPlayPause", "KeyÄ", "toString", "__proto__"]) {
    expect(scancodeFor(code), code).toBeNull();
  }
});

test("no two codes share a scancode and extended flag", () => {
  const seen = new Map<string, string>();
  for (const code of mappedCodes()) {
    const s = scancodeFor(code)!;
    expect(s.scancode).toBeGreaterThan(0);
    expect(s.scancode).toBeLessThan(0x80);
    const key = `${s.scancode}/${s.extended}`;
    expect(seen.get(key), `${code} clashes with ${seen.get(key)}`).toBeUndefined();
    seen.set(key, code);
  }
});
