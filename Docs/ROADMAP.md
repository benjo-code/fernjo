<!-- Generated from Docs/roadmap.json with `dotnet run tools/roadmap.cs -- generate` – do not edit by hand. -->
# fernjo – Roadmap

Overall roadmap for the project. Every milestone is sized so that it can be completed in **one** plan and one round of implementation.

> **This file is generated.** The source is [roadmap.json](roadmap.json) (schema: [roadmap.schema.json](roadmap.schema.json)). Edit only that file, then run `dotnet run tools/roadmap.cs -- generate`.

## How we work

1. **“Plan milestone N”:** Claude Code reads this roadmap, the linked spec and the current state of the code, and writes a plan to `Docs/superpowers/plans/YYYY-MM-DD-milestone-NN-<topic>.md`. The plan is linked from the milestone, status → `🟡 planned`.
2. **Implementation:** Work through the plan. Tick off every finished task immediately (`done: true` in `Docs/roadmap.json`).
3. **Completion:** Once all acceptance criteria are verified and ticked off → status `✅ done`, enter the date.
4. Scope changes during implementation are recorded under “Scope changes”, not silently in the code.

**Status legend:** `⬜ open` · `🟡 planned` · `🔵 in progress` · `✅ done`

**Rules:**
- Work through milestones in order as a rule. The prerequisites listed on each milestone are what counts.
- A milestone starts with `Create spec` if its subproject has no spec yet. The spec is developed through brainstorming and stored under `Docs/superpowers/specs/`.
- Tasks of later subprojects are deliberately kept coarse and are refined when the respective spec is written.

## Overview

| # | Milestone | Subproject | Status | Plan | Done on |
|---|---|---|---|---|---|
| 0 | Repo scaffold & CI | 1 – fernjo app | ✅ done | [Plan](superpowers/plans/2026-11-15-milestone-00-repo-scaffold.md) | 2026-11-15 |
| 1 | UI design direction | 1 – fernjo app | ✅ done | [Plan](superpowers/plans/2026-11-15-milestone-01-design-direction.md) | 2026-11-15 |
| 2 | Connection tree & data model | 1 – fernjo app | 🔵 in progress | [Plan](superpowers/plans/2026-11-15-milestone-02-connection-tree.md) | – |
| 3 | Credential vault | 1 – fernjo app | ⬜ open | – | – |
| 4 | SSH sessions in tabs | 1 – fernjo app | ⬜ open | – | – |
| 5 | mRemoteNG import & export | 1 – fernjo app | ⬜ open | – | – |
| 6 | Packaging & app settings | 1 – fernjo app | ⬜ open | – | – |
| – | **Release `v0.1.0`** – Daily SSH driver (M0 to M6) | – | ⬜ open | – | – |
| 7 | RDP sessions | 1 – fernjo app | ⬜ open | – | – |
| – | **Release `v0.2.0`** – RDP (M7) | – | ⬜ open | – | – |

## Scope changes

- **2026-11-15 (M1 planning):** M1 gains the task "App icon derived from the chosen direction". `Docs/DECISIONS.md` (entry "Project layout: one root pnpm package, Tauri identifier") already planned to replace the placeholder icon in M1, but the roadmap had no task for it; the owner chose M1 over M6 or the backlog.

---

# Subproject 1 – fernjo app

**Spec:** [2026-11-15-fernjo-design.md](superpowers/specs/2026-11-15-fernjo-design.md) Cross-platform SSH & RDP connection manager (Rust core, Tauri 2 UI)

## Milestone 0 – Repo scaffold & CI
**Status:** ✅ done (2026-11-15) · **Plan:** [2026-11-15-milestone-00-repo-scaffold.md](superpowers/plans/2026-11-15-milestone-00-repo-scaffold.md) · **Prerequisites:** none

Goal: An empty but fully set-up, public repo with a running Tauri shell and green CI that all further milestones build on.

