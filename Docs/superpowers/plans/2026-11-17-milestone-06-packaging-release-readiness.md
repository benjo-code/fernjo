# Milestone 6 – Packaging, app settings, release readiness and security check: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** fernjo installs from a GitHub release on Windows, macOS and Linux, has the theme setting the spec promises, meets its measured performance targets, and ends the milestone with a recorded security review and a go/no-go release-readiness report for `v0.1.0`.

**Architecture:** The theme becomes one more field of `settings.json` (Rust owns it, the UI only sets `data-theme`), shown in a "Settings" dialog that replaces `DataFolderDialog`. Packaging is the Tauri bundler driven by plain `pnpm tauri build` in one workflow (`release.yml`): build jobs hold `contents: read` and upload artifacts, one small `publish` job holds `contents: write` and attaches the installers plus `SHA256SUMS`. Small shell scripts in `tools/` carry the checks that would otherwise hide in YAML (version vs tag, expected installer set, workflow policy). The milestone closes with an audit task and a readiness task whose results are written to `Docs/release-checklist.md`.

**Tech Stack:** Rust (Tauri 2.12 bundler, `fernjo-core` settings), Svelte 5 + TypeScript, Vitest, GitHub Actions (hosted runners), bash, PowerShell (measurement script), `cargo-about`.

**Spec:** [Docs/superpowers/specs/2026-11-15-fernjo-design.md](../specs/2026-11-15-fernjo-design.md) §3 (theme), §5.4 (settings), §5.7 (packaging), §6 (performance targets), §7 (CI, Linux minimum). M6 has no spec of its own. Executors also read `CLAUDE.md` ("Project-specific"), `DESIGN.md` and the roadmap's "Scope changes".

**Additions to the roadmap text (owner request 2026-11-17, recorded as a scope change in `Docs/roadmap.json` while planning):** a release-readiness and security check at the end of the milestone (Tasks 9 and 10), third-party licence notices in the installers (Task 5, needed to ship MIT/Apache/OFL/MPL dependencies in binaries), and installer smoke tests in CI (Task 6). The four settings-dialog follow-ups parked in the backlog ("M4 review follow-ups") are fixed in Task 2 because it rebuilds that dialog.

## Global Constraints

