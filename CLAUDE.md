# fernjo

Cross-platform (Windows, macOS, Linux) SSH & RDP connection manager in the spirit of mRemoteNG: connection tree in a sidebar, sessions as tabs in one window, encrypted credential vault, mRemoteNG import. Built primarily for the owner's daily use; open source.

- **Stack:** Rust (edition 2024, Cargo workspace) · Tauri 2 · Svelte 5 + TypeScript (Vite, pnpm) · xterm.js · russh (SSH) · IronRDP (RDP)
- **Language:** Docs, commits, code and comments in English · UI texts English
- **Build:** `cargo build --workspace` · app: `pnpm tauri build` (dev: `pnpm tauri dev`)
- **Test:** `cargo test --workspace` · `pnpm test`
- **Format/lint:** `cargo fmt --all --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `pnpm lint` (ESLint, Prettier, `svelte-check`) · `cargo deny check`
- **Spec:** [Docs/superpowers/specs/2026-11-15-fernjo-design.md](Docs/superpowers/specs/2026-11-15-fernjo-design.md)
- **Licence:** MIT OR Apache-2.0
- **Roadmap:** [Docs/ROADMAP.md](Docs/ROADMAP.md) (generated, source [Docs/roadmap.json](Docs/roadmap.json), see "Maintaining the roadmap")
- **Decisions:** [Docs/DECISIONS.md](Docs/DECISIONS.md)
- **Specs:** `Docs/superpowers/specs/` · **Plans:** `Docs/superpowers/plans/`

## Project-specific

**Architecture** (details in the spec, §4–5)
- `crates/fernjo-core` (model, tree, inheritance, persistence), `crates/fernjo-vault`, `crates/fernjo-ssh`, `crates/fernjo-rdp`, `crates/fernjo-import`, `src-tauri/` (Tauri shell, commands, session manager), `ui/` (Svelte frontend). Each crate is created in the milestone that needs it, not earlier.
- Protocol, vault and import crates never depend on Tauri; they are testable without the UI.
- Frontend ↔ Rust: Tauri commands for requests, Tauri channels for streaming session data.
- Plain `cargo build/clippy/test --workspace` compiles Tauri as a dev build, which does not embed `ui/dist` (`devUrl` is set), so cargo needs no UI build first. Only `pnpm tauri build` embeds `ui/dist`, and it runs `pnpm ui:build` itself via `beforeBuildCommand`.
- UI styling uses only the tokens in `ui/src/styles/tokens.css` and follows `DESIGN.md`. Shipped CSS stays hex-only and free of `oklch()`, `color-mix()`, `light-dark()`, nesting, `@layer`, `:has()` and container queries (WebKitGTK 4.1 / `safari13` target); `ui/src/styles/tokens.test.ts` guards this and the contrast pairs. A new colour token goes into the light block and both dark blocks.
- Vite must not inline assets (`assetsInlineLimit: 0` in `ui/vite.config.ts`): the CSP only allows fonts from `'self'`, so an inlined `data:` font would be blocked.
- On Windows, stopping a background `pnpm tauri dev` can leave the Vite server (port 1420) and `fernjo.exe` running; the next `pnpm tauri dev` then fails with "Port 1420 is already in use". Check with `Get-NetTCPConnection -LocalPort 1420 -State Listen` and `Get-Process fernjo`, and stop the leftovers.
- Data folder: `<OS config dir>/fernjo/` (`%APPDATA%\fernjo` on Windows) holds `connections.json` (+ `.bak`); the environment variable `FERNJO_DATA_DIR` overrides it. Use it for any manual run that must not touch the real data. A deterministic sample tree for filter and restart checks: `cargo run -p fernjo-core --example generate_tree -- <dir> 500`, then `FERNJO_DATA_DIR=<dir> pnpm tauri dev`. Format and load rules: `Docs/DECISIONS.md` ("`connections.json`: format, location, write strategy").
- `tauri_plugin_single_instance::init` must stay the first `.plugin(...)` call in `src-tauri/src/lib.rs` (a second launch has to be intercepted before any other plugin initialises; note that `AppState::open_for_app` runs while the builder is assembled, so a second launch also opens the data before it exits; besides reading the data files it reads the keychain for auto-unlock, which never deletes the item, and, on macOS, may trigger a keychain prompt).
- Vault state: `AppState.vault` is `VaultState::None` (no `vault.fernjo`), `Open(Vault)` (usable; "locked" is `!vault.is_unlocked()`, there is no separate Locked variant) or `Unusable` (file present but unreadable or from a newer version; never overwritten). `AppState::vault_status()` is the single reader. `AppState.auto_unlock_issue` (set from the startup auto-unlock, cleared by a successful unlock or a new vault) is reported as `autoUnlockIssue` in `vault_status` (`key_not_accepted`, `keychain_unavailable`, `unavailable`, `rolled_back`; the stored key is never deleted and the UI shows a note); `AppState.generations` (`Arc<dyn GenerationStore>`: `FileGenerationStore` in `device_dir`, memory without one; tests get memory, `open_with_stores` shares one across a "restart") is attached to the vault before the startup auto-unlock and to a newly created vault; `AppState::open` is the test constructor (in-memory key store, `KdfParams::TEST`, no `settings.json`); `AppState::open_for_app` is the production one (`OsKeyStore`, default KDF cost) and is used by the real app even with `FERNJO_DATA_DIR` set, so a manual run with "Unlock automatically on this device" ticked writes a real Credential Manager/Keychain/Secret Service item (untick it, or delete the `fernjo` item, afterwards).
- `FERNJO_DATA_DIR` also disables `settings.json` (neither read nor written) and the data-folder dialog's Choose button; use it for every manual run. To test the data-folder dialog itself, run once without it and with `XDG_CONFIG_HOME` / `HOME` pointed at a scratch folder (Linux/macOS). On Windows overriding `APPDATA` does not work (the `directories` crate asks the Known Folder API), so the run uses the real `%APPDATA%\fernjo`; do it only on a machine where writing `settings.json` there is acceptable and delete it afterwards. `AppState.settings_unusable` (unreadable or newer-schema `settings.json`: never saved over) and `AppState.pending_data_dir` (folder chosen while a restart is pending, plus the data files that already existed there) belong to that mechanism; format and precedence in `Docs/DECISIONS.md` ("`settings.json` and data folder precedence").
- No-secret guard: the test `no_response_contains_a_secret` in `src-tauri/src/vault_commands.rs` runs every entry of its `table()` with sentinel secrets and asserts that no sentinel appears in any response, error, `Debug` output or `connections.json` (reveal returns the secret by design). `command_table_covers_every_tauri_command` is the count guard next to it. Adding a `#[tauri::command]` (also the `#[tauri::command(async)]` form) means updating three places: the function plus its `handlers` wrapper in `commands.rs`, `vault_commands.rs` or `data_folder.rs`, its entry in the test's `table()` (or `APP_HANDLE_EXEMPT` with a reason when it needs an `AppHandle`), and the `generate_handler!` list in `lib.rs`; the count guard compares them and fails if one is missing. A new source file with commands must also be added to the test's `sources` (`include_str!`). `table()` may also call plain functions that are deliberately not commands (`NOT_A_COMMAND`, currently `apply_data_folder`, `import_begin`, `export_connections_to` and `export_backup_to`, which take a path and so must never be reachable from the webview); the count guard asserts they are not registered. Commands that run an Argon2 derivation (`vault_create`, `vault_unlock`, `vault_change_password`, `import_inspect`) are `#[tauri::command(async)]` so they do not block the main thread; `export_backup` (Argon2 too, plus the save dialog) and `import_apply` (many writes, no Argon2) are plain `#[tauri::command]`s on `async fn`s that move the work into `spawn_blocking` and take the state from their `AppHandle` (`app.state::<AppState>()`); `import_apply` still has its `table()` entry through its plain function.
- Vault save guard: `Vault::save` refuses with `changed_on_disk` when `vault.fernjo` no longer starts with the header this handle last read or wrote (another device or a sync tool changed it); a missing file is recreated. It is a check-then-write, not a lock. Lock and unlock to load the other version. Rollback protection: the body carries `generation` (v1 files read as 0, the header version stays 1; golden fixture `crates/fernjo-vault/tests/fixtures/v1-no-generation.fernjo` must keep opening), every save writes `max(generation, floor) + 1`, and `unlock`/`unlock_with_key` refuse a lower generation than the device mark (`vault-marks.json` in the device folder) as `rolled_back` only after the password or key proved genuine; `vault_unlock_older` (`unlock_accepting_older`) opens it anyway and re-stamps at once; auto-unlock never accepts. Details: `Docs/vault-format.md`, `Docs/DECISIONS.md` ("Vault: rollback protection"). Every read of the vault file goes through `read_capped` with `fernjo_vault::MAX_FILE_LEN` (64 MiB); a larger file is `unreadable`, so the vault is `Unusable` and untouched. Details: `Docs/vault-format.md`.
- Credential updates: `CredentialUpdate` with a new `privateKey` and no `passphrase` keeps the stored passphrase; `passphrase` replaces it and `clearPassphrase: true` removes it (the UI sends it only from the "Remove stored passphrase" checkbox, never together with a typed passphrase).
- Tauri commands are plain functions over `AppState` in `src-tauri/src/commands.rs` (unit-tested without Tauri) with thin `#[tauri::command]` wrappers in `commands::handlers`; every mutating command returns the full snapshot. The UI (`ui/src/lib/store.svelte.ts`) keeps only view state and never derives validity or inherited values.
- Checking the UI in the real app on Windows without touching the user's mouse or keyboard: start the app with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` (plus `FERNJO_DATA_DIR`), then talk to `http://127.0.0.1:9222/json` over the Chrome DevTools Protocol (Node 22+ has a global `WebSocket`): `Runtime.evaluate` for DOM and layout measurements, `Input.dispatchMouseEvent`/`dispatchKeyEvent` for browser-level input with real hit-testing, `Page.captureScreenshot`, `Emulation.setEmulatedMedia` for light/dark, console and network events. Scroll a row into view before clicking it (a row hidden under the properties panel swallows nothing and the click lands on the panel). Do not use `SendKeys`, `SetCursorPos` or `mouse_event` on the user's desktop. Gotchas: the dev server does not apply the CSP (check it in a `pnpm tauri build --no-bundle` binary, `target/release/fernjo.exe`, page URL `http://tauri.localhost/`), code run through DevTools is not bound by the page's CSP (`eval` tests prove nothing), and a forced `Stop-Process` can lose not-yet-flushed `localStorage` (close the window instead when testing persistence of view state).
- Session manager (M4) lives in `src-tauri/`: `sessions.rs` (the manager, one `Entry` per tab session, prompts, output forwarder; the wire contract is the doc comment at the top of that file, rationale in `Docs/DECISIONS.md` "M4 session IPC"), `session_flow.rs` (ack-based flow control), `session_commands.rs` (the `#[tauri::command]` wrappers; they count in the three-places rule above) and `connect_plan.rs` (resolves a node plus vault into a `ConnectPlan` synchronously, so the connect task needs no `AppState`). `SessionManager::close_all` runs on every main-window page load (`on_page_load` in `lib.rs`), so a reload or a crashed page never leaves sessions running without a UI. The SSH client itself is `crates/fernjo-ssh` (no Tauri). The known hosts file is `<data folder>/known_hosts`. A key found there is trusted silently only when this device confirmed it too: `ConnectParams.confirmed_hosts` is `<device_dir>/known_hosts.confirmed` (`sessions.rs` sets it from `AppState.device_dir`; without a device folder `confirmed_in_memory`, a `KnownHosts::in_memory()` held by the `SessionManager`, keeps the answers for the run; `None` for both disables the layer, as the SSH integration tests do). Otherwise the handshake stops before authentication with `PromptView::HostKeySynced` (UI `HostKeySyncedPrompt.svelte`, answered with `PromptAnswer::HostKey`); no seeding, so every host asks once per device. Design: `Docs/DECISIONS.md` "M6 security fixes: hosts confirmed on this device". `fernjo_ssh::validate_host` (an allow-list: DNS-style names, IPv4, bracket-less IPv6 with optional `%zone`, at most 255 bytes) runs inside `host_port`, so every `KnownHosts` call, `Session::connect` and `connect_plan::resolve` refuse other hosts with `invalid_host`; a new code path that builds a `known_hosts` name must go through `host_port`.
- SSH integration tests (`crates/fernjo-ssh/tests/`) need the throwaway OpenSSH container: `eval "$(tools/ssh-test-server.sh start)"`, run the tests, always `tools/ssh-test-server.sh stop` afterwards (it removes the container, the temp key directory and the test agent). The script prints the `FERNJO_TEST_SSH_*` variables the tests read (`ADDR`, `USER`, `PASSWORD`, `KEY_ED25519`, `KEY_RSA`, `KEY_ENCRYPTED`, `KEY_PASSPHRASE`, `KEY_UNAUTHORISED`, and `AGENT` plus `SSH_AUTH_SOCK` where an agent could be started). Rule in `tests/support`: without `FERNJO_TEST_SSH_ADDR` (or `FERNJO_TEST_SSH_AGENT` for the agent test) the tests skip locally with a message and **fail when `CI` is set**, so a missing server cannot turn CI green. `tests/support` removes `SSH_AUTH_SOCK` once per process so a developer's agent never answers for the other tests; `tests/agent.rs` is its own test binary because it needs the agent. On Windows (Git Bash) the script cannot start an agent (the Rust client talks to the OpenSSH named pipe there), so the agent test skips; the script needs `docker` and `ssh-keygen` on `PATH`.
- Terminal and shortcuts (UI): `ui/src/lib/terminal/` (xterm.js controller, theme from the `--term-*` tokens, search bar) and `ui/src/lib/shortcuts.ts` (`routeKey` is the one place that maps a key press to an app action; plain Ctrl+letter and Alt+letter always belong to the shell). Clipboard and WebGL decisions: `Docs/DECISIONS.md` "M4 clipboard path" and "M4 terminal". Checking a session in the real app uses the DevTools-port recipe above with a container from `tools/ssh-test-server.sh`.
- Import, export and restore (M5): `crates/fernjo-import` (no Tauri, no I/O on app data: `mremoteng/` reads `confCons.xml` 2.6 and maps it to an `ImportPlan`, `connections.rs` is the JSON export/import, `backup.rs` the `FJBK` backup on `fernjo_vault::seal_with_password`, `report.rs` the capped report). `src-tauri/src/import_commands.rs` applies plans. One file waits in `AppState.pending` (`Pending`: picked path, plan or decrypted backup; secrets stay in Rust); it is cleared by apply, `import_cancel`, a new `import_begin` and every main-window page load (`clear_pending` in `lib.rs`). Plan-then-apply order and the `import_incomplete`/`restore_incomplete` errors: `Docs/DECISIONS.md` "M5 restore semantics and backup container"; formats: `Docs/backup-format.md`. Lock order is pending, then vault, then store.
- M5 commands: `import_pick`, `import_inspect`, `import_apply`, `import_cancel`, `export_connections` and `export_backup` (wrappers in `import_commands::handlers`, which also own the native dialogs; `import_pick`, `export_connections` and `export_backup` take an `AppHandle` and are in `APP_HANDLE_EXEMPT`). `import_commands.rs` is in the `sources` of `command_table_covers_every_tauri_command` (the count guard).
- Limits and budgets (M5 review fixes): `fernjo_core::MAX_TREE_DEPTH` (60) is enforced on every path that adds or moves a node (`Tree::insert` behind graft, create and duplicate, `Tree::move_node` for the whole moved subtree, `Store::replace_tree`; `too_deep`), and `Store::mutate` never saves a deeper tree, so `connections.json` and its `.bak` stay loadable. The mRemoteNG reader takes 59 container levels and at most `MAX_PBKDF2_ITERATIONS` (20 000 000) in all, and an export may not target the data folder (`export_target_not_allowed`). A byte scan (`mremoteng/scan.rs`) runs before `roxmltree` and refuses a tag with more than 200 attributes or a file with more than 8 `xmlns` names (`too_complex`, because the parser is superlinear in both); attribute strings are cut at read time (`MAX_*_CHARS` in `mremoteng/file.rs`: names, users, domains 256, description 4096, protocol 64, host 256 and dropped when longer, password 16 KiB) and the mapper never copies a name per node (`display`, `Rc<str>`), so keep new report text bounded the same way. Decisions and numbers: `Docs/DECISIONS.md` "M5 final review fixes" and "M6 security review: import limits". The dev profile optimises `fernjo-import` and `sha1` (root `Cargo.toml`), otherwise PBKDF2 in a debug build takes minutes.
- `FERNJO_DIALOG_FILE` (debug builds only; release builds do not read it) replaces the native pick and save dialogs of the import and export commands with that path, one path serves both; it lets a DevTools-driven check run the flows without OS input. Use it together with `FERNJO_DATA_DIR`.
- mRemoteNG test fixtures: `crates/fernjo-import/tests/fixtures/` (`vectors.json`, `default-key.xml`, `custom-key.xml`, `full-file.xml` with their `.expected.json`, plus hand-written error files) come from the independent .NET script `tools/mremoteng-vectors.cs` (BouncyCastle, needs the .NET 10 SDK; CI never runs it). Regenerate with `dotnet run tools/mremoteng-vectors.cs -- crates/fernjo-import/tests/fixtures`; it rewrites `vectors.json` and all generated fixture XMLs with fresh random salts, so undo unwanted churn with `git checkout -- crates/fernjo-import/tests/fixtures` (only commit regenerated fixtures when the script itself changed). mRemoteNG is GPL: its source is read to learn the format and never copied, fixtures included.
- Import UI: `ImportStore` in `ui/src/lib/import.svelte.ts` holds only view state (step, counts, report lines, the file name) and ignores a new action while `busy` is set (re-entrancy and stale-result guard); `ui/src/lib/components/ImportDialog.svelte` is the "Import & backup" dialog (with `ImportReport.svelte`), opened from the sidebar footer and from the empty tree's import offer.
- Theme (M6): `AppSettings.theme` (`Option<Theme>`: `System`, `Light`, `Dark`; unset or an unknown string means `System`) lives in `settings.json` (`crates/fernjo-core/src/settings.rs`); `src-tauri/src/app_settings.rs` has the `get_theme`/`set_theme` commands (they count in the three-places rule) and `AppState.theme`; `set_theme` keeps the other settings fields. The UI (`ui/src/lib/theme.svelte.ts`) sets `data-theme` on `<html>` and mirrors the choice to `localStorage` so `main.ts` can apply it as soon as its script runs; the Rust value wins after load. Under `FERNJO_DATA_DIR` the choice is memory-only and the mirror is not written (a scratch run must not overwrite the real run's mirror).
- Version source: the version lives in `[workspace.package]` of the root `Cargo.toml` and in `package.json`, nowhere else (`tauri.conf.json` has no `version` key, installer names come from the Cargo version). It stays `0.0.0` until the release procedure bumps both; `tools/check-release-version.sh` fails the release build when tag, `Cargo.toml` and `package.json` disagree.
- Release workflow (M6, `.github/workflows/release.yml`): job `build` (matrix: Windows NSIS, macOS dmg for aarch64 and x86_64, Ubuntu 22.04 AppImage and deb) has only `contents: read`, installs the pinned `cargo-about`, runs `node tools/third-party-notices.mjs src-tauri/resources/THIRD-PARTY-NOTICES.md`, `pnpm tauri build --bundles ...`, smoke-tests the installer (`tools/smoke-installed.sh`, `tools/smoke-installed.ps1`; both refuse to run outside GitHub Actions) and uploads artifacts. Job `publish` is the only one with `contents: write`: it runs `tools/verify-installers.sh`, writes `SHA256SUMS` and attaches everything with `gh release upload`. Triggers: a published release; `workflow_dispatch` with a `tag` (re-attach to an existing release; the file must be on the default branch, which is `dev`, so the manual trigger only works from `dev`); a `pull_request` touching the packaging inputs is a dry run (artifacts only, no publish). Tag, release text and inputs reach scripts only through `env:`. The committed `src-tauri/resources/THIRD-PARTY-NOTICES.md` is a placeholder (tauri-build needs the resource at compile time); a local run of the notices script overwrites it, restore with `git checkout -- src-tauri/resources/THIRD-PARTY-NOTICES.md` and never commit the output. Rationale: `Docs/DECISIONS.md` ("M6 release workflow: plain `pnpm tauri build`, split permissions, dry run on pull requests" and "M6 installer smoke tests: what they prove and where they run").
- Release scripts (M6, bash, `tools/`): `check-workflows.sh` (policy for workflows: pinned SHAs, the word `self-hosted` absent, no `pull_request_target`, a top-level `permissions:` block present (not what it grants), only safe `${{ }}` values inside `run:`; job `workflows` in `ci.yml` runs it), `check-release-version.sh <tag>`, `verify-installers.sh <version> <dir>` (the one place that lists the five expected installer names) and `test-release-scripts.sh` (tests of these scripts and of the notices generator; run `bash tools/test-release-scripts.sh`, CI runs it too). A change to an installer name goes into `verify-installers.sh`, its test and `release.yml` together.
- Performance: `tools/measure-performance.ps1 -Exe target/release/fernjo.exe -Runs 10` (Windows only, helper `tools/perf/cdp.mjs`, Node 22+) measures cold start, idle memory and the 500-connection filter on a scratch `FERNJO_DATA_DIR` over the WebView2 DevTools port and stops every process it starts. Definitions and the recorded results are in `Docs/performance.md`; re-measure with the same definitions, not another probe, when comparing.
- `deny.toml` carries the advisory ignore list with a reason per entry; run `cargo deny check` after dependency updates and drop ignores it reports as unused.

**Rules that must not be broken**
- **Secrets stay in Rust.** Passwords, private keys and passphrases are never sent to the webview (only IDs and names), never written to `connections.json`, logs or webview storage. The only exception is the explicit "reveal" action in the vault UI. Secrets in memory use `secrecy`/`zeroize`.
- No home-made crypto: Argon2id + XChaCha20-Poly1305 from RustCrypto crates; vault uses a wrapped random data key.
- All logic (model, vault, protocols, import) is Rust; TypeScript is for presentation only.
- Files in the data folder are written atomically (temp file + rename) and keep a `.bak`.
- The visual direction is fixed in M1 (`DESIGN.md` + CSS tokens in `ui/`); UI work follows it instead of inventing new styles.

**CI and repo**
- The repo is public (from M0). CI runs only on GitHub-hosted runners; **never use the org's self-hosted runners** in this repo's workflows, because fork PRs could execute code on them.
- Actions stay pinned to commit SHAs, workflows default to `permissions: contents: read`.
- Linux target: Wayland on recent GNOME/KDE distros is tested; minimum Ubuntu 22.04 (WebKitGTK 4.1).

---

## Maintaining the roadmap

The source of truth is `Docs/roadmap.json` (schema: `Docs/roadmap.schema.json`). `Docs/ROADMAP.md` is generated from it and never edited by hand. The .NET 10 SDK is required, even if the project otherwise uses a different stack.

1. Edit only `Docs/roadmap.json`: status, plan link, tick off tasks and criteria (`"done": true`, proof in `"evidence"`), scope changes, backlog. Omit optional fields, never set them to `null`.
2. Then run `dotnet run tools/roadmap.cs -- generate` and commit both files. Errors name the path in the JSON (for example `subprojects[0].milestones[2].completedOn`).
3. `dotnet run tools/roadmap.cs -- check` validates the schema, references and whether `ROADMAP.md` is up to date; CI (job `roadmap`) does the same.
4. Rules: `status` is `open`, `planned`, `in-progress` or `done`; `done` requires `completedOn` (YYYY-MM-DD), from `planned` on `plan` is required (path relative to `Docs/`); milestone ids are unique strings (`"5.1"` is allowed); prerequisites and `afterMilestone` must exist; title, goal and name stay on one line; texts are Markdown; anything that fits nowhere else goes into an entry in `blocks` (`title`, `text`).
5. A new milestone is an entry in `subprojects[].milestones`:

```json
{
  "id": "1", "title": "<Title>", "status": "open",
  "prerequisites": ["0"],
  "goal": "<One sentence: What becomes possible after this milestone?>",
  "tasks": [ { "text": "…", "done": false } ],
  "acceptance": [ { "text": "… (verifiable: test, command or a concrete manual check)", "done": false } ]
}
```

   A new subproject is an entry in `subprojects` (`number`, `name`, `spec` with `text` and/or `link`, `milestones`), a release is an entry in `releases` (`version`, `title`, `status`, `afterMilestone`, optionally `covers`, `date`, `intro`, `acceptance`, `blocks`).
6. Ideas belong in `backlog` (`title`, `text`, optionally `idea`). The skill `/ideen` (from `benjo-code/Labjo`) enters adopted ideas there and regenerates `Docs/ROADMAP.md`.

## Milestone workflow

Every change "in the roadmap" (linking a plan, setting status, ticking off tasks, scope changes) is a change to `Docs/roadmap.json` plus a regenerated `Docs/ROADMAP.md`, see "Maintaining the roadmap".

**"Plan milestone N":**
1. Create branch `feature/mNN-<topic>` from the current `dev`. Plan, roadmap maintenance and implementation of the milestone all go through this branch.
2. Read `Docs/ROADMAP.md`, the subproject's spec and the current state of the code.
3. Review the project's open ideas with `/ideen` (skill from `benjo-code/Labjo`, installation: `bash tools/install-ideen-skill.sh` there): adopt, implement or discard them. Whatever belongs in the milestone flows into the plan. The roadmap change lives on the milestone branch (when asked, choose "current branch").
4. If the milestone starts with "Create spec" or the subproject has no spec yet: first develop the spec through brainstorming with the user and store it under `Docs/superpowers/specs/`.
5. Write the plan to `Docs/superpowers/plans/YYYY-MM-DD-milestone-NN-<topic>.md`. The plan covers exactly the milestone's tasks and acceptance criteria, nothing more.
6. In the roadmap, link the plan and set the status to `🟡 planned` (overview table and milestone).

**Implementation:**
- Status `🔵 in progress`. Tick off every finished task in the roadmap immediately.
- Tick off acceptance criteria only once they have actually been verified, and note the proof behind them (test name, command + result, or manual check).
- At the end, status `✅ done` with date in table and milestone, then PR to `dev` (see "Git & branches").
- Record scope changes in the roadmap under "Scope changes", not silently in the code.
- Work only within the scope of the active milestone. Do not anticipate features, packages or architecture decisions of later milestones.

## Git & branches

- `main` holds only published states (releases). `dev` is the integration branch.
- Always work on `feature/<topic>` or `fix/<topic>`, branched from the current `dev` (`git switch dev && git pull && git switch -c feature/<topic>`).
- Before any work, check `git status` and the current branch. If on `main` or `dev`, create a feature branch first. Never commit directly to `main` or `dev`.
- Feature and fix branches go to `dev` via PR. Claude merges these PRs itself once the work is done and all CI checks are green (squash merge allowed).
- PRs to `main` exist only for releases and hotfixes, and only on the user's explicit instruction.
- PRs between `main` and `dev` (release, hotfix, back-merge `main` → `dev`) are always merged with **"Create a merge commit"**, never squash or rebase. Otherwise `main` and `dev` lose their common history, and every release causes conflicts.
- Hotfix: branch `hotfix/<topic>` from `main` → PR to `main` → release (see below) → then merge `main` into `dev` via PR.
- Stage selectively (`git add <file>`). No `git add .` or `git add -A` when foreign changes that do not belong to the task are in the working tree. For files that were already modified beforehand, read `git diff <file>` first and use `git add -p` if needed.
- After every commit, check `git show --stat HEAD` against the expected file list.
- Commit messages follow Conventional Commits: `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`, `ci:`, optionally with a scope (`feat(api): …`).
- Forbidden without explicit instruction: force push, `git reset --hard`, deleting branches, rewriting history, bypassing hooks with `--no-verify`.
- Always create new repos (including companion ones such as `-Releases` or `-Privacy`) in the `benjo-code` organization, always with the owner spelled out (`gh repo create benjo-code/<name> …`), never under the personal account. Then verify with `gh repo view benjo-code/<name>`.

## Releases

- Versioning according to SemVer (`vMAJOR.MINOR.PATCH`).
- `CHANGELOG.md` → `[Unreleased]` is updated in every PR with a notable user-facing change (not for pure planning, roadmap or internal cleanup PRs).
- Procedure (only on instruction):
  1. Branch `release/vX.Y.Z` from `dev`, turn `[Unreleased]` in `CHANGELOG.md` into `[X.Y.Z] – YYYY-MM-DD` (naming the milestones it contains) and add a new, empty `[Unreleased]` above it.
  2. PR `release/vX.Y.Z` → `main` (merge commit).
  3. Tag and GitHub release in one step: `gh release create vX.Y.Z --target main --title "vX.Y.Z" --notes "<CHANGELOG section>"`.
  4. Merge `main` back into `dev` via PR (merge commit) so the two do not diverge.

## Project memory

- Document anything important for the future in the repo without being asked:
  - Architecture and technology decisions, fixed versions, rejected approaches → `Docs/DECISIONS.md`
  - Scope changes → `Docs/ROADMAP.md`, section "Scope changes"
  - Lasting project knowledge (architecture, conventions, workarounds, pitfalls) → this file, section "Project-specific"
- Do not record guesses, intermediate states or temporary diagnoses as facts.
- References name the file and the type, member or section name, never line numbers (they go stale).

## Quality & proof

- "Done" means build, test and format/lint (commands above) are demonstrably green. State the command and the result.
- `dotnet run tools/roadmap.cs -- check` is part of "done": the roadmap is valid and `Docs/ROADMAP.md` is up to date.
- Anything that could not be verified (e.g. because real hardware or credentials are missing) is explicitly marked as open, together with the command that verifies it.
- Do not suppress warnings, skip tests or weaken checks just to get something green.

## Security

- No secrets (tokens, passwords, keys, real credentials) in files, commits, logs, PRs or replies.
- Never quote or output values from `.env` or similar; name only variable names or placeholders.
- Secrets come from env vars or a secret store. Maintain a placeholder in `.env.example` for every required variable. `.env` is in `.gitignore`.

## Collaboration

- When requirements are unclear, ask instead of guessing.
- Destructive or outward-facing actions (merge to `main`, release, deleting branches, tags, releases or repos, changes to GitHub settings) only on explicit instruction. Feature branches deleted automatically after the merge are exempt.
