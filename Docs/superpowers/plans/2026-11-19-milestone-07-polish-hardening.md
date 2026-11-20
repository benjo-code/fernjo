# Milestone 7: Daily-use polish and hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Release v0.1.1 closes the known gaps of v0.1.0: no silent loss of synced `connections.json` data, a quit confirmation, tabs that close on a deliberate shell exit, resizable panels, multi-select in the tree, a UI whose vault, import and settings can be found without a hint, and every Low finding and M6 deferred follow-up settled.

**Architecture:** Rust owns every rule (store fingerprint check, session end classification, batch move validity); the Svelte UI keeps view state only (selection, panel sizes, reload notice). New Tauri commands follow the three-places rule. Phase 4 contains an owner gate: the discoverability review produces a list that the owner agrees before it is implemented.

**Tech Stack:** Rust (edition 2024, workspace), Tauri 2.12, Svelte 5 + TypeScript (vitest, svelte-check), russh, `sha2` (already in `Cargo.lock` as 0.11.1).

**Spec:** [Docs/superpowers/specs/2026-11-19-m07-polish-hardening-design.md](../specs/2026-11-19-m07-polish-hardening-design.md) (approved by the owner on 2026-11-19). Tasks and acceptance criteria: Milestone 7 in `Docs/roadmap.json`.

## Global Constraints

- Branch `feature/m07-polish-hardening`, from `dev`. Never commit to `dev` or `main`. Stage selectively (`git add <file>`), then `git show --stat HEAD` against the expected file list. Conventional Commits (`feat:`, `fix:`, `test:`, `docs:`, `ci:`, `chore:`). No attribution lines in commit messages or PR text.
- Secrets stay in Rust; no new event, command response or log carries one. The no-secret test `no_response_contains_a_secret` in `src-tauri/src/vault_commands.rs` must stay green.
- A new `#[tauri::command]` means three places: the function plus its `handlers` wrapper, its entry in `table()` of that test (or `APP_HANDLE_EXEMPT`), and `generate_handler!` in `src-tauri/src/lib.rs`; `command_table_covers_every_tauri_command` fails otherwise.
- UI styling uses only tokens from `ui/src/styles/tokens.css` and `DESIGN.md`; shipped CSS stays hex-only, no `oklch()`, `color-mix()`, `light-dark()`, nesting, `@layer`, `:has()`, container queries. A new colour token goes into the light block and both dark blocks (`tokens.test.ts` guards this).
- Files in the data folder are written atomically with a `.bak`. Docs, code and UI text in English.
- Verify the UI in the real app only through the WebView2 DevTools port (CLAUDE.md "Checking the UI in the real app on Windows"); never `SendKeys`, `SetCursorPos` or `mouse_event`. Native dialogs cannot be driven that way: those checks are owner checks and stay open until run.
- SSH container tests: `eval "$(tools/ssh-test-server.sh start)"`, run, then always `tools/ssh-test-server.sh stop`.
- Scope: only M7's tasks. No RDP, no restore-tabs, no native clipboard, no merge of divergent trees.
- Roadmap upkeep: after finishing a roadmap task, set its `"done": true` in `Docs/roadmap.json` at once; acceptance criteria only with recorded proof (`evidence`). Regenerate with `dotnet run tools/roadmap.cs -- generate` at the end of each phase and commit both files.
- "Done" means these are green and stated with their output: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm lint`, `pnpm test`, `cargo deny check`, `dotnet run tools/roadmap.cs -- check`.

## Review Focus

Input classes the spec implies but no roadmap criterion tests; each has its test in the named task.

1. `connections.json` rewritten with identical bytes, deleted, or created after startup (empty store) must behave sensibly: identical bytes and a deleted file do not block a save; a file that appears after an empty start is a change (Task 2).
2. A shell that exits 0 within a few seconds (a failed login banner, `ssh host true` style) must stay open so output is read (Task 10).
3. Multi-select that contains a folder and one of its own descendants must move the folder only, and a target inside any selected subtree must be refused as a whole, never half applied (Tasks 14 and 15).
4. Closing the window while sessions are only `connecting` or `needsVault` closes at once; only `connected` counts (Task 12).
5. A stored panel size from a larger window, or a corrupt `localStorage` value, must never push the tree below its minimum rows or break startup (Task 13).

---

# Phase 1: Rust hardening

### Task 1: Size cap for `connections.json` and `settings.json` reads

Closes the vault Low finding "read without a size cap" and gives Task 2 one capped reader.

**Files:**
- Modify: `crates/fernjo-core/src/persist.rs` (reads in `load`), `crates/fernjo-core/src/settings.rs` (`read_file`), `crates/fernjo-core/src/lib.rs` (export)
- Test: unit tests in `persist.rs` and `settings.rs`

**Interfaces:**
- Produces: `pub const MAX_DATA_FILE_LEN: u64 = 64 * 1024 * 1024;` and `pub(crate) fn read_capped(path: &Path) -> io::Result<Vec<u8>>` in `persist.rs` (errors with `io::ErrorKind::InvalidData` when the file is longer; checks the length before reading, mirrors `fernjo_vault::format::read_capped`).

- [ ] **Step 1: Write the failing tests** `load_refuses_a_file_over_the_cap_as_unreadable` (a sparse or truncated file of `MAX_DATA_FILE_LEN + 1` bytes, `set_len`; expect `LoadError::Unreadable`, file untouched) and `settings_load_refuses_a_file_over_the_cap` (same for `settings::load`).
- [ ] **Step 2: Run** `cargo test -p fernjo-core cap` and expect FAIL.
- [ ] **Step 3: Implement** `read_capped` and use it wherever `persist.rs` and `settings.rs` call `fs::read` on the data file or `.bak`.
- [ ] **Step 4: Run** `cargo test -p fernjo-core` and expect PASS.
- [ ] **Step 5: Commit** `fix(core): cap the size of connections.json and settings.json reads` (files: the three above).

### Task 2: `connections.json` changed on disk is refused, not overwritten

**Files:**
- Modify: `Cargo.toml` (`[workspace.dependencies]`: `sha2 = "0.11.1"`), `crates/fernjo-core/Cargo.toml` (`sha2.workspace = true`), `crates/fernjo-core/src/error.rs`, `crates/fernjo-core/src/persist.rs`, `crates/fernjo-core/src/store.rs`
- Test: unit tests in `store.rs`

**Interfaces:**
- Produces: `pub type Fingerprint = [u8; 32];` in `persist.rs`; `persist::fingerprint(path: &Path) -> io::Result<Option<Fingerprint>>` (SHA-256 of the capped file bytes, `None` when the file is missing); `persist::save` now returns `Result<Fingerprint, SaveError>` (hash of exactly the bytes written, including the trailing newline); `StoreError::ChangedOnDisk` with code `changed_on_disk` and message "connections.json was changed by another program or device since fernjo last read it; reload to see those changes"; `Store::reload(&mut self) -> Result<Option<Notice>, LoadError>` (re-runs `persist::load` on `self.path`, replaces the tree, resets the fingerprint to the file now on disk; on error nothing changes).
- Behaviour: `Store` keeps `fingerprint: Option<Fingerprint>` of the bytes last read or written. `mutate` and `replace_tree` call a private `check_unchanged()` before `persist::save`: a missing file is recreated (fingerprint `None` vs `None`, or file gone), identical bytes pass, any other difference returns `ChangedOnDisk` without writing and without creating a `.bak`. `StoreError::code` maps the new variant. After the restore-from-`.bak` re-save in `open`, the fingerprint is the one `save` returned, or `persist::fingerprint(path).ok().flatten()` when that save failed.

- [ ] **Step 1: Write the failing tests** in `store.rs` (temp dir, `Store::open`, then change the file with `fs::write` behind the store's back):
  - `save_is_refused_when_the_file_changed_on_disk`: after one `create_folder`, write a different valid document to the file; the next `create_folder` is `Err(StoreError::ChangedOnDisk)`, the file still holds the external content, the in-memory tree is unchanged, `.bak` is unchanged.
  - `an_identical_rewrite_is_not_a_change`: rewrite the file with the same bytes; the next save succeeds.
  - `a_deleted_file_is_recreated`: `fs::remove_file`, next save succeeds and the file exists.
  - `a_file_that_appears_after_an_empty_start_counts_as_changed`: open in an empty dir, write a file, then save: `ChangedOnDisk`.
  - `replace_tree_is_checked_too`: `replace_tree` after an external change is `ChangedOnDisk`.
  - `reload_adopts_the_file_and_the_next_save_works`: after an external change, `reload()` shows the external tree and the following `create_folder` succeeds.
  - `changed_on_disk_has_its_stable_code` (`StoreError::ChangedOnDisk.code() == "changed_on_disk"`).
- [ ] **Step 2: Run** `cargo test -p fernjo-core store::` and expect FAIL (variant and method missing).
- [ ] **Step 3: Implement** as specified above; fix every `persist::save` caller the compiler reports.
- [ ] **Step 4: Run** `cargo test --workspace` and expect PASS; `cargo deny check` still ok (no new package, `sha2` 0.11.1 is already locked).
- [ ] **Step 5: Document** in `Docs/DECISIONS.md` a new entry "M7: connections.json fingerprint check" with the rejected alternatives from the spec and the sentence that it is a check, not a lock (a sync tool can still win the race between check and rename).
- [ ] **Step 6: Commit** `feat(core): refuse to overwrite a connections.json that changed on disk` (files: the five Rust/Cargo files, `Cargo.lock`, `Docs/DECISIONS.md`).

### Task 3: Reload command and the "changed on disk" bar

**Files:**
- Modify: `src-tauri/src/commands.rs`, `src-tauri/src/vault_commands.rs` (test `table()`), `src-tauri/src/lib.rs`, `ui/src/lib/types.ts` (`Api.reloadTree`), `ui/src/lib/api.ts`, `ui/src/lib/testing.ts` (fake api), `ui/src/lib/store.svelte.ts`, `ui/src/lib/components/NoticeBar.svelte`
- Test: `src-tauri/src/commands.rs` tests, `ui/src/lib/store.test.ts`, new `ui/src/lib/components/NoticeBar.test.ts`

**Interfaces:**
- Consumes: `Store::reload` (Task 2).
- Produces: `commands::reload_tree(state: &AppState) -> Result<Snapshot, CommandError>` (read-only: no `ensure_writable`; notices from `reload` go into the snapshot); wrapper `handlers::reload_tree`; TS `Api.reloadTree(): Promise<Snapshot>`; `TreeStore.changedOnDisk = $state(false)` and `TreeStore.reload(): Promise<ApiError | null>`.
- Behaviour: in `TreeStore.#run`, an error with code `changed_on_disk` sets `changedOnDisk = true` and does not set `error` (the bar replaces the error bar). `reload()` runs through the queue, applies the snapshot and clears the flag on success. `NoticeBar` shows, while the flag is set, a `role="alert"` bar: "connections.json was changed by another program or device. Your last edit was not saved." with a button "Reload".

