#!/usr/bin/env node
// Writes the third-party licence notices that ship inside the installers.
//
//   node tools/third-party-notices.mjs <out-file>
//   node tools/third-party-notices.mjs --fixture <packages.json>   (checks a package list only)
//   node tools/third-party-notices.mjs --check-rust <file.md>       (checks cargo-about output has sections)
//
// The Rust part comes from `cargo about generate tools/notices/about.hbs` (about.toml accepts only
// the licences deny.toml allows; cargo-about fails on anything else). The UI part covers the npm
// packages Vite bundles into the app, with each licence text. The script exits 1 when a listed
// package is missing or has no licence text.
//
// Release use: the `build` job in .github/workflows/release.yml writes
// src-tauri/resources/THIRD-PARTY-NOTICES.md before `pnpm tauri build`. That file is committed
// as a one-line placeholder (tauri-build needs the bundled resource to exist at compile time).
// A local run overwrites the placeholder; restore it afterwards and never commit the output:
//   git checkout -- src-tauri/resources/THIRD-PARTY-NOTICES.md
// Needs cargo-about (see Docs/DECISIONS.md, "M6 dependencies: cargo-about") and `pnpm install`.

import { execFileSync, execSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

// Packages Vite bundles into the app (runtime dependencies and the two fonts). A name ending in
// `/` is a scope: every installed package of that scope counts.
export const BUNDLED = [
  "@tauri-apps/api",
  "@xterm/",
  "svelte",
  "@fontsource-variable/hanken-grotesk",
  "@fontsource-variable/red-hat-mono",
];

export function isBundled(name) {
  return BUNDLED.some((entry) => (entry.endsWith("/") ? name.startsWith(entry) : name === entry));
}

// Pure: packages is [{ name, version, license, licenseText }]; returns the Markdown section.
// Throws when the list is empty or a package has no licence text.
export function renderUiNotices(packages) {
  if (!Array.isArray(packages) || packages.length === 0) {
    throw new Error("no bundled UI packages to list");
  }
  const missing = packages.filter((p) => typeof p.licenseText !== "string" || p.licenseText.trim() === "");
  if (missing.length > 0) {
    throw new Error(
      `no licence text for: ${missing.map((p) => `${p.name}@${p.version}`).join(", ")}`,
    );
  }
  const sorted = [...packages].sort((a, b) => a.name.localeCompare(b.name));
  const parts = ["## UI packages", "", "These npm packages are bundled into the app's web interface.", ""];
  for (const p of sorted) {
    parts.push(`### ${p.name} ${p.version} (${p.license})`, "", "```text", p.licenseText.trim(), "```", "");
  }
  return parts.join("\n");
}

// Pure: the cargo-about output must contain at least one licence section.
export function checkRustNotices(text) {
  if (!/^### /m.test(text)) throw new Error("cargo about produced no licence sections");
  return text;
}

function licenseFiles(dir) {
  return readdirSync(dir)
    .filter((f) => /^(licen[cs]e|copying)([-_.].*)?$/i.test(f) && !/\.spdx$/i.test(f))
    .sort();
}

function run(cmd, args) {
  const options = { cwd: root, encoding: "utf8", maxBuffer: 256 * 1024 * 1024 };
  if (process.platform === "win32" && cmd === "pnpm") {
    // pnpm is a .cmd shim on Windows, which needs a shell; the arguments are fixed strings.
    return execSync(`${cmd} ${args.join(" ")}`, options);
  }
  return execFileSync(cmd, args, options);
}

function collectUiPackages() {
  const listing = JSON.parse(run("pnpm", ["licenses", "list", "--json"]));
  const found = new Map();
  for (const [license, entries] of Object.entries(listing)) {
    for (const entry of entries) {
      if (!isBundled(entry.name)) continue;
      const dir = entry.paths[0];
      const files = existsSync(dir) ? licenseFiles(dir) : [];
      const text = files.map((f) => readFileSync(join(dir, f), "utf8").trim()).join("\n\n");
      found.set(entry.name, {
        name: entry.name,
        version: entry.versions[0],
        license,
        licenseText: text,
      });
    }
  }
  for (const entry of BUNDLED) {
    if (entry.endsWith("/")) {
      if (![...found.keys()].some((n) => n.startsWith(entry))) {
        throw new Error(`no installed package in scope ${entry} (run pnpm install)`);
      }
    } else if (!found.has(entry)) {
      throw new Error(`bundled package ${entry} is not installed (run pnpm install)`);
    }
  }
  return [...found.values()];
}

function main(argv) {
  if (argv[0] === "--fixture") {
    if (!argv[1]) throw new Error("usage: third-party-notices.mjs --fixture <packages.json>");
    process.stdout.write(renderUiNotices(JSON.parse(readFileSync(argv[1], "utf8"))));
    return;
  }
  if (argv[0] === "--check-rust") {
    if (!argv[1]) throw new Error("usage: third-party-notices.mjs --check-rust <cargo-about-output.md>");
    checkRustNotices(readFileSync(argv[1], "utf8"));
    return;
  }
  if (argv.length !== 1 || argv[0].startsWith("-")) {
    throw new Error("usage: third-party-notices.mjs <out-file> | --fixture <packages.json>");
  }
  const ui = renderUiNotices(collectUiPackages());
  // -o instead of stdout: cargo-about hangs on a redirected stdout on Windows (Git Bash) and
  // refuses it in PowerShell.
  const tmp = mkdtempSync(join(tmpdir(), "fernjo-notices-"));
  let rust;
  try {
    const file = join(tmp, "rust.md");
    run("cargo", ["about", "generate", "--workspace", "--locked", "--fail", "-o", file, "tools/notices/about.hbs"]);
    rust = checkRustNotices(readFileSync(file, "utf8"));
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
  const header = [
    "# Third-party licence notices",
    "",
    "fernjo is released under MIT OR Apache-2.0. It includes the third-party software listed below.",
    "",
  ].join("\n");
  writeFileSync(argv[0], `${header}\n${rust.trim()}\n\n${ui}\n`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(`third-party-notices: ${error.message}`);
    process.exit(1);
  }
}
