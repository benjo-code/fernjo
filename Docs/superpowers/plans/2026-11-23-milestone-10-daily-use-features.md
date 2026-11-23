# Milestone 10: Daily-use features Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Quick connect, restore tabs, vault auto-lock, Pageant, the Cmd+Q quit confirmation, native paste, clearer restore wording, a correct splitter focus style and the real M4 review bugs.

**Architecture:** Logic stays in Rust (`fernjo-core` for the quick-target parser and the new settings, `src-tauri` for commands, session targets, the idle lock and the open-tabs file, `fernjo-ssh` for Pageant and the host-key write order); the UI only mirrors state. A session's target becomes `Node(NodeId)` or `Quick(QuickTarget)`, which quick connect and restore tabs both use. Tasks are ordered small and independent first (1 to 7, auto-lock included), then quick connect (8, 9), then restore tabs (10, 11, it reuses the quick target), then native paste (12) and the close-out (13).

**Tech Stack:** Rust 2024 workspace, Tauri 2, russh 0.65 (`connect_pageant`), `arboard` (new, text only), Svelte 5 + TypeScript, vitest.

**Spec:** [Docs/superpowers/specs/2026-11-23-m10-daily-use-features-design.md](../specs/2026-11-23-m10-daily-use-features-design.md). Executors read both. Branch: `feature/m10-daily-use-features` (already created from `dev`).

## Global Constraints

- Secrets stay in Rust; nothing secret in `connections.json`, `settings.json`, `open-tabs.json`, logs, sync data or webview storage. `no_response_contains_a_secret` (`src-tauri/src/vault_commands.rs`) covers every new command.
- Adding a `#[tauri::command]` updates three places: the plain function plus its `handlers` wrapper, its entry in the test `table()` (or `APP_HANDLE_EXEMPT` with a reason), the `generate_handler!` list in `src-tauri/src/lib.rs`. A new source file with commands is added to the test's `sources` (`include_str!`). `command_table_covers_every_tauri_command` must stay green.
- UI uses only tokens from `ui/src/styles/tokens.css` and `DESIGN.md`; CSS stays hex-only (no `oklch()`, `color-mix()`, nesting, `:has()`); `ui/src/styles/tokens.test.ts` guards it.
- Reads of data or device folder files go through `persist::read_capped` with `fernjo_core::MAX_DATA_FILE_LEN`.
- Plain Ctrl+letter and Alt+letter belong to the shell; app chords are Ctrl+Shift+letter (Cmd+letter on macOS).
- Quick connect to RDP: clipboard **off**. Restored tabs come back **disconnected**; "Reconnect all" never raises more than one unlock prompt. Pageant is chosen once per connect (OpenSSH pipe first).
- Commits are Conventional Commits, no attribution lines; stage files by name. Work only inside M10.
- Quality gate at the end (all green): `cargo build --workspace`, `cargo test --workspace`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo deny check`, `pnpm test`, `pnpm lint`, `dotnet run tools/roadmap.cs -- check`.

## Review Focus

Inputs and conditions the spec implies but no feature test names directly (each is pinned by a test in the owning task, named there):

1. Odd quick-connect input: surrounding whitespace, `user@@host`, `ssh://` alone, port 0 or 65536, a host with `/`, `;` or a space, a missing user for SSH. Expected: a clear error, nothing connects, nothing reaches `known_hosts` (Task 8).
2. A damaged, empty, oversized or newer-schema `open-tabs.json`. Expected: the app starts, no tabs are restored, the next change rewrites a valid file (Task 10).
3. A saved tab whose node was deleted, became a folder, or changed protocol since the last run. Expected: deleted or folder entries are dropped; a protocol change uses the current protocol (Task 10).
4. Idle lock while the user works only in a terminal or a remote desktop, a limit changed while unlocked, a limit out of range. Expected: no lock while input arrives; the new limit applies at once; an invalid value is refused (Task 7).
5. Cmd+Q while a quit dialog is already open, and an explicit app exit (restart, update). Expected: one dialog only; an explicit exit with a code is never intercepted (Task 5).

---

### Task 1: Splitter focus style

**Files:**
- Modify: `ui/src/lib/components/Splitter.svelte`
- Test: `ui/src/lib/components/Splitter.test.ts`

**Interfaces:** Produces nothing other tasks use.

- [ ] **Step 1: Write the failing test** in `Splitter.test.ts`: `focus_style_is_a_one_pixel_inset_line_and_the_hit_area_clears_the_scrollbar` reads the component's source style (as the existing tests of CSS rules do, if any; else assert on the `<style>` text via `?raw` import): the focused rule uses `box-shadow: inset` or `outline` with `1px` and `var(--color-focus)`, not a `background` fill; the vertical splitter's `right` offset is not `-3px`.
- [ ] **Step 2: Run** `pnpm test -- Splitter` → FAIL.
- [ ] **Step 3: Implement** in `Splitter.svelte`: keyboard focus (`:focus-visible`) draws a 1 px inset line in `--color-focus` centred in the 6 px hit area; hover and drag keep `--color-focus`; the vertical splitter's hit area moves inward (for example `right: 0` with the visual line kept at the edge) so it does not cover the tree's scrollbar edge. Behaviour (arrow keys, pointer capture, double-click reset) unchanged; existing Splitter tests stay green.
- [ ] **Step 4: Run** `pnpm test -- Splitter` and `pnpm test -- tokens` → PASS.
- [ ] **Step 5: Commit** `git add ui/src/lib/components/Splitter.svelte ui/src/lib/components/Splitter.test.ts && git commit -m "fix(ui): splitter focus is a 1 px inset line and clears the scrollbar"`