- [ ] **Step 1: Write the failing tests:** Rust `reload_tree_returns_the_other_devices_edit` (open `AppState::open(Some(dir))`, rewrite `connections.json` externally, `create_folder` returns code `changed_on_disk`, `reload_tree` returns the external tree, `create_folder` then succeeds); UI `store.test.ts` "changed_on_disk shows the reload state, keeps the error bar empty, and reload clears it"; `NoticeBar.test.ts` "shows the reload bar and the button calls reload".
- [ ] **Step 2: Run** `cargo test -p fernjo-app reload_tree` (package name per `src-tauri/Cargo.toml`) and `pnpm test -- src/lib/store.test.ts src/lib/components/NoticeBar.test.ts`; expect FAIL.
- [ ] **Step 3: Implement** the function, wrapper, `table()` entry (a call that must contain no sentinel), `generate_handler!` entry, the Api method, the fake in `testing.ts`, store and bar.
- [ ] **Step 4: Run** `cargo test -p fernjo-app` (includes the count guard and the no-secret test) and `pnpm test`, `pnpm lint`; expect PASS.
- [ ] **Step 5: Commit** `feat(app): reload connections.json after a change on disk` (the files above).

### Task 4: Settings dialog names the real reason settings are not saved

**Files:**
- Modify: `src-tauri/src/session_commands.rs`, `src-tauri/src/app_settings.rs`, `ui/src/lib/types.ts`, `ui/src/lib/components/SettingsDialog.svelte`, `ui/src/lib/theme.svelte.ts` and `ui/src/lib/terminal-settings.svelte.ts` if they copy `persistent`
- Create: `ui/src/lib/unsaved.ts`
- Test: Rust tests in both command files, `ui/src/lib/components/SettingsDialog.test.ts`, new `ui/src/lib/unsaved.test.ts`

**Interfaces:**
- Produces (Rust, `session_commands.rs`): `pub enum UnsavedReason { Env, SettingsUnusable, NoConfigDir }` serialised camelCase (`env`, `settingsUnusable`, `noConfigDir`); `pub(crate) fn unsaved_reason(state: &AppState) -> Option<UnsavedReason>` with precedence `Env`, then `settings_unusable`, then missing `config_dir`; `persistent(state)` becomes `unsaved_reason(state).is_none()`. `TerminalSettingsInfo` and `ThemeInfo` each gain `unsaved_reason: Option<UnsavedReason>` (JSON `unsavedReason`, `null` when saved).
- Produces (TS): `unsavedNote(reason: UnsavedReason): string` in `unsaved.ts`: `env` "Applies to this run only; settings are not saved while FERNJO_DATA_DIR is set"; `settingsUnusable` "Not saved: settings.json cannot be read or was written by a newer version of fernjo. Fix or remove that file and restart."; `noConfigDir` "Not saved: this device has no settings folder."

- [ ] **Step 1: Write the failing tests:** Rust `unsaved_reason_distinguishes_env_unusable_and_no_config_dir` and `unsaved_reason_is_none_when_saved` (build states as the existing tests in `app_settings.rs` do, set `source = DataDirSource::Env` and `settings_unusable = true` separately), plus both infos carry the field; UI `unsaved.test.ts` for the three strings; `SettingsDialog.test.ts` "an unusable settings.json shows its own note, not the FERNJO_DATA_DIR one" in the theme and the terminal section.
- [ ] **Step 2: Run** the three targets; expect FAIL.
- [ ] **Step 3: Implement** the enum, the field, the helper and replace both hard-coded notes in `SettingsDialog.svelte` (two places) with `unsavedNote(...)`.
- [ ] **Step 4: Run** `cargo test -p fernjo-app`, `pnpm test`, `pnpm lint`; expect PASS.
- [ ] **Step 5: Commit** `fix(app): name the real reason settings are not saved`.

### Task 5: SSH agent sign step gets its own limit

