# Milestone 0 – Repo scaffold & CI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An empty but fully set-up, public repo with a running Tauri shell and green CI on GitHub-hosted runners that all further milestones build on.

**Architecture:** One Cargo workspace (`crates/*` plus `src-tauri`) and one pnpm package at the repo root. The Svelte 5 sources and their Vite config live in `ui/`; `src-tauri/` embeds `ui/dist` at compile time. CI is four Linux jobs (`rust`, `ui`, `tauri-build`, `cargo-deny`) next to the existing `roadmap` job, and CodeQL is a separate workflow. Repo-settings changes (visibility, security features, branch protection) are scripted with `gh api` and gated on the owner's confirmation.

**Tech Stack:** Rust 1.100.0 (edition 2024) · Tauri 2.12 · Svelte 5.57 · Vite 8 · TypeScript **6.0.x** · pnpm 12 · Node 24 · Vitest 5 · ESLint 10 · Prettier 3 · cargo-deny · gitleaks · GitHub Actions · CodeQL

**Spec:** [2026-11-15-fernjo-design.md](../specs/2026-11-15-fernjo-design.md) (§4 architecture, §6 quality, §7 constraints, §9 build order). Roadmap: [M0 in ROADMAP.md](../../ROADMAP.md).

## Global Constraints

- Rust edition 2024, stable toolchain pinned in `rust-toolchain.toml`; `license = "MIT OR Apache-2.0"` on every crate; Node LTS with pnpm.
- Only `crates/fernjo-core` and `src-tauri` are created in M0. No other crate, no Tauri command, no plugin, no `@tauri-apps/api`, no styling decisions (the visual direction is fixed in M1; the window is deliberately bare).
- CI runs only on GitHub-hosted runners (`ubuntu-latest`). The word `self-hosted` must not appear anywhere in `.github/workflows/`.
- Actions are pinned to full commit SHAs with a `# vX.Y.Z` comment; every workflow sets `permissions: contents: read` at the top level (a job may add more, e.g. CodeQL `security-events: write`). No `pull_request_target`, no secrets in PR workflows.
- Verified action SHAs (resolved 2026-11-15; re-resolve with `gh api repos/<owner>/<repo>/commits/<tag> --jq .sha` if a tag moved): `actions/checkout` v7.0.1 `3d3c42e5aac5ba805825da76410c181273ba90b1` · `actions/setup-node` v7.1.0 `949feb2413d6458794dcd2491c4babbbce0c15c1` · `pnpm/action-setup` v6.1.1 `73e46992e05b19abd31f2f3c20e0e7370d762115` · `Swatinem/rust-cache` v2.9.3 `d1527c8f67ef9af357290475d4af4b0fab8f12e0` · `EmbarkStudios/cargo-deny-action` v2.1.1 `3c6349835b2b7b196a839186cb8b78e02f7b5f25` · `github/codeql-action` v4.39.3 `33d7b85da1a04171e211befe2c739303b9a4c066` · `actions/setup-dotnet` v6.0.0 `a98b56852c35b8e3190ac28c8c2271da59106c68`.
- Secrets never go into files, commits, logs or replies. `.env.example` is created only if a variable is needed (none is: decided in Task 1).
- **Outward-facing steps need the owner's explicit confirmation at execution time** (CLAUDE.md "Collaboration"): pushing the branch, opening the PR, every `gh api` call that changes repo settings, the visibility flip, branch protection changes, and the merge. Steps marked **[CONFIRM]** below. Stop and ask before each group.
- Git: all work on `feature/m00-repo-scaffold`, Conventional Commits, stage files by name, run `git show --stat HEAD` after each commit. Each commit that finishes a roadmap task also ticks it in `Docs/roadmap.json` and regenerates `Docs/ROADMAP.md` (`dotnet run tools/roadmap.cs -- generate`); acceptance criteria are ticked only with evidence (Task 10).
- Format of "done": build, test, lint commands in CLAUDE.md demonstrably green, and `dotnet run tools/roadmap.cs -- check` green.

## Review Focus

Failure modes the spec implies but no acceptance criterion exercises, most likely first. Each has its pinning step in the named task.