### Task 2: Restore confirmation wording

**Files:**
- Modify: `ui/src/lib/components/ImportDialog.svelte`
- Test: `ui/src/lib/components/ImportDialog.test.ts`

**Interfaces:** Consumes nothing new.

- [ ] **Step 1: Read** `apply_backup` in `src-tauri/src/import_commands.rs` and "M5 restore semantics and backup container" in `Docs/DECISIONS.md` and write down exactly which data a restore replaces (connection tree, vault entries) and which it keeps (for example `known_hosts`, settings, sync state, the vault password and data key); use only what the code does.
- [ ] **Step 2: Write the failing test** `restore_confirmation_names_what_is_replaced_and_what_is_kept` in `ImportDialog.test.ts`: with a backup picked, the confirmation text contains the node count (as today), the words for the replaced items and a "Kept:" sentence naming the kept items from step 1.
- [ ] **Step 3: Run** `pnpm test -- ImportDialog` → FAIL.
- [ ] **Step 4: Implement** the text in `ImportDialog.svelte` (UI wording only).
- [ ] **Step 5: Run** `pnpm test -- ImportDialog` → PASS. **Commit** `git add ui/src/lib/components/ImportDialog.svelte ui/src/lib/components/ImportDialog.test.ts && git commit -m "feat(ui): restore confirmation names what is replaced and kept"`.

### Task 3: Pageant on Windows

**Files:**
- Modify: `crates/fernjo-ssh/src/agent.rs`, `crates/fernjo-ssh/src/session.rs` (call site of `agent::connect`), `README.md` (line "Pageant is not supported yet" and the agent paragraph)
- Test: `crates/fernjo-ssh/src/agent.rs` (`mod tests`)

**Interfaces:**
- Produces: `pub(crate) async fn first_with_identities<T, F, Fut>(sources: Vec<F>) -> Option<T> where F: FnOnce() -> Fut, Fut: Future<Output = Option<(T, usize)>>` — runs the sources in order, stops at the first that returns `Some((agent, n))` with `n > 0`; when none has identities it returns the first connected one; `None` when none connected. `connect()` keeps its signature.

- [ ] **Step 1: Write failing tests** (tokio `#[tokio::test]`, sources are closures returning `Some((label, count))`): `first_source_with_identities_wins` (pipe 1, pageant 2 → pipe), `pageant_is_used_when_the_pipe_is_empty` (pipe 0, pageant 2 → pageant), `pageant_is_used_when_the_pipe_is_unavailable` (None, 1 → pageant), `an_empty_agent_is_still_returned_when_nothing_has_identities` (pipe 0, pageant 0 → pipe), `a_later_source_is_not_started_when_an_earlier_one_has_identities` (counter closure not called).
- [ ] **Step 2: Run** `cargo test -p fernjo-ssh agent` → FAIL (function missing).
- [ ] **Step 3: Implement** `first_with_identities`, and in the Windows `connect()` call it with the pipe source (`AgentClient::connect_named_pipe(...)`, then `request_identities().len()`) and the Pageant source (`AgentClient::connect_pageant()`), both type-erased with `AgentClient::dynamic`. Unix and other platforms unchanged. Update README: Pageant is supported on Windows, tried after the OpenSSH pipe; if both hold keys only the OpenSSH agent is used.
- [ ] **Step 4: Run** `cargo test -p fernjo-ssh agent` and `cargo clippy -p fernjo-ssh --all-targets -- -D warnings` → PASS. A real Pageant is an owner check (Task 12 records it).
- [ ] **Step 5: Commit** `feat(ssh): use Pageant as the Windows agent when the OpenSSH pipe has no keys`.

### Task 4: M4 follow-ups, Rust half (host key write order, input queue cap)

**Files:**
- Modify: `crates/fernjo-ssh/src/handler.rs`, `crates/fernjo-ssh/src/session.rs`, `src-tauri/src/session_flow.rs`, `src-tauri/src/sessions.rs`, `src-tauri/src/sessions/rdp.rs`
- Test: `crates/fernjo-ssh/src/handler/tests.rs`, `src-tauri/src/session_flow.rs` (`mod tests`), `src-tauri/src/sessions/tests.rs`

**Interfaces:**
- Produces: in `handler.rs` `pub(crate) type PendingTrust = Arc<Mutex<Option<TrustAction>>>` and `pub(crate) enum TrustAction { Add(PublicKey), Replace(PublicKey) }` plus `ClientHandler::apply_pending_trust(&self) -> Result<(), SshError>`; in `session_flow.rs` `pub const MAX_QUEUED_INPUT: usize = 8 * 1024 * 1024;` and `pub struct InputBudget` with `pub fn try_reserve(&self, bytes: usize) -> bool` and `pub fn release(&self, bytes: usize)` (an `AtomicUsize`).

