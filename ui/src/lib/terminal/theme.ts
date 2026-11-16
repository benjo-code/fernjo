import type { ITheme } from "@xterm/xterm";

const ANSI = [
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

/** Maps the `--term-*` tokens (resolved for the current light/dark scheme) onto the xterm theme. */
export function terminalTheme(style: CSSStyleDeclaration): ITheme {
  const v = (name: string) => style.getPropertyValue(name).trim();
  const theme: ITheme = {
    background: v("--term-bg"),
    foreground: v("--term-fg"),
    cursor: v("--term-cursor"),
    selectionBackground: v("--term-selection"),
  };
  ANSI.forEach((key, i) => {
    theme[key] = v(`--term-ansi-${i}`);
  });
  return theme;
}