1. **Fresh clone, nothing built:** `cargo build --workspace` / `cargo clippy` fail because `ui/dist` does not exist yet (`tauri::generate_context!` embeds it). Expected: CI and CLAUDE.md state the order (`pnpm install`, `pnpm ui:build`, then cargo); CI jobs run from a clean checkout in that order. → Task 4 (CLAUDE.md note), Task 6 (job order).
2. **Secret-bearing files committed by accident:** `.env`, `vault.fernjo`, `connections.json`, `.bak` copies, private keys. Expected: all ignored, `.env.example` is not. → Task 1 (`git check-ignore` step).
3. **Workflow safe for fork PRs:** an unpinned action, a `pull_request_target`, or a self-hosted runner slips in. Expected: a grep check proves none exist. → Task 6 (check step), re-run in Task 10.
4. **Webview hardening drifts:** CSP set to `null`, `withGlobalTauri` enabled, or the default capability widened (the webview must never get more than `core:default`, secrets stay in Rust). Expected: a Rust test fails. → Task 4 (`tauri_conf.rs`).
5. **Dependabot proposes a breaking toolchain bump** (TypeScript 7 is outside typescript-eslint's `<6.1.0` and svelte-check's `^5 || ^6` peer ranges; the Rust toolchain is pinned and is not bumped by Dependabot). Expected: Dependabot ignores TypeScript majors until the tools support it. → Task 6 (`dependabot.yml`).

---

### Task 1: Repo hygiene – ignore rules, attributes, licences

**Files:**
- Create: `.gitignore`, `LICENSE-MIT`, `LICENSE-APACHE`
- Modify: `.gitattributes`, `.editorconfig`

**Interfaces:**
- Produces: ignore rules every later task relies on (`target/`, `node_modules/`, `ui/dist/`, `src-tauri/gen/` ignored; `Cargo.lock` and `pnpm-lock.yaml` **not** ignored).

- [ ] **Step 1: Write the check script** `scratchpad/check-ignore.sh` (outside the repo) that runs `git check-ignore -q <path>` and must succeed for: `target/x`, `node_modules/x`, `ui/dist/x`, `src-tauri/gen/schemas/x`, `.env`, `.env.local`, `vault.fernjo`, `connections.json`, `connections.json.bak`, `settings.json`, `id_ed25519`, `id_rsa`, `server.pem`; and must **fail** for `.env.example`, `Cargo.lock`, `pnpm-lock.yaml`, `ui/src/App.svelte`.
- [ ] **Step 2: Run it; expected FAIL** (no `.gitignore` yet).
- [ ] **Step 3: Create `.gitignore`** with the entries above (plus `*.pem`, `*.key`, `.DS_Store`, `Thumbs.db`, editor folders `.idea/`; keep `.vscode/` unignored only if shared settings exist, otherwise ignore). Comment the secret-file block "never commit".
- [ ] **Step 4: Run the script; expected PASS.**
- [ ] **Step 5: `.gitattributes` additions:** `*.icns binary`, `*.svg text`, `Cargo.lock -diff linguist-generated`, `pnpm-lock.yaml -diff linguist-generated`. **`.editorconfig` addition:** `[*.{ts,js,svelte,css,html}]` with `indent_size = 2` (web convention; Rust keeps 4).
- [ ] **Step 6: Licences.** `LICENSE-MIT`: standard MIT text, `Copyright (c) 2026 fernjo contributors`. `LICENSE-APACHE`: the unmodified Apache-2.0 text downloaded from `https://www.apache.org/licenses/LICENSE-2.0.txt` (do not retype it). Check: `grep -c "Apache License" LICENSE-APACHE` ≥ 1 and the file has 202 lines ±3.
- [ ] **Step 7: `.env.example` decision.** Not created: no secret/env var is needed (spec §7; secrets live in the vault, not the environment). Mention in the commit message so the roadmap task can be ticked.
- [ ] **Step 8: Tick** roadmap tasks "Stack-specific `.gitignore`…", "`LICENSE-MIT` and `LICENSE-APACHE` added", "`.env.example` …"; generate; commit `chore: ignore rules, attributes and licences` (files by name, incl. `Docs/roadmap.json`, `Docs/ROADMAP.md`).

### Task 2: Cargo workspace and `fernjo-core`

