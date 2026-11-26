# Milestone 13: Automatic updates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The app finds a strictly newer stable version on the pinned GitHub release feed, shows it, and one click downloads, verifies and installs it in an app-themed window and starts the new version; the release workflow signs the update files in a job that alone holds the key.

**Architecture:** `tauri-plugin-updater` is driven only from Rust (`src-tauri/src/updater.rs` with child modules for the pure decisions and the scheduler); the webview gets five commands and one event and no plugin permission. Install is two plugin calls (`download`, then `install`) with the existing close confirmation in between (`CloseThen::Install`). The release pipeline gains a key-less macOS tarball step, a `sign` job (only secret holder), a `verify-signatures` job (no secret, no write token) and a `publish` job that writes `latest.json` and attaches everything to the draft.

**Tech Stack:** Rust (Tauri 2, `tauri-plugin-updater` 2.x pinned exactly, `semver`, `minisign-verify` as a dev-dependency of `fernjo-core`'s example), Svelte 5 + TypeScript, bash and Node (`tools/`), GitHub Actions (no new action).

**Spec:** [Docs/superpowers/specs/2026-11-26-m13-auto-update-design.md](../specs/2026-11-26-m13-auto-update-design.md) (approved by the owner 2026-11-26). Requirements also: Milestone 13 in `Docs/ROADMAP.md`. Background: `CLAUDE.md` ("Project-specific", "Release workflow", "Release scripts"), `Docs/DECISIONS.md`.

## Global Constraints

- Feed: exactly one endpoint, `https://github.com/benjo-code/fernjo/releases/latest/download/latest.json`, no template variables; `pubkey` pinned in `tauri.conf.json`; the shipped config never sets `dangerousInsecureTransportProtocol` and never sets `bundle.createUpdaterArtifacts`; no runtime override of feed or key (no env var, no setting).
- Offer only a strictly greater stable semver (no pre-release or build tag) that is not the skipped version; a failed automatic check changes nothing and shows nothing.
- Timing: first check 60 s after start, then every 24 h; check timeout 15 s; download stall timeout 60 s; `MAX_UPDATE_BYTES` = 256 MiB (declared length above it refused, stream above it cut off); release notes cut to 16 KiB, control characters other than tab and newline removed, shown as text only (never `{@html}`).
- Never `AppHandle::restart`, never `download_and_install`; install goes `download` → confirmation if `connected_count` > 0 (re-read at that moment) → `close_all` → `sync_runner::finish_after_push` → `install(bytes)`.
- Commands: `update_status`, `update_check`, `update_install`, `update_skip`, `set_check_for_updates`, `update_open_release_page` (no argument: Rust builds the URL from the status; the sixth command is added by this plan, the spec listed five); event `update-status`; all in the three-places rule, in `table()` or `APP_HANDLE_EXEMPT`, the new source file in the count guard's `sources`; the webview capability grants no `updater:*`.
- Settings in `settings.json`: `checkForUpdates` (default on), `skippedUpdateVersion`; `FERNJO_DATA_DIR` → no automatic checks, memory-only settings, "Check now" still works; unusable `settings.json` → automatic checks off with a specific note.
- `tauri_plugin_single_instance::init` stays first, `nav_guard::plugin()` second, the updater plugin after them.
- Workflows: GitHub-hosted runners, no new action (reuse the pins in `release.yml`), workflow default `contents: read`, secrets only in `env:` of the signing step, plain block YAML, `bash tools/check-workflows.sh` green; the pull request dry run and forks build without the key and skip `sign`.
- UI: tokens in `ui/src/styles/tokens.css` only, `DESIGN.md`, hex-only CSS (no `oklch()`, `color-mix()`, nesting, `:has()`), no new colour token; every UI `<form>` uses `onsubmit` with `preventDefault`.
- Language English; Conventional Commits; stage files selectively; no attribution lines; never commit a private key; the real key is never used in a test.
- Do not claim what was not run: platform handover (Windows, macOS, Linux), the real GitHub feed and the owner checks stay open with their command or steps.

## Review Focus

1. A feed that answers with the same version, a lower one, a pre-release (`0.5.0-rc.1`), a non-semver string or an empty `platforms` map offers nothing and shows no error at start [Task 3, Task 4].
2. Release notes with `<script>`/HTML, control characters, a 5 MB body, invalid UTF-8-ish lookalikes: shown as inert text, cut at 16 KiB on a character boundary [Task 3, Task 7].
3. A download that declares more than 256 MiB, streams past it, stalls, or is cut short: state `Failed`, nothing installed, memory freed [Task 4].
4. A session connects (or ends) while the download runs: `connected_count` is read at install time, so the dialog appears or not by the state then; "Keep open" keeps `Ready` and a second `update_install` asks again without downloading [Task 5].
5. `settings.json` unusable, or `FERNJO_DATA_DIR` set: no automatic check starts; skip cannot be saved and says why [Task 2, Task 6].
6. A `.sig` that is missing, extra, for a `.deb` or `.dmg`, made with another key, or a `latest.json` that names another tag or file: the release job fails before anything is attached [Task 9, Task 10].

---

### Task 1: Owner key, spike, pinned versions

**Files:**
- Create: `Docs/DECISIONS.md` section "M13: update signing and the updater plugin" (spike findings, filled in as they are made)
- Modify: `src-tauri/tauri.conf.json` (the `plugins.updater.pubkey` and `endpoints` only), `src-tauri/Cargo.toml`, root `Cargo.toml` if versions are pinned there

**Interfaces:**
- Produces: the pinned `tauri-plugin-updater` version; the public key string in `tauri.conf.json`; four recorded facts (below) that Tasks 4, 5 and 9 rely on.

- [ ] **Step 1 (OWNER, blocked until done): generate the key pair outside the repo.** The owner runs, in their own terminal and a folder outside `d:\VS-Code\fernjo`: `pnpm tauri signer generate -w <folder>\fernjo-update.key` (chooses a password), backs both up in the password manager, and gives the executor **only the public key text**. The executor never reads the private key or its password. Fallback if the owner defers: a placeholder `pubkey` string `REPLACE-BEFORE-RELEASE` plus Task 9's guard step (record the choice in the DECISIONS section).
- [ ] **Step 2: current stable version.** `cargo info tauri-plugin-updater` and `crates.io`: pick the newest stable 2.x (not a 3.0 alpha); record it, its licences and its new transitive crates (`cargo tree -p tauri-plugin-updater --depth 1`).
- [ ] **Step 3: spike, in a scratch copy under the scratchpad directory, not in the repo.** Prove and record each fact in the DECISIONS section, with the command and the output that showed it:
  1. `pnpm tauri build --bundles nsis` (or `appimage` on Linux) works with `plugins.updater.pubkey` set and `bundle.createUpdaterArtifacts` unset; the Windows setup `.exe` and the AppImage need no repackaging.
  2. `Update::download` returns an error for a body whose signature does not verify (a one-file Rust program against a local HTTP server and a throwaway key from `tauri signer generate --ci -w <scratch>`).
  3. What `install` does on each OS (read the plugin source at the pinned version: Windows hands over to the setup and exits; macOS/Linux replace files and return) and which calls it exposes for a download progress callback and the declared length.
  4. The macOS tarball layout the plugin extracts (compare with the plugin source); the build itself can only be checked on a Mac (owner check).
- [ ] **Step 4: if any fact is wrong, stop and report to the owner** with what was found before Task 2 starts; otherwise commit the DECISIONS text and the pinned pubkey/endpoint (conf pinned test comes in Task 8).
- [ ] **Step 5: Commit** `docs: record the M13 spike findings` (add only the files changed).

### Task 2: Settings fields

**Files:**
- Modify: `crates/fernjo-core/src/settings.rs`, `crates/fernjo-core/src/lib.rs` (re-exports), `src-tauri/src/state.rs` (two fields), `src-tauri/src/app_settings.rs`

**Interfaces:**
- Produces in `fernjo-core`: `AppSettings.check_for_updates: Option<bool>` (`#[serde(rename = "checkForUpdates")]` if the file uses camelCase like the other fields — follow `reopen_tabs`), `AppSettings.skipped_update_version: Option<String>`, `pub const DEFAULT_CHECK_FOR_UPDATES: bool = true`, `pub fn validate_update_version(v: &str) -> Result<(), &'static str>` (`"invalid_version"`; accepts only `MAJOR.MINOR.PATCH`, at most 32 bytes).
- Produces in `AppState`: `check_for_updates: Mutex<bool>`, `skipped_update_version: Mutex<Option<String>>`, loaded at startup like `reopen_tabs`.
- Produces in `app_settings.rs`: `pub fn set_check_for_updates(state: &AppState, on: bool) -> Result<bool, CommandError>` and `pub(crate) fn set_skipped_version(state: &AppState, v: Option<String>) -> Result<(), CommandError>` (both through `update_settings`, which keeps the other fields).

- [ ] **Step 1: failing tests** in `settings.rs`: `update_fields_round_trip_and_keep_other_fields`, `old_settings_file_without_update_fields_loads_as_defaults`, `validate_update_version_rejects_prerelease_long_and_garbage` (`"1.2.3-rc.1"`, `"v1.2.3"`, `""`, 33 bytes, `"1.2"`). In `app_settings.rs`: `set_check_for_updates_saves_and_keeps_theme`, `set_check_for_updates_memory_only_under_env` (state with `DataDirSource::Env`), `set_skipped_version_refused_when_settings_unusable` (error code `settings_unusable`... use the same code `update_settings` already returns).
- [ ] **Step 2: run** `cargo test -p fernjo-core settings` and `cargo test -p fernjo-tauri app_settings` (use the real package name from `src-tauri/Cargo.toml`); expect FAIL (fields missing).
- [ ] **Step 3: implement** the fields, constant, validator, state fields and the two functions; no Tauri wrapper yet (Task 5 adds the command).
- [ ] **Step 4: run** both test sets; expect PASS; also `cargo clippy -p fernjo-core --all-targets -- -D warnings`.
- [ ] **Step 5: Commit** `feat(core): settings for update checks`.

### Task 3: Pure update decisions

**Files:**
- Create: `src-tauri/src/updater.rs` (module root: `mod decide;` only for now), `src-tauri/src/updater/decide.rs`
- Modify: `src-tauri/src/lib.rs` (`mod updater;`), `src-tauri/Cargo.toml` (`semver = "1"`)

**Interfaces:**
- Produces in `decide.rs`:
  - `pub const MAX_UPDATE_BYTES: u64 = 256 * 1024 * 1024;` `pub const MAX_NOTES_BYTES: usize = 16 * 1024;`
  - `pub fn is_offered(current: &str, candidate: &str, skipped: Option<&str>) -> bool`
  - `pub fn clean_notes(raw: &str) -> String`
  - `pub fn release_page_url(version: &str) -> Option<String>` (`https://github.com/benjo-code/fernjo/releases/tag/vX.Y.Z`, `None` for anything but `MAJOR.MINOR.PATCH`)
  - `pub enum Os { Windows, MacOs, Linux }`, `pub struct InstallEnv { pub os: Os, pub appimage: Option<PathBuf>, pub exe: PathBuf }`
  - `pub enum ReleasePageReason { PackageInstall, NotWritable }`, `pub enum InstallKind { SelfUpdate, ReleasePageOnly(ReleasePageReason) }`
  - `pub fn install_kind(env: &InstallEnv, writable: &dyn Fn(&Path) -> bool) -> InstallKind` (Windows: `SelfUpdate`; macOS: the `.app` bundle's parent folder writable; Linux: `appimage` set and its folder writable, unset → `PackageInstall`, set but not writable → `NotWritable`)
  - `pub fn within_download_cap(declared: Option<u64>, received: u64) -> bool` (false when `declared` or `received` is above `MAX_UPDATE_BYTES`)

- [ ] **Step 1: failing tests** (same file, `#[cfg(test)]`): `offers_strictly_newer_stable_only` (`"0.4.0"`→`"0.4.1"` yes; equal no; lower no; `"0.4.1-rc.1"` no; `"0.4.1+build5"` no; garbage no; empty no), `skipped_version_is_not_offered_but_newer_is`, `notes_are_cut_at_16_kib_on_a_char_boundary` (a 3-byte char straddling the limit), `notes_lose_control_characters_but_keep_tab_and_newline`, `notes_keep_markup_as_literal_text` (`"<script>x</script>"` comes back unchanged: escaping is the UI's job, the text is never interpreted), `release_page_url_only_for_plain_versions`, `install_kind_table` (one case per row of the interface above, including Linux without `APPIMAGE`), `download_cap_uses_declared_and_received`.
- [ ] **Step 2: run** `cargo test -p fernjo-app updater::decide`; expect FAIL.
- [ ] **Step 3: implement** the functions with `semver::Version`; `is_offered` rejects when `candidate.pre` or `candidate.build` is non-empty.
- [ ] **Step 4: run** the tests; expect PASS.
- [ ] **Step 5: Commit** `feat(updater): pure update decisions`.

### Task 4: Plugin, feed client and the fake-feed tests

**Files:**
- Create: `src-tauri/src/updater/feed.rs`, `src-tauri/tests/updater_feed.rs`, `src-tauri/tests/support/fake_feed.rs` (follow how `src-tauri/tests` already shares helpers; create the folder style the existing tests use), a test-only key pair under `src-tauri/tests/fixtures/update-test-key/` (generated with `tauri signer generate --ci`, labelled `TEST ONLY` in a `README` there)
- Modify: `src-tauri/Cargo.toml` (`tauri-plugin-updater` pinned from Task 1; dev-dependency feature `test` of `tauri` if the mock app is used), `src-tauri/tauri.conf.json` (`plugins.updater` with `endpoints`, `pubkey`; nothing else), `src-tauri/src/lib.rs` (`.plugin(tauri_plugin_updater::Builder::new().build())` after the nav guard)

**Interfaces:**
- Consumes: `decide::{is_offered, clean_notes, within_download_cap, MAX_UPDATE_BYTES}`.
- Produces in `feed.rs`:
  - `pub struct FeedUpdate { pub version: String, pub notes: String, pub(crate) handle: tauri_plugin_updater::Update }`
  - `pub enum FeedError { Unreachable, BadFeed, NotOffered, DownloadRefused(&'static str), BadSignature }` with `fn code(&self) -> &'static str` (`feed_unreachable`, `feed_invalid`, `not_offered`, `download_refused`, `signature_invalid`; fixed texts, no URL or body in them)
  - `pub async fn check(app: &AppHandle, current: &str, skipped: Option<&str>, endpoint_override: Option<&Url>) -> Result<Option<FeedUpdate>, FeedError>` (the endpoint override and insecure transport exist only behind a cargo feature `update-test-hooks` that is off by default and that the release workflow never enables; the integration tests turn it on; a test in `tauri_conf.rs` asserts the feature is not in `default`)
  - `pub async fn download(update: &FeedUpdate, on_progress: impl Fn(u64, Option<u64>) + Send + 'static) -> Result<Vec<u8>, FeedError>` (declared length and streamed bytes checked with `within_download_cap`, the download future dropped above the cap, 60 s stall timeout, 15 s check timeout as named constants; the signature check is the plugin's)

- [ ] **Step 1: failing tests** in `src-tauri/tests/updater_feed.rs` against a local HTTP server (insecure transport allowed in the test build only), the test key signing the fixture payload: `newer_version_is_offered`, `same_and_older_version_are_not_offered`, `pre_release_is_not_offered`, `skipped_version_is_not_offered`, `missing_signature_is_refused`, `wrong_key_signature_is_refused`, `tampered_download_is_refused` (valid `latest.json`, payload altered after signing), `unreachable_feed_is_feed_unreachable_and_changes_nothing`, `oversize_declared_length_is_refused_before_streaming`, `stream_past_the_cap_is_cut_off` (use a test-scoped lower cap through a parameter or a `#[cfg(test)]` constant, so the test does not send 256 MiB), `stalled_download_times_out`, `check_sends_exactly_one_get_to_the_pinned_path` (the logging server records method, path and headers; assert one `GET /latest.json`, no other request, no query string, no cookie), `notes_come_back_cleaned`. If the mock app cannot host the plugin, record the alternative used in the DECISIONS section.
- [ ] **Step 2: run** `cargo test -p fernjo-app --test updater_feed`; expect FAIL.
- [ ] **Step 3: implement** `feed.rs` and register the plugin; the endpoint and key come from the `tauri.conf.json` plugin config, not from code literals; check order: plugin result then `is_offered` must both agree.
- [ ] **Step 4: run** `cargo test -p fernjo-app --test updater_feed` and `cargo deny check`; expect PASS (new transitive crates and licences pass, otherwise record in `deny.toml`/`about.toml` with reasons per the existing rules).
- [ ] **Step 5: Commit** `feat(updater): feed client with signature and size checks`.

### Task 5: State, commands, install flow

**Files:**
- Modify: `src-tauri/src/updater.rs`, `src-tauri/src/state.rs` (`updater: Mutex<UpdaterRuntime>`; a leaf lock, never held across network I/O), `src-tauri/src/sessions.rs` (`CloseThen::Install` and its `close_prompt` text), `src-tauri/src/sync_runner.rs` (`finish_after_push` handles `CloseThen::Install`), `src-tauri/src/lib.rs` (handlers), `src-tauri/src/vault_commands.rs` (`table()`, `APP_HANDLE_EXEMPT`, `sources`)

**Interfaces:**
- Consumes: Task 2 settings functions, Task 3 `install_kind`/`release_page_url`, Task 4 `feed::{check, download}`.
- Produces in `updater.rs`:
  - `#[derive(Serialize)] #[serde(rename_all = "camelCase")] pub struct UpdateStatus { pub state: UpdateState, pub current_version: String, pub version: Option<String>, pub notes: Option<String>, pub install: Option<InstallView>, pub received: Option<u64>, pub total: Option<u64>, pub error: Option<String>, pub check_enabled: bool, pub skipped: bool, pub unsaved_reason: Option<UnsavedReason> }`
  - `pub enum UpdateState { Idle, Checking, UpToDate, Available, Downloading, Ready, Installing, Failed }` (serialised camelCase), `pub enum InstallView { SelfUpdate, ReleasePage { reason: &'static str } }`
  - `pub fn update_status(state: &AppState) -> UpdateStatus`, `pub fn update_skip(state: &AppState) -> Result<UpdateStatus, CommandError>` (skips the currently available version; `no_update` otherwise), `pub fn set_check_for_updates(state: &AppState, on: bool) -> Result<UpdateStatus, CommandError>`
  - `pub async fn update_check(app: &AppHandle, manual: bool) -> UpdateStatus` and `pub async fn update_install(app: &AppHandle) -> UpdateStatus` (take `AppHandle`, so `APP_HANDLE_EXEMPT` entries with reason "takes an AppHandle; returns only UpdateStatus")
  - `pub fn update_open_release_page(app: &AppHandle) -> Result<(), CommandError>` (opens `decide::release_page_url` of the available version through the same opener `open_external_url` uses; `no_update` when none; `APP_HANDLE_EXEMPT` "takes an AppHandle; returns nothing")
  - `sessions::CloseThen::Install` with `close_prompt` → title "Install update?", message "{n} session(s) still connected. Installing the update ends them.", confirm "Install"
  - every state change `emit("update-status", UpdateStatus)`

- [ ] **Step 1: failing tests** (`updater.rs` unit tests over a fake backend trait `UpdateBackend { check, download, install }` that the real code implements with `feed`; the commands' logic is generic over it): `check_available_sets_state_and_emits_once`, `second_check_while_checking_is_dropped` (one run slot), `automatic_check_failure_leaves_state_idle_and_no_error`, `manual_check_failure_sets_failed_with_code`, `install_with_no_connected_sessions_installs_after_download`, `install_with_connected_sessions_waits_for_confirmation_then_closes_sessions_first` (order asserted: dialog → `close_all` → push → `install`), `keep_open_keeps_ready_and_second_install_skips_download` (download called once), `connected_count_is_read_at_install_time` (a session connects during the fake download), `oversize_download_sets_failed_download_refused`, `release_page_only_install_never_downloads`, `skip_stores_version_and_hides_it_from_automatic_checks`, `skip_refused_when_settings_unusable`. In `vault_commands.rs`: `table()` entries `update_status`, `update_skip`, `set_check_for_updates` (the no-secret guard runs them), `APP_HANDLE_EXEMPT` gains `update_check` and `update_install`; `sources` gains `include_str!("updater.rs")`; the count guard must pass.
- [ ] **Step 2: run** `cargo test -p fernjo-app updater` and the guard `cargo test -p fernjo-app command_table_covers_every_tauri_command no_response_contains_a_secret`; expect FAIL.
- [ ] **Step 3: implement** the state machine, the backend trait (real impl delegates to `feed`), the five `#[tauri::command]` wrappers in `updater::handlers` (`update_check`/`update_install` are `async fn` taking `AppHandle`), the `CloseThen::Install` branch (`finish_after_push` calls `updater::install_now(app)`: Windows returns only if the plugin did not exit; macOS/Linux then `request_restart`), and the `generate_handler!` entries in `lib.rs`. The install dialog uses `SessionManager::restart_action` for its allow/prevent/ask decision and `ask_to_close`.
- [ ] **Step 4: run** the same tests plus `cargo test -p fernjo-app`; expect PASS.
- [ ] **Step 5: Commit** `feat(updater): update state, commands and install flow`.

### Task 6: Scheduler and runner

**Files:**
- Create: `src-tauri/src/updater/schedule.rs`
- Modify: `src-tauri/src/updater.rs` (`spawn(app)`), `src-tauri/src/lib.rs` (`updater::spawn(app.handle().clone())` in `setup`)

**Interfaces:**
- Produces: `pub const FIRST_CHECK_DELAY: Duration = 60 s; pub const CHECK_INTERVAL: Duration = 24 h;` `pub struct Scheduler` with `pub fn new(enabled: bool) -> Scheduler`, `pub fn set_enabled(&mut self, on: bool)`, `pub fn due(&self, now: Duration) -> bool`, `pub fn ran(&mut self, now: Duration)` (injected time like `sync_runner::Scheduler`); `pub fn automatic_checks_allowed(state: &AppState) -> bool` (false under `DataDirSource::Env`, with `settings_unusable`, or when `check_for_updates` is off).

- [ ] **Step 1: failing tests:** `first_check_after_60_seconds_not_before`, `next_check_24_hours_after_the_last`, `disabled_scheduler_never_fires`, `enabling_later_counts_from_now_plus_first_delay` (no immediate burst), `no_automatic_check_under_fernjo_data_dir`, `no_automatic_check_with_unusable_settings`, `check_for_updates_off_stops_checks`.
- [ ] **Step 2: run** `cargo test -p fernjo-app updater::schedule`; expect FAIL.
- [ ] **Step 3: implement** the pure scheduler and a thread (tick 1 s like `sync_runner`) that calls `update_check(app, false)` through the async runtime and re-reads `automatic_checks_allowed` each tick; a manual check does not reset the 24 h clock.
- [ ] **Step 4: run** the tests; expect PASS.
- [ ] **Step 5: Commit** `feat(updater): check after start and every 24 hours`.

### Task 7: UI

**Files:**
- Create: `ui/src/lib/update.svelte.ts`, `ui/src/lib/update.test.ts`, `ui/src/lib/components/UpdateDialog.svelte`, `UpdateDialog.test.ts`, `UpdateSettings.svelte`, `UpdateSettings.test.ts`, `ui/src/lib/update-text.ts` (stage and reason wording, like `sync-line.ts`)
- Modify: `ui/src/lib/types.ts` (`UpdateStatus`, `UpdateState`, API methods), `ui/src/lib/api.ts` (`updateStatus`, `updateCheck`, `updateInstall`, `updateSkip`, `setCheckForUpdates`; `testing.ts` mock), `ui/src/lib/components/Sidebar.svelte` (footer marker), `SettingsDialog.svelte` (the "Updates" section), `ui/src/App.svelte` (store wiring, dialog)

**Interfaces:**
- Consumes: the Rust `UpdateStatus` JSON of Task 5 (camelCase, same field names).
- Produces: `class UpdateStore { status = $state<UpdateStatus | null>(null); error = $state<ApiError | null>(null); get busy(): boolean; init(): Promise<void>; check(): Promise<void>; install(): Promise<void>; skip(): Promise<void>; setEnabled(on: boolean): Promise<void>; dispose(): void }` (listens to `update-status`; ignores a new action while `busy`); `stageText(status: UpdateStatus): string` and `reasonText(code: string): string` in `update-text.ts`.

- [ ] **Step 1: failing tests:** `update.test.ts`: `store_follows_update_status_events`, `ignores_second_action_while_busy`, `init_reads_status_once`. `UpdateDialog.test.ts`: `available_shows_both_versions_notes_and_three_buttons`, `notes_with_markup_render_as_text` (a `<img onerror>` string appears literally and no `img` element exists), `downloading_shows_progress_and_stage`, `installing_has_no_close_button`, `failed_offers_retry_and_release_page`, `release_page_only_has_no_install_button`, `skip_and_later_buttons_call_the_right_actions`. `UpdateSettings.test.ts`: `shows_installed_version_and_checkbox`, `check_now_shows_up_to_date_result`, `unsaved_note_names_the_real_cause` (env, settings_unusable). A repo-wide test `no_html_injection_in_ui` in `ui/src/lib/update.test.ts` that reads every `*.svelte` file and fails on `{@html`. Sidebar: `footer_marker_only_when_available_or_failed`.
- [ ] **Step 2: run** `pnpm test -- update` ; expect FAIL.
- [ ] **Step 3: implement** the store, texts and components following `ConfirmDialog.svelte`, `SyncStatus.svelte` and `SyncSettings.svelte`; progress is a native `<progress>` styled with existing tokens; focus moves into the dialog and returns to the footer marker on close; the dialog is not closable while `installing`; no URL ever comes from the UI (the "Open release page" button calls `update_open_release_page`, which takes no argument; add `updateOpenReleasePage` to `api.ts` and the mock).
- [ ] **Step 4: run** `pnpm test`, `pnpm lint`; expect PASS. Run the dev app with a scratch `FERNJO_DATA_DIR` and check the footer marker, dialog states and Settings section through the DevTools-port recipe in `CLAUDE.md` (state in the commit message that the harness mocked the update backend).
- [ ] **Step 5: Commit** `feat(ui): update window and settings section`.

### Task 8: Capability narrowing and config pins

**Files:**
- Modify: `src-tauri/capabilities/default.json`, `src-tauri/tests/tauri_conf.rs`

**Interfaces:**
- Produces: a capability whose `permissions` is exactly `["core:event:allow-listen", "core:event:allow-unlisten"]` plus any further permission the real-build check proves necessary (each added one gets a line in the DECISIONS section naming the UI call that needs it).

- [ ] **Step 1: failing tests** in `tauri_conf.rs`: replace `no_updater_and_no_signing_material` by `updater_config_is_pinned` (exactly one endpoint equal to the string in Global Constraints, `pubkey` non-empty, well-formed base64 of a minisign public key and not the test key's public key (skipped-with-message only while the placeholder fallback is active), no `dangerousInsecureTransportProtocol`, no `createUpdaterArtifacts`, no `plugins.updater` keys other than `endpoints` and `pubkey` and `windows` if the spike needs it), `capability_has_no_core_default_and_no_updater_permission`, `capability_lists_exactly_the_pinned_permissions`.
- [ ] **Step 2: run** `cargo test -p fernjo-app --test tauri_conf`; expect FAIL on the capability tests.
- [ ] **Step 3: narrow the capability** to the two event permissions; then build `pnpm tauri build --no-bundle`, start `target/release/fernjo.exe` with the DevTools-port recipe (scratch `FERNJO_DATA_DIR`, `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`) and check: tree loads, a session opens (SSH test container from `tools/ssh-test-server.sh`), terminal data arrives (Channel), `sync-status`/`update-status` events arrive, the update window opens, full screen and the other window calls the UI makes still work. Add a permission only for a call that fails, one at a time, and re-run until clean; stop the app afterwards.
- [ ] **Step 4: run** `cargo test -p fernjo-app`; expect PASS.
- [ ] **Step 5: Commit** `fix(tauri): narrow the webview capability`.

### Task 9: Release scripts

**Files:**
- Create: `tools/write-latest-json.mjs`, `crates/fernjo-core/examples/verify_update_signature.rs`
- Modify: `tools/verify-installers.sh`, `tools/test-release-scripts.sh`, `tools/check-release-version.sh` (only for the placeholder fallback), `crates/fernjo-core/Cargo.toml` (dev-dependency `minisign-verify`, `base64`)

**Interfaces:**
- `tools/verify-installers.sh`: the per-platform lists gain the macOS tarballs (`macos-arm64` → `fernjo_${version}_aarch64.dmg fernjo_${version}_aarch64.app.tar.gz`, `macos-x64` → `.dmg` and `fernjo_${version}_x64.app.tar.gz`); a new form `verify-installers.sh <version> --release <dir>` exits 1 unless `<dir>` holds exactly the seven installer and tarball files, the four `.sig` files (`fernjo_${version}_x64-setup.exe.sig`, `..._aarch64.app.tar.gz.sig`, `..._x64.app.tar.gz.sig`, `..._amd64.AppImage.sig`) and `latest.json` (no `.sig` for the `.deb` or `.dmg`), naming each missing or unexpected file.
- `node tools/write-latest-json.mjs <version> <dir> <changelog>` writes `<dir>/latest.json`: `{ "version", "notes", "pub_date", "platforms": { "windows-x86_64", "darwin-aarch64", "darwin-x86_64", "linux-x86_64": { "signature", "url" } } }` with `url = https://github.com/benjo-code/fernjo/releases/download/v<version>/<file>`, `signature` the `.sig` text, `notes` the `[<version>]` section of `CHANGELOG.md` (the tag section as written when the release branch turned `[Unreleased]` into it); `node tools/write-latest-json.mjs --check <version> <dir>` exits 1 unless the file's version equals `<version>`, every url is the exact release URL of an existing file in `<dir>` and every signature equals that file's `.sig`.
- `cargo run -p fernjo-core --example verify_update_signature -- <tauri.conf.json> <file> <file.sig>` exits 0 only when the signature verifies against the `pubkey` in the conf (both base64-wrapped minisign texts, as `tauri signer` writes them).

- [ ] **Step 1: failing tests** in `tools/test-release-scripts.sh` (fixture trees, the existing helper style; the sample files are tiny; signatures are fake text for the shape tests): `verify_release_accepts_the_full_set`, `verify_release_rejects_missing_sig`, `verify_release_rejects_sig_for_deb`, `verify_release_rejects_extra_file`, `verify_installers_artifacts_form_includes_tarballs`, `latest_json_has_four_platform_entries_with_release_urls`, `latest_json_notes_come_from_the_version_section`, `latest_json_check_rejects_other_tag_url`, `latest_json_check_rejects_signature_that_differs_from_sig_file`, `latest_json_check_rejects_unknown_platform`. In the example: unit tests `verifies_a_signature_made_with_the_test_key`, `rejects_a_signature_from_another_key`, `rejects_a_modified_file` (test key pair from Task 4).
- [ ] **Step 2: run** `bash tools/test-release-scripts.sh` and `cargo test -p fernjo-core --examples`; expect FAIL.
- [ ] **Step 3: implement** the three pieces; if the owner chose the placeholder fallback in Task 1, `check-release-version.sh` also fails while `tauri.conf.json` holds `REPLACE-BEFORE-RELEASE`, with its own test.
- [ ] **Step 4: run** `bash tools/test-release-scripts.sh` (all pass), `cargo test -p fernjo-core --examples`, `cargo deny check`.
- [ ] **Step 5: Commit** `feat(release): update file checks and latest.json writer`.

### Task 10: Release workflow

**Files:**
- Modify: `.github/workflows/release.yml`, `tools/check-workflows.sh` (only if it refuses `environment:`), `tools/test-release-scripts.sh` (checker tests for the new job)

**Interfaces:**
- Consumes: Task 9 scripts.
- Produces: jobs `sign` (`needs: build`, `contents: read`, `environment: release-signing`, tag ref only) and `verify-signatures` (`needs: sign`, `contents: read`, no secret) ahead of `publish`; `publish` `needs: [build, hub-image, verify-signatures]`.

- [ ] **Step 1: failing checks:** extend the workflow-policy tests so they fail until the workflow is right: `sign_job_is_the_only_one_that_names_the_signing_secrets`, `signing_secrets_only_in_env_of_one_step`, `sign_job_has_only_contents_read`, `environment_key_accepted_only_for_release_signing` (a fixture with another environment name is refused), `publish_needs_verify_signatures`, `pull_request_runs_skip_sign`. Run `bash tools/test-release-scripts.sh`; expect FAIL.
- [ ] **Step 2: build jobs:** macOS entries get `bundles: dmg,app`; a step after the build packs `…/bundle/macos/fernjo.app` into `fernjo_${VERSION}_<aarch64|x64>.app.tar.gz` (`tar -C <macos dir> -czf`; version from the existing tag check step's env, arch from `matrix.target`), and the `installers:` globs and the collect step include it; no secret in these jobs.
- [ ] **Step 3: `sign` job:** download the four `installers-*` artifacts (no `merge-multiple`), run `tools/verify-installers.sh <version> --artifacts artifacts installers` (as `publish` does today), install node/pnpm and dependencies in steps **without** the secret, then one step with `env: TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` that runs `pnpm tauri signer sign` on exactly the four update files; upload the `.sig` files as artifact `update-signatures`. Condition: tag ref only (same style as the existing push jobs).
- [ ] **Step 4: `verify-signatures` job:** checkout, pinned Rust toolchain step as in `build`, download installers and signatures, `cargo run -p fernjo-core --example verify_update_signature` for each of the four files against `src-tauri/tauri.conf.json`; fails the run on any mismatch.
- [ ] **Step 5: `publish` changes:** download `update-signatures` as well, copy into the verified `installers` folder, `node tools/write-latest-json.mjs <version> installers CHANGELOG.md`, `node tools/write-latest-json.mjs --check <version> installers`, `tools/verify-installers.sh <version> --release installers`, `SHA256SUMS` over everything, attach and publish as today; the build-provenance attestation also lists `latest.json` and the tarballs.
- [ ] **Step 6: run** `bash tools/check-workflows.sh` (extend narrowly for `environment: release-signing` only if it refuses, with its test) and `bash tools/test-release-scripts.sh`; expect PASS. Push the branch and open the pull request; the **dry run** must build all four platforms, skip `sign`, `verify-signatures` and `publish`, and push nothing (name the run id as evidence).
- [ ] **Step 7 (OWNER, blocked):** create the GitHub Environment `release-signing` restricted to tags `v*` and add the two secrets: `gh api -X PUT repos/benjo-code/fernjo/environments/release-signing` (then the deployment-branch/tag policy in the repository settings), `gh secret set TAURI_SIGNING_PRIVATE_KEY --env release-signing < <key file>` and `gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --env release-signing`. Only the owner runs these; the first signed run happens at the v0.4.0 release, so the signing path itself is **open** until then (record this in the roadmap evidence).
- [ ] **Step 8: Commit** `ci(release): sign update files in their own job and publish latest.json`.

### Task 11: Docs, roadmap evidence, final checks

**Files:**
- Modify: `README.md` (privacy paragraph, a short "Updating" note), `SECURITY.md`, `Docs/release-checklist.md` (drop "No update check"; the M13 section with evidence), `Docs/DECISIONS.md` (the M13 section completed: feed, no template variables, signing job and key lifecycle, no `createUpdaterArtifacts`, no runtime override, the spike facts, the download cap, scope changes of the spec), `CLAUDE.md` ("Project-specific": updater module, commands, settings, release jobs; "Release scripts": the new tools), `CHANGELOG.md` (`[Unreleased]`: one short user-readable line), `Docs/roadmap.json` and generated `Docs/ROADMAP.md`

- [ ] **Step 1: write the docs** as listed (privacy text: one HTTPS GET to the release feed shortly after start and every 24 h, only IP address and user agent reach GitHub, no account, no identifier, off with one setting; the download starts only after a click and comes from the same release).
- [ ] **Step 2: the recorded checks** (commands and results into the roadmap evidence and the checklist): the full verification list below; the logging-server test name for the one-GET criterion; the DevTools-port check of Task 8; the CI dry run id.
- [ ] **Step 3: run the full verification:** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm test`, `pnpm lint`, `cargo deny check`, `bash tools/check-workflows.sh`, `bash tools/test-release-scripts.sh`, `dotnet run tools/roadmap.cs -- check`; each must exit 0.
- [ ] **Step 4: roadmap:** tick tasks as they finish, tick only verified acceptance criteria with evidence; leave every "Manual, owner" criterion unticked with its steps (Windows, macOS and Linux AppImage update against a test feed built with a merged config file under `tools/`, `.deb` and read-only AppImage release-page check, no SmartScreen/Gatekeeper warning on the updated app, no administrator prompt on Windows); regenerate with `dotnet run tools/roadmap.cs -- generate`; set `done` with `completedOn` only when every task and criterion is ticked or consciously open as owner checks, as earlier milestones did.
- [ ] **Step 5: Commit** `docs: record the M13 update path` (roadmap JSON and generated MD together); open the pull request to `dev`; merge only with all CI checks green.
