# M10 design: daily-use features

Status: draft for approval (2026-11-23). Milestone 10 of `Docs/ROADMAP.md`, subproject 1. Prerequisite M8 (done); M9 (sync) is merged. M11 (hardening and security check) follows and reviews everything built here, so every feature stays small and testable.

## Intent and why

The owner uses fernjo daily and keeps missing nine small things: connecting to a host without first saving it, getting yesterday's tabs back after a restart, the vault locking itself when the machine sits idle, Pageant as an SSH agent on Windows, a quit confirmation on macOS, a context-menu Paste that works, a clearer restore dialog, a splitter focus style that matches `DESIGN.md`, and the real bugs among the M4 review leftovers. None of them is large; together they remove the daily friction that is left after M9.

## Goals

- Each feature below works as described, with automated tests for its logic (Rust for logic, TypeScript only for presentation).
- No secret reaches `connections.json`, `settings.json`, `open-tabs.json`, logs, sync data or webview storage. The no-secret guard (`no_response_contains_a_secret`) covers every new command.
- Every new command is registered in all three places (`CLAUDE.md`, "No-secret guard").

## Non-goals and anti-goals

- No "Save as connection" from quick connect, no connection history or MRU list, no quick-connect folder in the tree.
- No automatic reconnect on start; no restoring of terminal scrollback or RDP screen content.
- No idle lock of anything but the vault; no lock on OS sleep or OS screen lock; no change to how sync pauses while the vault is locked.
- No new settings for Pageant (it is tried automatically).
- Anti-goal: a restart that fires a prompt wall or network traffic nobody asked for; a quick connect that leaves a trace in synced data.
- The cosmetic M4 leftovers (tab ARIA, `.reconnect` weight, lock icon size, `ResizeObserver` coalescing and the like) are not part of M10 and stay in the backlog.

## Approaches considered

**Quick connect.** (A) A session target that is either a saved node or a quick target; chosen. (B) A hidden temporary node in the tree: reuses everything but writes `connections.json` and syncs a connection the owner never saved; rejected.

**Restore tabs.** (A) The UI reports the ordered tab list, Rust validates and stores it in the device folder; chosen. (B) Rust derives the list from running sessions: loses tab order and the active tab, which are view state; rejected.

**Vault auto-lock.** (A) A Rust timer fed by a throttled activity heartbeat from the UI; chosen. (B) A timer in the UI that calls `vault_lock`: a reloaded or hung page silently stops locking; rejected.

Decided by the owner on 2026-11-23: restored tabs come back **disconnected** with a "Reconnect all" button (not auto-connected, not "auto-connect only prompt-free tabs"); the BSL-1.0 licence is **allowed**.

## Feature design

### 1. Quick connect