- [ ] **Step 1: Verify each item is still real** (first read `handler.rs`, `sessions.rs` `Op` queue): the `TrustAlways`/`Replace` write happens inside `check_server_key` before the key-exchange signature is verified; the `Op` channel is `unbounded_channel`. Note any that M7 already fixed and skip that part.
- [ ] **Step 2: Write failing tests.** handler: `trust_always_writes_known_hosts_only_after_the_handshake` (after `decide` returns true the file is unchanged; after `apply_pending_trust` it holds the key) and `a_failed_handshake_leaves_known_hosts_unchanged` (decision made, `apply_pending_trust` never called, file unchanged; same for `Replace`); the device-confirmed file follows the same rule. session_flow: `input_budget_refuses_above_the_cap_and_recovers_after_release`. sessions: `input_over_the_cap_is_refused_with_input_queue_full` (error code `input_queue_full`, the session stays alive).
- [ ] **Step 3: Run** `cargo test -p fernjo-ssh handler` and `cargo test -p fernjo-app` filtered by the new names → FAIL.
- [ ] **Step 4: Implement.** Handler parks the decision in `PendingTrust` and returns true; `Session::connect` calls `apply_pending_trust` right after `connect_stream` succeeds (a failure there is an `SshError`, same as today). `SessionManager::input` and the RDP input path reserve before sending and the writer releases after handling the op; over the cap returns `error("input_queue_full", "too much input is waiting; try again")`.
- [ ] **Step 5: Run** `cargo test -p fernjo-ssh` and `cargo test -p fernjo-app` → PASS. SSH integration tests need the container (`eval "$(tools/ssh-test-server.sh start)"`, then `tools/ssh-test-server.sh stop`); run them and report, or mark open.
- [ ] **Step 6: Commit** `fix(ssh): write known_hosts after the handshake; cap the session input queue`.

### Task 5: Cmd+Q confirmation (and the shared dialog)

**Files:**
- Modify: `src-tauri/src/lib.rs`, `src-tauri/src/sessions.rs` (pure decision next to `needs_close_confirmation`)
- Test: `src-tauri/src/sessions/tests.rs`

**Interfaces:**
- Produces: `pub(crate) enum ExitAction { Allow, Ask, Prevent }` and `pub(crate) fn exit_action(code: Option<i32>, connected: usize, prompt_open: bool) -> ExitAction` — `Allow` when `code.is_some()` or `connected == 0`; `Prevent` (no new dialog) when `prompt_open`; else `Ask`.

- [ ] **Step 1: Write failing tests**: `an_explicit_exit_code_is_never_intercepted`, `no_connected_session_allows_the_exit`, `a_connected_session_asks`, `a_second_request_while_the_dialog_is_open_prevents_without_a_second_dialog`.
- [ ] **Step 2: Run** `cargo test -p fernjo-app exit_action` → FAIL.
- [ ] **Step 3: Implement.** Extract the dialog block of `on_window_event` into one function `ask_to_close(window: tauri::WebviewWindow/Window, connected: usize)` (same text, buttons, `close_prompt_open` flag, "Close" → `close_all` + `sync_runner::destroy_after_push`) used by both paths. Change `run()` from `.run(generate_context!())` to `.build(generate_context!())` then `.run(|app, event| ...)`; on `RunEvent::ExitRequested { api, code, .. }` call `exit_action(code, connected_count, close_prompt_open)`: `Allow` does nothing, `Ask` calls `api.prevent_exit()` and `ask_to_close` on the `main` window, `Prevent` only `api.prevent_exit()`. After "Close" the window is destroyed with the count at zero, so the following `ExitRequested` is allowed (no loop). `single_instance` stays the first plugin.
- [ ] **Step 4: Run** the tests → PASS; `cargo clippy --workspace --all-targets -- -D warnings`. The real Cmd+Q on macOS is an owner check.
- [ ] **Step 5: Commit** `feat: confirm quitting with Cmd+Q while sessions are connected`.

### Task 6: M4 follow-ups, UI half (layout-independent chords, Ctrl+Shift+W in the vault view)

**Files:**
- Modify: `ui/src/lib/shortcuts.ts`, `ui/src/lib/tabs.ts`, `ui/src/App.svelte`
- Test: `ui/src/lib/shortcuts.test.ts`, `ui/src/lib/tabs.test.ts`

**Interfaces:**
- Produces: `routeKey`'s `Key` type gains optional `code?: string`; `shortcutEvent` includes `code` (`Key${letter}`). `tabs.ts`: `export function tabToClose(view: string, activeId: string | null): string | null` (null while `view === "vault"`).

- [ ] **Step 1: Write failing tests**: `ctrl_shift_w_on_a_non_latin_layout_closes_the_tab` (`key: "ц"`, `code: "KeyW"`, ctrl+shift → `closeTab`), `a_non_letter_key_with_a_letter_code_is_not_a_chord_on_the_plain_key` unchanged behaviour for existing Latin tests, `shortcut_list_and_route_key_still_agree` stays green; tabs: `no_tab_is_closed_while_the_vault_view_is_open` (`tabToClose("vault","t1") === null`, `tabToClose("empty","t1") === "t1"`, `tabToClose("empty", null) === null`).
- [ ] **Step 2: Run** `pnpm test -- shortcuts tabs` → FAIL.
- [ ] **Step 3: Implement.** In `routeKey` the chord letter is `e.key.toLowerCase()` when it is `a`–`z`, else the letter from `e.code` (`/^Key([A-Z])$/`); the App key handler passes `e.code`. `App.svelte` closes through `tabToClose(view, sessions.activeId)`.
- [ ] **Step 4: Run** `pnpm test` → PASS, `pnpm lint`.
- [ ] **Step 5: Commit** `fix(ui): app chords work on non-Latin layouts; Ctrl+Shift+W ignores the vault view`.