**Tasks**
- [x] Create spec (brainstorming): goal, scope, stack, subprojects; link it here and enter the first milestones
- [x] Repo setup per README done: `dev` as default branch, merge options, branch protection for `main` and `dev`
- [x] Fill in the project header in `CLAUDE.md` and `README.md`, remove the template section from the README
- [x] Stack-specific `.gitignore` (Rust `target/`, `node_modules/`, UI build output, `.env`) and `.gitattributes` additions
- [x] Cargo workspace (edition 2024, `rust-toolchain.toml`, `license = "MIT OR Apache-2.0"`) with crate `fernjo-core` incl. a first test
- [x] Tauri 2 app in `src-tauri/` with Svelte 5 + TypeScript frontend in `ui/` (Vite, pnpm) showing an empty fernjo window; ESLint, Prettier, `svelte-check` and Vitest set up
- [x] `LICENSE-MIT` and `LICENSE-APACHE` added
- [x] `.env.example` with placeholders, only if secrets are needed (none expected)
- [x] CI on GitHub-hosted `ubuntu-latest`: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `cargo-deny`, UI lint/check/test and a Tauri build check (WebKitGTK installed in the job), on PRs to `dev` and `main`
- [x] Move the `roadmap` job in `.github/workflows/roadmap.yml` from `[self-hosted, dotnet-linux]` to `ubuntu-latest`; no workflow uses a self-hosted runner
- [x] Add the CI jobs (`roadmap` and the build/test/lint jobs) as required status checks in the branch protection of `main` and `dev`
- [x] Dependabot for Cargo, npm (pnpm) and GitHub Actions
- [x] Scan the full git history for secrets (e.g. gitleaks), then make the repo public
- [x] Repo security on: secret scanning with push protection, Dependabot alerts and security updates, CodeQL for Rust and JavaScript/TypeScript, private vulnerability reporting with `SECURITY.md`, approval required for workflows from outside contributors

**Acceptance criteria**
- [x] Build, test and format/lint run green locally (commands in `CLAUDE.md`) – 2026-11-15 on Windows, head aff35f0: `pnpm install --frozen-lockfile`, `pnpm ui:build`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (3 config tests + 1 core test pass), `pnpm lint` (0 errors, 0 warnings), `pnpm test` (1 test pass), `cargo deny --all-features check` (ok), `dotnet run tools/roadmap.cs -- check` (valid) all green
- [x] `pnpm tauri dev` opens the empty fernjo window (manual check on Windows; macOS/Linux checked when a machine is available, otherwise marked open) – Windows 11, 2026-11-15, checked programmatically: `pnpm tauri dev` started, process `fernjo` had main window title `fernjo`, http://localhost:1420 served the page with `<h1>fernjo</h1>`, process tree stopped cleanly. OPEN: owner's visual confirmation of the window, and macOS and Linux (no machine available yet; check with `pnpm tauri dev`)
- [x] CI is green on a PR to `dev` – PR #2 to `dev`: workflow `CI` run https://github.com/benjo-code/fernjo/actions/runs/43725362775 (rust, ui, tauri-build, cargo-deny success) and workflow `Roadmap` run https://github.com/benjo-code/fernjo/actions/runs/43725362692 (roadmap success), all on GitHub-hosted runners
- [x] Branch protection for `main` and `dev` is active: PR required, CI checks required, no force push, no deletion – 2026-11-15 `gh api repos/benjo-code/fernjo/branches/<branch>/protection --jq '[.required_status_checks.contexts, .allow_force_pushes.enabled, .allow_deletions.enabled, .required_pull_request_reviews != null]'` for `main` and `dev` returns `[["roadmap","rust","ui","tauri-build","cargo-deny"],false,false,true]`
- [x] Repo is public with secret scanning and push protection enabled, and CodeQL has a green run – 2026-11-15 `gh api repos/benjo-code/fernjo --jq '.visibility, .security_and_analysis'` returns `public`, secret_scanning `enabled`, secret_scanning_push_protection `enabled`, dependabot_security_updates `enabled`; private vulnerability reporting and Dependabot alerts enabled; fork-PR approval policy `all_external_contributors`; CodeQL run https://github.com/benjo-code/fernjo/actions/runs/43726355044 succeeded (javascript-typescript and rust); history scan: gitleaks 8.30.1 `no leaks found` in 17 commits
- [x] No workflow uses a self-hosted runner – `grep -rn self-hosted .github/workflows` returns nothing (also enforced by the workflow policy script); CI and Roadmap workflow runs of PR #2 ran on GitHub-hosted runners