- Input syntax `[ssh://|rdp://]user@host[:port]`; no scheme means SSH. For SSH the user is required; for RDP it is optional (the RDP username prompt exists). IPv6 hosts use brackets with a port (`user@[::1]:2222`). The parser lives in Rust (`fernjo-core` or `src-tauri`, builder's choice) and rejects everything `fernjo_ssh::validate_host` rejects (`invalid_host`), through `host_port`.
- A session's target becomes `Target::Node(NodeId)` or `Target::Quick(QuickTarget { protocol, host, port, user })` in the session manager's `Entry` (today it holds `node_id`). `connect_plan` gets a resolve path for a quick target: no vault credential, so the plan has `CredentialPlan` without a stored credential and the existing typed-password (SSH) and username/password (RDP) prompts apply. Agent keys apply as for any SSH session. RDP uses the built-in defaults for resolution and colour depth (there is no tree to inherit from) and has no domain (a domain typed into the username, if the RDP prompt accepts one, is the builder's check; no new prompt). The RDP clipboard for a quick target is **off** (owner decision, 2026-11-23: a typed, unsaved host is the least trusted case; the global built-in default for saved connections stays on, `DEFAULT_RDP_CLIPBOARD`, and M11 reviews that default separately).
- Reconnect works for a quick session (the target stays in the `Entry`). Nothing is written to `connections.json`, the vault or `known_hosts` beyond what any session does (a host key confirmation is the same as for a saved connection).
- A new command (`open_quick_session` or an argument variant of `open_session`; builder's choice) takes the typed string, so parsing and validation stay in Rust. The UI only shows the field and the error text.
- UI: an entry point that is discoverable without a hint (a field or button near the tree filter) plus a shortcut. Plain Ctrl+letter and Alt+letter belong to the shell (`routeKey`), so the shortcut is a Ctrl+Shift+letter (Cmd+letter on macOS) chord that is not yet in `SHORTCUTS` (Ctrl+Shift+K, W, F, C and V are taken; suggest Ctrl+Shift+N; builder's call, not an owner decision); add it to `SHORTCUTS` so Settings lists it and the test that `routeKey` agrees with the list stays green. Follow `DESIGN.md` and the tokens.

### 2. Restore tabs after restart

- A setting "Reopen tabs on start" (default on), stored in `settings.json`. Under `FERNJO_DATA_DIR` the setting is memory-only like the theme (`unsavedReason`).
- The list lives in `open-tabs.json` in the device folder (per device, never synced, never in the data folder; for scratch runs `<FERNJO_DATA_DIR>/.device`). Content: schema version, ordered entries (`node` id, or the quick target's protocol, host, port and user), the active entry. No secrets, no session ids. Capped read (`read_capped`), atomic write, no `.bak`. A file that is unreadable or from a newer schema is treated as empty and replaced by the next save (it is a convenience, so it must never block the start).
- The UI sends the ordered list through one command after every explicit tab change (open, close, reorder, active tab switch). Rust drops ids that no longer exist in the tree and caps the count.
- **Pitfall 1:** `SessionManager::close_all` runs on every main-window page load and at quit, so closes that come from it must never be saved. Only UI-driven changes save.
- **Pitfall 2:** the UI starts with no tabs, so it must not report its list before the restore has finished, or it would overwrite the saved list with an empty one.
- On start, with the setting on, the UI asks for the saved list and Rust creates one session entry per tab in the disconnected state (a new command next to `open_session`; the existing reconnect then works). A disconnected entry that never connected may not exist as a valid state in `sessions.rs` today; if it does not, adding it (a phase or a flag, with the wire contract at the top of `sessions.rs` and `SessionStore` updated) is part of this feature. A restored tab must never be treated as "ended cleanly" (no auto-close), and it does not count as connected for the close confirmation (only `Phase::Connected` counts).
- **Nothing is snapshotted.** The saved entry holds only the target. A restored tab takes its title, protocol and icon from the node as it is now (or from the quick target), and the plan is resolved again at Reconnect time, so a changed host, protocol or credential is picked up. The reason bar for a restored tab reads "Restored from your last session. Not connected." (owner may reword); it is not the dropped-connection text.
- **What saves the list:** every change of the UI's tab list (open, close, reorder, active tab), including a tab the UI closes itself because a shell ended cleanly (`closeOnExit`); a cleanly exited tab must not come back after the next start. One effect over the tab list in `SessionStore` does it, and it is switched off until the restore has finished (pitfall 2).
- **"Reconnect all" and a locked vault:** if any restored tab references a vault credential and the vault is locked, "Reconnect all" asks for the unlock once first (the normal unlock dialog) and only then connects the tabs. A tab without a credential connects at once. Minimum acceptable behaviour: never N unlock prompts for N tabs from this action. Other prompts (host key, password) stay per tab; each shows in its own tab. Connect the tabs one after another with a small concurrency limit (builder's choice), so a restart does not open a burst of connections.

### 3. Vault auto-lock after idle time

- `AppSettings` gets `vaultAutoLockMinutes` (`Option<u32>`, unset means off, validated range 1 to 1440; an out-of-range value is `invalid_...` like the terminal ranges). New commands: get and set the setting (like `get_theme`/`set_theme`, with `persistent` and `unsavedReason`) and `note_activity`.
- The UI sends `note_activity` on keyboard and pointer input, throttled (about once per 5 s), from passive capture-phase listeners on `window` (`capture: true`), so xterm.js, the RDP canvas and any `stopPropagation` in a view cannot hide the input. Typing in a terminal and working in a remote desktop count (a test with a view that stops propagation proves it); terminal output and background sync do not. A hidden or minimised window produces no input, so it ages out. Locking while someone is actively working would make the feature unusable, so this is decided here and not left open.
- Rust owns the clock: a pure idle tracker over an injected time source (the same pattern as `sync_runner::Scheduler`) and a background thread that checks every few seconds. When the vault is unlocked and the idle time exceeds the setting, it locks through the same path as `vault_lock` (same lock order: pending, vault, store) and emits an event so the UI refetches the vault status.
- Connected sessions keep running; only new credential lookups need the unlock. A connect that needs the vault uses the existing `NeedsVault` flow; sync shows its existing "paused: unlock the vault" line. Startup auto-unlock is unchanged.
- UI: a select or number field in Settings ("Lock the vault after … minutes of inactivity", off by default), explained in the same words style as the other settings.

### 4. Pageant on Windows

- `russh` 0.65 ships `AgentClient::connect_pageant()` on Windows (the `pageant` crate is already in `Cargo.lock`). In `crates/fernjo-ssh/src/agent.rs` the Windows connect tries the OpenSSH named pipe first and uses Pageant when the pipe is unavailable or lists no identities. The type-erased `Agent` stays one type for every platform. The source is chosen once per connect (pick one, not "try each in turn"): a user who has the OpenSSH agent running with keys and a different key only in Pageant gets no Pageant attempt. This is a known limit, recorded in `Docs/DECISIONS.md`; the OpenSSH agent service is off by default on Windows, so both at once is rare.
- README line "Pageant is not supported yet" and the SSH authentication order text are updated.
- Pageant needs a key loaded in Pageant on a real Windows machine (owner check); the automated tests cover the choice between the two sources through an injected connector (a real Pageant cannot run in CI).

### 5. Quit confirmation on macOS Cmd+Q

- Cmd+Q leaves through `RunEvent::ExitRequested`, not `CloseRequested`. The builder switches `run()` to `build()` plus a run callback and, when `needs_close_confirmation(connected_count)` holds, calls `prevent_exit`, shows the same "Close fernjo?" dialog guarded by `close_prompt_open`, and on "Close" runs `close_all`, the sync close push and the exit. After `close_all` the connected count is zero, so the exit does not prompt again; the builder verifies that the exit path does not raise a second `ExitRequested` loop. The dialog and the exit are owner checks on macOS. The pure decision stays unit-tested.

### 6. Native clipboard commands

- Decision (owner): the BSL-1.0 licence is allowed. Add `BSL-1.0` to `deny.toml` `allow` and to `about.toml` `accepted` (the two lists stay in step), so the generated third-party notices include `clipboard-win`.
- Add a command that reads clipboard text through a Rust crate (`arboard`, text only, minimal features; the builder checks that the Linux feature set including Wayland resolves and that `cargo deny check` stays green). It is called only from the context-menu Paste (a user gesture) and returns the text to the terminal. Behind a small trait so the no-secret guard table and unit tests do not need a real clipboard. Keyboard paste is unchanged (the native `paste` event). Copy stays on `navigator.clipboard.writeText`.
- Remove the 2.5 s timeout path and its tests in `ui/src/lib/terminal/controller.ts` once the command works; record the decision in `Docs/DECISIONS.md`, superseding "M4 clipboard path". Clipboard text may be a secret the user copied elsewhere: it is never logged and never persisted, and the command's error text never contains it.

### 7. Restore confirmation wording

- The restore confirmation in the import dialog names what is replaced (connections tree, vault entries) and what is kept, instead of counts only. The builder reads `apply_backup` and the restore semantics ("M5 restore semantics and backup container" in `Docs/DECISIONS.md`) to state exactly which things survive, and writes the text from that, not from assumption. UI wording only; no behaviour change.

### 8. Splitter focus style

- Keyboard focus on `Splitter.svelte` is drawn as a 1 px inset line in `--color-focus` inside the hit area (`DESIGN.md`), not an opaque 6 px bar. The vertical splitter's hit area moves so it no longer overlaps the tree's scrollbar edge. Keyboard behaviour, pointer capture and double-click reset stay as they are; hover and drag keep using `--color-focus` with no new token.

### 9. M4 review follow-ups (subset)

The builder first checks that each item is still real in the current code (the M7 triage may have fixed some). Only these are in scope:

- Cap the session input queue (about 8 MiB) so a stalled connection cannot grow memory without bound.
- `TrustAlways` and `Replace` write `known_hosts` before the key-exchange signature is verified; write it after the handshake succeeded.
- Chords use `e.key`; use `e.code` where that fixes Ctrl+Shift chords on non-Latin layouts, without breaking the existing `routeKey` tests.
- Ctrl+Shift+W with the vault view open closes an invisible tab.

Everything else in the roadmap block "Source: M4 review follow-ups" stays in the backlog unchanged (the roadmap records which items moved and which stayed).

## Constraints

- Secrets stay in Rust; all logic is Rust; TypeScript presents. No home-made crypto.
- UI uses only the tokens and `DESIGN.md`; shipped CSS stays within the WebKitGTK 4.1 limits (hex only, no `oklch()`, nesting, `:has()` and so on). A new colour token, if any, goes into the light block and both dark blocks.
- Every new `#[tauri::command]`: function plus `handlers` wrapper, entry in the test `table()` (or `APP_HANDLE_EXEMPT` with a reason), and the `generate_handler!` list in `lib.rs`; the count guard enforces it. A command that takes a path must not be a command.
- All reads of files in the data or device folders use the capped helpers.
- Work stays inside M10; no features, packages or architecture of M11 or later.
- Branch `feature/m10-daily-use-features` from `dev`; PR to `dev`; `CHANGELOG.md` `[Unreleased]` for the user-facing changes; `Docs/DECISIONS.md` entries for the choices above; `CLAUDE.md` "Project-specific" for the lasting knowledge (open-tabs file, activity heartbeat, `Target`); roadmap changes only in `Docs/roadmap.json`, regenerated with `dotnet run tools/roadmap.cs -- generate`.

## Acceptance (mirrors the roadmap)

1. Each feature has automated tests for its logic; the test names are listed as evidence in the roadmap.
2. Owner checks on real machines: quick connect, restore tabs and vault auto-lock each used once; Pageant on Windows with a key loaded in Pageant; Cmd+Q on macOS with a connected session. Anything not run is marked open with the command or steps that verify it.
3. The no-secret guard covers every new command; nothing secret appears in `connections.json`, `settings.json`, `open-tabs.json`, logs or webview storage.
4. `cargo build --workspace`, `cargo test --workspace`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo deny check`, `pnpm test`, `pnpm lint` and `dotnet run tools/roadmap.cs -- check` are green.

## Left to the builder

Names of commands and types; where the quick-connect parser lives; the exact UI placement and shortcut of quick connect; the idle check interval and heartbeat throttle; the cap on saved tabs; the concurrency of "Reconnect all"; the clipboard crate's feature set; which M4 items turn out to be already fixed; the order of the plan's tasks (suggested: small independent items first, then quick connect, then restore tabs because it reuses the quick target, then auto-lock).