### Task 7: Vault auto-lock after idle time

**Files:**
- Modify: `crates/fernjo-core/src/settings.rs`, `src-tauri/src/state.rs`, `src-tauri/src/app_settings.rs`, `src-tauri/src/lib.rs`, `src-tauri/src/vault_commands.rs` (table entries, exemption lists), `ui/src/lib/types.ts`, `ui/src/lib/api.ts`, `ui/src/lib/testing.ts`, `ui/src/lib/vault.svelte.ts`, `ui/src/lib/components/SettingsDialog.svelte`, `ui/src/App.svelte`
- Create: `src-tauri/src/idle_lock.rs`, `ui/src/lib/activity.ts`
- Test: `crates/fernjo-core/src/settings.rs`, `src-tauri/src/idle_lock.rs`, `ui/src/lib/activity.test.ts`, `ui/src/lib/components/SettingsDialog.test.ts`, `ui/src/lib/vault.test.ts`

**Interfaces:**
- Produces (core): `AppSettings.vault_auto_lock_minutes: Option<u32>` (serde `vaultAutoLockMinutes`, skipped when none), `pub const AUTO_LOCK_MINUTES_RANGE: RangeInclusive<u32> = 1..=1440`, `pub fn validate_auto_lock(minutes: Option<u32>) -> Result<(), &'static str>` (error `invalid_auto_lock`).
- Produces (`idle_lock.rs`): `pub struct IdleTracker` (`new(now: Instant)`, `note(&self, now: Instant)`, `is_due(&self, now: Instant, limit_minutes: Option<u32>) -> bool`; interior `Mutex<Instant>`); `pub fn spawn(app: AppHandle)` — a thread that every 5 s, when the vault is unlocked and `is_due`, calls `vault_commands::vault_lock(&state)` and emits event `vault-status` with the returned `VaultStatus`. `AppState` gains `pub idle: IdleTracker` and `pub vault_auto_lock_minutes: Mutex<Option<u32>>`.
- Commands (counted in the three places): `get_vault_auto_lock(state) -> AutoLockInfo { minutes: Option<u32>, persistent: bool, unsaved_reason: Option<UnsavedReason> }`, `set_vault_auto_lock(state, minutes: Option<u32>) -> AutoLockInfo`, `note_activity(state) -> ()`.
- Produces (UI): `startActivityHeartbeat(send: () => void, target: Window, now: () => number, minGapMs = 5000): () => void` in `activity.ts` — passive capture-phase `keydown`, `pointerdown`, `pointermove`, `wheel` listeners on `target`, calls `send` at most once per `minGapMs`, returns a stop function. `Api` gains `getVaultAutoLock`, `setVaultAutoLock(minutes: number | null)`, `noteActivity()`, `onVaultStatus(handler)`.

- [ ] **Step 1: Write failing tests.** core: `auto_lock_defaults_to_off_and_round_trips`, `auto_lock_range_is_1_to_1440` (0 and 1441 → `invalid_auto_lock`, None ok). `idle_lock.rs`: `not_due_while_activity_is_within_the_limit`, `due_after_the_limit_without_input`, `never_due_when_the_limit_is_off`, `a_lowered_limit_applies_at_once`, `activity_resets_the_clock`. `app_settings`: `set_vault_auto_lock_persists_and_keeps_other_fields`, `get_vault_auto_lock_reports_unsaved_reason_under_the_env_override`, `an_invalid_minutes_value_changes_nothing`. UI: `activity.test.ts`: `a_keydown_on_a_view_that_stops_propagation_is_still_counted` (child listener calls `stopPropagation`; `send` still called), `activity_is_throttled_to_one_call_per_gap`, `stop_removes_the_listeners`; `vault.test.ts`: `a_vault_status_event_updates_the_chip`; `SettingsDialog.test.ts`: `the_auto_lock_field_saves_minutes_and_off`.
- [ ] **Step 2: Run** `cargo test -p fernjo-core settings`, `cargo test -p fernjo-app idle_lock`, `pnpm test -- activity vault SettingsDialog` → FAIL.
- [ ] **Step 3: Implement** the types above. Locking goes through `vault_lock` (same lock order); a lock error is ignored and retried on the next tick. `note_activity` and `spawn` need no secrets. Wire `idle_lock::spawn(app.handle().clone())` in the Tauri `setup`. Register the three commands (function + `handlers` wrapper, `table()` entries, `generate_handler!`); the settings field is a labelled number input "Lock the vault after … minutes of inactivity" with an off state, using `unsavedNote()` like the other settings. In `App.svelte` start the heartbeat with `api.noteActivity` and stop it on destroy.
- [ ] **Step 4: Run** the same commands plus `cargo test -p fernjo-app command_table_covers_every_tauri_command no_response_contains_a_secret` and `pnpm lint` → PASS.
- [ ] **Step 5: Commit** `feat: lock the vault after a configurable idle time`.

### Task 8: Quick connect, Rust side

**Files:**
- Create: `crates/fernjo-core/src/quick.rs`
- Modify: `crates/fernjo-core/src/lib.rs`, `src-tauri/src/sessions.rs` (`Entry.target`, `open`, `begin`), `src-tauri/src/connect_plan.rs`, `src-tauri/src/session_commands.rs`, `src-tauri/src/lib.rs`, `src-tauri/src/vault_commands.rs` (table)
- Test: `crates/fernjo-core/src/quick.rs`, `src-tauri/src/connect_plan.rs`, `src-tauri/src/sessions/tests.rs`