- **Targets (spec §5.7):** Windows NSIS; macOS `.dmg` for Apple Silicon and Intel; Linux AppImage and `.deb`, minimum Ubuntu 22.04 (WebKitGTK 4.1). Builds are **unsigned**; no updater, no code signing, no notarisation (spec §2 non-goals).
- **CI (spec §7):** GitHub-hosted runners only, **never self-hosted**; every action pinned to a 40-character commit SHA with a version comment; workflows default to `permissions: contents: read`; no `pull_request_target`; untrusted values (tag names, release text, inputs) reach `run:` only through `env:`, never as `${{ … }}` inside the script.
- **Secrets stay in Rust.** The theme is not a secret, but nothing new may put a secret into an IPC response, log, `settings.json`, webview storage or an installer. Every new `#[tauri::command]` follows the `CLAUDE.md` three-places rule (function and `handlers` wrapper, entry in `table()` of `vault_commands.rs`, `generate_handler!` in `lib.rs`; a new source file with commands goes into the `sources` of `command_table_covers_every_tauri_command`).
- **UI:** only tokens from `ui/src/styles/tokens.css`, hex-only CSS, none of `oklch()`, `color-mix()`, `light-dark()`, nesting, `@layer`, `:has()`, container queries; UI text in English; TypeScript holds view state only. The theme works by setting `data-theme` on `<html>` (`Docs/DECISIONS.md`, "UI tokens, theming and bundled fonts").
- **Settings file rules (`Docs/DECISIONS.md`, "`settings.json` and data folder precedence"):** under `FERNJO_DATA_DIR`, with an unusable `settings.json` or without a config folder, values live in memory only (`persistent: false`); an unreadable or newer-schema file is never written over; the schema stays `schemaVersion: 1` (the new field is additive).
- **Licences:** `deny.toml` allow-list unchanged unless a task says so; dependency additions are recorded in `Docs/DECISIONS.md`.
- **Release limits of this milestone:** the milestone prepares `v0.1.0` and never tags, releases, merges to `main` or changes GitHub settings (`CLAUDE.md`, "Collaboration"). The version stays `0.0.0` until the release procedure bumps it.
- Work on `feature/m06-packaging-release-readiness`; Conventional Commits; stage files by name; `git show --stat HEAD` after every commit and compare with the task's file list; no attribution lines; never `--no-verify`. Docs, commits, code and comments in English.
- "Done" = the commands of Task 11 are green and shown with their results; anything not verifiable (macOS, Linux desktops, the owner's real hosts) is marked open with the command that verifies it.

## Review Focus

Failure modes the spec implies that no happy-path test covers, most likely first (each has a test or check in the named task):

1. `settings.json` edited by hand or written by a future build with `"theme": "banana"` or `"theme": 3`: the app must start, treat the theme as unset (system), keep `dataDir` and `terminal`, not refuse the file (Task 1).
2. First paint with a stored explicit theme: no flash of the wrong theme while the async load runs; "System" follows an OS switch live, and a stored value that is not a theme name is ignored (Task 2).
3. An installer upgrade, uninstall or reinstall must never touch the data folder or `vault.fernjo` (`%APPDATA%\fernjo` is outside the install and the Tauri app-data directory) (Task 6, Windows smoke).
4. A release run with a tag that does not match the version, a missing or extra installer, or a tag name with shell metacharacters must fail before anything is uploaded and never execute the text (Tasks 4 and 6).
5. The release binary must not honour the debug-only hooks (`FERNJO_DIALOG_FILE`), must not expose DevTools, and the installers must not embed the builder's home directory (Task 9).

---

### Task 1: Theme setting in `fernjo-core` and the Rust commands

**Files:**
- Modify: `crates/fernjo-core/src/settings.rs`, `crates/fernjo-core/src/lib.rs` (export `Theme`)
- Create: `src-tauri/src/app_settings.rs`
- Modify: `src-tauri/src/state.rs` (`AppState.theme`), `src-tauri/src/session_commands.rs` (use the shared save helper), `src-tauri/src/lib.rs` (`mod`, `generate_handler!`), `src-tauri/src/vault_commands.rs` (`table()` entries, `sources`)

**Interfaces:**
- Produces (core): `pub enum Theme { System, Light, Dark }` (`#[serde(rename_all = "lowercase")]`, `Default` = `System`, `Copy`); `AppSettings.theme: Option<Theme>`, skipped when `None`, read through `fn lenient_theme<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Theme>, D::Error>` that maps an unknown string or a wrong JSON type to `None` instead of an error.
- Produces (app): `pub struct ThemeInfo { pub theme: Theme, pub persistent: bool }` (camelCase), `pub fn get_theme(state: &AppState) -> Result<ThemeInfo, CommandError>`, `pub fn set_theme(state: &AppState, theme: Theme) -> Result<ThemeInfo, CommandError>`, `pub(crate) fn update_settings(state: &AppState, change: impl FnOnce(&mut AppSettings)) -> Result<(), CommandError>` (loads `settings.json`, applies `change`, saves; only when persistence is allowed; `settings_unusable` / `settings_write_failed` codes exactly as `set_terminal_settings` has them today). Wire commands `get_theme()` and `set_theme(theme)`.
- Consumes: `session_commands::persistent(&AppState) -> bool` (make it `pub(crate)`).

- [ ] **Step 1: Write the failing core tests** in `settings.rs`: `theme_round_trips_through_the_settings_file` (save `Theme::Dark`, load, equal), `unset_theme_is_not_written` (file value is `{"schemaVersion":1}`), `unknown_theme_value_loads_as_unset_and_keeps_other_fields` (`{"schemaVersion":1,"dataDir":"/x","theme":"banana"}` → `theme == None`, `data_dir == Some("/x")`), `theme_of_the_wrong_type_loads_as_unset` (`"theme": 3`, `"theme": null`, `"theme": {}`).
- [ ] **Step 2:** `cargo test -p fernjo-core settings` → FAIL (no `Theme`).
- [ ] **Step 3:** Implement `Theme`, the field and `lenient_theme` in `settings.rs`; export from `lib.rs`.
- [ ] **Step 4:** `cargo test -p fernjo-core settings` → PASS.
- [ ] **Step 5: Write the failing app tests** in `app_settings.rs`: `set_theme_persists_and_keeps_data_dir_and_terminal` (settings file holds all three), `set_theme_survives_reopening_the_state` (`AppState::open` over the same config dir returns it from `get_theme`), `set_theme_under_the_env_override_is_memory_only_and_writes_nothing` (`persistent == false`, no `settings.json`), `set_theme_with_unusable_settings_is_refused_and_the_file_untouched` (`settings_unusable`, bytes equal), `set_terminal_settings_still_keeps_the_theme` (regression for the extracted helper). Add `get_theme` and `set_theme` entries to `table()` and the new file to `sources`.
- [ ] **Step 6:** `cargo test -p fernjo-app app_settings` → FAIL.
- [ ] **Step 7:** Implement `app_settings.rs` (commands plus `update_settings`), add `AppState.theme: Mutex<Theme>` filled from the loaded settings exactly where `terminal` is filled (same Env / `settings_error` rule), make `set_terminal_settings` call `update_settings`, register both commands.
- [ ] **Step 8:** `cargo test -p fernjo-app` (all, including `no_response_contains_a_secret` and `command_table_covers_every_tauri_command`) → PASS; `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] **Step 9: Commit** `feat(settings): theme setting stored in settings.json` (files above; `git show --stat HEAD`).

### Task 2: Theme in the UI and the Settings dialog

**Files:**
- Modify: `ui/src/lib/types.ts` (`Theme`, `ThemeInfo`, `Api.getTheme`, `Api.setTheme`), `ui/src/lib/api.ts`, `ui/src/lib/testing.ts` (mock api), `ui/src/main.ts`, `ui/src/App.svelte`
- Create: `ui/src/lib/theme.svelte.ts`, `ui/src/lib/theme.test.ts`
- Rename (`git mv`): `ui/src/lib/components/DataFolderDialog.svelte` → `SettingsDialog.svelte`, `DataFolderDialog.test.ts` → `SettingsDialog.test.ts`; update every import and the `CLAUDE.md` mentions of the dialog name

**Interfaces:**
- Produces: `THEME_KEY = "fernjo.theme"`; `applyTheme(theme: Theme): void` (sets `document.documentElement.dataset.theme` to `light` or `dark`, removes the attribute for `system`); `readStoredTheme(): Theme` (localStorage inside try/catch, anything but the three names → `"system"`); `class ThemeStore { info: ThemeInfo | null; load(): Promise<void>; set(theme: Theme): Promise<ApiError | null> }` which calls `applyTheme` and mirrors the value to localStorage after every successful load and set. `SettingsDialog` gets a `theme: ThemeStore` prop next to `terminal`.
- Consumes: Task 1 commands `get_theme`, `set_theme`.

- [ ] **Step 1: Write the failing tests.** `theme.test.ts`: `applyTheme_sets_data_theme_for_light_and_dark`, `applyTheme_system_removes_the_attribute`, `readStoredTheme_ignores_values_that_are_not_theme_names` (`"banana"`, `""`, `"{}"`), `readStoredTheme_survives_a_throwing_localStorage`, `load_applies_and_mirrors_the_theme_from_rust`, `a_rejected_set_leaves_attribute_and_mirror_unchanged`. `SettingsDialog.test.ts` (adapt the old tests): `appearance_shows_system_light_dark_as_a_radio_group_with_the_current_choice`, `choosing_a_theme_applies_it_at_once`, `a_memory_only_theme_says_it_is_not_remembered` (`persistent: false` note, same wording style as the terminal section), plus the four follow-ups: `saved_note_disappears_when_a_terminal_field_is_edited`, `a_late_load_does_not_overwrite_what_the_user_already_typed`, `a_failed_load_shows_an_error_and_keeps_save_disabled`, `enter_during_ime_composition_does_not_submit` (`isComposing: true`).
- [ ] **Step 2:** `pnpm test -- theme SettingsDialog` → FAIL.
- [ ] **Step 3:** Implement the store and helpers, wire `main.ts` (`applyTheme(readStoredTheme())` before `mount`), create the store in `App.svelte` and load it on start, rename and extend the dialog (sections: Appearance, Terminal, Data folder; theme radios are the only control without a Save button), implement the four follow-ups. Appearance styling uses existing tokens only; no new colour token.
- [ ] **Step 4:** `pnpm test` all green; `pnpm lint` clean (0 errors, 0 warnings).
- [ ] **Step 5: Real-app check (Windows, DevTools recipe from `CLAUDE.md`, scratch `FERNJO_DATA_DIR`):** pick Dark, `html[data-theme="dark"]` and a screenshot; `Emulation.setEmulatedMedia` dark with "System" restyles live; after a graceful close and restart under a scratch `APPDATA` (settings persist) the stored explicit theme is on `<html>` before the first screenshot frame (no light flash: sample `documentElement.dataset.theme` at `DOMContentLoaded`). Read the light and dark screenshots of the dialog and judge them.
- [ ] **Step 6:** In `Docs/roadmap.json` remove the "Settings dialog: …" bullet from the backlog entry "M4 review follow-ups" (done here). **Commit** `feat(ui): theme setting and Settings dialog` (+ a separate `refactor(ui): rename DataFolderDialog to SettingsDialog` commit if the rename is staged alone first).

### Task 3: Bundle configuration and release profile

**Files:**
- Modify: `src-tauri/tauri.conf.json`, `Cargo.toml` (root: `[profile.release]`), `src-tauri/tests/tauri_conf.rs`, `Docs/DECISIONS.md`

**Interfaces:**
- Produces: bundle output names `fernjo_<version>_x64-setup.exe`, `fernjo_<version>_aarch64.dmg`, `fernjo_<version>_x64.dmg`, `fernjo_<version>_amd64.AppImage`, `fernjo_<version>_amd64.deb` (Tauri naming; Task 4's `verify-installers.sh` depends on these exact names, so confirm them in Step 5 and adjust the script if Tauri names differ).

- [ ] **Step 1: Write the failing test** in `tauri_conf.rs`: `bundle_is_active_with_exactly_the_planned_targets` (`bundle.active == true`, `bundle.targets == ["nsis","dmg","appimage","deb"]`), `bundle_icons_exist_on_disk` (every path in `bundle.icon` exists relative to `src-tauri`), `version_comes_from_cargo_not_from_the_conf` (no `version` key; one source of truth), `no_updater_and_no_signing_material` (no `plugins.updater`, no `createUpdaterArtifacts`, no `signCommand`, no `signingIdentity`, no `certificateThumbprint`).
- [ ] **Step 2:** `cargo test -p fernjo-app --test tauri_conf` → FAIL.
- [ ] **Step 3:** Edit `tauri.conf.json`: `bundle.active: true`; `targets` as above; `icon` list (the PNGs at 32/128/128@2x, `icon.icns`, `icon.ico`); `category: "DeveloperTool"`; `shortDescription` and `longDescription` (one honest sentence each, no feature the app lacks); `copyright: "MIT OR Apache-2.0"`; `windows.nsis.installMode: "currentUser"` (no administrator rights needed for an unsigned per-user tool) and `languages: ["English"]`; `macOS.minimumSystemVersion: "11.0"`; remove the `version` key. Root `Cargo.toml`: `[profile.release]` with `strip = "symbols"`, `lto = "thin"`, `codegen-units = 1`. Add the `Docs/DECISIONS.md` entry "M6 packaging: targets, per-user NSIS, one version source, release profile" with the rejected alternatives (per-machine NSIS install, MSI, `tauri-action`).
- [ ] **Step 4:** `cargo test -p fernjo-app --test tauri_conf` → PASS.
- [ ] **Step 5: Build locally on Windows:** `pnpm tauri build --bundles nsis`; expected: `target/release/bundle/nsis/fernjo_0.0.0_x64-setup.exe` exists (this proves the version fallback and the name); note its size in the commit body. If Tauri downloads NSIS tooling, that is expected.
- [ ] **Step 6: Commit** `feat(packaging): bundle configuration and release profile`.

### Task 4: Workflow policy, version and installer scripts, release workflow

**Files:**
- Create: `tools/check-workflows.sh`, `tools/check-release-version.sh`, `tools/verify-installers.sh`, `.github/workflows/release.yml`
- Modify: `.github/workflows/ci.yml` (job `workflows`)

**Interfaces:**
- Produces: `tools/check-workflows.sh [dir]` (default `.github/workflows`) exits 1 and prints `file:line: reason` for: a `uses:` without a 40-hex SHA (local `./` actions excepted), `self-hosted`, `pull_request_target`, a workflow without top-level `permissions:`, `${{ github.event.` or `${{ inputs.` inside a `run:` block. `tools/check-release-version.sh <tag>` strips one leading `v` and exits 1 unless the workspace version (`[workspace.package] version` in `Cargo.toml`) and `package.json` version both equal it, and the argument matches `^v?[0-9]+\.[0-9]+\.[0-9]+$`. `tools/verify-installers.sh <version> <dir>` exits 1 unless `<dir>` holds exactly the five files named in Task 3, naming each missing or unexpected one.
- Consumes: the file names from Task 3.

- [ ] **Step 1: Write failing script tests** as a runnable `tools/test-release-scripts.sh` (plain bash, temp dirs, `assert_exit` helper; no new dependency): `check_workflows_passes_on_the_real_workflows`; a bad-fixture case per rule (`unpinned action`, `self-hosted`, `pull_request_target`, `missing permissions`, `inline github.event in run`) each expecting exit 1 and the reason in the output; `check_release_version_accepts_the_matching_tag`, `rejects_a_mismatching_tag`, `rejects_a_tag_with_shell_metacharacters` (e.g. `v1.0.0; echo pwned`, `$(id)`) without evaluating it; `verify_installers_passes_on_the_complete_set`, `fails_naming_the_missing_file`, `fails_on_an_unexpected_extra_file`.
- [ ] **Step 2:** `bash tools/test-release-scripts.sh` → FAIL (scripts missing).
- [ ] **Step 3:** Implement the three scripts (bash, `set -euo pipefail`, no `eval`).
- [ ] **Step 4:** `bash tools/test-release-scripts.sh` → PASS.
- [ ] **Step 5: Write `release.yml`.** Triggers: `release: types: [published]`; `workflow_dispatch` with optional input `tag` (re-attach to an existing release); `pull_request` limited by `paths` to `.github/workflows/release.yml`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `Cargo.toml`, `tools/**`, `about.toml` (a dry run: build and upload artifacts, never publish). Top-level `permissions: contents: read`. Job `build` (matrix: `windows-latest` → `nsis`; `macos-latest` → `dmg` with `--target aarch64-apple-darwin`; `macos-latest` → `dmg` with `--target x86_64-apple-darwin` after `rustup target add`; `ubuntu-22.04` → `appimage,deb`): checkout with `persist-credentials: false` at the release tag when there is one, `tools/check-release-version.sh` only on a release or tag input (tag read from `env:`), Linux build dependencies as in `ci.yml`, pnpm, Node 24, pinned Rust toolchain, `rust-cache`, `pnpm tauri build --bundles <bundles> [--target <triple>]`, copy the produced installers into `installers/`, `actions/upload-artifact`. Job `publish` (`needs: build`, `if` release or tag input, `permissions: contents: write`, `environment`-free): `actions/download-artifact`, `tools/verify-installers.sh`, `sha256sum` into `SHA256SUMS`, `gh release upload "$TAG" … --clobber` with `GH_TOKEN` and `TAG` from `env:`. Look up the SHAs of `actions/upload-artifact` and `actions/download-artifact` (`gh api repos/actions/<name>/git/ref/tags/<tag>`, dereference annotated tags) and record the version in the comment; never guess a SHA. Add `concurrency`, `timeout-minutes: 45`.
- [ ] **Step 6:** In `ci.yml` add job `workflows` (ubuntu-latest, checkout, `bash tools/check-workflows.sh`, `bash tools/test-release-scripts.sh`), no new required check yet. `bash tools/check-workflows.sh` → exit 0 on all four workflows.
- [ ] **Step 7:** `docs`: `Docs/DECISIONS.md` entry "M6 release workflow: plain `pnpm tauri build`, split permissions, dry run on pull requests" (rejected: `tauri-action`, a single job with `contents: write`; note that `workflow_dispatch` only exists once the file is on `dev`, which is why the pull-request trigger is the dry run).
- [ ] **Step 8: Commit** `ci: release workflow with installers for all three platforms` (+ separate `ci: workflow policy check` commit for the policy script and the CI job if it reads better).

### Task 5: Third-party licence notices in the installers

**Files:**
- Create: `about.toml`, `tools/notices/about.hbs`, `tools/third-party-notices.mjs`
- Modify: `.gitignore` (`src-tauri/resources/THIRD-PARTY-NOTICES.md`), `src-tauri/tauri.conf.json` (`bundle.resources`), `.github/workflows/release.yml`, `Docs/DECISIONS.md`

**Interfaces:**
- Produces: `node tools/third-party-notices.mjs <out-file>` writes one Markdown file: the Rust part from `cargo about generate tools/notices/about.hbs` and the UI part from `pnpm licenses list --json` limited to the packages Vite bundles (`@tauri-apps/api`, `@xterm/*`, `svelte` and the two `@fontsource-variable/*` fonts, with each licence text), failing (exit 1) when a listed package has no licence text or a Rust crate has a licence outside `deny.toml`'s allow-list.

- [ ] **Step 1: Write the failing check:** `node tools/third-party-notices.mjs` with the script absent → FAIL; then a negative case in `tools/test-release-scripts.sh`: `notices_fail_when_a_bundled_package_has_no_licence_text` (run the script's pure function against a fixture list through a `--fixture <json>` flag).
- [ ] **Step 2:** Add `about.toml` (`accepted` = the allow-list of `deny.toml`, `targets` = the five bundle triples' OS families, `ignore-dev-dependencies = true`) and the template; implement the script. `cargo install --locked cargo-about@<version at least two weeks old>` is the install step in the workflow (cached by `rust-cache`); record the version in `DECISIONS.md` ("M6 dependencies: cargo-about").
- [ ] **Step 3:** Run it locally: output file exists, contains `MIT`, `Apache-2.0`, `OFL-1.1` (the fonts) and `MPL-2.0` entries, is over 10 KB, and mentions no absolute path of this machine.
- [ ] **Step 4:** Generate it in the `build` job before `pnpm tauri build` into `src-tauri/resources/`, add the resource to `bundle.resources`, extend `tauri_conf.rs` test `bundle_icons_exist_on_disk` neighbour `notices_are_a_bundle_resource`; `cargo test -p fernjo-app --test tauri_conf` PASS; `bash tools/check-workflows.sh` PASS.
- [ ] **Step 5: Commit** `feat(packaging): third-party notices shipped with the installers`.

### Task 6: Installer smoke tests in the release workflow

**Files:**
- Modify: `.github/workflows/release.yml`
- Create: `tools/smoke-installed.sh`, `tools/smoke-installed.ps1`

**Interfaces:**
- Produces: per-OS steps after the build that prove the installer is usable: **Linux** `sudo apt-get install -y ./fernjo_*_amd64.deb`, `ldd /usr/bin/fernjo` shows no `not found`, `dpkg-deb --info` shows `Depends:` with `libwebkit2gtk-4.1-0`, and `xvfb-run -a timeout 15 fernjo` with `FERNJO_DATA_DIR` set is still alive after 15 s (exit 124 from `timeout`); the AppImage is executable and `--appimage-extract` succeeds. **macOS** `hdiutil attach`, `fernjo.app/Contents/MacOS/fernjo` exists and `lipo -archs` equals the matrix architecture. **Windows** `Start-Process <setup>.exe -ArgumentList '/S' -Wait`, the installed `fernjo.exe` exists under `%LOCALAPPDATA%\fernjo` (per-user), the app starts with a scratch `FERNJO_DATA_DIR` and its main window exists after 15 s (`Get-Process fernjo`, `MainWindowTitle -eq 'fernjo'`), a pre-created `%APPDATA%\fernjo\sentinel.txt` is **unchanged** after install and after the silent uninstall (`uninstall.exe /S`), and no `fernjo.exe` remains.

- [ ] **Step 1:** Write `smoke-installed.sh` (Linux and macOS branches, selected by `uname`) and `smoke-installed.ps1` with the checks above; each prints one `ok:` or `FAIL:` line per check and exits non-zero on the first failure.
- [ ] **Step 2:** Add the smoke steps to the `build` job; run the Windows script locally against the Task 3 installer only after asking the owner once (it installs per-user into the owner's profile and uninstalls again); otherwise leave the local run out and rely on the hosted runner.
- [ ] **Step 3:** Push the branch, open the PR as draft, watch the `release.yml` dry-run of the PR (`gh run watch`): all four build jobs green, five installers uploaded as artifacts (`gh run download` and `bash tools/verify-installers.sh 0.0.0 <dir>` exit 0). If the Linux window start fails for an environment reason (no GL under Xvfb), set `WEBKIT_DISABLE_COMPOSITING_MODE=1` for that step and record why in `DECISIONS.md`; do not drop the check.
- [ ] **Step 4: Commit** `ci(release): smoke-test every installer`.

### Task 7: Measure the performance targets (spec §6)

**Files:**
- Create: `tools/measure-performance.ps1`, `Docs/performance.md`
- Modify: `Docs/DECISIONS.md`, `Docs/roadmap.json` (evidence only, in Task 11)

**Interfaces:**
- Produces: `tools/measure-performance.ps1 -Exe <path> -Runs 10` prints one JSON object: `coldStartMs` (`median`, `max`, `runs[]`), `idleMemoryMiB` (`fernjoOnly`, `withWebView`), `filter500Ms` (`median`, `max`). Definitions (recorded in `Docs/performance.md`): start = `Start-Process`; usable = the first moment the DevTools protocol reports the tree area populated (rows, or the empty-tree offer) on a scratch `FERNJO_DATA_DIR` built with `cargo run -p fernjo-core --example generate_tree -- <dir> 500`; idle memory = sum of private working sets 30 s after usable with no session open, once for `fernjo.exe` alone and once for the whole process tree including the WebView2 children; filter = M2's method (set the filter text, time to the second painted frame, worst-case alternating queries).

- [ ] **Step 1:** Write the script (CDP over `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`, Node 22 `WebSocket` via a small inline `node -e`, as in `CLAUDE.md`); a run against the debug binary must at least return all three keys (smoke).
- [ ] **Step 2:** Build the release binary (`pnpm tauri build --no-bundle`, the CI-profile of Task 3), run `-Runs 10` three times; record the medians, the machine (CPU, RAM, Windows build) and the commit in `Docs/performance.md`.
- [ ] **Step 3: Judge against the spec, honestly:** cold start median under 2000 ms; idle memory under about 150 MiB. If either misses, **do not redefine the number**: write both readings (fernjo-only and with WebView) down, first try the cheap fixes the numbers point to (for example lazy work at start, a smaller initial page), re-measure, and if it still misses stop and ask the owner whether the target or the definition moves; record the answer as a scope change.
- [ ] **Step 4:** `Docs/DECISIONS.md` entry "M6 performance measurements: definitions and results"; the Linux and macOS equivalents (`/usr/bin/time -v`, `ps -o rss`, `footprint`) go into `Docs/performance.md` as owner-run commands and into the roadmap block "Open manual checks (owner)" in Task 11.
- [ ] **Step 5: Commit** `docs(perf): measure cold start, idle memory and filter latency`.

### Task 8: README, changelog and project docs

**Files:**
- Modify: `README.md`, `CHANGELOG.md`, `SECURITY.md`, `CLAUDE.md`

- [ ] **Step 1: README.** Replace "Status: in development / Nothing is installable yet" and "Features (planned)" with the real status and feature list; add "Install": download from the GitHub release, verify with `SHA256SUMS` (`sha256sum -c`, `Get-FileHash`); **Windows** (SmartScreen: "More info" then "Run anyway"; per-user install, no administrator rights; uninstall keeps `%APPDATA%\fernjo`), **macOS** (unsigned and not notarised: "Open Anyway" in System Settings > Privacy & Security after the first blocked start, or `xattr -dr com.apple.quarantine /Applications/fernjo.app`; Apple Silicon and Intel images), **Linux** (`sudo apt install ./fernjo_<version>_amd64.deb`; AppImage: `chmod +x`, needs `libfuse2` on Ubuntu 22.04 and `libfuse2t64` on 24.04; Wayland tested). Mark the macOS text "from Apple's documented behaviour, not yet run on a Mac" until the owner confirms. Fix the Security section (SECURITY.md exists). Add a short "Settings" paragraph (theme, terminal, data folder).
- [ ] **Step 2: CHANGELOG `[Unreleased]`:** entries for the theme setting, the Settings dialog fixes, installers for three platforms, SHA256SUMS, third-party notices, per-user Windows install (M6).
- [ ] **Step 3: SECURITY.md:** "Supported versions" says the latest release from v0.1.0 on (and `dev`); keep the rest.
- [ ] **Step 4: CLAUDE.md "Project-specific":** release workflow shape (jobs, permissions, dry run on PR, `workflow_dispatch` only from `dev`), the three `tools/*.sh` scripts and `tools/test-release-scripts.sh`, `tools/measure-performance.ps1`, the theme field and `app_settings.rs`, the rule that the version lives in the Cargo workspace and `package.json` only.
- [ ] **Step 5:** `pnpm lint` (Prettier covers only `ui` and root json/js, so check the Markdown tables by eye) and `dotnet run tools/roadmap.cs -- check`. **Commit** `docs: installation, changelog and project notes for M6`.

### Task 9: Security audit of the release candidate

**Files:**
- Create: `Docs/release-checklist.md` (section "v0.1.0 security review": a findings table `# | area | finding | severity | decision | fixed in`)
- Modify (only as findings demand): the code named by a finding, with a test per fix; `Docs/DECISIONS.md`; `deny.toml`

Triage rule: **High** (a secret reaching the webview, disk, a log or an installer; a bypass of host-key verification; a webview-to-command hole; remote code execution) blocks `v0.1.0` and is fixed in this task with a regression test. **Medium** is fixed here when it is a small change (about one task), otherwise the owner decides between "fix before release" and backlog. **Low** goes to the backlog. Every check records the command, the result and the date.

- [ ] **Step 1: Secrets.** `cargo test -p fernjo-app no_response_contains_a_secret command_table_covers_every_tauri_command` PASS. `rg -n "println!|eprintln!|dbg!|tracing::|log::" crates src-tauri/src` lists nothing outside tests and examples (a hit is read, not assumed harmless). `rg -n "localStorage|sessionStorage" ui/src -g '!*.test.ts'` shows only view-state keys. Real app (release binary, scratch `FERNJO_DATA_DIR`): with sentinel secrets typed into the vault and a connection used, `grep -r` for the sentinels over the data folder **and** the WebView2 user-data folder (`%LOCALAPPDATA%\io.github.benjo-code.fernjo`) finds nothing.
- [ ] **Step 2: Webview and IPC surface.** `src-tauri/capabilities/default.json` is `core:default` only; from the page (`Runtime.evaluate`) `window.__TAURI_INTERNALS__.invoke('plugin:dialog|open')` and `…'plugin:opener|open_url'` are refused; `rg "@html" ui/src` is empty; `check_external_url` tests cover `file:`, `javascript:`, `data:`, UNC paths and a host-less `http:`; the served CSP of the release binary is the one in `tauri.conf.json`. Decide whether a navigation guard (`on_navigation` allowing only the app origin) is worth adding: a top-level navigation of the main webview to a remote URL would run remote content with access to the IPC; if the check shows it is possible, that is a Medium finding.
- [ ] **Step 3: Release-binary hygiene.** `FERNJO_DIALOG_FILE` does not occur in the release binary (`strings` or a byte search of `target/release/fernjo.exe`); DevTools are not reachable in the release binary without the debugging environment variable (accepted: whoever can set the environment already runs code as the user, record it); in the installers of the PR dry-run (downloaded artifacts), a byte search for the CI runner's home path shows no builder-specific string, and for the owner's own account name nothing (the local builds are not distributed; the CI builds are).
- [ ] **Step 4: Network behaviour.** `cargo tree -e normal` shows no HTTP client crate pulled by fernjo's own code (`reqwest`, `ureq`, `hyper-util` client use are reported with the crate that pulls them); the release binary idle for 60 s has no remote TCP connection (`Get-NetTCPConnection -OwningProcess` over `fernjo.exe` and its WebView2 children shows loopback only); no update check, no telemetry exists. Record it as the privacy statement for the README if all three hold.
- [ ] **Step 5: Data at rest.** Read `write_atomic` (`crates/fernjo-core/src/persist.rs`) and the vault writer for the file mode they create on Unix: `vault.fernjo`, `connections.json`, `settings.json`, `known_hosts`. Default umask (usually 0644) means other local users can read host names, user names and the encrypted vault: record it, fix as Medium when confirmed (mode 0600 on creation for those four files and 0700 for a data folder fernjo creates itself, with a unix-only test), keep Windows ACL inheritance as is.
- [ ] **Step 6: SSH crypto policy.** List the key-exchange, cipher, MAC and host-key algorithms `fernjo-ssh` offers (russh `Preferred` as configured) and compare with OpenSSH's current defaults; flag SHA-1 signatures (`ssh-rsa`), CBC ciphers, 3DES, and the `diffie-hellman-group1`/`group14-sha1` exchanges; record the list in `Docs/DECISIONS.md` ("M6 security review: SSH algorithm policy"); a weak algorithm that OpenSSH itself no longer offers is Medium.
- [ ] **Step 7: Supply chain.** `cargo deny check` and `pnpm audit --prod` (record every advisory); review each `deny.toml` ignore against the current `Cargo.lock` (is `RUSTSEC-2023-0071` still needed, does `russh` now use `rsa >= 0.10.0`: drop the ignore if not needed); `pnpm install --frozen-lockfile` and `cargo build --locked` succeed; `gh api repos/benjo-code/fernjo/dependabot/alerts --jq '[.[]|select(.state=="open")]|length'` and the open CodeQL alerts (`gh api repos/benjo-code/fernjo/code-scanning/alerts`) are listed; the known test-sentinel alerts are named as an owner action ("dismiss as used in tests"), not dismissed by this task.
- [ ] **Step 8: Pipeline and repository.** `bash tools/check-workflows.sh` PASS; `grep -rn "secrets\." .github/workflows` shows only `GITHUB_TOKEN` use; branch protection of `main` and `dev`, secret scanning with push protection, private vulnerability reporting and the fork-PR approval policy re-read with the `gh api` commands of M0's evidence and compared; history re-scanned: gitleaks over the whole history (`gitleaks detect --redact`), expected `no leaks found`; the release workflow is read once more for any value from the event reaching a shell.
- [ ] **Step 9: Independent review.** Run `/security-review` on the full milestone diff (`git diff dev...HEAD`), and give a fresh reviewer agent the code, not these conclusions, of the three places where a mistake costs most and that M6 does not touch: `crates/fernjo-vault` (key handling, file format, keychain), `crates/fernjo-ssh` (host-key verification path) and `crates/fernjo-import` (hostile input). Triage every finding with the rule above; record each in the table, including those rejected with the reason.
- [ ] **Step 10:** Fix the findings the rule assigns to this milestone, each with its test and its own commit; update the table (`fixed in <sha>`). **Commit** `docs(security): v0.1.0 security review`.

### Task 10: Release-readiness gate

**Files:**
- Modify: `Docs/release-checklist.md` (reusable checklist at the top, "v0.1.0 result" table below), `CHANGELOG.md` (only if an entry is missing)

The checklist is reusable for `v0.2.0`; its first section holds the steps of the `CLAUDE.md` release procedure, the second the evidence for `v0.1.0`. This task **prepares** the release and stops there: no branch `release/v0.1.0`, no tag, no GitHub release, no merge to `main`.

- [ ] **Step 1: Version and metadata consistency.** `Cargo.toml` workspace version, `package.json` version and the CHANGELOG heading agree on what the release will be (`0.1.0`), and `tools/check-release-version.sh v0.1.0` fails today only because the files still say `0.0.0` (record this as the expected state; the release procedure bumps them). `Cargo.lock` and `pnpm-lock.yaml` are current. Licence files exist in the repo and `THIRD-PARTY-NOTICES.md` is in the installer resources (`pnpm tauri build --no-bundle` plus `tauri_conf.rs`, and the dry-run artifacts).
- [ ] **Step 2: Documentation matches reality.** Every claim in the README is checked against the app (SSH auth order, shortcut table, import limits, backup format link, "Pageant is not supported", the install section); `CHANGELOG.md` `[Unreleased]` covers M0 to M6 and names milestones; `SECURITY.md` and the issue templates are current; `Docs/ROADMAP.md` is generated and `dotnet run tools/roadmap.cs -- check` is valid.
- [ ] **Step 3: Build and test evidence on the final head.** Run the full gate of Task 11 once more on the exact commit the checklist names, with the SSH test container started and stopped (`tools/ssh-test-server.sh`), and CI of the PR green (`gh pr checks`), including the `release.yml` dry run.
- [ ] **Step 4: Known gaps, stated.** Write the table the owner decides on, one row each with the proposed disposition (blocker / accepted for v0.1.0 / after release): unsigned installers (SmartScreen and Gatekeeper warnings, documented in the README); the open manual checks of the roadmap (Linux and macOS for M0 to M5, the owner's real hosts for M4, the four unused native dialogs of M5, the attach step of the release workflow against a real release); Medium and Low findings of Task 9 that were not fixed; the `deny.toml` advisory ignores with their reasons; the performance figures of Task 7.
- [ ] **Step 5: Rehearsal of the attach step (owner-gated).** Ask the owner once whether to create a **draft** release named `v0.0.0-rehearsal` for `workflow_dispatch` with `tag` (possible only after this PR is on `dev`), then delete it. If the owner declines or it is deferred, the roadmap criterion for the release workflow keeps "installers produced" ticked and notes "attach step unproven until v0.1.0" as open.
- [ ] **Step 6: Verdict.** The checklist ends with a go / no-go line per area (security review, installers, docs, performance, repo settings) and an overall line **"ready for the owner to decide on v0.1.0"** or the list of blockers. Commit `docs(release): v0.1.0 readiness report`.

### Task 11: Final gate, roadmap and pull request

**Files:**
- Modify: `Docs/roadmap.json`, `Docs/ROADMAP.md` (generated), `Docs/DECISIONS.md`, `CHANGELOG.md`

- [ ] **Step 1: Run and record** (Windows, with the SSH test container from `eval "$(tools/ssh-test-server.sh start)"`, stopped afterwards): `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace` (counts per crate); `pnpm install --frozen-lockfile`; `pnpm lint`; `pnpm test`; `pnpm ui:build`; `cargo deny check`; `bash tools/test-release-scripts.sh`; `bash tools/check-workflows.sh`; `pnpm tauri build --no-bundle` (CSP in the served page, no console message); `dotnet run tools/roadmap.cs -- generate` then `-- check`.
- [ ] **Step 2: Roadmap (`Docs/roadmap.json`, then regenerate).** Tick tasks as they were finished; tick acceptance criteria only with evidence: release workflow run URL for the dry run (criterion 1), the Windows smoke and any real-machine installs (criterion 2; macOS, Linux and "attach step" marked OPEN), the performance JSON (criterion 3), the gate output (criterion 4), the review table and the verdict (the new criteria). Add the open owner checks of this milestone to the block "Open manual checks (owner)": install on a real Mac and a real Linux desktop (Wayland), README workarounds confirmed on macOS, Linux AppImage with FUSE, performance commands on Linux and macOS. Status `done` with `completedOn` only when every criterion is ticked or explicitly carried as open in that block.
- [ ] **Step 3:** CHANGELOG final check, `Docs/DECISIONS.md` has the entries named in Tasks 3 to 9. **Commit** `docs: milestone 6 done` and open the PR to `dev` (template filled: Milestone "M6 – Packaging, app settings, release readiness", Verification with the commands and results, what is open). Merge it yourself only after all required checks are green (`CLAUDE.md`, "Git & branches"); do not start the release.
