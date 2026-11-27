#!/usr/bin/env node
// Writes and checks `latest.json`, the update feed the app's updater reads.
//
//   node tools/write-latest-json.mjs <version> <dir> <changelog>
//   node tools/write-latest-json.mjs --check <version> <dir>
//
// <dir> is the release folder: the four update artifacts and their `.sig` files (names follow
// tools/verify-installers.sh). The write form reads each `.sig`, takes the release notes from the
// `## [<version>]` section of the changelog (the release procedure turns `[Unreleased]` into that
// section before the tag) and writes <dir>/latest.json. The check form exits 1 unless the file
// matches the folder: same version, exactly the four platforms, each url the exact release URL of
// an existing file, each signature the text of that file's `.sig`, notes not empty.
//
// `SOURCE_DATE_EPOCH` (seconds) overrides the clock, for reproducible output and tests.

import { existsSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const REPO_URL = "https://github.com/benjo-code/fernjo/releases/download";
const MAX_NOTES_BYTES = 16 * 1024;

// Platform key of the updater -> artifact name for a version.
export const platformFiles = (version) => ({
  "windows-x86_64": `fernjo_${version}_x64-setup.exe`,
  "darwin-aarch64": `fernjo_${version}_aarch64.app.tar.gz`,
  "darwin-x86_64": `fernjo_${version}_x64.app.tar.gz`,
  "linux-x86_64": `fernjo_${version}_amd64.AppImage`,
});

const fail = (message) => {
  throw new Error(message);
};

const checkVersion = (version) => {
  if (!/^\d+\.\d+\.\d+$/.test(version ?? "")) fail(`'${version}' is not a valid version`);
};

// The text of the `## [<version>]` section, without its heading, cut to MAX_NOTES_BYTES on a
// character boundary.
export function sectionNotes(changelog, version) {
  const lines = changelog.split(/\r?\n/);
  // Compared as text: a pattern built from the argument would let it match other headings.
  const start = lines.findIndex((line) => {
    const [marks, title] = line.split(/\s+/);
    return marks === "##" && title === `[${version}]`;
  });
  if (start < 0) fail(`CHANGELOG has no [${version}] section`);
  let end = lines.findIndex((line, i) => i > start && /^##\s/.test(line));
  if (end < 0) end = lines.length;
  const notes = lines.slice(start + 1, end).join("\n").trim();
  if (notes === "") fail(`the [${version}] section of the changelog is empty`);
  return capBytes(notes, MAX_NOTES_BYTES);
}

export function capBytes(text, limit) {
  const bytes = Buffer.from(text, "utf8");
  if (bytes.length <= limit) return text;
  let cut = limit;
  // Step back to the start of a UTF-8 sequence (continuation bytes are 10xxxxxx).
  while (cut > 0 && (bytes[cut] & 0xc0) === 0x80) cut -= 1;
  return bytes.subarray(0, cut).toString("utf8").trimEnd();
}

const readSig = (dir, file) => {
  const path = join(dir, `${file}.sig`);
  if (!existsSync(path)) fail(`missing: ${file}.sig`);
  const text = readFileSync(path, "utf8").trim();
  if (text === "") fail(`empty: ${file}.sig`);
  return text;
};

const pubDate = () => {
  const epoch = process.env.SOURCE_DATE_EPOCH;
  const date = epoch === undefined || epoch === "" ? new Date() : new Date(Number(epoch) * 1000);
  if (Number.isNaN(date.getTime())) fail("SOURCE_DATE_EPOCH is not a number");
  return date.toISOString().replace(/\.\d{3}Z$/, "Z");
};

export function build(version, dir, changelogPath) {
  checkVersion(version);
  const notes = sectionNotes(readFileSync(changelogPath, "utf8"), version);
  const platforms = {};
  for (const [key, file] of Object.entries(platformFiles(version))) {
    if (!existsSync(join(dir, file))) fail(`missing: ${file}`);
    platforms[key] = { signature: readSig(dir, file), url: `${REPO_URL}/v${version}/${file}` };
  }
  return { version, notes, pub_date: pubDate(), platforms };
}

// Returns a list of problems, empty when latest.json matches the folder.
export function check(version, dir) {
  checkVersion(version);
  const path = join(dir, "latest.json");
  if (!existsSync(path)) return ["missing: latest.json"];
  let doc;
  try {
    doc = JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    return [`latest.json is not valid JSON: ${error.message}`];
  }
  const problems = [];
  if (doc.version !== version) {
    problems.push(`version is ${JSON.stringify(doc.version)}, expected ${version}`);
  }
  if (typeof doc.notes !== "string" || doc.notes.trim() === "") problems.push("notes is empty");
  if (typeof doc.pub_date !== "string" || !/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d+)?Z$/.test(doc.pub_date)) {
    problems.push("pub_date is not an RFC 3339 UTC time");
  }
  const want = platformFiles(version);
  const have = doc.platforms && typeof doc.platforms === "object" ? doc.platforms : {};
  for (const key of Object.keys(have)) {
    if (!(key in want)) problems.push(`unexpected platform: ${key}`);
  }
  for (const [key, file] of Object.entries(want)) {
    const entry = have[key];
    if (!entry) {
      problems.push(`missing platform: ${key}`);
      continue;
    }
    const url = `${REPO_URL}/v${version}/${file}`;
    if (entry.url !== url) problems.push(`${key}: url is ${JSON.stringify(entry.url)}, expected ${url}`);
    if (!existsSync(join(dir, file))) {
      problems.push(`${key}: ${file} is not in the folder`);
      continue;
    }
    try {
      if (entry.signature !== readSig(dir, file)) {
        problems.push(`${key}: signature differs from ${file}.sig`);
      }
    } catch (error) {
      problems.push(`${key}: ${error.message}`);
    }
  }
  return problems;
}

function main(argv) {
  if (argv[0] === "--check" && argv.length === 3) {
    const problems = check(argv[1], argv[2]);
    if (problems.length > 0) {
      for (const problem of problems) console.error(`error: ${problem}`);
      return 1;
    }
    console.log(`latest.json matches ${argv[2]}`);
    return 0;
  }
  if (argv.length !== 3 || argv[0].startsWith("--")) {
    console.error("usage: write-latest-json.mjs <version> <dir> <changelog>");
    console.error("       write-latest-json.mjs --check <version> <dir>");
    return 2;
  }
  const [version, dir, changelog] = argv;
  const target = join(dir, "latest.json");
  const temp = `${target}.tmp`;
  try {
    const text = `${JSON.stringify(build(version, dir, changelog), null, 2)}\n`;
    writeFileSync(temp, text);
    renameSync(temp, target);
  } catch (error) {
    rmSync(temp, { force: true });
    console.error(`error: ${error.message}`);
    return 1;
  }
  console.log(`wrote ${target}`);
  return 0;
}

import { fileURLToPath } from "node:url";
if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  process.exitCode = main(process.argv.slice(2));
}