**Interfaces:**
- Produces (core): `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")] pub struct QuickTarget { pub protocol: Protocol, pub host: String, pub port: u16, pub user: Option<String> }`; `pub fn parse_quick(input: &str) -> Result<QuickTarget, QuickError>` where `QuickError { pub code: &'static str }`, codes `quick_empty`, `quick_user_required` (SSH without user), `quick_bad_scheme`, `quick_bad_port` (0 or above 65535), `quick_bad_target`. Syntax `[ssh://|rdp://]user@host[:port]`, trimmed; IPv6 as `[::1]` with optional `:port`; default ports from `DEFAULT_SSH_PORT`/`DEFAULT_RDP_PORT`; the user may contain no whitespace or `@`.
- Produces (src-tauri): `pub enum Target { Node(NodeId), Quick(QuickTarget) }` (in `connect_plan.rs`, `Clone`); `connect_plan::resolve_target(state: &AppState, target: &Target, skip_vault: bool) -> Result<Plan, CommandError>` (`Node` → `resolve_session`; `Quick` → `resolve_quick`, which runs `fernjo_ssh::validate_host` through `host_port`, `invalid_host` on failure, SSH plan `CredentialPlan::None` and `DEFAULT_KEEPALIVE_SECS`, RDP plan `RdpCredentialPlan::None`, domain `None`, resolution `Fit`, `DEFAULT_RDP_COLOR_DEPTH`, `clipboard: false`). `SessionManager::open_target(&self, state, target: Target, channel, cols, rows) -> Result<String, CommandError>`; the existing `open` calls it with `Target::Node`. `Entry.node_id` becomes `target: Target`.
- Commands (three places): `quick_parse(input: String) -> Result<QuickTarget, CommandError>` (errors carry the codes above and a plain sentence) and `open_quick_session(target: QuickTarget, on_event: Channel<InvokeResponseBody>, cols: u32, rows: u32) -> Result<String, CommandError>` (re-validates the target).

- [ ] **Step 1: Write failing tests.** core `quick.rs`: `parses_user_host`, `parses_scheme_and_port`, `rdp_user_is_optional`, `ssh_without_user_is_refused`, `trims_whitespace`, `parses_bracketed_ipv6_with_port`, `rejects_double_at_and_empty_parts`, `rejects_port_zero_and_65536`, `rejects_unknown_scheme`, `rejects_host_with_slash_semicolon_or_space` (syntax only; the host allow-list test is below). `connect_plan.rs`: `quick_ssh_plan_has_no_credential_and_the_typed_user`, `quick_rdp_plan_has_the_clipboard_off_and_no_credential`, `a_quick_host_outside_the_allow_list_is_invalid_host` (`user@bad host` and `a/b`), `nothing_is_written_to_connections_json_by_resolving_a_quick_target`. sessions tests: `reconnect_of_a_quick_session_resolves_the_same_target_again` and `a_quick_session_never_touches_the_tree`.
- [ ] **Step 2: Run** `cargo test -p fernjo-core quick`, `cargo test -p fernjo-app quick` → FAIL.
- [ ] **Step 3: Implement** the types and commands; `begin` resolves through `resolve_target(state, &entry.target, skip_vault)`; the "gone" text applies to `Node` only. Add table entries (`quick_parse` with a sentinel-free string; `open_quick_session` goes through its plain function like `open_session`), count guard, `generate_handler!`.
- [ ] **Step 4: Run** the tests plus `command_table_covers_every_tauri_command no_response_contains_a_secret`, `cargo clippy --workspace --all-targets -- -D warnings` → PASS.
- [ ] **Step 5: Commit** `feat: session targets and quick connect commands`.

### Task 9: Quick connect, UI

**Files:**
- Create: `ui/src/lib/components/QuickConnect.svelte`, `ui/src/lib/components/QuickConnect.test.ts`
- Modify: `ui/src/lib/types.ts`, `ui/src/lib/api.ts`, `ui/src/lib/testing.ts`, `ui/src/lib/sessions.svelte.ts`, `ui/src/lib/shortcuts.ts`, `ui/src/lib/components/Sidebar.svelte`, `ui/src/lib/components/TabStrip.svelte`, `ui/src/App.svelte`
- Test: `ui/src/lib/sessions.test.ts`, `ui/src/lib/shortcuts.test.ts`

**Interfaces:**
- Consumes: Task 8 commands. Produces: `Tab.nodeId: string | null` and `Tab.quick: QuickTarget | null` (exactly one set); `SessionStore.openQuick(input: string): Promise<void>` (calls `api.quickParse`, creates the tab with title `user@host` or `host`, protocol from the target, `clipboard: false`, no `#loadMode`; on a parse error sets `this.error`-style inline error returned to the caller as `ApiError`); `Api.quickParse(input)`, `Api.openQuickSession(target, onEvent, cols, rows)`; `AppAction` gains `"quickConnect"`, chord `n` (Ctrl+Shift+N, Cmd+N), `SHORTCUTS` entry `{ label: "Quick connect", action: "quickConnect", key: "N" }`. `lampFor` and the tree skip tabs with `nodeId === null`.

