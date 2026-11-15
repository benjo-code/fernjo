# fernjo – Milestone 2 design: connection tree & data model

**Status:** approved by the owner on 2026-11-15 · **Date:** 2026-11-15 · **Parent spec:** [2026-11-15-fernjo-design.md](2026-11-15-fernjo-design.md) (§3, §5.1, §5.4) · **Roadmap:** M2

This document records the decisions for M2 only. Intent, goals, non-goals and anti-goals are those of the parent spec; the visual language is `DESIGN.md`.

## 1. Intent

After M2 the owner can organise connections in a persistent folder tree with mRemoteNG-style inheritance and edit them in a properties panel. Nothing connects yet. The tree is the foundation every later milestone reads from (vault references in M3, tabs in M4, import in M5).

## 2. Scope

**In:** model, inheritance, persistence, Tauri commands, sidebar tree (filter, expand/collapse, drag and drop, context menu), properties panel, single instance, IPC CSP follow-up from `Docs/DECISIONS.md`.

**Out (later milestones):** credential reference (M3), opening sessions, lamps, tabs (M4), RDP settings (M7), host column and its setting (M6), undo.

**Anti-goals for M2:** UI-owned tree state; any logic about validity or inheritance in TypeScript; a secret anywhere in `connections.json` (there are none yet, the format must not invite them).

## 3. Approaches

**State ownership.** *Chosen:* Rust owns the tree behind a lock; every mutating command validates, saves atomically and returns the new full snapshot; the UI renders snapshots. At 500 nodes a snapshot is ~100 KB. *Rejected:* UI-owned tree pushed to Rust (breaks "all logic in Rust", drifts); diff/event streams (complexity with no benefit at this size).