**Files:**
- Modify: `crates/fernjo-ssh/src/params.rs`, `crates/fernjo-ssh/src/session.rs` (`agent_attempt` and its call site)
- Test: unit test in `params.rs`; manual check noted below

**Interfaces:**
- Produces: `ConnectParams::DEFAULT_AGENT_SIGN_TIMEOUT: Duration = Duration::from_secs(120)` and field `pub agent_sign_timeout: Duration` (set by `ConnectParams::new`); `agent_attempt` takes it as an extra `sign_limit: Duration` and uses it for the `authenticate_publickey_with` call. `rsa_hash` keeps the connect timeout.

- [ ] **Step 1: Write the failing test** `new_sets_the_agent_sign_timeout_apart_from_the_connect_timeout` (`p.agent_sign_timeout == DEFAULT_AGENT_SIGN_TIMEOUT` and `> p.connect_timeout`).
- [ ] **Step 2: Run** `cargo test -p fernjo-ssh params` and expect FAIL.
- [ ] **Step 3: Implement** the field and thread it through `agent_attempt`.
- [ ] **Step 4: Run** `cargo test -p fernjo-ssh` and `cargo clippy -p fernjo-ssh --all-targets -- -D warnings`; expect PASS.
- [ ] **Step 5: Record** in the roadmap evidence that the long wait itself (an agent that waits for a touch, `ssh-add -c`) is an owner check on a machine with such an agent: command `ssh-add -c <key>`, connect, wait over 15 s before confirming, expect the login to succeed; leave it unticked until run.
- [ ] **Step 6: Commit** `fix(ssh): give the agent sign step its own, longer timeout`.

### Task 6: Release binaries carry no build paths

**Files:**
- Create: `tools/check-no-build-paths.sh`
- Modify: `.github/workflows/release.yml`, `tools/test-release-scripts.sh`, `Docs/release-checklist.md` (reusable checklist: one line), `Docs/DECISIONS.md`

**Interfaces:**
- Produces: `tools/check-no-build-paths.sh <binary>`: exit 0 when the file contains none of `.cargo/registry`, `.cargo\registry`, `runneradmin`, `/home/runner`, `/Users/runner` (use `grep -a -c`), exit 1 listing the pattern that matched, exit 2 for a missing file.

- [ ] **Step 1: Write the failing test** in `tools/test-release-scripts.sh` in the style of its existing cases: three synthetic files (clean, with `/home/runner/.cargo/registry/x.rs`, with `C:\Users\runneradmin\.cargo\registry\x.rs`) and a missing path; expected exit codes 0, 1, 1, 2.
- [ ] **Step 2: Run** `bash tools/test-release-scripts.sh` and expect the new cases to FAIL.
- [ ] **Step 3: Implement** the script.
- [ ] **Step 4: Edit `release.yml`:** in "Build the installers" export `RUSTFLAGS` with `--remap-path-prefix` for `$GITHUB_WORKSPACE` (to `/build`) and the cargo home (to `/cargo`), keeping any existing `RUSTFLAGS`; on Windows convert both with `cygpath -w` because rustc matches the native form `C:\Users\...`. Add a step after the build running `bash tools/check-no-build-paths.sh` on `target/release/fernjo` (`.exe` on Windows; `target/$TARGET/release/fernjo` when `TARGET` is set). Tag and inputs reach scripts only through `env:`.
- [ ] **Step 5: Run** `bash tools/test-release-scripts.sh` (all pass) and `bash tools/check-workflows.sh` (exit 0).
- [ ] **Step 6: Commit** `ci(release): remap build paths and check the binaries for them`. After the PR exists, the release dry run on the pull request is the proof on a CI-built binary: read its log with `gh run view` and record the run URL as evidence.

### Task 7: Vault Low findings

Source: roadmap block "Vault Low findings". Proposed dispositions (recorded one by one in a `Docs/DECISIONS.md` entry "M7: vault Low findings triage"; flip one only with a reason there):

| Finding | Disposition |
|---|---|
| KDF cost accepted at load (1 GiB, 20 passes, 16 lanes) | Fix: refuse a header whose memory, passes or lanes exceed 4 times the shipped default cost, as `unreadable` naming the limit |
| Fixed temp name, followed through symlinks, no `create_new`, no directory fsync | Fix: random temp name, `create_new`, fsync the directory on Unix |
| Windows keychain item uses Enterprise persistence | Fix if the `keyring` crate version exposes the persistence; otherwise accept with the reason |
| Secret residue in freed buffers | Accept: partial cleanup cannot be proven complete; the vault is unlocked memory anyway |
| `change_password` never upgrades the KDF cost | Fix: re-wrap with the current default cost when the stored cost is lower |
| Header re-encoded from parsed fields | Accept: only matters at `CURRENT_VERSION` 2; the golden test `golden_v1` fails first |
| Public `Secret` derives `Serialize` | Fix: remove the derive if the compiler shows no use |
| `connections.json` / `settings.json` without size cap | Done in Task 1 |

**Files:**
- Modify: `crates/fernjo-vault/src/vault.rs`, `format.rs`, `crypto.rs`, `keystore.rs`, `entry.rs` or wherever `Secret` lives (grep first)
- Test: `crates/fernjo-vault/src/entry_tests.rs`, `keystore_tests.rs`, `generation_tests.rs` style unit tests

- [ ] **Step 1: Write the failing tests**, one per "Fix" row: `load_refuses_a_header_with_an_excessive_kdf_cost` (header built with memory above the cap; vault is `Unusable`, file untouched), `save_never_follows_a_planted_temp_symlink` (Unix only, `#[cfg(unix)]`: a symlink at the old fixed temp name does not receive the vault bytes), `change_password_upgrades_a_weak_cost`, and a compile-time absence check for `Secret` serialisation (a `static_assertions`-free test is fine: keep it as a comment-free removal verified by `cargo test`).
- [ ] **Step 2: Run** `cargo test -p fernjo-vault` and expect the new tests to FAIL.
- [ ] **Step 3: Implement** each fix; keep the golden fixture `v1-no-generation.fernjo` opening.
- [ ] **Step 4: Run** `cargo test -p fernjo-vault`, `cargo clippy -p fernjo-vault --all-targets -- -D warnings`; expect PASS.
- [ ] **Step 5: Write the DECISIONS entry** with the final disposition of all eight rows.
- [ ] **Step 6: Commit** `fix(vault): triage and fix the M6 Low findings`.

### Task 8: SSH Low findings and the `known_hosts` deferred follow-ups

Source: roadmap blocks "SSH Low findings" and "M6 security fixes: deferred follow-ups" (items 1, 4, 5). Proposed dispositions, recorded in "M7: SSH Low findings triage":

| Finding | Disposition |
|---|---|
| `replace` removes only the first matching entry of a hand-edited `h,H` line | Fix |
| `Unknown` plus "trust always" does not replace a different key of the same type in the confirmed-hosts file (follow-up 5) | Fix, same code path |
| `known_hosts` and confirmed-hosts writes use a fixed temp name, no `create_new`, no lock (and concurrent confirmations lose an update, follow-up 1) | Fix: one write helper with a random temp name plus a process-wide mutex around read-modify-write |
| Server keyboard-interactive text and prompt count unbounded in the UI | Fix: cap prompt count and text length where the prompt view is built in `sessions.rs` |
| Key of another algorithm for a stored host reads as a new host; key-type downgrade shows no `previous` (follow-up 4) | Accept: it asks, never trusts silently; the prompt still shows the fingerprint. Reason recorded |
| `@revoked` ignored | Accept (documented since M4) |
| Secrets copied into plain `String`s for the wire | Accept: the strings live only in the call; same reason as the vault residue |
| No host key re-check on re-key (russh) | Accept: upstream behaviour |
| Windows agent pipe squatting | Accept: same as OpenSSH for Windows |