- [ ] **Step 1: Write failing tests.** sessions: `open_quick_creates_a_tab_for_the_parsed_target` (SSH and RDP, protocol and title), `open_quick_reports_a_parse_error_and_opens_no_tab`, `a_quick_tab_reconnects_through_the_same_session`, `a_quick_tab_does_not_light_a_tree_lamp`. shortcuts: `ctrl_shift_n_is_quick_connect_and_plain_ctrl_n_is_not`, the list/`routeKey` agreement test covers the new entry. `QuickConnect.test.ts`: `enter_opens_the_target_and_clears_the_field`, `a_bad_target_shows_the_error_text_and_keeps_the_input`, `escape_clears_and_leaves_the_field`, the placeholder shows `user@host[:port]`.
- [ ] **Step 2: Run** `pnpm test -- sessions shortcuts QuickConnect` → FAIL.
- [ ] **Step 3: Implement.** `QuickConnect.svelte` sits under the filter box in `Sidebar.svelte` with a visible label or placeholder "Quick connect: user@host" (discoverable without a hint); the shortcut focuses it (App handles `quickConnect` like `focusFilter`). In `SessionStore.#open` use `api.openQuickSession(tab.quick, …)` when `tab.quick` is set. Follow `DESIGN.md` and tokens only.
- [ ] **Step 4: Run** `pnpm test` and `pnpm lint` → PASS.
- [ ] **Step 5: Commit** `feat(ui): quick connect field and shortcut`.

### Task 10: Restore tabs, Rust side

**Files:**
- Create: `src-tauri/src/open_tabs.rs`
- Modify: `crates/fernjo-core/src/settings.rs`, `src-tauri/src/state.rs`, `src-tauri/src/sessions.rs`, `src-tauri/src/session_commands.rs` (or `open_tabs.rs` handlers), `src-tauri/src/lib.rs`, `src-tauri/src/vault_commands.rs` (table, `sources`)
- Test: `src-tauri/src/open_tabs.rs`, `src-tauri/src/sessions/tests.rs`