**Drag and drop.** *Chosen (owner, 2026-11-15):* pointer-based dragging in the UI; a pure function maps the cursor position to a drop target (before / after / inside); Rust rejects invalid moves. *Rejected:* native HTML5 drag and drop (needs `dragDropEnabled: false`, least reliable on WebKitGTK/Wayland, which is the owner's Linux setup); a Svelte drag library (not built for nested trees with three drop zones).

**Delete.** *Chosen (owner):* confirm dialog naming the node and its descendant count, final, no undo. `connections.json.bak` is the only safety net. *Rejected:* instant delete with undo toast (needs an undo history in the core).

## 4. Data model (`fernjo-core`)

- Nodes: `Folder` and `Connection`, each with UUID v4 `id`, `name`, `description`, `settings`. A connection also has `protocol` (`ssh` | `rdp`) and `host`. The root is implicit (a list of top-level nodes), not a node. Order is the position in the parent's child list.
- **Settings (all optional = inherit):** `username`, `port`, `keepaliveSecs` (SSH; `0` = off). Each field is added to the struct as later milestones need it; unknown fields in a *newer* file are not silently dropped (see §5).
- **Effective value:** first value walking from the node up through its folders, then the built-in default: port by protocol (ssh 22, rdp 3389), keepalive 30 s, username none. `get_node` returns local value, effective value and the source (node itself, a folder id and name, or default) per field, so the UI shows "from <folder>" without implementing inheritance. *(Builder call: keepalive default 30 s.)*
- **Operations:** create folder / connection (at a parent and index), update (patch: each settings field is *set value* or *reset to inherit*), move (new parent and index; rejects a folder into itself or a descendant, an unknown parent, an index out of range), duplicate (deep copy with fresh UUIDs, name "<name> copy", placed right after the original), delete (recursive). Names need not be unique. A new connection may have an empty host; validation happens at connect time (M4). Port must be 1–65535, enforced in Rust.
- Layering: a pure `Tree` (all rules, no I/O) and a `Store` (a `Tree` plus the file path; every mutation saves). Commands are thin wrappers, so nearly everything is testable without Tauri.

## 5. Persistence

- **Format:** `connections.json`, human-readable, nested: `{ "schemaVersion": 1, "children": [ { "id", "type": "folder"|"connection", "name", "description", "settings": {…}, "children": […] | "protocol", "host" } ] }`. Field names are the builder's to finalise within this shape. The format is documented in `Docs/DECISIONS.md`.
- **Location:** `<OS config dir>/fernjo/` (`%APPDATA%\fernjo`, `~/Library/Application Support/fernjo`, `~/.config/fernjo`); the env var `FERNJO_DATA_DIR` overrides it (used for tests and the 500-connection check). The in-app folder chooser is M3/M6. *(Builder call: plain `<config dir>/fernjo`, not the `directories` crate's `ProjectDirs` layout, which would add a `config` subfolder on Windows.)*
- **Write:** temp file in the same directory, flush, copy the current file to `connections.json.bak`, rename the temp file over `connections.json`. A missing file means an empty tree and is created on the first mutation.
- **Load failures:** a file with `schemaVersion` *newer* than the app is refused and never overwritten (the UI shows an error naming the file). An unparsable file falls back to `.bak`; the bad file is first renamed `connections.json.corrupt-<timestamp>` so a later save cannot overwrite the good `.bak` with garbage; the UI shows a notice. If both fail, the app starts with an error state and writes nothing. *(Builder call.)*
- Migration code exists as a function chain from version N to N+1, with a test, even though only version 1 exists.

## 6. Tauri layer

- Commands: `list_tree`, `get_node`, `create_folder`, `create_connection`, `update_node`, `move_node`, `duplicate_node`, `delete_node`. Errors are a typed enum serialised with a stable `code` and a message the UI can show inline.
- **Single instance:** `tauri-plugin-single-instance`, registered first; a second launch shows, unminimises and focuses the `main` window.
- **CSP:** add `connect-src ipc: http://ipc.localhost` (Tauri IPC); relax `src-tauri/tests/tauri_conf.rs` from "no `http:` substring" to "no `*` and no external origin"; verify no CSP warning in the webview console. Capabilities stay at `core:default` unless a new plugin demands more.
- The window config is not changed for drag and drop (pointer-based).

## 7. UI

- **State:** a Svelte 5 store holds the latest snapshot, the selected id and the set of expanded folder ids. Expanded ids persist per device in `localStorage` (try/catch, renders without it); selection does not persist. No secrets exist at this stage; the rule still applies.
- **Tree:** flattened visible rows, 24 px, no virtualization. ARIA `tree` roles. Keyboard: up/down, left/right collapse/expand, F2 rename, Delete asks for confirmation. Enter does nothing until M4. Inline rename in the row.
- **Filter:** case-insensitive substring over name, host and description; shows matches plus their ancestors, which are force-expanded while a filter is active; drag and drop is disabled while filtering. Pure TS function over the snapshot, Vitest-covered. *(Owner agreed: filtering is presentation.)*
- **Context menu:** New folder, New connection, Duplicate, Rename, Delete (no "Connect" until M4). Right-click on empty space targets the root.
- **Drag and drop:** pointer events; a pure `dropTarget` function (row rect + cursor y + node kind → before/after/inside or none) with Vitest tests; an insertion line / inside-highlight drawn with existing tokens; Escape cancels; a small movement threshold separates click from drag; Rust validates and the snapshot is the only source of the new order.
- **Properties panel:** per `DESIGN.md`: header with name and kind, rows with a 84 px key column, muted "from <folder>" for inherited values, outlined "Inherit" reset for local ones. Fields: name, description, host and protocol (connections), port, username, keepalive (shown for SSH). Edits commit on blur or Enter; a Rust validation error shows inline and keeps the old value. Collapsible.
- **Delete dialog:** modal, names the node and "and N items inside" for folders; the one yellow key is not used here (destructive actions are not "the one action that matters"; the confirm button uses the secondary style with danger text). *(Builder call, `DESIGN.md` leaves this open.)*
- New UI uses only existing tokens; if one is missing, it is added to the light block and both dark blocks (`tokens.test.ts` guards parity).

## 8. Verification (feeds the roadmap acceptance criteria)

- **Rust unit tests:** inheritance (multi-level, override, reset, default fallback, source reporting); tree operations (move validity incl. cycles, duplicate, delete, ordering); persistence (round trip, atomic write leaves no temp file, `.bak` holds the previous version, corrupt file falls back to `.bak` and is renamed, newer `schemaVersion` refused and untouched, migration chain).
- **UI unit tests:** filter, flatten-with-expansion, `dropTarget`.
- **500-connection check:** `cargo run -p fernjo-core --example generate_tree -- <dir> 500` writes a deterministic nested tree; start the app with `FERNJO_DATA_DIR=<dir>`; filter must show no visible lag (manual).
- **Manual:** nested folders and connections created, moved by drag and drop, state identical after restart; second launch focuses the window; no CSP warning.
- **Gate:** the commands in `CLAUDE.md` (build, tests, `pnpm lint`, `cargo deny check`, roadmap check) green.

## 9. Docs to update during implementation

`Docs/DECISIONS.md` (format, location and write strategy, pointer-based drag and drop, CSP `connect-src`, new dependencies and versions), `CHANGELOG.md` `[Unreleased]`, `Docs/roadmap.json` (+ regenerated `ROADMAP.md`). No roadmap scope change: the three inheritable settings in §4 follow M3 (credential reference) and M7 (RDP settings) as already scheduled.

## 10. Left to the builder

Crate choices and versions (`serde`, `uuid`, `thiserror`, `tempfile`, `tauri-plugin-single-instance`; each checked with `cargo deny`), exact JSON field names within §5, in-memory tree representation (arena vs nested), Svelte component split, context-menu and dialog markup, the pointer-drag state machine details, error code names.