**Files:**
- Modify: `crates/fernjo-ssh/src/known_hosts.rs`, `src-tauri/src/sessions.rs` (prompt caps)
- Test: `known_hosts.rs` unit tests, `src-tauri/src/sessions/tests.rs` or a pure helper test

- [ ] **Step 1: Write the failing tests:** `replace_removes_every_matching_entry_of_a_comma_list_line`, `trust_always_replaces_a_different_key_of_the_same_type`, `concurrent_confirmations_do_not_lose_an_update` (two threads adding different hosts to one file, both present afterwards), `temp_file_is_never_a_fixed_name` (Unix symlink at the old name is not followed), and `ki_prompts_are_capped_in_count_and_length` against a pure helper `fn cap_ki_round(round: KiRound) -> KiPromptSet` (name free, signature fixed in the step; caps 16 prompts, 1024 characters each, 4096 for the instruction).
- [ ] **Step 2: Run** `cargo test -p fernjo-ssh known_hosts` and `cargo test -p fernjo-app ki_prompts`; expect FAIL.
- [ ] **Step 3: Implement** the fixes.
- [ ] **Step 4: Run** `cargo test --workspace` and clippy; expect PASS (the container tests run in CI).
- [ ] **Step 5: Write the DECISIONS entry** with all nine rows.
- [ ] **Step 6: Commit** `fix(ssh): triage and fix the M6 Low findings and known_hosts follow-ups`.

### Task 9: Import Low findings and the remaining M6 follow-ups

Source: roadmap blocks "Import and backup Low findings" and "M6 security fixes: deferred follow-ups" (items 2, 3). Proposed dispositions, recorded in "M7: import Low findings and M6 follow-ups triage":

| Finding | Disposition |
|---|---|
| Unbounded attacker strings in error messages and in backup `createdAt` and `app` | Fix: cut to 256 characters where the message or report is built |
| Export-target guard covers only the data folder | Fix: also refuse the config folder, the device folder and a pending data folder (`export_target_not_allowed`) |
| JSON and backup import paths do not validate control characters or length of names, hosts, descriptions | Fix: reuse the mRemoteNG caps (256 names, 4096 description, host 256 dropped when longer) and strip control characters |
| `import_apply` holds the pending lock across disk work while the page-load hook takes it on the UI thread | Fix: `clear_pending` uses `try_lock` and skips when an apply is in flight (apply clears it itself) |
| Residual unzeroised non-secret copies | Accept: not secrets |
| Restore confirmation could name more than counts | Move back to the backlog: UI wording, no safety effect |
| Vault equal-generation divergence (follow-up 2) | Accept: undetectable without a per-device history; the generation check still catches every rollback |
| Startup notice when `settings.json` holds a `dataDir` containing the device-local folder (follow-up 3) | Fix: new `Notice::DeviceFolderInsideDataDir { path }`, shown by `NoticeBar` |

**Files:**
- Modify: `crates/fernjo-import/src/connections.rs`, `backup.rs`, `report.rs`, `src-tauri/src/import_commands.rs`, `src-tauri/src/state.rs` (`clear_pending`, startup notice), `crates/fernjo-core/src/persist.rs` (`Notice`), `ui/src/lib/types.ts`, `ui/src/lib/components/NoticeBar.svelte`, `Docs/ROADMAP.md` source via `roadmap.json` (backlog entry)
- Test: unit tests in the touched crates, `NoticeBar.test.ts`

- [ ] **Step 1: Write the failing tests:** `long_backup_header_strings_are_cut_in_reports`, `json_import_strips_control_characters_and_caps_lengths`, `export_refuses_config_device_and_pending_folders`, `clear_pending_does_not_block_while_an_apply_holds_the_lock` (hold the pending lock on another thread, `clear_pending` returns promptly), `startup_notice_when_data_dir_contains_the_device_folder`, `notice_serialises_as_deviceFolderInsideDataDir` (JSON shape in the style of the existing notice tests in `persist.rs`), UI `NoticeBar` text for the new kind.
- [ ] **Step 2: Run** `cargo test -p fernjo-import`, `cargo test -p fernjo-app import`, `cargo test -p fernjo-core notice`, `pnpm test -- src/lib/components/NoticeBar.test.ts`; expect FAIL.
- [ ] **Step 3: Implement** the fixes; add the backlog entry for the restore confirmation wording in `Docs/roadmap.json`.
- [ ] **Step 4: Run** `cargo test --workspace`, `pnpm test`, `pnpm lint`; expect PASS.
- [ ] **Step 5: Write the DECISIONS entry** with all rows; tick roadmap tasks "five deferred follow-ups" and "triage of the Low findings" (with Tasks 7 and 8) and the criterion "Every item ... has a recorded outcome" with the entry names as evidence.
- [ ] **Step 6: Commit** `fix(import): triage and fix the M6 Low findings and remaining follow-ups`.

- [ ] **Phase gate:** run the full "done" command list; `dotnet run tools/roadmap.cs -- generate`; commit `docs(roadmap): tick phase 1 of M7`.

---

# Phase 2: Session end and quit confirmation

### Task 10: The session end carries an exit status and a "clean end" verdict

**Files:**
- Modify: `src-tauri/src/sessions.rs` (wire contract doc comment, `UiEvent`, `forward`, the connect task), `crates/fernjo-core/src/settings.rs` (`TerminalPrefs`), `src-tauri/src/session_commands.rs` (`TerminalSettingsInfo`, `TerminalSettingsInput`), `Docs/DECISIONS.md` ("M4 session IPC" gets an addendum entry "M7: session end event")
- Test: `sessions.rs` unit tests, `settings.rs` tests, `session_commands.rs` tests, `crates/fernjo-ssh/tests/connect.rs`

**Interfaces:**
- Produces: `pub const MIN_AUTO_CLOSE_SESSION: Duration = Duration::from_secs(5);` and `pub(crate) struct Ended { pub text: String, pub exit_status: Option<u32>, pub clean: bool }` with `pub(crate) fn classify_end(reason: CloseReason, connected_for: Duration) -> Ended` in `sessions.rs`: `clean` only for `CloseReason::Exit(Some(0))` with `connected_for >= MIN_AUTO_CLOSE_SESSION`; `exit_status` is the code for `Exit(Some(code))` and `None` otherwise; `text` is today's `close_text`.
- `UiEvent::State` gains `exit_status: Option<u32>` and `ended_cleanly: bool` (JSON `exitStatus`, `endedCleanly`; `null`/`false` for every state except an end). `Sink::state(state, detail)` keeps its signature and sets the defaults; a new `Sink::ended(detail: String, exit_status: Option<u32>, ended_cleanly: bool)` sends the disconnect. `forward` returns `Ended`; the connect task measures `connected_for` from the moment it sets `Phase::Connected`; `Shared::disconnected` takes the `Ended`.
- `TerminalPrefs.close_on_exit: Option<bool>` (unset means `DEFAULT_CLOSE_ON_EXIT = true`, exported next to `DEFAULT_FONT_SIZE`); `TerminalSettingsInfo.close_on_exit: bool`; `TerminalSettingsInput.close_on_exit: bool` with `#[serde(default = "...")]` returning `true`. Existing `settings.json` files without the key keep loading.
- The setting is applied by the UI (Task 11); Rust only reports what happened.

