// Guards the CSS written inside Svelte components: same rules as tokens.test.ts applies to
// the token files (hex only, nothing WebKitGTK 4.1 / safari13 cannot render, no data: assets).
import { describe, expect, test } from "vitest";

const sources = import.meta.glob("/src/**/*.svelte", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

export function styleBlocks(source: string): string[] {
  return [...source.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map((m) =>
    m[1].replace(/\/\*[\s\S]*?\*\//g, ""),
  );
}

export function violations(css: string): string[] {
  const banned: [string, RegExp][] = [
    ["oklch(", /oklch\(/],
    ["color-mix(", /color-mix\(/],
    ["light-dark(", /light-dark\(/],
    ["@layer", /@layer/],
    [":has(", /:has\(/],
    ["@container", /@container/],
    ["nesting (&)", /&/],
    ["hex colour literal", /#[0-9a-fA-F]{3,8}\b/],
    ["data: URI", /data:/],
    ["external url", /url\(\s*['"]?https?:/],
  ];
  return banned.filter(([, re]) => re.test(css)).map(([name]) => name);
}

describe("component CSS", () => {
  test("the_guard_detects_each_banned_construct", () => {
    expect(violations("a { color: oklch(1 0 0) }")).toEqual(["oklch("]);
    expect(violations("a { color: #fff }")).toEqual(["hex colour literal"]);
    expect(violations("a { background: url(data:image/png;base64,AA) }")).toEqual(["data: URI"]);
    expect(violations("a { & b { color: red } }")).toEqual(["nesting (&)"]);
    expect(violations("a:has(b) { }")).toEqual([":has("]);
    expect(violations("a { color: var(--color-text); padding: var(--space-3) }")).toEqual([]);
    expect(styleBlocks("<p>x</p><style>a { b: c }</style>")).toEqual(["a { b: c }"]);
  });

  test("the_app_shell_is_among_the_checked_files", () => {
    expect(Object.keys(sources)).toContain("/src/App.svelte");
  });

  test("no_svelte_component_uses_unsupported_css", () => {
    for (const [file, source] of Object.entries(sources)) {
      for (const css of styleBlocks(source)) {
        expect(violations(css), file).toEqual([]);
      }
    }
  });
});