## Milestone 1 – UI design direction
**Status:** ✅ done (2026-11-15) · **Plan:** [2026-11-15-milestone-01-design-direction.md](superpowers/plans/2026-11-15-milestone-01-design-direction.md) · **Prerequisites:** M0

Goal: The visual direction of fernjo is chosen by the owner from at least three concepts and recorded, so every UI milestone builds on it.

**Tasks**
- [x] Design brief from spec §3: one-window layout, tree density for hundreds of connections, light and dark
- [x] Create at least three distinct design concepts with the `impeccable` skill, each showing the main window (tree with folders, tabs, an SSH terminal, properties panel) and the vault unlock dialog, in light and dark
- [x] Publish all concepts together as one Claude artifact with a switcher between them
- [x] Owner picks a concept (or combines parts); iterate on the chosen direction
- [x] Record the direction in `DESIGN.md` (principles, typography, colour, spacing, components, terminal colour scheme)
- [x] Design tokens as CSS variables (light and dark) in `ui/`, applied to the empty shell
- [x] Decision entry in `Docs/DECISIONS.md` naming the chosen concept and why
- [x] App icon derived from the chosen direction (source SVG committed, icon set generated with `pnpm tauri icon`), replacing the M0 placeholder

**Acceptance criteria**
- [x] The artifact shows at least three concepts, switchable, each in light and dark – https://claude.ai/artifact/9BhGCKAfNbHvbsUUQQwhas (private to the owner; source `Docs/design/m01-concepts.html`) shows Fernverkehr, Ulm, Gate and Workbench plus Ulm, revised, switchable by concept, Light/Dark and Main window/Unlock/Unlock error (plus an App icon view). Every combination rendered with headless Edge during Task 4 and the revisions, palettes pass the WCAG contrast script in both themes; the owner worked with it for the feedback round on 2026-11-15
- [x] Owner has approved the direction (recorded in `Docs/DECISIONS.md`) – `Docs/DECISIONS.md` entry "Visual direction: "Ulm, revised"": owner approval on 2026-11-15 ("ulm revised is approved") after one feedback round; contract in `.impeccable/surfaces/ui-src-app-svelte.md`
- [x] `DESIGN.md` exists and the shell uses the token file; build, test and lint are green – `DESIGN.md` and `.impeccable/design.json` written by impeccable's documenter after the finish review (fix batch, all 8 findings resolved). `ui/src/main.ts` imports `styles/tokens.css` and `styles/base.css`; the built `ui/dist/assets/index-*.css` contains `--color-bg`; owner checked `pnpm tauri dev` on Windows 11 in light and dark mode, DevTools console without CSP or font errors. Gate 2026-11-15 on Windows: `pnpm install --frozen-lockfile`, `pnpm ui:build`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (3 config tests + 1 core test), `pnpm lint` (0 errors, 0 warnings), `pnpm test` (8 tests), `cargo deny check` (ok), `dotnet run tools/roadmap.cs -- check` (valid) all green. OPEN: visual check of the shell and icon on macOS and Linux (`pnpm tauri dev`), no machine available yet

## Milestone 2 – Connection tree & data model
**Status:** 🔵 in progress · **Plan:** [2026-11-15-milestone-02-connection-tree.md](superpowers/plans/2026-11-15-milestone-02-connection-tree.md) · **Prerequisites:** M1

Goal: Connections can be organised in a persistent folder tree with mRemoteNG-style inheritance, without connecting anywhere yet.

**Tasks**
- [ ] `fernjo-core`: tree of folders and connections (UUIDs, manual order), inheritable settings, effective-value resolution with built-in defaults
- [ ] Persistence in `connections.json` (`schemaVersion`, atomic write, `.bak`) in the data folder from the OS config directory
- [ ] Tauri commands for creating, editing, moving and deleting nodes
- [ ] Sidebar: tree with filter (name, host, description), expand/collapse, drag and drop, context menu (new folder/connection, duplicate, rename, delete)
- [ ] Properties panel: edit fields, show "inherited from <folder>", reset to inherit
- [ ] Single instance: a second launch focuses the running window