- [ ] **Step 1: Write the failing tests:**
  - `classify_end_exit_zero_after_five_seconds_is_clean` (`Exit(Some(0))`, 5 s: `clean`, `exit_status == Some(0)`).
  - `classify_end_exit_zero_within_five_seconds_is_not_clean` (4.9 s; Review Focus 2).
  - `classify_end_nonzero_exit_is_not_clean` (`Exit(Some(3))`, 60 s: not clean, `exit_status == Some(3)`).
  - `classify_end_without_status_is_not_clean` (`Exit(None)`), `classify_end_connection_lost_is_not_clean`, `classify_end_closed_by_user_is_not_clean`.
  - `state_event_json_carries_exit_status_and_ended_cleanly` (serde shape of a disconnected event and a `connected` event).
  - `close_on_exit_defaults_to_true_and_persists` and `settings_without_close_on_exit_still_load` (`session_commands.rs` / `settings.rs`).
  - Container tests in `connect.rs`: `exit_zero_reports_exit_status_zero` (send `exit 0\n`, expect `Closed(CloseReason::Exit(Some(0)))`), `exit_three_reports_exit_status_three`, `a_killed_connection_is_connection_lost` (the test server container is restarted or the TCP connection dropped as `tools/ssh-test-server.sh` allows; if the script has no way to drop one connection, kill the sshd child with `docker exec ... pkill -f "sshd: "` and note the command in the test).
- [ ] **Step 2: Run** `cargo test -p fernjo-app classify_end` and `cargo test -p fernjo-core settings`; expect FAIL. Start the container (`eval "$(tools/ssh-test-server.sh start)"`) and run `cargo test -p fernjo-ssh --test connect exit`; the new container tests fail first only if something is missing, so also confirm they PASS against today's code (the SSH layer already reports the status); they pin the behaviour.
- [ ] **Step 3: Implement** the types, the event change, `forward` and the settings fields.
- [ ] **Step 4: Run** `cargo test --workspace`, then the container tests, then `tools/ssh-test-server.sh stop`; expect PASS.
- [ ] **Step 5: Update the wire contract** doc comment at the top of `sessions.rs` (the `state` event line) and write the DECISIONS addendum (why the verdict is in Rust and the preference in the UI: the setting applies live without capturing it per session).
- [ ] **Step 6: Commit** `feat(app): report the exit status and a clean-end verdict when a session ends`.

### Task 11: The tab closes on a clean shell exit; setting in Settings

**Files:**
- Modify: `ui/src/lib/types.ts` (`SessionEvent` state variant gains `exitStatus: number | null; endedCleanly: boolean`; `TerminalSettings.closeOnExit`), `ui/src/lib/sessions.svelte.ts`, `ui/src/lib/terminal-settings.svelte.ts`, `ui/src/lib/components/SettingsDialog.svelte`, `ui/src/App.svelte` (`DEFAULT_TERMINAL.closeOnExit: true`, third `SessionStore` argument), `ui/src/lib/testing.ts`
- Test: `ui/src/lib/sessions.test.ts`, `ui/src/lib/components/SettingsDialog.test.ts`

**Interfaces:**
- Consumes: the event fields and `closeOnExit` from Task 10.
- Produces: `new SessionStore(api, nameOf, closeOnExit: () => boolean)`. In `#onMessage`, a `state` event with `state === "disconnected"`, `endedCleanly === true` and `closeOnExit()` closes the tab (`this.close(id)`); every other disconnect keeps the yellow tab with Reconnect, as today. The Settings "Terminal" section gets the checkbox "Close the tab when the shell exits" bound to `closeOnExit`.

- [ ] **Step 1: Write the failing tests** in `sessions.test.ts`: `a clean end closes the tab`, `a clean end keeps the tab when the setting is off`, `exit status 3 keeps the tab with Reconnect`, `a lost connection keeps the tab`, `a clean end of a session that never connected closes nothing` (state `disconnected` with `endedCleanly: false`), and in `SettingsDialog.test.ts` `the close-on-exit checkbox saves through setTerminalSettings`.
- [ ] **Step 2: Run** `pnpm test -- src/lib/sessions.test.ts src/lib/components/SettingsDialog.test.ts`; expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `pnpm test`, `pnpm lint`; expect PASS. Tick roadmap criterion "SSH container tests: exit 0 closes the tab..." with evidence: the container test names (Task 10) plus the UI test names above.
- [ ] **Step 5: Commit** `feat(ui): close the tab when the shell exits cleanly`.

### Task 12: Confirm closing the window while a session is connected

**Files:**
- Modify: `src-tauri/src/sessions.rs` (`SessionManager::connected_count`), `src-tauri/src/state.rs` (`pub close_prompt_open: AtomicBool`), `src-tauri/src/lib.rs` (`.on_window_event`)
- Test: `sessions.rs` unit tests

**Interfaces:**
- Produces: `impl SessionManager { pub fn connected_count(&self) -> usize }` counting entries in `Phase::Connected` only; `pub(crate) fn needs_close_confirmation(connected: usize) -> bool` (`connected > 0`).
- Behaviour: in `run()`, `.on_window_event` for the `main` window on `WindowEvent::CloseRequested { api, .. }`: if `needs_close_confirmation(state.sessions.connected_count())` and `close_prompt_open` was not already set (`swap(true)`), call `api.prevent_close()` and open a non-blocking warning dialog with `tauri_plugin_dialog` (`message(...)`, buttons "Close" and "Keep open"), whose callback resets the flag and, when confirmed, calls `window.destroy()`. Text: "N session(s) are still connected. Closing fernjo ends them." Otherwise the close proceeds untouched.

- [ ] **Step 1: Write the failing tests:** `connected_count_counts_only_connected_sessions` (entries in `Connecting`, `NeedsVault`, `Connected`, `Disconnected`: count 1; Review Focus 4; build `Entry` values the way `sessions/tests.rs` does) and `a_confirmation_is_needed_only_with_a_connected_session`.
- [ ] **Step 2: Run** `cargo test -p fernjo-app connected_count`; expect FAIL.
- [ ] **Step 3: Implement** the method, the flag and the handler.
- [ ] **Step 4: Run** `cargo test -p fernjo-app` and clippy; expect PASS.
- [ ] **Step 5: Record the owner check** (cannot be driven without real input): Windows first, then macOS and Linux (blocking-free dialog from the close handler): open a connection, close the window, expect the question; "Keep open" keeps the app; "Close" ends it; with no connected session the window closes at once. Leave the criterion "Manual: closing the window with a connected session asks first" unticked until run; add the check to the roadmap block "Open manual checks (owner)".
- [ ] **Step 6: Commit** `feat(app): confirm closing the window while a session is connected`.

- [ ] **Phase gate:** full "done" list; roadmap generate; commit `docs(roadmap): tick phase 2 of M7`.

---

# Phase 3: Panels and multi-select

### Task 13: Resizable sidebar and properties panel

**Files:**
- Create: `ui/src/lib/layout.svelte.ts`, `ui/src/lib/components/Splitter.svelte`
- Modify: `ui/src/App.svelte` (grid columns from the store), `ui/src/lib/components/Sidebar.svelte` (vertical splitter at the right edge, `position: relative`), `ui/src/lib/components/PropertiesPanel.svelte` (height from the store, horizontal splitter above it)
- Test: new `ui/src/lib/layout.test.ts`, new `ui/src/lib/components/Splitter.test.ts`, `ui/src/App.test.ts`