**Interfaces:**
- Consumes: `Target` (Task 8). Produces: `AppSettings.reopen_tabs: Option<bool>` (serde `reopenTabs`, unset means on). In `open_tabs.rs`: `pub const FILE_NAME: &str = "open-tabs.json"`, `pub const MAX_TABS: usize = 50`, `#[derive(Serialize, Deserialize)] pub struct SavedTab { pub node: Option<NodeId>, pub quick: Option<QuickTarget> }` (exactly one), `pub struct OpenTabsFile { pub schema: u32, pub tabs: Vec<SavedTab>, pub active: Option<usize> }`, `pub fn load(device_dir: &Path) -> OpenTabsFile` (capped read; missing, unreadable, newer schema or oversized → empty), `pub fn save(device_dir: &Path, file: &OpenTabsFile) -> io::Result<()>` (atomic temp+rename, no `.bak`, random temp name like the other device files), `pub fn remove(device_dir: &Path)`.
- Commands (three places): `get_open_tabs(state) -> OpenTabsInfo { reopen: bool, persistent: bool, unsaved_reason: Option<UnsavedReason>, active: Option<usize>, tabs: Vec<RestorableTab> }` where `RestorableTab { target: TargetView (node id or QuickTarget), protocol: Protocol, title: String, uses_vault: bool }` built only from entries that still resolve (deleted ids and folders dropped, protocol and title from the current node, `uses_vault` = the node's effective credential is set); `set_reopen_tabs(state, enabled: bool) -> OpenTabsInfo` (off removes the file); `save_open_tabs(state, tabs: Vec<SavedTab>, active: Option<usize>) -> ()` (no-op and no file while reopen is off or there is no device folder; caps at `MAX_TABS`; drops invalid entries); `restore_session(state, target: TargetArg, on_event, cols, rows) -> String` (`SessionManager::restore(state, Target, channel, cols, rows)` creates an `Entry` with `Phase::Disconnected` and sends the disconnected state with the text `RESTORED_TEXT = "Restored from your last session. Not connected."`, `endedCleanly` false; `not_found` / `not_a_connection` for a bad node).

- [ ] **Step 1: Write failing tests.** `open_tabs.rs`: `a_missing_file_loads_empty`, `a_damaged_file_loads_empty_and_the_next_save_rewrites_it`, `a_newer_schema_file_loads_empty`, `a_file_over_the_cap_loads_empty`, `save_then_load_round_trips_nodes_and_quick_targets`, `the_file_contains_no_secret_field_names` (serialise a list, assert the keys are exactly `schema`, `tabs`, `active`, `node`, `quick`, `protocol`, `host`, `port`, `user`), `the_file_lives_in_the_device_folder_never_the_data_folder`. Commands: `get_open_tabs_drops_deleted_nodes_and_folders`, `get_open_tabs_uses_the_current_protocol_of_a_changed_node`, `get_open_tabs_marks_a_tab_that_uses_a_vault_credential`, `save_open_tabs_is_a_no_op_while_reopen_is_off`, `turning_reopen_off_removes_the_file`, `save_open_tabs_caps_the_count`, `set_reopen_tabs_persists_and_keeps_other_settings`. Sessions: `a_restored_session_is_disconnected_and_does_not_count_as_connected` (`connected_count() == 0`), `a_restored_session_never_ends_cleanly`, `restore_of_a_deleted_node_is_not_found`, `reconnect_of_a_restored_session_connects_with_the_current_plan`, `close_all_does_not_touch_the_open_tabs_file`.
- [ ] **Step 2: Run** `cargo test -p fernjo-app open_tabs` → FAIL.
- [ ] **Step 3: Implement** as specified; the file path is `<device_dir>/open-tabs.json` (`<FERNJO_DATA_DIR>/.device` in scratch runs). A write error is reported but never blocks anything.
- [ ] **Step 4: Run** the tests, the guard tests (`command_table_covers_every_tauri_command`, `no_response_contains_a_secret`), `cargo clippy --workspace --all-targets -- -D warnings` → PASS.
- [ ] **Step 5: Commit** `feat: open-tabs file, restore commands and restored sessions`.

### Task 11: Restore tabs, UI side

**Files:**
- Create: `ui/src/lib/open-tabs.svelte.ts`, `ui/src/lib/open-tabs.test.ts`
- Modify: `ui/src/lib/types.ts`, `ui/src/lib/api.ts`, `ui/src/lib/testing.ts`, `ui/src/lib/sessions.svelte.ts`, `ui/src/lib/components/TabStrip.svelte`, `ui/src/lib/components/SettingsDialog.svelte`, `ui/src/App.svelte`
- Test: `ui/src/lib/sessions.test.ts`, `ui/src/lib/components/TabStrip.test.ts`, `ui/src/lib/components/SettingsDialog.test.ts`

**Interfaces:**
- Consumes: Task 10 commands. Produces: `Tab.restored: boolean`; `SessionStore.restore(tabs: RestorableTab[], active: number | null): void` (creates the tabs; each starts at `attach*` via `api.restoreSession`, never `openSession`; state `disconnected`, the text arrives from Rust); `SessionStore.reconnectAll(ensureVault: () => Promise<boolean>): Promise<void>` (reconnects restored disconnected tabs one after another with at most 3 at a time; if any such tab has `usesVault` and `ensureVault` resolves false it connects none); `SessionStore.restoredDisconnected: number` (for the button). `OpenTabsStore` in `open-tabs.svelte.ts`: `constructor(api, sessions)`, `async load()` (`getOpenTabs`; with `reopen` on calls `sessions.restore`, then sets `ready = true`), `ready`, `reopen`, `setReopen(enabled)`, and one `$effect.root` effect over `sessions.tabs` and `sessions.activeId` that, only once `ready`, sends `api.saveOpenTabs` debounced by 300 ms. Api: `getOpenTabs`, `setReopenTabs`, `saveOpenTabs`, `restoreSession`.

- [ ] **Step 1: Write failing tests.** `open-tabs.test.ts`: `nothing_is_saved_before_the_restore_has_finished` (pitfall 2), `an_empty_list_after_the_restore_is_saved_as_empty_only_when_the_user_closed_the_tabs`, `opening_closing_reordering_and_switching_save_the_list`, `a_tab_closed_because_the_shell_ended_cleanly_drops_out_of_the_list`, `closeAll_on_unload_is_not_saved` (the unload path does not call `saveOpenTabs`), `saves_are_debounced`, `nothing_is_saved_while_reopen_is_off`, `quick_tabs_are_saved_as_their_target`. sessions: `restore_creates_disconnected_tabs_without_opening_a_session`, `a_restored_tab_asks_rust_to_restore_when_its_view_attaches`, `reconnect_all_connects_each_restored_tab_in_order_with_a_limit_of_three`, `reconnect_all_asks_for_the_vault_once_and_connects_nothing_when_refused`, `reconnect_all_skips_the_vault_question_when_no_tab_uses_a_credential`. TabStrip: `reconnect_all_shows_only_while_a_restored_tab_is_disconnected`. SettingsDialog: `the_reopen_tabs_checkbox_saves_and_shows_the_unsaved_note`.
- [ ] **Step 2: Run** `pnpm test -- open-tabs sessions TabStrip SettingsDialog` → FAIL.
- [ ] **Step 3: Implement.** `App.svelte` creates the `OpenTabsStore` after the tree loads and calls `load()` once; `ensureVault` opens the vault unlock dialog and resolves true when the vault ends unlocked (reuse the existing vault store/manager flow). The tab's reason bar already shows `tab.detail`, so the restored text appears without new UI. "Reconnect all" is a button in `TabStrip.svelte` using existing button tokens. A tab the user closes with the × button or the shortcut, a clean-exit auto-close and a reorder all change `sessions.tabs`, so the one effect covers them; `closeAll` on unload must not call the save.
- [ ] **Step 4: Run** `pnpm test` and `pnpm lint` → PASS.
- [ ] **Step 5: Commit** `feat(ui): reopen tabs on start with Reconnect all`.

### Task 12: Native clipboard paste

**Files:**
- Create: `src-tauri/src/clipboard.rs`
- Modify: `deny.toml`, `about.toml`, `src-tauri/Cargo.toml`, `Cargo.lock`, `src-tauri/src/lib.rs`, `src-tauri/src/vault_commands.rs` (table, `sources`, the by-design-secret exemption list), `ui/src/lib/types.ts`, `ui/src/lib/api.ts`, `ui/src/lib/testing.ts`, `ui/src/lib/terminal/controller.ts`
- Test: `src-tauri/src/clipboard.rs`, `ui/src/lib/terminal/controller.test.ts`

**Interfaces:**
- Produces: `pub trait ClipboardText { fn read_text(&self) -> Result<String, String>; }`, `pub struct OsClipboard` (implements it with `arboard`, `default-features = false`, text only; add the Wayland data-control feature only if it resolves and passes `cargo deny check`), `pub const MAX_PASTE_BYTES: usize = 1024 * 1024`, `pub fn clipboard_read_text(clip: &dyn ClipboardText) -> Result<String, CommandError>` (codes `clipboard_unavailable`, `clipboard_too_large`; the error text never contains clipboard content). Command `clipboard_read_text` (handler uses `OsClipboard`; its `table()` entry uses a fake and the response containing the text is listed with `reveal` and `sync_pairing_code` as returning content by design). `Api.clipboardReadText(): Promise<string>`.

- [ ] **Step 1: Write failing tests.** Rust: `read_returns_the_clipboard_text`, `text_over_the_cap_is_refused_without_echoing_it`, `an_unavailable_clipboard_is_clipboard_unavailable`, `the_error_text_contains_no_clipboard_content`. UI controller: `menu_paste_reads_through_the_native_command_and_writes_the_text` (no `navigator.clipboard.readText` call), `menu_paste_failure_shows_nothing_and_does_not_hang`; remove the 2.5 s timeout tests (`paste_gives_up_after_the_timeout_and_ignores_a_late_clipboard_answer`) and the `PASTE_TIMEOUT` constant.
- [ ] **Step 2: Run** `cargo test -p fernjo-app clipboard` and `pnpm test -- controller` → FAIL.
- [ ] **Step 3: Implement.** Add `"BSL-1.0"` to `deny.toml` `[licenses] allow` and to `about.toml` `accepted` (keep the two lists in step); add the dependency; implement the module and command; the controller's menu Paste calls `api.clipboardReadText()` (keyboard paste and copy unchanged).
- [ ] **Step 4: Run** the tests, `cargo deny check`, `cargo build --workspace`, `bash tools/test-release-scripts.sh` → PASS. A local `node tools/third-party-notices.mjs` run is not needed (restore the placeholder with `git checkout -- src-tauri/resources/THIRD-PARTY-NOTICES.md` if run). Real paste in WebView2 is an owner check.
- [ ] **Step 5: Commit** `feat: native clipboard paste (BSL-1.0 allowed)`.

### Task 13: Documentation, roadmap evidence and the pull request

**Files:**
- Modify: `Docs/DECISIONS.md`, `CLAUDE.md`, `README.md`, `CHANGELOG.md`, `Docs/roadmap.json`, `Docs/ROADMAP.md` (generated), `Docs/design/` only if a wording note is needed

- [ ] **Step 1: DECISIONS.** One entry per decision, each with context, decision, rationale, rejected alternatives: "M10 session targets and quick connect" (Node or Quick target, RDP clipboard off), "M10 restore tabs: open-tabs file, disconnected restore, Reconnect all" (device folder, pitfalls, nothing snapshotted), "M10 vault auto-lock: Rust clock and capture-phase heartbeat", "M10 Pageant: pick one source", "M10 native clipboard: BSL-1.0 allowed" (supersedes the paste part of "M4 clipboard path"), "M10 quit confirmation on ExitRequested", "M10 M4 follow-ups: what was done, what stayed in the backlog".
- [ ] **Step 2: CLAUDE.md** "Project-specific": the open-tabs file and its two pitfalls, `Target` and the quick RDP clipboard rule, the activity heartbeat and `vault-status` event, the Cmd+Q path (`exit_action`), the new commands in the three-places rule, BSL-1.0 in both licence lists. Remove the now-false "Pageant is not supported" mentions. README and `CHANGELOG.md` `[Unreleased]` list the user-facing changes (quick connect, reopen tabs, auto-lock, Pageant, Cmd+Q, native paste).
- [ ] **Step 3: Roadmap.** In `Docs/roadmap.json` tick each M10 task as finished, tick acceptance criteria only with evidence (test names from Tasks 1–12 and the green gate commands with results), leave the owner checks open with the commands or steps that verify them (quick connect, restore tabs, auto-lock, Pageant with a key in Pageant, Cmd+Q on macOS, real paste), record the M4 follow-ups that stayed in the backlog under "Scope changes", set status `done` with `completedOn` only after the PR is merged. Run `dotnet run tools/roadmap.cs -- generate` then `dotnet run tools/roadmap.cs -- check`.
- [ ] **Step 4: Full gate**, every command from Global Constraints, results recorded in the PR text.
- [ ] **Step 5: PR** to `dev`: `git push -u origin feature/m10-daily-use-features`, `gh pr create --base dev` with a body listing the tasks, the evidence and the open owner checks (only on the user's say-so for the push); merge when CI is green.

---

## Self-review notes

- Spec coverage: quick connect (T8, T9), restore tabs (T10, T11), auto-lock (T7), Pageant (T3), Cmd+Q (T5), clipboard (T12), restore wording (T2), splitter (T1), M4 subset (T4, T6), documentation and roadmap (T13). Non-goals have no task.
- Names used across tasks: `Target`, `QuickTarget`, `resolve_target`, `open_target`, `restore`, `RESTORED_TEXT`, `OpenTabsStore`, `reconnectAll`, `IdleTracker`, `exit_action`, `InputBudget`, `first_with_identities`; each is defined in the task that produces it.
- Order note: Task 7 (auto-lock) does not depend on Tasks 8 to 11; Tasks 10 and 11 depend on 8 and 9.
- The `src-tauri` package is `fernjo-app` (`cargo test -p fernjo-app`).