**Files:**
- Create: `Cargo.toml` (workspace), `rust-toolchain.toml`, `rustfmt.toml`, `crates/fernjo-core/Cargo.toml`, `crates/fernjo-core/src/lib.rs`

**Interfaces:**
- Produces: workspace root with `members = ["crates/*"]` (Task 4 appends `"src-tauri"`), `[workspace.package]` carrying `version = "0.0.0"`, `edition = "2024"`, `rust-version = "1.90"` (Tauri 2.12's minimum), `license = "MIT OR Apache-2.0"`, `repository = "https://github.com/benjo-code/fernjo"`, `publish = false`. Crate API: `pub fn version() -> &'static str` in `fernjo_core`.

- [ ] **Step 1: Write the failing test** in `crates/fernjo-core/src/lib.rs` (`#[cfg(test)] mod tests`): `version_is_the_crate_version` asserts `version() == env!("CARGO_PKG_VERSION")` and that `version().split('.').count() == 3` with every part parsing as `u32`. Add the crate manifest (`license.workspace = true` etc.) and workspace first so it compiles to a test failure, not a manifest error; `version()` is declared with `todo!()`.
- [ ] **Step 2: Run** `cargo test -p fernjo-core` → FAIL (panic `not yet implemented`).
- [ ] **Step 3: Implement** `version()` returning `env!("CARGO_PKG_VERSION")`.
- [ ] **Step 4: `rust-toolchain.toml`:** `channel = "1.100.0"`, `components = ["rustfmt", "clippy"]`, `profile = "minimal"`. `rustfmt.toml`: `newline_style = "Unix"` (Windows checkouts must not produce CRLF diffs). Commit `Cargo.lock`.
- [ ] **Step 5: Run** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` → all green (the only test passes).
- [ ] **Step 6: Tick** roadmap task "Cargo workspace…"; generate; commit `feat: cargo workspace with fernjo-core`.

### Task 3: UI toolchain – Svelte 5, Vite, lint and test tooling

**Files:**
- Create: `package.json`, `pnpm-lock.yaml`, `eslint.config.js`, `.prettierrc`, `.prettierignore`, `ui/index.html`, `ui/vite.config.ts`, `ui/svelte.config.js`, `ui/tsconfig.json`, `ui/src/main.ts`, `ui/src/App.svelte`, `ui/src/App.test.ts`

**Interfaces:**
- Produces (root `package.json` scripts, used by CI, Task 4 and CLAUDE.md): `ui:dev` (Vite dev server, port 1420, `strictPort`), `ui:build` (emits `ui/dist/index.html`), `tauri` (`tauri` CLI), `lint` (= ESLint + Prettier check + svelte-check), `check`, `test` (`vitest run`), `format`. `package.json` has `"packageManager": "pnpm@12.21.0"` and `engines.node >=22.12.0`.

- [ ] **Step 1: Install pnpm** `corepack enable pnpm` (or `npm i -g pnpm@12.21.0`); verify `pnpm --version` → `12.21.0`.
- [ ] **Step 2: Dependencies (devDependencies, exact majors):** `svelte@^5.57`, `vite@^8`, `@sveltejs/vite-plugin-svelte@^7`, `typescript@~6.0.3` (**not 7**, see Review Focus 5), `@tsconfig/svelte`, `svelte-check`, `vitest@^5`, `jsdom`, `eslint@^10`, `@eslint/js`, `typescript-eslint`, `eslint-plugin-svelte`, `svelte-eslint-parser`, `globals`, `prettier@^3`, `prettier-plugin-svelte`, `@tauri-apps/cli@^2.12` (add `eslint-config-prettier` only if the Svelte flat Prettier config requires it). If pnpm blocks a build script, approve only that package with `pnpm approve-builds` and list it in `package.json`; do not allow all.
- [ ] **Step 2b: Write the failing test** `ui/src/App.test.ts`: mounts `App` (Svelte 5 `mount`) into `document.body` under jsdom and asserts `document.querySelector("h1")?.textContent === "fernjo"`.
- [ ] **Step 3: Run** `pnpm test` → FAIL (`App.svelte` missing).
- [ ] **Step 4: Implement** `ui/src/App.svelte` (a single `<h1>fernjo</h1>`, no styles) and `main.ts`/`index.html`. `ui/vite.config.ts`: `root` = `ui`, `server.port 1420 strictPort`, `clearScreen false`, `build.outDir "dist"` (`ui/dist`), build `target` per the current Tauri v2 Vite guide (`chrome105` on Windows, `safari13` elsewhere, via `TAURI_ENV_PLATFORM`), Vitest `test.environment "jsdom"` and `resolve.conditions` including `browser` so Svelte mounts in tests.
- [ ] **Step 5: Run** `pnpm test` → PASS.
- [ ] **Step 6: Lint set-up.** `eslint.config.js` (flat; recommended JS + typescript-eslint + Svelte, ignores `ui/dist`, `src-tauri`, `target`, `Docs`, `tools`). Prettier scope is `ui/` and root config files only (`.prettierignore` excludes `Docs/` because `ROADMAP.md` is generated, plus `src-tauri/`, `target/`, `tools/`, lockfile); `.prettierrc`: `endOfLine "lf"`, `plugins: ["prettier-plugin-svelte"]`. `pnpm lint` runs `eslint .`, `prettier --check ui "*.{js,json}"`, and `svelte-check --tsconfig ui/tsconfig.json --fail-on-warnings`.
- [ ] **Step 7: Run** `pnpm lint` and `pnpm ui:build` → both green; `ui/dist/index.html` exists. Commit `pnpm-lock.yaml`.
- [ ] **Step 8: Commit** `feat(ui): svelte 5 frontend with lint and test tooling`. The roadmap line for the Tauri + Svelte task is ticked in Task 4, because the line covers both.

### Task 4: Tauri 2 shell showing the empty window

**Files:**
- Create: `src-tauri/Cargo.toml`, `src-tauri/build.rs`, `src-tauri/src/main.rs`, `src-tauri/src/lib.rs`, `src-tauri/tauri.conf.json`, `src-tauri/capabilities/default.json`, `src-tauri/icons/*` (generated), `src-tauri/tests/tauri_conf.rs`
- Modify: `Cargo.toml` (members += `"src-tauri"`), `CLAUDE.md` (pitfall note)

**Interfaces:**
- Consumes: `pnpm ui:dev` and `pnpm ui:build` from Task 3; workspace package metadata from Task 2.
- Produces: package `fernjo-app` (bin name `fernjo`) with `pub fn run()` in `lib.rs`; `tauri.conf.json` with `identifier "io.github.benjo-code.fernjo"` (decision, recorded in Task 7), `productName "fernjo"`, `build.devUrl "http://localhost:1420"`, `build.frontendDist "../ui/dist"`, `beforeDevCommand "pnpm ui:dev"`, `beforeBuildCommand "pnpm ui:build"`, one window `main` titled `fernjo` (1280×800, min 900×600), `bundle.active false` (installers are M6), non-null CSP.

- [ ] **Step 1: Write the failing test** `src-tauri/tests/tauri_conf.rs` (dev-dependency `serde_json`; reads `tauri.conf.json` and `capabilities/default.json` relative to `CARGO_MANIFEST_DIR`):
  - `csp_is_strict`: `app.security.csp` is a string containing `default-src 'self'`, with no `unsafe-eval` and no `http:`/`https:` origins.
  - `global_tauri_is_off`: `app.withGlobalTauri` is absent or `false`.
  - `default_capability_is_core_only`: `permissions == ["core:default"]` and `windows == ["main"]`.
- [ ] **Step 2: Add the crate skeleton** (Tauri `2.12`, `tauri-build 2.7` as build-dependency, `serde_json` dev-dependency; `license.workspace = true`) and run `cargo test -p fernjo-app --test tauri_conf` → FAIL (no config yet).
- [ ] **Step 3: Implement the shell.** `main.rs`: `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` and calls `fernjo_app_lib::run()` (lib name `fernjo_app_lib`); `lib.rs`: `tauri::Builder::default().run(tauri::generate_context!())`; `build.rs`: `tauri_build::build()`. Write `tauri.conf.json` and `capabilities/default.json` per Interfaces. Placeholder icon: generate a plain 1024×1024 PNG (solid colour with a lowercase "f", made with any tool, no brand claims) and run `pnpm tauri icon <png>`; commit the generated set; note in the commit that M1 replaces it. Add `"src-tauri"` to the workspace members.
- [ ] **Step 4: Run (clean tree order):** `pnpm ui:build` then `cargo test --workspace` → PASS (3 conf tests + the core test).
- [ ] **Step 5: Build and format/lint:** `cargo build --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check` → green.
- [ ] **Step 6: Manual check:** `pnpm tauri dev` opens a window titled `fernjo` showing the heading "fernjo"; closing it ends the process cleanly. Record the OS and result; macOS/Linux stay open until a machine is available.
- [ ] **Step 7: CLAUDE.md pitfall** under "Project-specific": "The Tauri crate embeds `ui/dist` at compile time, so on a fresh checkout run `pnpm install && pnpm ui:build` before `cargo build/clippy/test --workspace`." (names the script, no line numbers).
- [ ] **Step 8: Tick** roadmap task "Tauri 2 app in `src-tauri/`…"; generate; commit `feat: tauri shell with empty fernjo window`.

### Task 5: cargo-deny policy

**Files:**
- Create: `deny.toml`

**Interfaces:**
- Produces: `cargo deny check` (advisories, bans, licenses, sources) green on the workspace; consumed by the `cargo-deny` CI job (Task 6).

- [ ] **Step 1: Install** `cargo install cargo-deny --locked`; `cargo deny --version` works.
- [ ] **Step 2: Write `deny.toml`:** `[licenses]` allow-list starting from `MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, Unicode-3.0, CC0-1.0, MPL-2.0`; `[sources]` only crates.io (`unknown-registry = "deny"`, `unknown-git = "deny"`); `[advisories]` default deny for vulnerabilities; `[bans] multiple-versions = "warn"`.
- [ ] **Step 3: Run** `cargo deny check` → FAIL expected on first run. Resolve findings one by one: add a licence to the allow-list only when the crate is genuinely under it and compatible with `MIT OR Apache-2.0` (never a copyleft licence without asking the owner); for unmaintained-advisory findings from Tauri's Linux GTK3 stack (expected `RUSTSEC` entries for gtk-rs GTK3 bindings and similar), add an `ignore` entry **with the advisory id and a one-line reason**, never a blanket ignore.
- [ ] **Step 4: Run** `cargo deny check` → green (warnings for duplicate versions are acceptable).
- [ ] **Step 5: DECISIONS.md entry** "cargo-deny: licence allow-list and advisory ignores" listing every ignore with reason and the condition to remove it.
- [ ] **Step 6: Commit** `ci: cargo-deny policy`. (Roadmap tick happens with the CI task.)

### Task 6: CI workflows and Dependabot

**Files:**
- Create: `.github/workflows/ci.yml`, `.github/dependabot.yml`
- Modify: `.github/workflows/roadmap.yml`

**Interfaces:**
- Consumes: scripts `ui:build`, `lint`, `test`, `tauri` (Task 3); `deny.toml` (Task 5).
- Produces: status check names **`roadmap`, `rust`, `ui`, `tauri-build`, `cargo-deny`** (job `name:` equals the id; Task 10 requires exactly these).

- [ ] **Step 1: Write the failing policy check** `scratchpad/check-workflows.sh`: (a) `grep -rn self-hosted .github/workflows` returns nothing; (b) every `uses:` line in `.github/workflows/*.yml` matches `@[0-9a-f]{40}( |$)`; (c) no `pull_request_target`; (d) every workflow file has a top-level `permissions:` block. Run it → FAIL on (a) (`roadmap.yml`) and (d) is already satisfied there.
- [ ] **Step 2: `roadmap.yml`:** `runs-on: ubuntu-latest`; remove the "Install the .NET SDK into HOME" step and the "no root" comment; keep selftest and check steps, `setup-dotnet` stays at the pinned SHA with `10.0.x`.
- [ ] **Step 3: `ci.yml`:** triggers `pull_request` and `push` to `dev` and `main`; `permissions: contents: read`; `concurrency` per ref with `cancel-in-progress`; all jobs `runs-on: ubuntu-latest`, `timeout-minutes: 30`. Jobs, each starting with `actions/checkout` (`persist-credentials: false`):
  - `rust` – install Linux build deps (`libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev`); `pnpm/action-setup`, `actions/setup-node` (node 24, pnpm cache); `pnpm install --frozen-lockfile`; `pnpm ui:build`; `rustup show` (installs the pinned toolchain from `rust-toolchain.toml`; no third-party toolchain action); `Swatinem/rust-cache`; then `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
  - `ui` – pnpm + node set-up, `pnpm install --frozen-lockfile`, `pnpm lint`, `pnpm test`.
  - `tauri-build` – same Linux deps and set-up as `rust`, then `pnpm tauri build --debug --no-bundle`.
  - `cargo-deny` – `EmbarkStudios/cargo-deny-action` with default `check`.
- [ ] **Step 4: `dependabot.yml`:** ecosystems `cargo` (`/`), `npm` (`/`, covers pnpm), `github-actions` (`/`), weekly; npm `ignore` for `typescript` `version-update:semver-major` with a comment pointing at the peer ranges; open-PR limit 5 each.
- [ ] **Step 5: Run the policy check** → PASS. Run `dotnet run tools/roadmap.cs -- check` → PASS.
- [ ] **Step 6: Tick** roadmap tasks "CI on GitHub-hosted `ubuntu-latest`…", "Move the `roadmap` job…", "Dependabot for Cargo, npm (pnpm) and GitHub Actions"; generate; commit `ci: github-hosted workflows and dependabot`.
  (Real workflow behaviour is proven by the PR run in Task 8; until then these ticks stay provisional: if Task 8 fails, reopen the line.)

### Task 7: README, DECISIONS and project memory

**Files:**
- Modify: `README.md`, `Docs/DECISIONS.md`, `CLAUDE.md`

- [ ] **Step 1: README:** in "Setup" add the `pnpm install && pnpm ui:build` order and the `cargo`/`pnpm lint`/`pnpm test` commands; delete the line "The workspace and these commands are being set up in milestone 0."; keep the `SECURITY.md` pointer (file arrives in Task 9).
- [ ] **Step 2: DECISIONS.md** (new entries on top, template format, one per bullet): fixed versions (Rust 1.100.0 pinned, Tauri 2.12 line, Svelte 5, Vite 8, pnpm 12, Node 24); **TypeScript pinned to 6.0.x** until typescript-eslint and svelte-check support 7; single root pnpm package with sources in `ui/`; Tauri identifier `io.github.benjo-code.fernjo` (changing it later moves app data paths); strict CSP and `core:default`-only capability from day one; toolchain installed via `rustup` instead of a third-party action; secret scanning of history uses the gitleaks CLI, not `gitleaks-action` (needs a licence for organisations).
- [ ] **Step 3: Verify** `dotnet run tools/roadmap.cs -- check` still green; commit `docs: readme setup, decisions for the scaffold`.

### Task 8: Push the branch and get CI green

**[CONFIRM]** before pushing and opening the PR.

- [ ] **Step 1: Run the full local gate once more:** `pnpm install --frozen-lockfile`, `pnpm ui:build`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm lint`, `pnpm test`, `cargo deny check`, `dotnet run tools/roadmap.cs -- check` → all green; keep the output for the PR.
- [ ] **Step 2: Push and open the PR** `gh pr create --base dev --head feature/m00-repo-scaffold` (title `feat: milestone 0 – repo scaffold and CI`; body lists the commands and results from Step 1 and what is still open).
- [ ] **Step 3: Watch the run** `gh pr checks --watch`. Expected: `roadmap`, `rust`, `ui`, `tauri-build`, `cargo-deny` green. A failure is fixed at the cause (no skipped tests, no weakened checks) and pushed as a new commit.
- [ ] **Step 4: Record** the run URL for acceptance "CI is green on a PR to `dev`".

### Task 9: History scan, go public, repo security

**[CONFIRM]** before **every** sub-step group (a)–(d); these change GitHub settings and cannot be taken back (once public, the history may be cached).

- [ ] **(a) Scan history for secrets.** Download the gitleaks release (v8.30.1) into a new, empty directory in the scratchpad, verify its published checksum, run `gitleaks git --redact --verbose <repo>` and `gitleaks dir --redact <repo>`. Expected: `no leaks found` for both. Any finding stops the task and goes to the owner (rotate first; never print the value).
- [ ] **(b) Close the fork-PR gap before the flip.** `gh api -X PUT repos/benjo-code/fernjo/actions/permissions/fork-pr-contributor-approval -f approval_policy=all_external_contributors` and `gh api -X PUT repos/benjo-code/fernjo/actions/permissions/workflow -f default_workflow_permissions=read`. Verify with the matching `GET`s. (`dev` still carries the old runner label until this PR merges; the approval policy is what covers that short window, so merge promptly after (d).)
- [ ] **(c) Flip visibility** `gh repo edit benjo-code/fernjo --visibility public --accept-visibility-change-consequences`; verify `gh repo view benjo-code/fernjo --json visibility`.
- [ ] **(d) Security features and files.**
  - Create `SECURITY.md` (supported versions: latest release; report via GitHub private vulnerability reporting; no public issues for vulnerabilities; expected first response time stated) and commit `docs: security policy`.
  - `gh api -X PATCH repos/benjo-code/fernjo` with `security_and_analysis[secret_scanning][status]=enabled` and `security_and_analysis[secret_scanning_push_protection][status]=enabled`; `gh api -X PUT repos/benjo-code/fernjo/vulnerability-alerts`; `gh api -X PUT repos/benjo-code/fernjo/automated-security-fixes`; `gh api -X PUT repos/benjo-code/fernjo/private-vulnerability-reporting`.
  - Create `.github/workflows/codeql.yml`: triggers push and PR on `dev`/`main` plus a weekly schedule; top-level `permissions: contents: read`; one job `analyze` with `security-events: write`, a matrix over `javascript-typescript` and `rust` with `build-mode: none`, using `github/codeql-action` `init` and `analyze` at the pinned SHA. Commit `ci: codeql for rust and javascript-typescript`, push, and wait for the run.
  - Verify: `gh api repos/benjo-code/fernjo --jq '.visibility, .security_and_analysis'` shows secret scanning and push protection `enabled`; `gh api repos/benjo-code/fernjo/vulnerability-alerts` returns 204; `gh run list --workflow codeql.yml --limit 1` shows `success`.
  - If an org policy blocks one setting, record it as open with the exact command, do not work around it.

### Task 10: Required checks, acceptance, merge

**[CONFIRM]** before the branch-protection change and before the merge.

- [ ] **Step 1: Required status checks.** For both `main` and `dev`: `gh api -X PATCH repos/benjo-code/fernjo/branches/<branch>/protection/required_status_checks` with `strict=true` and checks `roadmap`, `rust`, `ui`, `tauri-build`, `cargo-deny` (`app_id` 15368, GitHub Actions). Verify with `gh api repos/benjo-code/fernjo/branches/<branch>/protection --jq '.required_status_checks.contexts, .allow_force_pushes.enabled, .allow_deletions.enabled'` → the five names, `false`, `false`. (CodeQL is monitored, not required.) Tick the roadmap task for required checks.
- [ ] **Step 2: Tick the remaining tasks** ("Scan the full git history…", "Repo security on…").
- [ ] **Step 3: Tick acceptance criteria, each with `evidence`:**
  1. build/test/lint locally: the commands and results from Task 8 Step 1;
  2. `pnpm tauri dev`: OS and result from Task 4 Step 6 (macOS/Linux explicitly marked open in the evidence text);
  3. CI green on a PR to `dev`: the run URL;
  4. branch protection: output of the `gh api …/protection` verification from Step 1;
  5. public with secret scanning, push protection and green CodeQL: the `gh api` output and run URL from Task 9;
  6. no self-hosted runner: `grep -rn self-hosted .github/workflows` returns nothing; also re-run `scratchpad/check-workflows.sh` → PASS.
- [ ] **Step 4: Set M0 status `done`** with `completedOn` (today's date at that time), `dotnet run tools/roadmap.cs -- generate`, then `dotnet run tools/roadmap.cs -- check` → green. Commit `docs: milestone 0 done`, push; wait for all checks to be green on the PR.
- [ ] **Step 5: Merge** the PR into `dev` with squash (allowed for feature branches) once every check is green; the branch is deleted automatically. No `CHANGELOG.md` entry: M0 is scaffold and internal, not user-facing.
- [ ] **Step 6: Report** what is verified (with commands) and what remains open (macOS/Linux manual check of `pnpm tauri dev`).