**Interfaces:**
- Produces: `export const SIDEBAR_DEFAULT = 300; SIDEBAR_MIN = 240; SIDEBAR_MAX = 560;` and `export function clampSidebarWidth(value: number, viewportWidth: number): number` (never above `min(SIDEBAR_MAX, viewportWidth / 2)`, never below `SIDEBAR_MIN`); `export function clampPanelHeight(value: number, available: number, treeMin: number): number` (at least 26, the header; at most `available - treeMin`, where `available` is the sidebar height minus fixed chrome and `treeMin` the tree-slot minimum from `Sidebar.svelte`'s `min-height`); `export class LayoutStore { sidebarWidth = $state(SIDEBAR_DEFAULT); propertiesHeight = $state<number | null>(null); setSidebarWidth(px: number): void; setPropertiesHeight(px: number | null): void; reset...}` persisted under the key `fernjo.layout.v1` in `localStorage`, every read and write in try/catch, a corrupt or non-numeric value ignored (Review Focus 5). `null` height means today's content-sized panel.
- `Splitter.svelte` props: `orientation: "vertical" | "horizontal"`, `value: number`, `min: number`, `max: number`, `label: string`, `onchange: (px: number) => void`, `onreset: () => void`. It renders `role="separator"` with `aria-orientation`, `aria-valuenow/min/max`, `aria-label`, `tabindex="0"`; pointer drag with pointer capture; arrow keys on its axis move by 16 px (Shift: 64), Home and End jump to `min` and `max`; double-click calls `onreset`. Colours only from existing tokens (the hover/focus colour is the token `DESIGN.md` names for focus rings).

- [ ] **Step 1: Write the failing tests:** `layout.test.ts`: `clampSidebarWidth keeps the width between the minimum and half the window`, `clampPanelHeight leaves the tree its minimum rows (23 rows at 1280x800)` (numbers: viewport height 800 and `--row-height-tree` 24 px), `a corrupt stored value falls back to the defaults`, `sizes survive a new LayoutStore (restart)`, `blocked storage does not throw`; `Splitter.test.ts`: `ArrowRight grows a vertical separator by 16 and Shift by 64`, `Home and End jump to min and max`, `aria values reflect the props`, `double click calls onreset`, `pointer drag reports the new size`; `App.test.ts`: `the shell grid uses the stored sidebar width`.
- [ ] **Step 2: Run** `pnpm test -- src/lib/layout.test.ts src/lib/components/Splitter.test.ts src/App.test.ts`; expect FAIL.
- [ ] **Step 3: Implement** store, splitter and wiring; the default must reproduce today's layout exactly when nothing is stored (`var(--sidebar-width)` stays the token's 300 px default).
- [ ] **Step 4: Run** `pnpm test`, `pnpm lint`; expect PASS.
- [ ] **Step 5: Check in the real app** (DevTools-port recipe, scratch `FERNJO_DATA_DIR` with `cargo run -p fernjo-core --example generate_tree -- <dir> 500`): at 1280x800 the tree shows at least 23 rows with the largest allowed properties height; keyboard resize through `Input.dispatchKeyEvent`; close the window (not `Stop-Process`) and restart: sizes are restored. Record the numbers as evidence for the criterion "Manual: both panels resize with mouse and keyboard and keep their size after a restart".
- [ ] **Step 6: Commit** `feat(ui): resizable sidebar and properties panel`.

### Task 14: Selection model in the tree store

**Files:**
- Modify: `ui/src/lib/store.svelte.ts`, `ui/src/lib/rows.ts` (helper `selectionTops`)
- Test: `ui/src/lib/store.test.ts`, `ui/src/lib/rows.test.ts`

**Interfaces:**
- Produces on `TreeStore`: `selectedIds = new SvelteSet<string>()`, `anchorId = $state<string | null>(null)`, `selectOnly(id: string | null): void` (`select` keeps its current behaviour by delegating to it), `toggleSelect(id: string): void` (Ctrl or Cmd), `selectRange(id: string): void` (Shift: the visible rows between `anchorId` and `id`, in `rows` order, so collapsed children and filtered-out rows are not included), `isSelected(id: string): boolean`, `selectionTops(): string[]`. `selectedId` remains the single primary node that drives `detail` and the properties panel; `detail` is loaded only while `selectedIds.size === 1` and is `null` otherwise. `#apply` prunes `selectedIds` and `anchorId` of ids that no longer exist.
- `rows.ts`: `export function selectionTops(rows: Row[], selected: ReadonlySet<string>): string[]` returns the selected ids whose ancestors are not selected, in row order.

- [ ] **Step 1: Write the failing tests:** `selectOnly replaces the selection`, `toggleSelect adds and removes one id and keeps the anchor sensible`, `selectRange selects the visible rows between anchor and target in either direction`, `selectRange skips children of collapsed folders`, `selectionTops drops a node whose ancestor is selected` (folder plus its child: only the folder; Review Focus 3), `deleting nodes prunes the selection`, `detail is cleared while several nodes are selected`.
- [ ] **Step 2: Run** `pnpm test -- src/lib/store.test.ts src/lib/rows.test.ts`; expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `pnpm test`; expect PASS.
- [ ] **Step 5: Commit** `feat(ui): selection model for several tree nodes`.

### Task 15: Atomic batch move in Rust

**Files:**
- Modify: `crates/fernjo-core/src/tree.rs`, `crates/fernjo-core/src/store.rs`, `src-tauri/src/commands.rs`, `src-tauri/src/vault_commands.rs` (test `table()`), `src-tauri/src/lib.rs`
- Test: `tree.rs` and `store.rs` unit tests, `commands.rs` tests

**Interfaces:**
- Produces: `Tree::move_nodes(&mut self, ids: &[NodeId], new_parent: Option<NodeId>, index: usize) -> Result<(), TreeError>`; `Store::move_nodes(&mut self, ids: &[NodeId], new_parent: Option<NodeId>, index: usize) -> Result<(), StoreError>` (through `mutate`, which already discards a failed attempt, so the batch is atomic); `commands::move_nodes(state, ids: Vec<NodeId>, parent_id: Option<NodeId>, index: usize) -> Result<Mutation, CommandError>` with wrapper `handlers::move_nodes` (JSON argument names `ids`, `parentId`, `index`).
- Semantics: duplicates in `ids` are ignored; an id that lies inside another id's subtree is skipped (it travels with its ancestor); the moved nodes keep the order of `ids` and are inserted contiguously; `index` counts among the new parent's children after all moved nodes were taken out (`index == len` appends), the same rule `move_node` documents; a target that is, or lies inside, any moved subtree is `InvalidMove` for the whole batch; a non-folder target is `NotAFolder`; an unknown id is `NotFound`; depth is checked for every moved subtree (`TooDeep`, same-parent reorder never refused for depth); an empty `ids` is `Ok(())` with no change. Nothing changes on error.

