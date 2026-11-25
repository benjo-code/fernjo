// Every input that can hold or show a secret opts out of the browser's autofill and saved
// history, so a value never lands in the WebView profile on disk (M11 review #63; rule
// "secrets stay in Rust", CLAUDE.md). A source scan, because the reveal inputs only render
// after a reveal and a new secret field would otherwise slip in unnoticed.
import { describe, expect, test } from "vitest";

const sources = import.meta.glob("./components/*.svelte", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

// A tag may contain `=>` or `>` inside an attribute expression, so read up to the first `>`
// that is not inside braces or quotes.
function tags(source: string): string[] {
  const out: string[] = [];
  const open = /<(input|textarea)\b/g;
  let m: RegExpExecArray | null;
  while ((m = open.exec(source)) !== null) {
    let depth = 0;
    let quote = "";
    let i = m.index;
    for (; i < source.length; i++) {
      const c = source[i];
      if (quote) {
        if (c === quote) quote = "";
      } else if (c === '"' || c === "'") quote = c;
      else if (c === "{") depth++;
      else if (c === "}") depth--;
      else if (c === ">" && depth === 0) break;
    }
    out.push(source.slice(m.index, i + 1));
  }
  return out;
}

const SECRET =
  /"password"|-password|passphrase|private-key|pairing|secret|aria-label="Pairing code"/i;
const OPT_OUT = /autocomplete="(off|new-password)"/;
const isSecret = (tag: string) => !/type="checkbox"/.test(tag) && SECRET.test(tag);

describe("secret inputs", () => {
  test("the_scan_finds_the_known_secret_inputs", () => {
    const all = Object.entries(sources).flatMap(([file, src]) =>
      tags(src)
        .filter(isSecret)
        .map((t) => `${file}: ${t.slice(0, 60)}`),
    );
    // PasswordField, the two reveal inputs, the private key, the pairing code
    expect(all.length).toBeGreaterThanOrEqual(5);
  });

  test("every_secret_input_opts_out_of_autofill", () => {
    const missing: string[] = [];
    for (const [file, src] of Object.entries(sources)) {
      for (const tag of tags(src)) {
        if (isSecret(tag) && !OPT_OUT.test(tag)) {
          missing.push(`${file}: ${tag.replace(/\s+/g, " ").slice(0, 100)}`);
        }
      }
    }
    expect(missing).toEqual([]);
  });

  test("the_masked_field_uses_new_password_so_chromium_never_offers_a_saved_password", () => {
    const field = tags(sources["./components/PasswordField.svelte"]!)[0]!;
    expect(field).toContain('autocomplete="new-password"');
  });
});
