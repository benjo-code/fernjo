# M7 Daily-use polish and hardening: design decisions

Status: draft for owner approval, 2026-11-19. Scope, tasks and acceptance criteria are those of Milestone 7 in `Docs/roadmap.json`; this note records only the decisions the roadmap leaves open. The plan follows from both.

## Intent

v0.1.1 closes the known gaps of v0.1.0: no silent loss of synced data, a UI whose vault, import and settings can be found without a hint, resizable panels, multi-select in the tree, and every Low finding of the M6 reviews settled (fixed, accepted with a reason, or back in the backlog).

## Non-goals

- No new protocol, no RDP work (M8), no restore-tabs-after-restart, no native clipboard commands, no merge of divergent trees.
- No new visual direction: the discoverability work follows `DESIGN.md` and the tokens.

## Decisions

### 1. `connections.json` changed on disk (owner pick: refuse and offer reload)

- `Store` remembers a SHA-256 of the bytes it last read or wrote. Before every save it re-reads the file and compares; a missing file is recreated (as `Vault::save` does), any other difference refuses with `changed_on_disk` and nothing is written.
- The UI shows a notice with a "Reload" action that re-opens the store from disk. The unsaved edit is dropped; nothing from the other device is lost.
- Rejected: length plus mtime (sync tools can preserve both); keeping the pending edit for replay (about twice the work); silent merge (unsafe for deletes and moves).
- It stays a check, not a lock: a sync tool can still win the race between check and rename. Said in `Docs/DECISIONS.md`.

### 2. Close confirmation (my call)

- Rust intercepts the main window's close request; with at least one connected session it prevents the close and asks through a native confirm dialog (the dialog plugin is already registered). Without one the window closes at once.
- Rejected: an in-app dialog (needs a new command and wire event, so the three-places rule and the no-secret table, for the same result).
- To verify during implementation: that blocking inside the close handler behaves on macOS and Linux; the check on those machines stays in the owner's manual list.

### 3. Shell exit rule (my call, within the roadmap's design notes)

- `fernjo-ssh` already reports the remote exit status; the end-of-session event gets an `exitStatus` field in the wire contract (top of `sessions.rs`, rationale in `Docs/DECISIONS.md` "M4 session IPC").
- The tab closes itself only for exit status 0 and only if the session lasted longer than a few seconds. A non-zero status, a transport error, a timeout and a server disconnect stay yellow with Reconnect.
- Setting "Close the tab when the shell exits" in Settings, default on, stored in `settings.json`.

### 4. Plan shape (my call)

One plan file, phases in this order: (1) Rust hardening: data safety, agent sign timeout, settings note, build paths, M6 deferred follow-ups, Low-finding triage; (2) session end handling and close confirmation; (3) resizable panels and multi-select; (4) discoverability review, then an owner gate, then the agreed changes; (5) Linux icon, WebView2 connection, docs, roadmap, release evidence. Phase 4 stops for the owner's agreed list; the plan says so explicitly.

## Left to the builder

Fingerprint API shape, exact wording of notices, the "few seconds" threshold (start at 5 s, record the value), selection-model details of multi-select (range and toggle keys follow platform convention), and the outcome of each Low finding (recorded one by one in `Docs/DECISIONS.md`).

## Constraints that apply throughout

Secrets stay in Rust; new commands follow the three-places rule; shipped CSS stays hex-only and token-based; every acceptance criterion is ticked only with recorded proof.