- [ ] **Step 1: Write the failing tests** (`tree.rs`): `move_nodes_moves_several_nodes_in_order`, `move_nodes_skips_a_descendant_of_another_selected_node` (Review Focus 3), `move_nodes_refuses_a_target_inside_a_moved_subtree_and_changes_nothing`, `move_nodes_into_the_same_parent_reorders_contiguously`, `move_nodes_index_counts_after_removal`, `move_nodes_refuses_too_deep_for_any_subtree`, `move_nodes_empty_is_a_no_op`, `move_nodes_unknown_id_is_not_found`; `commands.rs`: `move_nodes_returns_the_full_snapshot` and `move_nodes_error_leaves_the_file_unchanged`.
- [ ] **Step 2: Run** `cargo test -p fernjo-core move_nodes`; expect FAIL.
- [ ] **Step 3: Implement** by validating everything first, then taking out and inserting; reuse `check_depth`, `take`, `insert_unchecked`.
- [ ] **Step 4: Add** the `table()` entry and `generate_handler!` entry; run `cargo test --workspace` (count guard and no-secret test included) and clippy; expect PASS.
- [ ] **Step 5: Commit** `feat(core,app): atomic batch move of several tree nodes`.

### Task 16: Multi-select in the tree view: gestures, batch drag, delete confirmation

**Files:**
- Modify: `ui/src/lib/types.ts` and `ui/src/lib/api.ts` (`Api.moveNodes(ids, parentId, index)`), `ui/src/lib/testing.ts`, `ui/src/lib/store.svelte.ts` (`moveMany`, `removeMany`), `ui/src/lib/drop-target.ts` (`resolveDropMany`), `ui/src/lib/drag.ts`, `ui/src/lib/components/TreeView.svelte`, `ui/src/lib/components/TreeRow.svelte`, `ui/src/lib/components/PropertiesPanel.svelte` ("N items selected")
- Test: `ui/src/lib/drop-target.test.ts`, `ui/src/lib/drag.test.ts`, `ui/src/lib/components/TreeView.test.ts`, `TreeView.drag.test.ts`, `ui/src/lib/store.test.ts`

