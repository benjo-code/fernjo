# Milestone 4 – SSH sessions in tabs: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Any SSH connection in the tree opens as a terminal tab, authenticated with the agent, the stored key or password (or typed answers) against host keys fernjo has verified.

**Architecture:** A new Tauri-free crate `fernjo-ssh` (russh) owns connect, host keys, authentication order, PTY and keepalive and talks to the user through an `Interaction` trait. `src-tauri` adds a session manager that resolves a connection into connect parameters (credentials from the vault, in Rust), implements `Interaction` by emitting prompt events on one Tauri channel per session, and streams terminal bytes as raw binary with acknowledgement-based flow control. The UI adds a session store, an xterm.js terminal per tab, a tab strip, inline prompts and lamps in the tree.

**Tech Stack:** Rust (russh 0.65 with `ring`, tokio), Tauri 2 channels, `tauri-plugin-opener` (Rust API only), Svelte 5 + TypeScript, `@xterm/xterm` with fit, WebGL, search and web-links addons, Vitest, an OpenSSH server container in CI.

**Spec:** [Docs/superpowers/specs/2026-11-16-m04-ssh-sessions-design.md](../specs/2026-11-16-m04-ssh-sessions-design.md) (parent: [2026-11-15-fernjo-design.md](../specs/2026-11-15-fernjo-design.md) §5.3). Executors read both and `DESIGN.md` ("Tabs", "Lamp column", "Inline unlock prompt", "Terminal").

## Global Constraints

- Rust edition 2024, toolchain pinned in `rust-toolchain.toml`; `fernjo-ssh` depends on neither Tauri nor `fernjo-core` nor `fernjo-vault`; secrets are `secrecy::SecretString` inside it.
- **Secrets stay in Rust:** no password, key, passphrase or typed answer in an event, response, error text, `Debug` output, log or webview storage. Typed answers travel UI → Rust only and are wrapped in `SecretString` on arrival. The session commands join the no-secret test.
- russh: `default-features = false`, features `flate2`, `ring`, `rsa`. `deny.toml` gets an ignore for `RUSTSEC-2023-0071` with the reason (russh pins `rsa 0.10.0-rc.18`; fixed in `rsa 0.10.0`) and the removal condition (russh depends on `rsa` ≥ 0.10.0). No other new ignore without a reason.
- PTY `xterm-256color`; keepalive from the inherited `keepaliveSecs` (default 30, 0 = off), three unanswered keepalives end the session; connect timeout 15 s; authentication attempts capped at 6 in total, at most 3 typed password prompts, at most 5 agent identities.
- Host keys: own `known_hosts` in the data folder, OpenSSH format, `[host]:port` for ports other than 22, written in clear, atomic write; hashed entries from other tools are read and matched. Nothing is sent to the server before the host key decision.
- `src-tauri/capabilities/default.json` stays `["core:default"]`; plugins are used through their Rust API only; `tauri_plugin_single_instance::init` stays the first `.plugin(...)`.
- Every new `#[tauri::command]` follows the `CLAUDE.md` checklist: function plus `handlers` wrapper, entry in `table()` (or `APP_HANDLE_EXEMPT` with a reason), `generate_handler!` in `lib.rs`, new source file added to the test's `sources`.
- UI: only tokens from `ui/src/styles/tokens.css`; hex-only CSS, no `oklch()`, `color-mix()`, `light-dark()`, nesting, `@layer`, `:has()`, container queries; a new colour token goes into the light block and both dark blocks. One yellow key per context; yellow also on Reconnect of every dropped tab; "Replace key and connect" is never yellow. UI text in English.
- App shortcuts inside a focused terminal: only Ctrl+Shift chords (Cmd on macOS) and Ctrl+Tab; plain Ctrl+letter, Alt+letter and Escape always reach the program.
- Flow control: when the output not yet acknowledged by the UI exceeds the high-water mark (start: 256 KiB) the session stops reading from the SSH channel until it falls below the low-water mark. After `yes` has run for 5 s, Ctrl+C must return the prompt within 1 s.
- Integration tests read `FERNJO_TEST_SSH_ADDR`; when it is unset they skip with a message locally and **fail when `CI` is set**. Test keys and passwords are generated per run and never committed.
- Work on `feature/m04-ssh-sessions`; Conventional Commits; stage files by name (no `git add .`); run `git show --stat HEAD` after every commit and compare with the task's file list; no attribution lines in commit messages; never `--no-verify`.
- "Done" = build, test, format and lint green (commands in the final task) plus `dotnet run tools/roadmap.cs -- check`. Tick roadmap criteria only after verification, with the proof as evidence. Unverifiable items are marked open with the command that verifies them.

## Review Focus

Failure modes the spec implies but no happy-path test exercises, most likely first (each has a test in the named task):

1. A command floods the terminal (`yes`, `cat` of a large file): the UI must stay responsive and Ctrl+C must work (Task 4 `flood_blocks_when_receiver_is_slow`, Task 7 `flood_stops_at_high_water_mark_and_ctrl_c_returns_within_one_second`).
2. `known_hosts` as other tools leave it: IPv6 literals, non-default ports, hashed hosts, several keys per host, comment and garbage lines, CRLF, a BOM; a rewrite must never lose or reorder the lines it does not own (Task 2).
3. The tab is closed or the server drops while a prompt is open: no hung task, no leaked pending answer, no later write of a half-trusted host key (Task 7 `closing_the_session_while_a_prompt_is_open_ends_connect`).
4. A hidden tab or a collapsed layout fits to 0×0: no `session_resize` with a zero size, no crash, and the right size on show (Task 8 `resize_ignores_zero_sizes`, Task 9 `fit_while_hidden_sends_nothing`).
5. The connection is edited or deleted while its session is open: the tab keeps working; Reconnect uses the current settings, and a deleted node ends in a clear message (Task 7 `reconnect_uses_current_settings_and_reports_a_deleted_node`).

---

## File Structure

| Path | Responsibility |
|---|---|
| `crates/fernjo-ssh/src/lib.rs`, `error.rs` | crate root; `SshError` with stable codes |
| `crates/fernjo-ssh/src/known_hosts.rs` | `known_hosts` read/match/add/replace, fingerprints |
| `crates/fernjo-ssh/src/interaction.rs`, `params.rs` | `Interaction` trait and prompt types; `ConnectParams`, `Credential` |
| `crates/fernjo-ssh/src/auth_plan.rs` | pure authentication order and attempt budget |
| `crates/fernjo-ssh/src/session.rs`, `handler.rs`, `agent.rs` | connect, auth loop, PTY, events; russh handler; agent access |
| `crates/fernjo-ssh/tests/` | container integration tests and shared `support` module |
| `tools/ssh-test-server.sh` | starts and stops the OpenSSH test container, prints env lines |
| `crates/fernjo-core/src/settings.rs` | `TerminalPrefs` in `settings.json` |
| `src-tauri/src/connect_plan.rs` | node → connect parameters (username, credential, vault state) |
| `src-tauri/src/sessions.rs`, `session_flow.rs`, `session_commands.rs` | session manager, `Interaction` impl, flow control; terminal-settings and session commands |
| `ui/src/lib/sessions.svelte.ts`, `tabs.ts`, `shortcuts.ts` | session store, tab reorder, key routing |
| `ui/src/lib/terminal/` | xterm controller, theme mapping, terminal view, search bar |
| `ui/src/lib/components/` | `TabStrip`, `SessionArea`, `Lamp`, `SessionPrompt` and the prompt variants; changes to `TreeRow`, `TreeView`, `Sidebar`, `Icon`, `DataFolderDialog`, `App.svelte` |

---

### Task 1: `fernjo-ssh` crate skeleton, dependencies, supply-chain policy

**Files:**
- Create: `crates/fernjo-ssh/Cargo.toml`, `crates/fernjo-ssh/src/lib.rs`, `crates/fernjo-ssh/src/error.rs`
- Modify: `Cargo.toml` (workspace dependencies `russh`, `tokio`), `deny.toml`, `Docs/DECISIONS.md`

**Interfaces:**
- Produces: `fernjo_ssh::SshError` (thiserror enum) with `pub fn code(&self) -> &'static str` returning exactly: `connect_failed`, `timeout`, `host_key_rejected`, `host_key_changed_rejected`, `auth_failed`, `connection_lost`, `closed`, `protocol`, `io`. `AuthFailed` carries `key_unreadable: bool` (Task 5 sets it). No variant stores user-supplied secret text.