**Acceptance criteria**
- [ ] Unit tests for inheritance (multi-level, override, reset) and persistence (round trip, atomic write, `.bak`) are green
- [ ] Manual: nested folders and connections created, moved by drag and drop, state preserved after restart
- [ ] Manual: filtering a generated tree of 500 connections shows no visible lag
- [ ] Build, test and lint are green

## Milestone 3 – Credential vault
**Status:** ⬜ open · **Plan:** – · **Prerequisites:** M2

Goal: Passwords and SSH keys are stored encrypted, unlocked by a master password or the OS keychain, and attachable to connections and folders.

**Tasks**
- [ ] `fernjo-vault`: file format with wrapped random data key, Argon2id, XChaCha20-Poly1305, atomic writes; format documented
- [ ] Entries (password, private key with optional passphrase, username, domain, notes) with `secrecy`/`zeroize`
- [ ] Master password on first run, unlock dialog, lock action, change master password (re-wrap only)
- [ ] "Unlock automatically on this device" via the OS keychain (`keyring`)
- [ ] Credential reference as inheritable setting in the properties panel
- [ ] Vault UI: list, create, edit, explicit reveal, delete with usage warning
- [ ] Data folder selectable in settings; warning for sync-conflict copies

**Acceptance criteria**
- [ ] Tests: round trip, wrong password rejected, tampered file rejected, master password change keeps entries and the keychain key valid
- [ ] Test: serialised `connections.json` and IPC responses to the UI contain no secret values
- [ ] Manual: auto-unlock via keychain works on Windows (macOS/Linux checked when available, otherwise marked open)
- [ ] Build, test and lint are green

## Milestone 4 – SSH sessions in tabs
**Status:** ⬜ open · **Plan:** – · **Prerequisites:** M3

Goal: Any SSH connection in the tree opens as a terminal tab with stored credentials and verified host keys.

**Tasks**
- [ ] `fernjo-ssh`: russh client with PTY (`xterm-256color`), resize, keepalive
- [ ] Auth in OpenSSH order: agent → key → password → keyboard-interactive → prompt
- [ ] Host keys: own `known_hosts`, first-connect fingerprint dialog, blocking warning on mismatch
- [ ] Tab area: open or focus, second session, states (connecting/connected/disconnected), reconnect, reorder, close
- [ ] xterm.js terminal with fit, WebGL, search and web-links addons; copy/paste; font and scrollback settings
- [ ] App shortcuts that do not steal terminal keys, documented in the README
- [ ] SSH integration tests against an OpenSSH server container in CI

**Acceptance criteria**
- [ ] Integration tests (password, key, keyboard-interactive, host key mismatch) are green in CI
- [ ] Manual: connect to a real host with each auth method; `vim` and `htop` render correctly; resize works; 10 tabs open at once
- [ ] Build, test and lint are green

## Milestone 5 – mRemoteNG import & export
**Status:** ⬜ open · **Plan:** – · **Prerequisites:** M3

Goal: The owner's existing mRemoteNG connection file, passwords included, moves into fernjo in one step, and fernjo data can be backed up and restored.

**Tasks**
- [ ] Owner provides a sanitised `confCons.xml` as a test fixture
- [ ] Parser for `confCons.xml` (current AES-GCM format, full-file encryption, default key `mR3m` and custom key)
- [ ] Mapping: containers → folders, `SSH2`/`RDP` → connections, inheritance flags; other protocols skipped and reported
- [ ] Passwords → vault entries, identical (username, domain, password) combinations shared
- [ ] Import UI: pick file, custom key prompt, import into "Imported from mRemoteNG (YYYY-MM-DD)", report dialog
- [ ] Export connections as fernjo JSON; full backup (connections + vault) encrypted with an export password; restore both
- [ ] Legacy CBC format only if cheap, otherwise stays in the backlog

