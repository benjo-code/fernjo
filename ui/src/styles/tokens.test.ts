// Guards the design-token interface (see DESIGN.md): names later milestones rely on,
// light/dark parity, colours the target webviews can render, and WCAG contrast in both themes.
import { describe, expect, test } from "vitest";
import base from "./base.css?raw";
import tokens from "./tokens.css?raw";

type Block = Map<string, string>;

function parseBlock(css: string, selector: RegExp): Block {
  css = css.replace(/\/\*[\s\S]*?\*\//g, "");
  const match = selector.exec(css);
  if (!match) throw new Error(`block not found: ${selector}`);
  const start = match.index + match[0].length;
  const end = css.indexOf("}", start);
  const block: Block = new Map();
  for (const decl of css.slice(start, end).split(";")) {
    const m = /^\s*(--[\w-]+)\s*:\s*([^;]+?)\s*$/.exec(decl.replace(/\/\*[\s\S]*?\*\//g, ""));
    if (m) block.set(m[1], m[2]);
  }
  return block;
}

const light = parseBlock(tokens, /^:root\s*\{/m);
const darkAttr = parseBlock(tokens, /^:root\[data-theme="dark"\]\s*\{/m);
const darkMedia = parseBlock(
  tokens,
  /@media \(prefers-color-scheme: dark\)\s*\{\s*:root:not\(\[data-theme="light"\]\)\s*\{/,
);

const themed = [
  "--color-bg",
  "--color-surface",
  "--color-surface-raised",
  "--color-sidebar",
  "--color-border",
  "--color-border-strong",
  "--color-border-input",
  "--color-tab-text",
  "--color-text",
  "--color-text-muted",
  "--color-accent",
  "--color-accent-text",
  "--color-selection",
  "--color-hover",
  "--color-focus",
  "--color-danger",
  "--color-warning",
  "--color-success",
  "--color-lamp-live",
  "--color-lamp-connecting",
  "--color-lamp-dropped",
  "--color-ssh",
  "--color-rdp",
  "--color-overlay",
  "--term-bg",
  "--term-fg",
  "--term-cursor",
  "--term-selection",
  ...Array.from({ length: 16 }, (_, i) => `--term-ansi-${i}`),
];

const unthemed = [
  "--font-ui",
  "--font-mono",
  "--text-xs",
  "--text-sm",
  "--text-md",
  "--text-lg",
  "--weight-regular",
  "--weight-strong",
  ...Array.from({ length: 6 }, (_, i) => `--space-${i + 1}`),
  "--radius-sm",
  "--radius-md",
  "--row-height-tree",
  "--tree-indent",
  "--shadow-raised",
  "--duration-fast",
  "--duration-normal",
  "--duration-pulse",
  "--easing-pulse",
  "--lamp-size",
];

const isColour = (name: string) => name.startsWith("--color-") || name.startsWith("--term-");
const colourNames = (block: Block) => [...block.keys()].filter(isColour).sort();

function rgb(hex: string): [number, number, number, number] {
  const h = hex.slice(1);
  const a = h.length === 8 ? parseInt(h.slice(6, 8), 16) / 255 : 1;
  return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16), a];
}

// Translucent colours are measured composited over the theme's --color-bg.
function opaque(hex: string, block: Block): [number, number, number] {
  const [r, g, b, a] = rgb(hex);
  if (a === 1) return [r, g, b];
  const [br, bg, bb] = rgb(block.get("--color-bg")!);
  return [r * a + br * (1 - a), g * a + bg * (1 - a), b * a + bb * (1 - a)];
}

function luminance([r, g, b]: [number, number, number]): number {
  const lin = (v: number) => {
    const c = v / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

function contrast(fg: string, bg: string, block: Block): number {
  const [hi, lo] = [luminance(opaque(fg, block)), luminance(opaque(bg, block))].sort(
    (x, y) => y - x,
  );
  return (hi + 0.05) / (lo + 0.05);
}

const pairs: [string, string, number][] = [
  ["--color-text", "--color-bg", 4.5],
  ["--color-text", "--color-surface", 4.5],
  ["--color-text", "--color-surface-raised", 4.5],
  ["--color-text", "--color-sidebar", 4.5],
  ["--color-text", "--color-selection", 4.5],
  ["--color-text-muted", "--color-bg", 4.5],
  ["--color-text-muted", "--color-sidebar", 4.5],
  ["--color-accent-text", "--color-accent", 4.5],
  ["--color-danger", "--color-bg", 4.5],
  ["--color-danger", "--color-surface-raised", 4.5],
  ["--term-fg", "--term-bg", 4.5],
  ["--term-fg", "--term-selection", 4.5],
  ["--color-focus", "--color-bg", 3],
  ["--color-focus", "--color-sidebar", 3],
  ["--color-success", "--color-sidebar", 3],
  ["--color-warning", "--color-sidebar", 3],
  ["--color-danger", "--color-sidebar", 3],
  ["--color-tab-text", "--color-sidebar", 4.5],
  ["--color-border-input", "--color-surface", 3],
  ["--color-border-strong", "--color-sidebar", 3],
  ...["live", "connecting", "dropped"].flatMap((lamp): [string, string, number][] => [
    [`--color-lamp-${lamp}`, "--color-sidebar", 3],
    [`--color-lamp-${lamp}`, "--color-selection", 3],
  ]),
  ...[1, 2, 3, 4, 5, 6, 9, 10, 11, 12, 13, 14].map((n): [string, string, number] => [
    `--term-ansi-${n}`,
    "--term-bg",
    3,
  ]),
];

describe("design tokens", () => {
  test("every interface token is defined", () => {
    for (const name of [...themed, ...unthemed]) {
      expect(light.has(name), name).toBe(true);
    }
    for (const name of themed) {
      expect(darkAttr.has(name), name).toBe(true);
    }
  });

  test("dark blocks are identical", () => {
    expect([...darkMedia.entries()].sort()).toEqual([...darkAttr.entries()].sort());
  });

  test("themes define the same colour tokens", () => {
    expect(colourNames(darkAttr)).toEqual(colourNames(light));
  });

  test("colours are hex", () => {
    for (const block of [light, darkAttr]) {
      for (const name of colourNames(block)) {
        expect(block.get(name), name).toMatch(/^#([0-9a-f]{6}|[0-9a-f]{8})$/);
      }
    }
  });

  test("no unsupported css", () => {
    for (const css of [tokens, base].map((c) => c.replace(/\/\*[\s\S]*?\*\//g, ""))) {
      for (const banned of [
        "oklch(",
        "color-mix(",
        "light-dark(",
        "@layer",
        ":has(",
        "@container",
        "&",
        "@import url(http",
      ]) {
        expect(css.includes(banned), banned).toBe(false);
      }
    }
  });

  test.each([
    ["light", light],
    ["dark", darkAttr],
  ])("contrast in the %s theme", (_theme, block) => {
    for (const [fg, bg, min] of pairs) {
      const ratio = contrast(block.get(fg)!, block.get(bg)!, block);
      expect(ratio, `${fg} on ${bg}`).toBeGreaterThanOrEqual(min);
    }
  });
});
