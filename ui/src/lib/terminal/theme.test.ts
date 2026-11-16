import { expect, test } from "vitest";
import { terminalTheme } from "./theme";

function stub(values: Record<string, string>): CSSStyleDeclaration {
  return { getPropertyValue: (name: string) => values[name] ?? "" } as CSSStyleDeclaration;
}

test("theme_maps_every_term_token_to_the_xterm_theme", () => {
  const values: Record<string, string> = {
    "--term-bg": " #111111",
    "--term-fg": "#222222 ",
    "--term-cursor": "#333333",
    "--term-selection": "#444444",
  };
  for (let i = 0; i < 16; i++)
    values[`--term-ansi-${i}`] = `#0000${i.toString(16).padStart(2, "0")}`;
  const t = terminalTheme(stub(values));
  expect(t.background).toBe("#111111");
  expect(t.foreground).toBe("#222222");
  expect(t.cursor).toBe("#333333");
  expect(t.selectionBackground).toBe("#444444");
  const names = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "brightBlack",
    "brightRed",
    "brightGreen",
    "brightYellow",
    "brightBlue",
    "brightMagenta",
    "brightCyan",
    "brightWhite",
  ] as const;
  names.forEach((n, i) => expect(t[n]).toBe(values[`--term-ansi-${i}`]));
});