**Acceptance criteria**
- [ ] Import tests against the owner's sanitised fixture and synthetic fixtures (default key, custom key, full-file encryption) are green
- [ ] Manual: the owner's real file imports; five spot-checked connections connect with their imported passwords
- [ ] Test: backup → restore round trip yields identical tree and vault
- [ ] Build, test and lint are green

## Milestone 6 – Packaging & app settings
**Status:** ⬜ open · **Plan:** – · **Prerequisites:** M4, M5

Goal: fernjo installs from a GitHub release on Windows, macOS and Linux and can replace mRemoteNG for daily SSH work.

**Tasks**
- [ ] Settings dialog: theme (light/dark/system), terminal font and size, scrollback, data folder
- [ ] Tauri bundler: Windows NSIS, macOS `.dmg` (Apple Silicon and Intel), Linux AppImage and `.deb` (minimum Ubuntu 22.04)
- [ ] Release workflow on GitHub-hosted Windows, macOS and Ubuntu runners attaches the installers to the GitHub release
- [ ] README: installation and first-launch workarounds for unsigned builds
- [ ] Measure the performance targets from the spec (§6)

**Acceptance criteria**
- [ ] A release workflow run produces all installers
- [ ] Manual: installers install and start on Windows, macOS and Linux (Wayland); unavailable machines marked open
- [ ] Cold start under 2 s and idle memory under ~150 MB measured
- [ ] Build, test and lint are green

## Release `v0.1.0` – Acceptance

First installable version: connection tree, vault, SSH tabs and mRemoteNG import on Windows, macOS and Linux.

## Milestone 7 – RDP sessions
**Status:** ⬜ open · **Plan:** – · **Prerequisites:** M6

Goal: RDP connections in the tree open as tabs next to SSH sessions, smoothly enough for daily admin work.

**Tasks**
- [ ] Throwaway spike: IronRDP → framebuffer → Tauri channel → canvas at 1920×1080 on all three OSes, measured; result in `Docs/DECISIONS.md`
- [ ] `fernjo-rdp`: IronRDP client with TLS + NLA (CredSSP), credentials and domain from the vault
- [ ] RDP tab: fit to tab with dynamic resize (fixed size as fallback), keyboard and mouse input, text clipboard both ways, fullscreen
- [ ] RDP settings (domain, resolution/scaling, colour depth, clipboard) as inheritable settings
- [ ] If the bar is missed after optimising the canvas path: record a scope change and insert a native-surface milestone before v0.2.0

**Acceptance criteria**
- [ ] Spike measured ≥ 30 frames/s while scrolling and dragging; owner judges keystroke latency acceptable
- [ ] Manual: connect to Windows 11 / Windows Server with NLA; clipboard both ways, resize and fullscreen work
- [ ] Build, test and lint are green

## Release `v0.2.0` – Acceptance

RDP sessions in tabs next to SSH.

## Backlog

Deliberately not scheduled. Ideas that `/ideen` files here are listed; if needed, one becomes a milestone.

- **Built-in device sync:** End-to-end-encrypted sync of connections and vault between devices. Until then the data folder can be synced with Syncthing/Nextcloud/iCloud.
- **SFTP file browser:** Browse and transfer files over an open SSH connection.
- **Port forwarding and jump hosts:** Local/remote forwards and ProxyJump per connection or folder.
- **Local shell tabs:** Open a local terminal as a tab.
- **Split panes and detached tabs:** Show several sessions side by side; pop a tab out into its own window.
- **Quick connect:** Type `user@host` and connect without saving a connection.
- **Broadcast input:** Send keystrokes to several SSH tabs at once.
- **Export to mRemoteNG format:** Write `confCons.xml` for migration back or to other tools.
- **Legacy mRemoteNG format:** Import of files from mRemoteNG ≤ 1.75 (AES-CBC), if not already done in M5.
- **Vault auto-lock:** Lock the vault after a configurable idle time.
- **Pageant support:** Use PuTTY's Pageant as SSH agent on Windows.
- **More protocols:** VNC, Telnet, serial.
- **Code signing, notarisation and auto-update:** Signed installers for Windows and macOS, update channel.