- [ ] **Step 1: Write the failing test** `error::tests::codes_are_stable_and_display_is_secret_free` in `error.rs`: builds one value of every variant, asserts `code()` equals the literal list above, and asserts each `to_string()` is non-empty and contains none of the words `password`, `passphrase`, `private key` in a context that would embed a value (the strings are fixed sentences, so assert they do not contain `SENTINEL`, which the test passes as the only free-text field, `io`'s source message being replaced by a fixed text).
- [ ] **Step 2: Run** `cargo test -p fernjo-ssh` → FAIL (crate does not exist).
- [ ] **Step 3: Create the crate** (`name = "fernjo-ssh"`, workspace package fields, `publish = false`); workspace dependencies `russh = { version = "0.65.1", default-features = false, features = ["flate2", "ring", "rsa"] }` and `tokio` (features `rt`, `net`, `time`, `sync`, `macros`, `io-util`); crate dependencies `russh`, `tokio`, `secrecy`, `thiserror`. Do not add `ssh-key` directly: use `russh::keys::ssh_key` re-exports so the versions cannot diverge. Implement `SshError`.
- [ ] **Step 4: Run** `cargo test -p fernjo-ssh` → PASS; `cargo build --workspace` → OK on Windows.
- [ ] **Step 5: Run** `cargo deny check`. Expected: fails on `RUSTSEC-2023-0071`. Add the ignore to `deny.toml` with the reason and removal condition from Global Constraints; re-run → green, and confirm no existing ignore became "unused" noise beyond what was there before.
- [ ] **Step 6: Add a `Docs/DECISIONS.md` entry "M4 dependencies: russh, ring, RSA advisory"**: versions as resolved (`Cargo.lock`), why `ring` (built on Windows without a C toolchain; `aws-lc-rs` rejected), why `rsa` stays on with a time-boxed ignore (owner decision 2026-11-16), the removal condition, and that "pure Rust" no longer holds for the crypto backend (`ring` contains C and assembly).
- [ ] **Step 7: Commit** `git add crates/fernjo-ssh Cargo.toml Cargo.lock deny.toml Docs/DECISIONS.md` · `git commit -m "feat(ssh): fernjo-ssh crate skeleton, russh with ring, RUSTSEC-2023-0071 ignore"`.

---

### Task 2: `known_hosts` file

**Files:**
- Create: `crates/fernjo-ssh/src/known_hosts.rs`
- Modify: `crates/fernjo-ssh/src/lib.rs`

**Interfaces:**
- Consumes: `russh::keys::{PublicKey, Algorithm}`, `russh::keys::known_hosts::known_host_keys_path` for matching (it handles `[host]:port` and hashed entries).
- Produces:
  - `pub struct KnownHosts` with `new(path: impl Into<PathBuf>) -> Self`
  - `check(&self, host: &str, port: u16, key: &PublicKey) -> Result<HostKeyStatus, SshError>`
  - `stored_algorithms(&self, host: &str, port: u16) -> Result<Vec<Algorithm>, SshError>`
  - `add(&self, host: &str, port: u16, key: &PublicKey) -> Result<(), SshError>`
  - `replace(&self, host: &str, port: u16, key: &PublicKey) -> Result<(), SshError>` (drops the stored keys of the same algorithm for that host and port, appends the new one)
  - `pub enum HostKeyStatus { Unknown, Known, Changed { stored: Vec<String> } }` (fingerprints of the same-algorithm keys)
  - `pub fn fingerprint(key: &PublicKey) -> String` → `SHA256:<base64 without padding>`

- [ ] **Step 1: Write the failing tests** (temp dir, a fixed Ed25519 and a fixed RSA public key as constants; public keys are not secrets):
  - `fingerprint_matches_ssh_keygen` (expected string produced with `ssh-keygen -lf` on the same key, pasted into the test)
  - `unknown_then_added_is_known`; `different_key_of_same_type_is_changed_with_old_fingerprints`; `other_key_type_is_unknown_not_changed`
  - `port_other_than_22_uses_bracket_form_and_does_not_match_port_22`; `ipv6_literal_with_port_matches`
  - `hashed_entry_written_by_openssh_matches` (line produced with `ssh-keygen -H` on a scratch file, pasted as a constant)
  - `several_keys_per_host_are_all_listed_by_stored_algorithms`
  - `replace_swaps_only_the_same_algorithm_and_keeps_every_other_line_byte_for_byte` (file contains comments, a blank line, a garbage line, CRLF endings, a leading BOM)
  - `add_creates_the_file_and_leaves_bak_of_the_previous_version`; `write_is_atomic_no_temp_file_left`
- [ ] **Step 2: Run** `cargo test -p fernjo-ssh known_hosts` → FAIL.
- [ ] **Step 3: Implement** `KnownHosts`. Reading and matching go through russh's helper; `add` and `replace` do their own line-preserving rewrite (read all bytes, edit by line index as russh reports it, keep unknown lines and line endings untouched) and write temp file → `sync_all` → copy current to `known_hosts.bak` → rename, as the other data files do (a small local `write_atomic`; no dependency on `fernjo-core`). Missing file is an empty list.
- [ ] **Step 4: Run** `cargo test -p fernjo-ssh known_hosts` and `cargo clippy -p fernjo-ssh --all-targets -- -D warnings` → PASS.
- [ ] **Step 5: Commit** `git add crates/fernjo-ssh` · `git commit -m "feat(ssh): known_hosts with fingerprints, add and replace"`.

---

### Task 3: Interaction trait, connect types, authentication planner

**Files:**
- Create: `crates/fernjo-ssh/src/interaction.rs`, `crates/fernjo-ssh/src/params.rs`, `crates/fernjo-ssh/src/auth_plan.rs`
- Modify: `crates/fernjo-ssh/src/lib.rs`

**Interfaces:**
- Produces (`params.rs`):
  - `pub enum Credential { Password(SecretString), PrivateKey { key: SecretString, passphrase: Option<SecretString> } }` (redacted `Debug`)
  - `pub struct ConnectParams { pub host: String, pub port: u16, pub username: Option<String>, pub keepalive_secs: u32, pub credential: Option<Credential>, pub known_hosts: PathBuf, pub cols: u32, pub rows: u32, pub connect_timeout: Duration }` (`Default` for `connect_timeout` is 15 s via `ConnectParams::DEFAULT_TIMEOUT`)
- Produces (`interaction.rs`): `pub trait Interaction: Send + Sync + 'static` with async methods (native `async fn` in trait, used through generics, not `dyn`): `host_key(&self, HostKeyPrompt) -> HostKeyDecision`; `host_key_changed(&self, HostKeyChangedPrompt) -> HostKeyChangedDecision`; `keyboard_interactive(&self, KiRound) -> Option<Vec<SecretString>>`; `password(&self, user: &str, host: &str) -> Option<SecretString>`; `username(&self, host: &str) -> Option<String>`; `key_passphrase(&self) -> Option<SecretString>`. Types: `HostKeyPrompt { host, port, key_type: String, fingerprint: String }`, `HostKeyDecision { TrustOnce, TrustAlways, Reject }`, `HostKeyChangedPrompt { host, port, key_type, old: Vec<String>, new: String }`, `HostKeyChangedDecision { Replace, Reject }`, `KiRound { name, instructions, prompts: Vec<KiPrompt> }`, `KiPrompt { text: String, echo: bool }`.
- Produces (`auth_plan.rs`): `pub struct Available { pub agent_identities: usize, pub has_key: bool, pub has_password: bool }`; `pub struct Offered { pub publickey: bool, pub password: bool, pub keyboard_interactive: bool }`; `pub enum Attempt { Agent(usize), CredentialKey, CredentialPassword, KeyboardInteractive, TypedPassword }`; `pub struct AuthPlanner` with `new(Available) -> Self` and `next(&mut self, offered: Offered) -> Option<Attempt>` (each call returns the next not-yet-used attempt the server's current `offered` set allows, or `None`).
- Budget rule (decided here): `MAX_TOTAL = 6`. `reserved = has_key + has_password + 1 (keyboard-interactive) + 1 (typed password)`. Agent attempts = `min(agent_identities, 5, MAX_TOTAL - reserved)` (never below 0). Typed password attempts = `min(3, MAX_TOTAL - attempts already returned)`, at least one while any budget remains. Order: Agent(0..), CredentialKey, CredentialPassword, KeyboardInteractive, TypedPassword×n. Agent and key need `publickey`; both password kinds need `password`; keyboard-interactive needs `keyboard_interactive`.

- [ ] **Step 1: Write the failing tests** in `auth_plan.rs`:
  - `order_is_agent_key_password_kbdint_typed` (2 agent ids, key, password, all offered → `Agent(0), Agent(1), CredentialKey, CredentialPassword, KeyboardInteractive, TypedPassword`, then `None` since 2+1+1+1+1 = 6)
  - `skips_methods_the_server_does_not_offer` (only `keyboard_interactive` offered → only `KeyboardInteractive`; no `password` offered → no typed password)
  - `ten_agent_identities_are_capped_by_the_budget` (all credentials present → exactly 2 `Agent`; no credential → exactly 4 `Agent` then `KeyboardInteractive`, one `TypedPassword`)
  - `typed_password_gets_three_tries_when_budget_allows` (no agent, no credential, all offered → `KeyboardInteractive`, `TypedPassword`×3)
  - `offered_set_may_change_between_calls` (publickey offered first, then only keyboard-interactive after "partial success" → planner skips the remaining key steps)
  - `nothing_available_and_nothing_offered_is_none`
- [ ] **Step 2: Run** `cargo test -p fernjo-ssh auth_plan` → FAIL.
- [ ] **Step 3: Implement** the types and `AuthPlanner` (pure, no I/O).
- [ ] **Step 4: Run** `cargo test -p fernjo-ssh` and clippy → PASS.
- [ ] **Step 5: Commit** `git add crates/fernjo-ssh` · `git commit -m "feat(ssh): interaction trait, connect types and authentication planner"`.

---

### Task 4: Connect, host keys, password login, PTY, test server and CI step

**Files:**
- Create: `crates/fernjo-ssh/src/session.rs`, `handler.rs`, `crates/fernjo-ssh/tests/support/mod.rs`, `crates/fernjo-ssh/tests/connect.rs`, `tools/ssh-test-server.sh`
- Modify: `crates/fernjo-ssh/src/lib.rs`, `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: `ConnectParams`, `Credential`, `Interaction`, `KnownHosts`, `AuthPlanner` (Tasks 2–3).
- Produces:
  - `pub async fn Session::connect<I: Interaction>(params: ConnectParams, interaction: Arc<I>) -> Result<(Session, tokio::sync::mpsc::Receiver<SessionEvent>), SshError>`
  - `Session::write(&self, data: Vec<u8>) -> Result<(), SshError>`, `resize(&self, cols: u32, rows: u32) -> Result<(), SshError>`, `close(&self)`
  - `pub enum SessionEvent { Output(Vec<u8>), Closed(CloseReason) }`; `pub enum CloseReason { Exit(Option<u32>), ConnectionLost, ClosedByUser }`
  - `pub(crate) fn client_config(keepalive_secs: u32) -> russh::client::Config` (keepalive interval `Some(secs)` or `None` for 0, `keepalive_max` 3)
  - The events channel is **bounded** (64 entries) so a slow consumer blocks the session task.
  - Test support: `support::server() -> Option<TestServer>` (`addr`, `user`, `password`, key paths from env), `support::ScriptedInteraction` (queued answers, records every prompt and call count).
  - Env contract of `tools/ssh-test-server.sh start` (prints `export NAME=value` lines; `stop` removes the container): `FERNJO_TEST_SSH_ADDR` (`127.0.0.1:<port>`), `FERNJO_TEST_SSH_USER`, `FERNJO_TEST_SSH_PASSWORD`, `FERNJO_TEST_SSH_KEY_ED25519`, `FERNJO_TEST_SSH_KEY_RSA` (private key file paths), `FERNJO_TEST_SSH_KEY_ENCRYPTED` and `FERNJO_TEST_SSH_KEY_PASSPHRASE`, `FERNJO_TEST_SSH_KEY_UNAUTHORISED` (valid format, not authorised on the server).

- [ ] **Step 1: Write `tools/ssh-test-server.sh`**: `ssh-keygen` generates the keys above in a temp dir with `-N ""` (and with a random passphrase for the encrypted one); the server is a `lscr.io/linuxserver/openssh-server` container **pinned by digest** (resolve the digest with `docker buildx imagetools inspect`, record it in the script) with a random password, password and public-key access, keyboard-interactive on (PAM), mapped to a free port on `127.0.0.1`. Wait until the port accepts connections. Verify by hand: `eval "$(tools/ssh-test-server.sh start)"` then `ssh -p ${FERNJO_TEST_SSH_ADDR##*:} -i $FERNJO_TEST_SSH_KEY_ED25519 $FERNJO_TEST_SSH_USER@127.0.0.1 echo ok` prints `ok`. (Docker and OpenSSH are present on the owner's machine.)
- [ ] **Step 2: Write the failing tests** in `tests/connect.rs` (each starts with `let Some(s) = support::server() else { return };`; the helper panics when `CI` is set and the variable is not):
  - `password_login_runs_a_command` (credential `Password`, writes `echo fernjo$((20+22))\n`, expects output containing `fernjo42`)
  - `unknown_host_trust_always_writes_known_hosts_and_second_connect_does_not_prompt`
  - `trust_once_does_not_write_known_hosts`; `rejecting_an_unknown_host_ends_with_host_key_rejected`
  - `changed_host_key_is_blocked_before_any_authentication` (known_hosts holds a different Ed25519 key for `[127.0.0.1]:<port>`; `Reject` → `host_key_changed_rejected`; `ScriptedInteraction` recorded zero `password`, `keyboard_interactive`, `username` calls)
  - `changed_host_key_replace_continues_and_rewrites_the_line`
  - `wrong_credentials_end_in_auth_failed_with_at_most_three_typed_password_prompts` (no credential; `keyboard_interactive` and `password` answered wrongly)
  - `resize_reaches_the_server` (`stty size` after `resize(100, 30)` prints `30 100`)
  - `flood_blocks_when_receiver_is_slow` (run `yes`; read nothing for 2 s; then drain what is queued and assert the total is under 16 MiB; then send `\x03` and expect the marker of a following `echo` within 1 s)
  - `close_ends_with_closed_by_user`
  - unit test `client_config_sets_keepalive_or_none` (30 → `Some(30 s)`, `keepalive_max` 3; 0 → `None`)
- [ ] **Step 3: Run** `cargo test -p fernjo-ssh` with the server env set → FAIL (no `Session`); without it → tests skip with a message.
- [ ] **Step 4: Implement** `handler.rs`: a russh `client::Handler` whose `check_server_key` consults `KnownHosts` and `Interaction` (unknown → `host_key`; changed → `host_key_changed`; `TrustAlways` → `add`, `Replace` → `replace`) and stores the real reason in shared state so `connect` returns `host_key_rejected` or `host_key_changed_rejected` instead of a generic russh error. `session.rs`: resolve and connect with `connect_timeout` (`timeout` / `connect_failed`), restrict offered host key algorithms to `stored_algorithms` when the host has stored keys, probe with `authenticate_none` to learn the offered methods, loop `AuthPlanner::next` (this task wires only `CredentialPassword`, `KeyboardInteractive` and `TypedPassword`; Task 5 adds key and agent), ask `Interaction::username` when `username` is `None`, open the session channel, `request_pty("xterm-256color", cols, rows, 0, 0, &[])`, `request_shell`, then a task that forwards `ChannelMsg::Data`/`ExtendedData` as `Output` and turns EOF/exit/close into `Closed`. If russh keeps buffering internally when the receiver is slow, bound it through `client::Config` (`window_size`, `channel_buffer_size`) until `flood_blocks_when_receiver_is_slow` passes.
- [ ] **Step 5: Run** the tests with the server env set → PASS; `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] **Step 6: CI:** in the `rust` job of `.github/workflows/ci.yml`, add a step before `cargo test --workspace` that runs `tools/ssh-test-server.sh start >> "$GITHUB_ENV"` (convert `export ` lines to `NAME=value` lines) and a final `if: always()` step that runs `tools/ssh-test-server.sh stop`. No new job (required checks unchanged), no new action. Push the branch and confirm the job is green and the tests ran (not skipped): the job log shows `connect` tests passing.
- [ ] **Step 7: Commit** `git add crates/fernjo-ssh tools/ssh-test-server.sh .github/workflows/ci.yml` · `git commit -m "feat(ssh): connect with host key checks, password login, PTY and CI test server"`.

---

### Task 5: Key, agent and keyboard-interactive authentication

**Files:**
- Create: `crates/fernjo-ssh/src/agent.rs`, `crates/fernjo-ssh/tests/auth.rs`
- Modify: `crates/fernjo-ssh/src/session.rs`, `crates/fernjo-ssh/src/error.rs`, `tools/ssh-test-server.sh` (starts an `ssh-agent`, adds the unauthorised key first and the Ed25519 key second, prints `FERNJO_TEST_SSH_AGENT=1` and `SSH_AUTH_SOCK`), `.github/workflows/ci.yml` only if the step needs the agent variables

**Interfaces:**
- Consumes: `AuthPlanner`, `Credential`, `Interaction` (Task 3), `Session::connect` (Task 4).
- Produces: `agent::connect() -> Option<AgentClient<…>>` (Unix: `AgentClient::connect_env()`; Windows: `connect_named_pipe(r"\\.\pipe\openssh-ssh-agent")`; any failure is `None`); `pub(crate) fn order_identities(identities: Vec<PublicKey>, preferred: Option<&PublicKey>) -> Vec<PublicKey>` (the identity equal to `preferred` first, others keep their order); `SshError::AuthFailed { key_unreadable: bool }` filled when the credential key could not be decoded (message then adds "the stored private key could not be read").

- [ ] **Step 1: Write the failing tests.** Unit (`agent.rs`): `preferred_identity_moves_to_the_front_and_the_rest_keep_order`. Integration (`tests/auth.rs`, same skip rule): `ed25519_key_from_credential_logs_in`, `rsa_key_from_credential_logs_in` (SHA-2 signature), `encrypted_key_asks_interaction_for_the_passphrase`, `encrypted_key_uses_the_stored_passphrase_without_asking`, `unreadable_key_is_skipped_and_reported_in_the_final_error` (garbage key text and no other method → `auth_failed` with `key_unreadable: true`, a working typed password still succeeds), `agent_tries_identities_in_order_and_succeeds_on_the_authorised_one` (requires `FERNJO_TEST_SSH_AGENT`; follows the same skip-or-fail-in-CI rule), `keyboard_interactive_round_is_shown_and_answered` (no credential, `KiRound` prompts reach the interaction, answer logs in).
- [ ] **Step 2: Run** `cargo test -p fernjo-ssh auth` → FAIL.
- [ ] **Step 3: Implement** the remaining attempts in the auth loop: agent identities (request identities, `order_identities` with the public key of the credential key when it decodes, sign through the agent with `authenticate_publickey_with` and the hash from `best_supported_rsa_hash` for RSA); credential key via `russh::keys::decode_secret_key` (stored passphrase, else `Interaction::key_passphrase` when the key is encrypted, wrong or missing passphrase counts as unreadable) and `authenticate_publickey` with `PrivateKeyWithHashAlg`; keyboard-interactive via `authenticate_keyboard_interactive_start`/`respond` looping over `InfoRequest` rounds with `Interaction::keyboard_interactive` (an empty prompt list is answered with an empty list; `None` cancels). Pass `Available` to the planner from what was found (agent identity count, key present, password present) and the server's remaining methods after every failure.
- [ ] **Step 4: Run** `cargo test -p fernjo-ssh` (with and without server env) and `cargo clippy --workspace --all-targets -- -D warnings` → PASS. Push and confirm the CI job ran the new tests.
- [ ] **Step 5: Commit** `git add crates/fernjo-ssh tools/ssh-test-server.sh .github/workflows/ci.yml` · `git commit -m "feat(ssh): key, agent and keyboard-interactive authentication"`.

---

### Task 6: Terminal settings and connect-plan resolution (Rust)

**Files:**
- Create: `src-tauri/src/connect_plan.rs`, `src-tauri/src/session_commands.rs`
- Modify: `crates/fernjo-core/src/settings.rs` (+ export in `lib.rs`), `src-tauri/Cargo.toml` (`fernjo-ssh`), `src-tauri/src/state.rs`, `src-tauri/src/vault_commands.rs` (make `with_vault` `pub(crate)`; extend `table()` and `sources` for `get_terminal_settings`, `set_terminal_settings`), `src-tauri/src/lib.rs`

**Interfaces:**
- Produces (core): `TerminalPrefs { font_family: Option<String>, font_size: Option<u16>, scrollback: Option<u32> }` (camelCase JSON) as `AppSettings.terminal: Option<TerminalPrefs>` (skipped when `None`; `CURRENT_VERSION` stays 1); `DEFAULT_FONT_SIZE = 13`, `DEFAULT_SCROLLBACK = 10_000`, `FONT_SIZE_RANGE = 8..=32`, `SCROLLBACK_RANGE = 1000..=100_000`; `TerminalPrefs::validate(&self) -> Result<(), &'static str>` (codes `invalid_font_size`, `invalid_scrollback`).
- Produces (Tauri):
  - `get_terminal_settings(state) -> TerminalSettingsInfo { font_family: Option<String>, font_size: u16, scrollback: u32, persistent: bool }`
  - `set_terminal_settings(state, settings: TerminalSettingsInput { font_family: Option<String>, font_size: u16, scrollback: u32 }) -> TerminalSettingsInfo`
  - `connect_plan::resolve(state: &AppState, node_id: NodeId, skip_vault: bool) -> Result<ConnectPlan, CommandError>`
  - `ConnectPlan { title: String, host: String, port: u16, username: Option<String>, keepalive_secs: u32, credential: CredentialPlan }`; `CredentialPlan { None, Missing(MissingReason), NeedsVault, Ready(fernjo_ssh::Credential) }`; `MissingReason { Dangling, VaultNone, VaultUnusable, Skipped }` (each has a short display text for the tab caption)
  - Errors: `not_found` (no such node), `not_a_connection`, `unsupported_protocol` ("RDP sessions are not available yet"), `invalid_host` (empty host).

- [ ] **Step 1: Write the failing tests.** Core: `terminal_prefs_round_trip_and_leave_data_dir_alone`, `validate_rejects_font_size_7_and_33_and_scrollback_999_and_100001`, `settings_without_terminal_still_load` (existing files). `connect_plan::tests` (plain functions over `AppState::open` with a tempdir, tree built with `Store`, vault created and unlocked with `KdfParams::TEST`): `connection_username_beats_inherited_beats_entry_username`, `entry_username_is_the_fallback`, `no_username_anywhere_gives_none`, `port_and_keepalive_are_inherited_or_default`, `locked_vault_with_reference_gives_needs_vault_and_no_secret_is_read`, `skip_vault_gives_missing_skipped_and_ignores_entry_username`, `dangling_reference_gives_missing_dangling`, `unusable_vault_gives_missing_vault_unusable`, `no_reference_gives_none`, `password_entry_becomes_password_credential`, `private_key_entry_keeps_passphrase`, `rdp_is_refused`, `folder_is_refused`, `empty_host_is_refused`, `plan_debug_output_contains_no_secret`. Settings commands: `set_then_get_round_trips`, `invalid_values_are_refused_and_nothing_changes`, `under_data_dir_env_the_values_apply_but_are_not_persistent_and_no_file_is_written`, `persisted_value_keeps_data_dir_in_settings_json`, `unusable_settings_file_is_never_saved_over`.
- [ ] **Step 2: Run** `cargo test -p fernjo-core -p fernjo-app` → FAIL.
- [ ] **Step 3: Implement.** Core: add `TerminalPrefs` and validation. `AppState` gets `terminal: Mutex<TerminalPrefs>` loaded at startup from `settings.json` (extend `Location`/`locate` or load once more; `settings.json` is neither read nor written while `FERNJO_DATA_DIR` is set, so then the prefs start default and stay in memory) and `set_terminal_settings` does read-modify-write on `settings.json` preserving `data_dir`, skipped when `source == Env` or `settings_unusable`. `connect_plan::resolve`: effective values from `Tree::resolve_node`, credential through the vault (`list/get` for the entry's username, `reveal` for the secret) only when `skip_vault` is false and the vault is unlocked; username rule from the spec §5. Register the two commands (wrapper, `generate_handler!`, `table()`, `sources`).
- [ ] **Step 4: Run** `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` → PASS (`no_response_contains_a_secret` and the count guard included).
- [ ] **Step 5: Commit** `git add crates/fernjo-core src-tauri Cargo.lock` · `git commit -m "feat(app): terminal settings and connect-plan resolution"`.

---

### Task 7: Session manager, flow control, session commands

**Files:**
- Create: `src-tauri/src/sessions.rs`, `src-tauri/src/session_flow.rs`
- Modify: `src-tauri/src/session_commands.rs`, `src-tauri/src/state.rs` (`sessions: SessionManager`), `src-tauri/src/lib.rs` (`mod`s, `tauri_plugin_opener::init()` registered after the dialog plugin, handlers), `src-tauri/Cargo.toml` (`tauri-plugin-opener = "2"`), `src-tauri/src/vault_commands.rs` (`table()`, `APP_HANDLE_EXEMPT`, `sources`), `Docs/DECISIONS.md` (entry on the opener plugin)

**Interfaces:**
- Consumes: `connect_plan::resolve`, `fernjo_ssh::{Session, SessionEvent, Interaction, ConnectParams, …}`.
- Produces (wire, JSON on the session channel; terminal output is raw bytes on the same channel):
  - `SessionEvent` (serde `tag = "kind"`, camelCase): `{kind:"state", state:"connecting"|"needsVault"|"connected"|"disconnected", detail: string|null}` and `{kind:"prompt", id: number, prompt: PromptView}`
  - `PromptView` (tag `kind`): `hostKey {host, port, keyType, fingerprint}`, `hostKeyChanged {host, port, keyType, old: string[], new: string}`, `keyboardInteractive {name, instructions, prompts: {text, echo}[]}`, `password {user, host}`, `username {host}`, `keyPassphrase {}`
  - `PromptAnswer` (tag `kind`, `Deserialize` only, redacted `Debug`): `hostKey {decision: "once"|"always"|"reject"}`, `hostKeyChanged {decision: "replace"|"reject"}`, `secrets {values: string[]}`, `text {value: string}`, `cancel {}`
- Produces (plain functions, each with a `handlers` wrapper): `open_session(state, node_id, on_event: Channel<InvokeResponseBody>, cols, rows) -> Result<String, CommandError>` (session id, a UUID string); `session_input(state, id, data: Vec<u8>)` (wrapper takes the raw request body, id in a header); `session_resize(state, id, cols, rows)`; `session_ack(state, id, bytes: usize)`; `close_session(state, id)`; `reconnect_session(state, id, cols, rows)` (only when disconnected, else `session_active`); `answer_prompt(state, id, prompt_id: u64, answer: PromptAnswer)`; `resume_session(state, id, skip_credential: bool)`; `open_external_url(app: AppHandle, url: String)` (only `http` and `https`, else `invalid_url`; opens through the opener plugin's Rust API). Unknown ids give `not_found`.
- Produces (`session_flow.rs`): `FlowControl::new(high: usize, low: usize)`, `on_sent(&mut self, n: usize)`, `on_ack(&mut self, n: usize)`, `should_send(&self) -> bool` (hysteresis: stops above `high`, resumes at or below `low`).

- [ ] **Step 1: Write the failing tests.**
  - `session_flow::tests`: `stops_above_high_and_resumes_at_low`, `ack_larger_than_unacked_saturates_at_zero`.
  - `sessions::tests` (no server needed): `prompt_hub_delivers_the_answer_to_the_waiting_ask`, `answer_for_an_unknown_prompt_id_is_not_found`, `dropping_the_hub_resolves_every_waiting_ask_with_none`, `session_event_json_has_no_secret_field` (serialises one of each event and prompt), `answer_debug_is_redacted`, `open_external_url_accepts_only_http_and_https` (pure URL check function).
  - `sessions::tests` against the OpenSSH container (skip rule as in Task 4; plain functions with a recording `Channel::new`): `open_session_password_round_trip` (vault unlocked with a password entry referenced by the node → `connecting`, `connected`, output contains the shell prompt, `session_input` echo works), `host_key_prompt_is_emitted_and_answer_continues`, `needs_vault_waits_and_resume_continues` (locked vault; `needsVault` state; unlock; `resume_session(id, false)` connects), `resume_with_skip_asks_for_the_password_instead`, `flood_stops_at_high_water_mark_and_ctrl_c_returns_within_one_second` (run `yes`, ack nothing for 2 s, assert received bytes ≤ high + 2 × max batch; then ack everything as it arrives, send `\x03`, then `echo MARK$((1+2))\n`, expect `MARK3` within 1 s), `closing_the_session_while_a_prompt_is_open_ends_connect` (pending host key prompt; `close_session`; the connect task ends, `known_hosts` unchanged), `reconnect_uses_current_settings_and_reports_a_deleted_node` (change the node's host after connecting, drop the session, `reconnect_session` connects to the new host; delete the node, reconnect → `disconnected` with the detail "the connection no longer exists"), `server_drop_ends_in_disconnected_with_a_reason`.
  - No-secret test: add every new command to `table()` with sentinel secrets (`answer_prompt` with a `secrets` sentinel on an unknown id: the error must not contain it), list `open_external_url` in `APP_HANDLE_EXEMPT` ("takes an AppHandle; returns nothing"); add `session_commands.rs` and `sessions.rs` to `sources` if they contain commands.
- [ ] **Step 2: Run** `cargo test -p fernjo-app` → FAIL.
- [ ] **Step 3: Implement.** `SessionManager` (sessions by id; each entry holds the node id, the stored channel, the current `Session`, the prompt hub, flow control, abort handles). `TauriInteraction` implements `Interaction` by emitting a `prompt` event and awaiting a oneshot from the hub; a dropped sender returns the safe answer (`Reject`, `None`) so connect ends. Each session runs in a spawned task: emit `connecting`; resolve the plan (`NeedsVault` → emit `needsVault` and wait for `resume_session`); connect with `ConnectParams` (known_hosts path `<data_dir>/known_hosts`); emit `connected`; forward `Output` in batches (about 16 ms or 64 KiB) as raw channel messages while `FlowControl::should_send`; on `Closed` emit `disconnected` with the reason text. `session_ack` feeds `on_ack` and wakes the forwarder. `close_session` aborts the tasks, drops the hub and removes the entry. `reconnect_session` re-resolves the plan from the tree (a missing node → `disconnected` with "the connection no longer exists"), reuses the stored channel and starts a fresh `Session`. Verify the raw-binary channel and raw request body against the Tauri 2 docs (`InvokeResponseBody::Raw`, `tauri::ipc::Request`) before relying on them; record any deviation in `Docs/DECISIONS.md`.
- [ ] **Step 4: Run** `cargo test --workspace` (with the server env) and `cargo clippy --workspace --all-targets -- -D warnings`; `cargo deny check` (the opener plugin adds dependencies) → PASS.
- [ ] **Step 5: Commit** `git add src-tauri Cargo.lock Docs/DECISIONS.md` · `git commit -m "feat(app): session manager with prompts, flow control and session commands"`.

---

### Task 8: UI api, session store, tab reorder, shortcut routing

**Files:**
- Create: `ui/src/lib/sessions.svelte.ts`, `sessions.test.ts`, `tabs.ts`, `tabs.test.ts`, `shortcuts.ts`, `shortcuts.test.ts`
- Modify: `ui/src/lib/types.ts`, `ui/src/lib/api.ts`, `ui/src/lib/testing.ts`

**Interfaces:**
- Produces (`types.ts`): `SessionState = "connecting"|"needsVault"|"connected"|"disconnected"`; `SessionEvent`, `Prompt`, `PromptAnswer` mirroring the wire types of Task 7; `TerminalSettings = { fontFamily: string|null; fontSize: number; scrollback: number; persistent: boolean }`; new `Api` members: `openSession(nodeId: string, onEvent: (e: SessionEvent | ArrayBuffer) => void, cols: number, rows: number): Promise<string>`, `sessionInput(id: string, data: Uint8Array): Promise<void>`, `sessionResize(id, cols, rows)`, `sessionAck(id, bytes)`, `closeSession(id)`, `reconnectSession(id, cols, rows)`, `answerPrompt(id, promptId, answer)`, `resumeSession(id, skipCredential)`, `openExternalUrl(url)`, `getTerminalSettings(): Promise<TerminalSettings>`, `setTerminalSettings(s: { fontFamily: string|null; fontSize: number; scrollback: number }): Promise<TerminalSettings>`. `tauriApi` implements them (a `Channel` from `@tauri-apps/api/core`; input as a raw body with the session id header); `mockApi` in `testing.ts` gets matching `vi.fn()` defaults.
- Produces (`tabs.ts`): `reorder<T extends { id: string }>(tabs: T[], fromId: string, beforeId: string | null): T[]` (`null` = to the end; unknown ids return the input unchanged).
- Produces (`shortcuts.ts`): `type AppAction = "nextTab"|"prevTab"|"closeTab"|"focusFilter"|"terminalSearch"|"copy"|"paste"`; `routeKey(e: Pick<KeyboardEvent,"key"|"ctrlKey"|"shiftKey"|"altKey"|"metaKey">, mac: boolean, inTerminal: boolean): AppAction | null` (`null` = not an app shortcut; the terminal or the page gets it). Chords: Ctrl+Tab / Ctrl+Shift+Tab (all platforms), close Ctrl+Shift+W (Cmd+W on macOS), focus filter Ctrl+Shift+K (Cmd+K), terminal search Ctrl+Shift+F (Cmd+F), copy Ctrl+Shift+C (Cmd+C), paste Ctrl+Shift+V (Cmd+V); Cmd chords only when `mac`; copy and paste only when `inTerminal`.
- Produces (`SessionStore`): `tabs = $state<Tab[]>([])`, `activeId`, with `Tab { id: string /* local key */; sessionId: string|null; nodeId: string; title: string; state: SessionState; detail: string|null; prompt: {id: number; prompt: Prompt}|null }`; `open(nodeId: string, opts?: { another?: boolean })` (focus the existing tab of that node unless `another`; second tab titled `name (2)`), `activate(id)`, `next()`, `previous()`, `close(id)` (calls `closeSession` when a session exists), `reconnect(id)`, `reorder(fromId, beforeId)`, `answer(id, answer)` (calls `answerPrompt` and clears the prompt), `resume(id, skipCredential)`, `lampFor(nodeId): "live"|"connecting"|"dropped"|null` (priority live > connecting > dropped; `needsVault` counts as connecting), and the terminal bridge `attachTerminal(id, bridge: { write(bytes: Uint8Array): void; size(): {cols: number; rows: number} }): void` (starts the session after the first fit; flushes output that arrived before attach), `input(id, bytes)`, `resize(id, cols, rows)` (ignored when `cols` or `rows` is below 1), `ack(id, bytes)`.

- [ ] **Step 1: Write the failing tests.** `tabs.test.ts`: `reorder_moves_before_target`, `reorder_to_end_with_null`, `reorder_unknown_id_returns_input`. `shortcuts.test.ts`: `plain_ctrl_letters_and_alt_letters_reach_the_terminal` (Ctrl+C, Ctrl+W, Ctrl+B, Alt+B → `null`), `escape_is_never_an_app_shortcut`, `ctrl_tab_and_ctrl_shift_tab_switch_tabs_everywhere`, `ctrl_shift_chords_map_to_actions`, `cmd_chords_only_on_mac`, `ctrl_shift_w_is_not_close_on_mac`, `copy_and_paste_only_inside_the_terminal`. `sessions.test.ts` (mock api): `open_creates_a_tab_and_starts_the_session_after_attach`, `open_again_focuses_the_existing_tab`, `another_opens_a_second_tab_titled_name_2`, `state_events_update_the_tab_and_detail`, `prompt_event_sets_the_prompt_and_answer_clears_it_and_calls_the_api`, `output_before_attach_is_flushed_on_attach`, `close_calls_closeSession_and_removes_the_tab_and_activates_a_neighbour`, `close_before_the_session_id_is_known_still_ends_the_session` (race), `reconnect_calls_reconnectSession_with_the_current_size`, `reconnect_writes_a_dim_separator_line_into_the_terminal_and_keeps_earlier_output` (the sink receives `\r\n\x1b[2m— reconnected —\x1b[0m\r\n`), `resize_ignores_zero_sizes`, `ack_and_input_forward_to_the_api`, `lamp_priority_live_over_connecting_over_dropped`, `lamp_is_null_without_tabs`, `open_error_shows_a_message_and_leaves_no_tab`.
- [ ] **Step 2: Run** `pnpm test` → FAIL.
- [ ] **Step 3: Implement** the types, `api.ts` wrappers (follow the existing `call` pattern; `openSession` builds the `Channel`, its `onmessage` forwards `ArrayBuffer` or parsed events to the callback), `testing.ts` mocks, `tabs.ts`, `shortcuts.ts`, `SessionStore`. `pnpm lint` must stay green.
- [ ] **Step 4: Run** `pnpm test`, `pnpm lint` → PASS.
- [ ] **Step 5: Commit** `git add ui/src/lib` · `git commit -m "feat(ui): session store, tab reorder and shortcut routing"`.

---

### Task 9: Terminal (xterm.js controller and view)

**Files:**
- Create: `ui/src/lib/terminal/theme.ts`, `theme.test.ts`, `controller.ts`, `controller.test.ts`, `TerminalView.svelte`, `TerminalView.test.ts`, `SearchBar.svelte`
- Modify: `package.json`, `pnpm-lock.yaml` (`pnpm add -E @xterm/xterm @xterm/addon-fit @xterm/addon-webgl @xterm/addon-search @xterm/addon-web-links`, exact pinned versions)

**Interfaces:**
- Consumes: `SessionStore` terminal bridge (Task 8), `Api.openExternalUrl`, `TerminalSettings`.
- Produces: `terminalTheme(style: CSSStyleDeclaration): ITheme` (maps `--term-bg`, `--term-fg`, `--term-cursor`, `--term-selection` and `--term-ansi-0..15`); `class TerminalController { constructor(opts: { fontFamily: string|null; fontSize: number; scrollback: number }, deps: { onData(bytes: Uint8Array): void; onResize(cols: number, rows: number): void; onAck(bytes: number): void; openUrl(url: string): void }); mount(el: HTMLElement): void; write(bytes: Uint8Array): void; setVisible(visible: boolean): void; setOptions(o: { fontFamily: string|null; fontSize: number; scrollback: number }): void; fit(): void; focus(): void; search(term: string, dir: "next"|"previous"): boolean; copySelection(): Promise<void>; paste(): Promise<void>; refreshTheme(): void; dispose(): void }`; `<TerminalView tab sessions settings visible />` (one per tab, hidden tabs stay mounted with `display: none`).
- Behaviour fixed here: `onData` strings go out as UTF-8 bytes and `onBinary` as Latin-1 bytes; `write` acknowledges the byte count in the xterm write callback; `minimumContrastRatio` from `--terminal-min-contrast`; WebGL addon created on `setVisible(true)`, disposed on `setVisible(false)` and on context loss (fall back to the DOM renderer, no error shown); `fit()` does nothing and `onResize` is not called while the element has no size; links open only through `openUrl`.

- [ ] **Step 1: Write the failing tests** (xterm and its addons mocked with `vi.mock`; theme test uses a stub `CSSStyleDeclaration`): `theme_maps_every_term_token_to_the_xterm_theme`, `onData_sends_utf8_bytes_and_onBinary_sends_latin1_bytes`, `write_acks_the_byte_count_after_xterm_has_written`, `fit_while_hidden_sends_nothing`, `fit_after_show_sends_the_size_once`, `webgl_is_loaded_only_while_visible_and_disposed_on_hide`, `webgl_context_loss_falls_back_without_throwing`, `minimum_contrast_ratio_comes_from_the_token`, `setOptions_updates_font_size_and_scrollback_live`, `links_call_openUrl_and_never_navigate`, `search_returns_whether_a_match_was_found`, `terminal_view_registers_the_bridge_and_disposes_on_destroy`.
- [ ] **Step 2: Run** `pnpm test` → FAIL.
- [ ] **Step 3: Clipboard check on Windows, then implement.** In the real app (`pnpm tauri dev` with `FERNJO_DATA_DIR` set and the DevTools port, see `CLAUDE.md`), check whether `navigator.clipboard.writeText` works from a user gesture and whether `readText()` pastes without a permission prompt in WebView2. If both work, `copySelection`/`paste` use the browser API; if not, add Rust clipboard commands (new dependency, own `table()` entries) in a follow-up step of this task. Either way record the result and the reason in `Docs/DECISIONS.md` during Task 13. Implement the controller (fit, WebGL, search, web-links, theme from computed style, context menu items Copy / Paste / Search…, Escape closes the search bar), `TerminalView` (creates the controller with the `SessionStore` bridge, `ResizeObserver` → `fit`, visibility effect, settings effect → `setOptions`, theme change → `refreshTheme`) and `SearchBar` (tokens only). Import `@xterm/xterm/css/xterm.css`; check `ui/src/styles/tokens.test.ts` still passes (the hex-only guard covers the token files).
- [ ] **Step 4: Run** `pnpm test`, `pnpm lint`, `pnpm ui:build` → PASS; confirm `ui/dist` contains the xterm CSS and no inlined `data:` assets.
- [ ] **Step 5: Commit** `git add ui package.json pnpm-lock.yaml` · `git commit -m "feat(ui): xterm.js terminal with WebGL, search and links"`.

---

### Task 10: Tab strip, session area, lamps and connect actions in the tree

**Files:**
- Create: `ui/src/lib/components/TabStrip.svelte`, `SessionArea.svelte`, `Lamp.svelte` and their tests (`TabStrip.test.ts`, `SessionArea.test.ts`, `Lamp.test.ts`, `TreeView.sessions.test.ts`)
- Modify: `Icon.svelte` (`close`, `reconnect`), `TreeRow.svelte` (lamp), `TreeView.svelte` (double-click, Enter, menu items, sessions prop), `Sidebar.svelte`, `ui/src/App.svelte`

**Interfaces:**
- Consumes: `SessionStore` (Task 8), `TerminalView` (Task 9), `ConfirmDialog`, `ContextMenu`.
- Produces: `<Lamp state="connecting"|"live"|"dropped" size="tree"|"tab" />` (shapes per `DESIGN.md`: pulsing ring, filled dot, struck ring; 9 px in the tree, 10 px in tabs; pulse off under reduced motion via the existing tokens); `<TabStrip {sessions} />`; `<SessionArea {sessions} {vault} {settings} />`; `TreeView` and `Sidebar` take `sessions: SessionStore`.
- Behaviour fixed here: double-click and Enter on a connection call `sessions.open(id)`; context menu gets "Connect" (connections only, first) and "Open another session" (only when the node already has a tab) that calls `open(id, { another: true })`; an RDP connection shows the message "RDP sessions are not available yet" through the store's existing message mechanism and opens no tab; `Tab` shows state mark, protocol icon, title, 11 px status caption (`Connecting…`, `Needs vault`, the detail text when disconnected), close button; **Reconnect** (yellow, 11 px) on every disconnected tab; closing a **connected** tab opens `ConfirmDialog` (Cancel focused, Escape keeps the tab), closing any other tab closes immediately; tabs reorder by pointer drag using `reorder` from `tabs.ts` (pointer events with a 4 px threshold, Escape cancels, no HTML5 drag and drop); tab area with no tab keeps the empty hint "Open a connection from the tree"; the vault manager view replaces the session area while open and the terminals keep running behind it (they stay mounted).

- [ ] **Step 1: Write the failing tests.** `Lamp.test.ts`: `renders_a_distinct_shape_per_state` (assert `data-state` and the shape element, not colour). `TabStrip.test.ts`: `shows_state_mark_icon_title_and_caption`, `reconnect_button_only_on_disconnected_tabs`, `clicking_reconnect_calls_the_store`, `closing_a_connected_tab_asks_first_and_cancel_keeps_it`, `closing_a_dropped_tab_does_not_ask`, `pointer_drag_reorders_and_escape_cancels`, `active_tab_is_marked_for_assistive_tech`. `TreeView.sessions.test.ts`: `double_click_opens_a_connection`, `enter_opens_the_selected_connection_and_f2_still_renames`, `context_menu_has_connect_and_open_another_only_when_open`, `rdp_connection_shows_a_message_and_opens_no_tab`, `row_shows_the_lamp_for_the_strongest_session_state`, `folder_double_click_still_toggles`. `SessionArea.test.ts`: `renders_a_terminal_view_per_tab_and_only_the_active_one_is_visible`, `vault_view_hides_but_keeps_terminals_mounted`.
- [ ] **Step 2: Run** `pnpm test` → FAIL.
- [ ] **Step 3: Implement** the components and wiring in `App.svelte` (a `SessionStore` created once like the other stores; `view` stays `"empty" | "vault"` and the session area is shown whenever tabs exist and the vault view is not open).
- [ ] **Step 4: Run** `pnpm test`, `pnpm lint` → PASS; check the layout against `DESIGN.md` ("Tabs": 36 px strip, 150–260 px tabs, 1 px rules) in `pnpm tauri dev` over the DevTools port (screenshot light and dark, attach nothing sensitive).
- [ ] **Step 5: Commit** `git add ui` · `git commit -m "feat(ui): tab strip, session area, lamps and connect actions"`.

---

### Task 11: Inline session prompts and DESIGN.md

**Files:**
- Create: `ui/src/lib/components/SessionPrompt.svelte`, `HostKeyPrompt.svelte`, `HostKeyChangedPrompt.svelte`, `SecretsPrompt.svelte` (keyboard-interactive, password, key passphrase, username share it), `NeedsVaultPrompt.svelte` and a test file per component
- Modify: `ui/src/lib/components/SessionArea.svelte`, `DESIGN.md`

**Interfaces:**
- Consumes: `Tab.prompt`, `SessionStore.answer/resume`, `VaultStore` unlock method used by `VaultNote.svelte` (read it for the exact name), `PasswordField`, `TextField`, `Icon`.
- Produces: `<SessionPrompt {tab} {sessions} {vault} />` choosing the variant from `tab.state === "needsVault"` or `tab.prompt.prompt.kind`.
- Behaviour fixed here (layout of the existing inline unlock prompt: 420 px column, 88 px from the top, 14 px gaps; other tabs keep running): **host key**: shows host, port, key type, fingerprint (mono); actions "Trust and connect" (yellow, always), "Trust once" (outlined), "Cancel" (outlined; answers `reject`); **host key changed**: warning icon and title, both fingerprints, explanation that the server's key differs from the stored one; initial focus on Cancel; "Replace key and connect" outlined with `--color-danger` text, never yellow; Tab cycles only between the two buttons; **secrets** (keyboard-interactive, password, passphrase): one field per prompt, masked unless the server's `echo` is true, show toggle for masked ones, yellow "Continue", outlined "Cancel" (answers `cancel`); username prompt uses a plain text field; **needs vault**: lock icon, "<name> needs the vault", muted line naming that other tabs keep running, master password field, "Unlock automatically on this device" checkbox, yellow "Unlock and connect" (calls vault unlock then `resume(id, false)`; a wrong password shows the existing danger state and keeps the prompt), outlined "Cancel" (calls `resume(id, true)`). Escape cancels; typed values are cleared from component state after submit.
- `DESIGN.md` gets a "Session prompts" section (the variants above, the focus rules, the danger action) using only existing tokens.

- [ ] **Step 1: Write the failing tests** (one file per component): `host_key_prompt_shows_fingerprint_and_maps_buttons_to_answers`, `host_key_changed_focuses_cancel_and_replace_is_not_the_accent_button`, `host_key_changed_tab_cycles_between_the_two_buttons_only`, `secrets_prompt_masks_unless_echo_and_sends_values_in_order`, `secrets_prompt_cancel_sends_cancel_and_escape_does_the_same`, `typed_values_are_cleared_after_submit`, `username_prompt_sends_text`, `needs_vault_unlock_calls_vault_then_resume`, `needs_vault_wrong_password_shows_error_and_keeps_prompt`, `needs_vault_cancel_resumes_with_skip`, `session_prompt_picks_the_variant`.
- [ ] **Step 2: Run** `pnpm test` → FAIL.
- [ ] **Step 3: Implement** the components, mount `SessionPrompt` over the terminal area of its tab (the terminal stays mounted underneath and keeps receiving output), and write the `DESIGN.md` section.
- [ ] **Step 4: Run** `pnpm test`, `pnpm lint` → PASS. In the running app against the test container (`tools/ssh-test-server.sh start`, `FERNJO_DATA_DIR` scratch folder, DevTools port) walk through: host key prompt, then trust always, then reconnect without prompt; a password prompt; the needs-vault prompt; screenshot each in light and dark.
- [ ] **Step 5: Commit** `git add ui DESIGN.md` · `git commit -m "feat(ui): inline session prompts for host keys, secrets and the vault"`.

---

### Task 12: Terminal settings in the gear dialog and app shortcuts wiring

**Files:**
- Create: `ui/src/lib/terminal-settings.svelte.ts`, `terminal-settings.test.ts`
- Modify: `ui/src/lib/components/DataFolderDialog.svelte` (+ its test; rename the visible title and the gear's accessible name to "Settings"), `Sidebar.svelte` (gear label), `ui/src/App.svelte` (window `keydown` routing through `routeKey`), `TerminalView.svelte` (settings effect already added; confirm)

**Interfaces:**
- Produces: `TerminalSettingsStore(api)` with `info = $state<TerminalSettings|null>`, `load()`, `save(input: { fontFamily: string|null; fontSize: number; scrollback: number })` (errors shown inline by the dialog from the Rust code).
- Behaviour fixed here: the dialog keeps the data folder section unchanged and gains a "Terminal" section (font family text, blank = default stack; font size 8–32; scrollback 1000–100000), Save applies live to open terminals; while `persistent` is false (`FERNJO_DATA_DIR` set) the section says "Applies to this run only; settings are not saved while FERNJO_DATA_DIR is set"; `App.svelte` routes `keydown` through `routeKey` (target inside `.xterm` means `inTerminal`), performs `nextTab`/`prevTab`/`closeTab` (same confirm rule as the × button)/`focusFilter`/`terminalSearch`, calls `preventDefault` only for handled chords.

- [ ] **Step 1: Write the failing tests:** `terminal-settings.test.ts`: `load_fills_info`, `save_updates_info_and_surfaces_rust_error_codes`. Dialog: `terminal_section_saves_the_values_and_shows_inline_errors`, `non_persistent_note_is_shown_when_persistent_is_false`, `data_folder_section_still_works` (existing tests keep passing with the renamed label). App: `ctrl_shift_w_closes_the_active_tab_with_confirm_for_a_connected_one`, `ctrl_tab_cycles_tabs_and_plain_ctrl_w_inside_the_terminal_is_untouched`, `ctrl_shift_k_focuses_the_filter`.
- [ ] **Step 2: Run** `pnpm test` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `pnpm test`, `pnpm lint`, `pnpm ui:build` → PASS.
- [ ] **Step 5: Commit** `git add ui` · `git commit -m "feat(ui): terminal settings in the settings dialog and app shortcuts"`.

---

### Task 13: Docs, manual verification, roadmap evidence, PR

**Files:**
- Modify: `README.md` (shortcut table and a short "SSH" section), `CHANGELOG.md` (`[Unreleased]`), `CLAUDE.md` ("Project-specific": session manager, `FERNJO_TEST_SSH_*` variables and the skip-or-fail rule, `tools/ssh-test-server.sh`), `Docs/DECISIONS.md` (xterm versions and addons, WebGL policy, clipboard path, flow control numbers, `known_hosts` location and format, raw channel and request decisions from Task 7), `Docs/roadmap.json` (tick tasks and criteria with evidence, backlog entries, manual checks), regenerated `Docs/ROADMAP.md`

- [ ] **Step 1: Docs.** Write the README shortcut table (the chords of Task 8, Ctrl vs Cmd), the CHANGELOG entry (M4: SSH tabs, host key verification, agent/key/password/keyboard-interactive auth, terminal settings, shortcuts), the `CLAUDE.md` and `Docs/DECISIONS.md` entries listed above.
- [ ] **Step 2: Backlog.** Add to `Docs/roadmap.json` `backlog`: "Confirm quit with live sessions" and "Restore tabs after restart". Add under `blocks` "Open manual checks (owner)" an M4 paragraph: Linux (WebGL fallback on WebKitGTK, Wayland copy and paste, Secret Service vault path with a session, `SSH_AUTH_SOCK` agent) and macOS (Cmd shortcuts, launchd agent socket, keychain prompt on second launch while sessions run).
- [ ] **Step 3: Manual verification on Windows** (real app, `FERNJO_DATA_DIR` scratch folder, `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`, drive input over the DevTools protocol, never the user's mouse or keyboard): against the test container, install `vim` and `htop` in it (`docker exec … apk add vim htop`) and check they render and survive a resize; ten tabs open at once with `htop` in one and `yes` in another (UI stays responsive, Ctrl+C within 1 s in the `yes` tab); host key prompt, trust always, mismatch warning after editing `known_hosts`; password, key (Ed25519, RSA, encrypted), agent (Windows OpenSSH agent) and keyboard-interactive; copy and paste; a dropped connection (`docker stop`) shows the struck lamp and the yellow Reconnect, which works after `docker start`; light and dark screenshots. Real hosts: the owner's remaining check is each auth method against a real host; mark it **open** with the command that verifies it (`pnpm tauri dev`, connect, confirm) if it was not run.
- [ ] **Step 4: Full gate** (state each command and its result in the PR and in the roadmap evidence): `pnpm install --frozen-lockfile`, `pnpm ui:build`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (with `FERNJO_TEST_SSH_*` set), `pnpm lint`, `pnpm test`, `cargo deny check`, `dotnet run tools/roadmap.cs -- generate` then `-- check`, and `pnpm tauri build --no-bundle` (check the CSP served by `http://tauri.localhost/` has no new warning and the terminal and IPC work in the release binary).
- [ ] **Step 5: Roadmap.** Tick every finished task in `Docs/roadmap.json`; tick each acceptance criterion only with its evidence (CI run URL for the container tests, the manual-check notes, the gate output); set status `done` with `completedOn` only after all are verified; regenerate `Docs/ROADMAP.md`; `dotnet run tools/roadmap.cs -- check`.
- [ ] **Step 6: Commit and PR.** `git add README.md CHANGELOG.md CLAUDE.md Docs/DECISIONS.md Docs/roadmap.json Docs/ROADMAP.md` · `git commit -m "docs: milestone 4 done – docs, decisions and roadmap evidence"`; open a PR to `dev`; merge it (squash allowed) once every CI check is green, as `CLAUDE.md` allows.

---

## Self-review

- **Spec coverage:** crate with PTY, resize, keepalive, timeout (T1, T4); auth order and budget (T3, T5); host keys, prompts, mismatch (T2, T4, T11); session manager, channel events, raw output, flow control, commands, opener (T7); credential resolution incl. vault-locked, dangling, unusable (T6, T7, T11); terminal with the four addons, theme, minimum contrast, WebGL policy, clipboard decision, search, links (T9); tab strip, lamps, connect actions, reconnect keeps buffer (T8, T10, the buffer is kept because the xterm instance is not recreated; no separate test needed beyond `reconnect_calls_reconnectSession_with_the_current_size`), confirm on closing a connected tab, reorder (T10); prompts and `DESIGN.md` (T11); terminal settings and the M6 scope-change note (T6, T12, roadmap); shortcuts documented in the README (T8, T12, T13); integration tests in CI without a new required check (T4, T5); no-secret test extended (T6, T7); docs and backlog items (T13).
- **Type consistency:** `SessionEvent`/`PromptView`/`PromptAnswer` are defined once (T7) and mirrored in T8; `TerminalSettings` (UI, with `persistent`) versus `TerminalSettingsInput` (no `persistent`) match `get`/`set` in T6; `resume_session(id, skip_credential)` is the same in T7, T8 and T11 (the spec's one-argument form gained the flag so Cancel can skip the credential; the spec text is updated to match); `Tab.id` is the local key and `sessionId` the Rust id throughout.
- **Proportion:** the plan is longer than the spec because it names signatures, tests and values; function bodies are left to the implementer.