**Interfaces:**
- Consumes: `selectionTops`, `selectOnly`, `toggleSelect`, `selectRange` (Task 14); `move_nodes` command (Task 15).
- Produces: `resolveDropMany(rows: Row[], dragIds: string[], targetId: string, position: DropPosition): Drop | null` (the target must not be a dragged node or lie inside a dragged subtree; `index` counts the target parent's remaining children, with every dragged node removed, before the insertion point; `null` when the result is where the nodes already are); `DragOptions.onMove` becomes `(dragIds: string[], parentId: string | null, index: number) => void`, `DragController.dragIds: string[]`; `pointerDown(id, x, y, ids?: string[])` drags the whole selection when the pressed row is part of a selection of more than one, otherwise only that row; `TreeStore.moveMany(ids, parentId, index)` and `TreeStore.removeMany(ids)` (the latter deletes `selectionTops` in order through the queue).
- Gestures: click selects only; Ctrl/Cmd+click toggles; Shift+click selects the range; Ctrl/Cmd+A is not added. Delete with several selected opens `ConfirmDialog` titled "Delete N items?" with a detail naming how many connections and folders are inside; one selected keeps today's text. The context menu on a selected row of a multi-selection offers Delete only; Rename and Duplicate need a single node.

- [ ] **Step 1: Write the failing tests:** `resolveDropMany` cases (inside a folder, before and after, a target inside a dragged subtree is `null`, a target that is itself dragged is `null`, same-parent contiguous reorder, no-op is `null`); `drag.test.ts` `dragging a selected row moves the whole selection` and `dragging an unselected row moves only that row`; `TreeView.test.ts` `ctrl click toggles`, `shift click selects the range`, `deleting several asks first and names the count`, `escape cancels the confirmation`; `store.test.ts` `moveMany sends one batch command and applies the snapshot`.
- [ ] **Step 2: Run** the five test files; expect FAIL.
- [ ] **Step 3: Implement.** Keyboard: Shift+Arrow extends the selection like the platform tree convention.
- [ ] **Step 4: Run** `pnpm test`, `pnpm lint`; expect PASS. Tick the criterion "Tests: multi-select store, batch move command ... and drag" with the test names (Tasks 14, 15, 16) as evidence.
- [ ] **Step 5: Check on Windows in the real app** via the DevTools port (`Input.dispatchMouseEvent` with modifiers): select three rows with Ctrl, drag them into a folder, delete two with the confirmation; note the result as evidence for "manual check on Windows".
- [ ] **Step 6: Commit** `feat(ui): multi-select, batch drag and confirmed batch delete in the tree`.

- [ ] **Phase gate:** full "done" list; roadmap generate; commit `docs(roadmap): tick phase 3 of M7`.

---

# Phase 4: Discoverability (owner gate in the middle)

### Task 17: Discoverability review with real screenshots

**Files:**
- Create: `Docs/design/m07-discoverability-review.md` and screenshots under `Docs/design/m07-review/` (PNG, small, one per state)

- [ ] **Step 1: Capture the states** with the DevTools recipe on a scratch `FERNJO_DATA_DIR`: first run (empty tree, no vault), tree with connections and the properties panel, vault locked, vault unlocked, vault manager, Settings, Import & backup, a host key prompt, a disconnected tab, a session tab; light and dark (`Emulation.setEmulatedMedia`). `Page.captureScreenshot` for each.
- [ ] **Step 2: Run the critique** with the `impeccable` skill (command `critique`), constrained by `DESIGN.md` and the tokens: no new visual direction. Cover the roadmap's list: the vault chip's label and wording, the path from a connection's "Credential" property to the vault manager, where Import & backup, Export and Settings are and how they are named, empty states, first-run guidance, icon-only buttons, properties panel wording, context menu and shortcut discoverability, confusing states.
- [ ] **Step 3: Write the findings** as a short numbered list in the review file: each item with the screenshot, what is unclear, the proposed change, and its size (S/M).
- [ ] **Step 4: Commit** `docs(design): discoverability review of the whole app`.

### Task 18: Owner gate

- [ ] **Step 1: Stop.** Hand the owner the list from Task 17 and ask which items to implement, change or decline. Do not start Task 19 before the answer.
- [ ] **Step 2: Record** the answer in `Docs/roadmap.json` as a block "M7 discoverability review: agreed list" (every item marked implement, change or declined with the owner's words) and as a "Scope changes" line if the list changes the milestone's scope. Commit `docs(roadmap): record the agreed discoverability list`.

### Task 19: Implement the agreed discoverability changes

The items are not known before the gate, so this task is expanded then, one sub-task per agreed item, in the same step style (failing test, run, implement, run, commit). Known anchors from the roadmap: a "Vault" label on the sidebar chip and clear locked/unlocked wording (`VaultChip.svelte`, `VaultChip.test.ts`); a first-run hint where the credential is chosen and a link from the "Credential" property to the vault manager (`PropertiesPanel.svelte`, `VaultManager.svelte`); labelled footer actions for Settings, Import & backup and Export (`Sidebar.svelte`); empty-state and first-run text in `App.svelte` and `TreeView.svelte`.

- [x] **Step 1: Expand** this task into sub-tasks from the agreed list and append them here. Owner's answer on 2026-11-20: "implement all of them" (D1-D13, none changed, none declined). The exact wording, files and screenshots of each item are in `Docs/design/m07-discoverability-review.md`; a sub-task implements the proposed change there, and the review file wins where this list is shorter. Sub-tasks run in this order (they share files, so strictly one after the other):
  - **19.1 (D1, D2, D7) vault wording:** `VaultChip.svelte` ("Vault: unlocked / locked / none", `title`), `VaultManager.svelte` (title "Vault", list heading and empty state), `VaultNote.svelte`, `NeedsVaultPrompt.svelte` (note body, prompt caption and heading, always-visible auto-unlock hint) and their tests.
  - **19.2 (D3) Credential row:** `PropertiesPanel.svelte` gets "Create vault" / "Unlock" / "Manage…" next to the value and a last dropdown entry "New credential…" that opens the manager on a new entry; `App.svelte` and `Sidebar.svelte` pass the open-vault and focus-form callbacks down; tests.
  - **19.3 (D4) footer:** `Sidebar.svelte` footer buttons get text ("Settings", "Import / Export"); `ImportDialog.svelte` title, `aria-label` and `title` become "Import, export & backup"; tests.
  - **19.4 (D5) properties panel:** raise the default height or reorder the rows as the review proposes so Name, Host and Credential are visible at 1280x800 without losing the 23-row tree guarantee (`PropertiesPanel.svelte`, `layout.svelte.ts`, tests).
  - **19.5 (D6, D13) first run and opening:** `App.svelte` and `TreeView.svelte`: numbered start list while the tree is empty, "Double-click a connection (or press Enter) to open it." otherwise; tests.
  - **19.6 (D8, D11) Settings:** `SettingsDialog.svelte` ("Save terminal settings", the live-versus-saved hint, a read-only platform-correct "Keyboard and mouse" section), `FilterBox.svelte` tooltip, `shortcuts.ts` exports the list; tests.
  - **19.7 (D9) dropped tab:** `TabStrip.svelte` and `SessionArea.svelte`: the disconnected tab keeps its title, the reason becomes a one-line bar above the terminal with Reconnect; tests.
  - **19.8 (D10, D12) hints:** `ContextMenu.svelte` (and `TreeView.svelte`) right-aligned shortcut hints (Connect Enter, Rename F2, Delete Del); `HostKeyPrompt.svelte` (and the synced prompt if it has the same buttons) gets the explanation line; tests.
  Each sub-task: failing component test, implement with tokens only, `pnpm test`, `pnpm lint`, screenshot of the changed state in light and dark through the DevTools port into the scratchpad (not the repo), one commit `feat(ui): <items>`.
- [ ] **Step 2: For each sub-task:** write the failing component test, run it, implement with tokens only, run `pnpm test` and `pnpm lint`, check the changed state in the real app by screenshot (DevTools port), commit `feat(ui): <item>`.
- [ ] **Step 3: Tick** the criterion "Manual, owner: the owner finds the vault, Import & backup and Settings without a hint" only after the owner ran it; until then it stays open with the check described in the roadmap block "Open manual checks (owner)". Each declined item is recorded as declined.
- [ ] **Phase gate:** full "done" list; roadmap generate; commit.

---

# Phase 5: Platform checks, docs and finish

### Task 20: Linux title bar icon

**Files:**
- Modify: `src-tauri/src/lib.rs` (`setup` with an explicit window icon), `src-tauri/Cargo.toml` (the `tauri` feature `image-png` that `tauri::include_image!` needs), `README.md` (fallback sentence)

- [ ] **Step 1: Implement the cheap candidate fix** as its own commit: in `setup`, set the main window's icon from `tauri::include_image!("icons/32x32.png")` (the file `Docs/ROADMAP.md` names as the correct fernjo icon).
- [ ] **Step 2: Build** `cargo build --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`; expect PASS.
- [ ] **Step 3: Owner check on the CachyOS laptop** with the AppImage from the pull request's dry-run artifact: note the desktop environment and session type, then look at the title bar. If the placeholder is gone, tick the task with that evidence. If it is unchanged, revert the commit of Step 1 (`git revert`, no history rewrite) and add one sentence to the README known limits: an unintegrated AppImage may show a generic title bar icon under Wayland while the taskbar icon is right. Either way record the outcome in the roadmap and `Docs/release-checklist.md` gap 27.
- [ ] **Step 4: Commit** `fix(app): set the window icon explicitly` or `docs: note the Linux title bar icon limit`.

### Task 21: The WebView2 background connection

**Files:**
- Modify: `README.md` (privacy statement), `Docs/DECISIONS.md` ("M7: WebView2 background connection"), `Docs/roadmap.json`

- [ ] **Step 1: Measure on Windows** with a `pnpm tauri build --no-bundle` binary on a scratch `FERNJO_DATA_DIR`: for three minutes sample `Get-NetTCPConnection` for the `msedgewebview2.exe` processes that belong to fernjo (parent `fernjo.exe`), write down remote addresses with their reverse DNS or owner, and note the time of each new connection.
- [ ] **Step 2: Research** with `WebSearch` what the WebView2 runtime contacts in the background (Microsoft's documentation on WebView2 data and telemetry, `--disable-features` options) and test one candidate switch at a time through `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` (the documented ones only), repeating Step 1 for each.
- [ ] **Step 3: Decide** with the evidence: a setting that stops it is applied (`additionalBrowserArgs` in the window configuration of `tauri.conf.json`, with a `tauri_conf` test pinning the value), otherwise the limit is documented. macOS (WKWebView) and Linux (WebKitGTK) cannot be measured here: add them to the roadmap block "Open manual checks (owner)" with the command `lsof -i -n -P | grep -i fernjo` (macOS) and `ss -tnp | grep -i fernjo` (Linux).
- [ ] **Step 4: Update** the README privacy statement to say exactly what was found (and what was not checked) and write the DECISIONS entry.
- [ ] **Step 5: Run** `cargo test -p fernjo-app --test tauri_conf` if the configuration changed; expect PASS.
- [ ] **Step 6: Commit** `docs: record what the WebView2 runtime contacts`.

### Task 22: Docs, roadmap, final proof and pull request

**Files:**
- Modify: `CLAUDE.md` ("Project-specific": the fingerprint check in `Store`, the `changed_on_disk` reload flow, the session end event fields and `closeOnExit` ownership, the close confirmation handler, the layout store, the multi-select store and `move_nodes`), `CHANGELOG.md` (`[Unreleased]`), `Docs/roadmap.json`, `Docs/ROADMAP.md` (generated), `Docs/release-checklist.md` (reusable checklist addition from Task 6)

- [ ] **Step 1: Update** `CLAUDE.md` and `CHANGELOG.md` `[Unreleased]` with the user-facing changes (data-safety refusal and reload, quit confirmation, close on clean exit and its setting, resizable panels, multi-select, the agreed discoverability changes, the agent timeout).
- [ ] **Step 2: Ticks.** Every roadmap task done gets `"done": true`; each acceptance criterion gets `"done": true` only with evidence (command, result, or run URL). Criteria that are still owner checks (quit confirmation on all platforms, the discoverability walk-through, the agent wait) stay open and are listed in "Open manual checks (owner)". Set the milestone status per the roadmap rules.
- [ ] **Step 3: Run the full gate** and state each result: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, the SSH container tests (start, `cargo test -p fernjo-ssh`, stop), `pnpm lint`, `pnpm test`, `pnpm ui:build`, `cargo deny check`, `bash tools/test-release-scripts.sh`, `bash tools/check-workflows.sh`, `dotnet run tools/roadmap.cs -- generate` then `-- check`.
- [ ] **Step 4: Open the PR** to `dev` (`gh pr create --base dev`), wait for CI (`rust`, `ui`, `tauri-build`, `cargo-deny`, `workflows`, `roadmap`, the release dry run, CodeQL), read the runs with `gh run view`, record the URLs as evidence, and merge when everything is green (squash allowed). The release `v0.1.1` is a separate step on the owner's instruction.
- [ ] **Step 5: Commit** `docs: finish M7 documentation and roadmap evidence` (one commit before the PR; later evidence commits as needed).
