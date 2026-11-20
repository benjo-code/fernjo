# Milestone 2 – Connection tree & data model Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Connections can be organised in a persistent folder tree with mRemoteNG-style inheritance, edited in a properties panel, without connecting anywhere yet.

**Architecture:** `fernjo-core` holds a pure `Tree` (all rules and inheritance) and a `Store` (tree plus `connections.json`, saving after every mutation). `src-tauri` owns one `Store` behind a mutex; every mutating command returns the new full snapshot. The Svelte UI renders snapshots, keeps view state (selection, expansion, filter, drag) and contains no validity or inheritance logic.

**Tech Stack:** Rust 2024 (`serde`, `serde_json`, `uuid`, `thiserror`, `directories`, `tempfile` for tests, `tauri-plugin-single-instance`) · Tauri 2.12 · Svelte 5 runes + TypeScript · `@tauri-apps/api` · Vitest + jsdom.

**Spec:** [2026-11-15-m02-connection-tree-design.md](../specs/2026-11-15-m02-connection-tree-design.md) (decisions for M2), parent spec [2026-11-15-fernjo-design.md](../specs/2026-11-15-fernjo-design.md) §3, §5.1, §5.4. Visual rules: `DESIGN.md`. Roadmap: [M2 in ROADMAP.md](../../ROADMAP.md).

## Global Constraints

- **Secrets stay in Rust; M2 has none.** `connections.json` and every IPC response hold no secret and the format must not invite one (no credential field in M2).
- **All logic is Rust.** Validity (names, ports, moves), inheritance and effective values live in `fernjo-core`; TypeScript only presents. Filtering, flattening, drop-target geometry and the descendant count for the delete dialog are presentation and stay in TS.
- **Inheritable settings in M2:** `username`, `port`, `keepaliveSecs` only. Defaults: port ssh 22 / rdp 3389, keepalive 30 s (`0` = off), username none. Host and protocol are never inherited. Credential reference is M3, RDP settings M8; do not add them.
- **File:** `connections.json` in `<OS config dir>/fernjo/` (`FERNJO_DATA_DIR` overrides), `schemaVersion` 1, nested `children`, human-readable. Write = temp file in the same directory, `sync_all`, copy current file to `connections.json.bak`, rename temp over target. Newer `schemaVersion` is refused and never overwritten; unparsable file falls back to `.bak` after renaming the bad file to `connections.json.corrupt-<unix seconds>`.
- **CSS:** only tokens from `ui/src/styles/tokens.css`; hex-only; no `oklch()`, `color-mix()`, `light-dark()`, nesting, `@layer`, `:has()`, container queries; no `data:` assets; no network at runtime. A new colour token goes into the light block and both dark blocks; non-colour tokens only into `:root`. Token names only grow.
- **DESIGN.md rules that bite here:** tree rows 24 px (`--row-height-tree`), at least 23 rows visible at 1280×800 with the properties panel open; selected row = darker/lighter row + 3 px bar (`--color-selection-bar`) + semibold; keyboard focus = global 1 px inset outline; no shadow except `--shadow-raised` on dialogs and popovers; normal-case 12 px muted labels; yellow (`--color-accent`) never used in M2 (no "one action that matters" exists yet); no lamp without a session.
- **Webview hardening:** CSP stays strict; capabilities stay `core:default` unless a plugin requires more.
- **Git:** work on `feature/m02-connection-tree`; Conventional Commits; stage files by name; `git show --stat HEAD` after every commit and compare with the task's file list. A commit that finishes a roadmap task ticks it in `Docs/roadmap.json` and regenerates `Docs/ROADMAP.md` (`dotnet run tools/roadmap.cs -- generate`). Acceptance criteria are ticked only with evidence (Task 11). No force push, no `--no-verify`.
- **"Done"** = `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm lint`, `pnpm test`, `pnpm ui:build`, `cargo deny check`, `dotnet run tools/roadmap.cs -- check` green, with command and result stated.

## Review Focus

Failure modes the spec implies but no acceptance criterion exercises, most likely first. Each is pinned in the named task.

1. **Disk write fails** (read-only folder, disk full, sync tool holding the file). Expected: the mutation returns an error, memory and disk stay identical, the UI shows the message and the old tree. → Task 4 (`failed_save_rolls_back_memory`), Task 6 (`api_error_sets_error_and_keeps_snapshot`).
2. **UI holds a stale reference** after a mutation: selected node deleted, rapid drag-drop-drag before the first move returned, expanded id of a removed folder. Expected: selection cleared, panel shows its empty state, commands run strictly one after another, stale expansion ids pruned. → Task 6.
3. **Awkward names and text:** whitespace-only, very long, `<b>x</b>`, emoji. Expected: whitespace-only rejected by Rust (`empty_name`), long names ellipsised with the full name in `title`, markup shown literally. → Task 2 (`empty_name`), Task 7.
4. **Filter and empty states:** query with regex characters (`[`, `(`, `.*`), no match, empty tree on first run. Expected: literal substring match, "No matches" text, an empty-tree state offering New folder / New connection. → Task 6, Task 7.
5. **Drag edge cases:** drop on itself or a descendant, release outside the tree, Escape mid-drag, a click with a tiny wobble, drag while a filter is active. Expected: no call to Rust except for a valid move, a click is never a drag, dragging is off while filtering. → Task 9.

---

### Task 1: Core model, settings and effective-value resolution

**Files:**
- Modify: `Cargo.toml` (workspace dependencies), `crates/fernjo-core/Cargo.toml`, `crates/fernjo-core/src/lib.rs`
- Create: `crates/fernjo-core/src/model.rs`, `crates/fernjo-core/src/resolve.rs`
- Modify: `Docs/roadmap.json`, `Docs/ROADMAP.md` (M2 status `in-progress`)

**Interfaces:**
- Produces (all `pub`, re-exported from `lib.rs`):
  - `type NodeId = uuid::Uuid`
  - `enum Protocol { Ssh, Rdp }` (serde `lowercase`)
  - `struct Settings { username: Option<String>, port: Option<u16>, keepalive_secs: Option<u32> }` (serde `camelCase`, `skip_serializing_if` none, `Default`)
  - `struct Node { id: NodeId, name: String, description: String, settings: Settings, kind: NodeKind }` with `#[serde(flatten)] kind`, `description`/`settings` `#[serde(default)]`
  - `enum NodeKind { Folder { children: Vec<Node> }, Connection { protocol: Protocol, host: String } }` (internally tagged `type`, `lowercase`) so JSON is `{"id","type":"folder","name","description","settings":{…},"children":[…]}` / `{…,"type":"connection","protocol":"ssh","host":"…"}`
  - `struct Field<T> { local: Option<T>, effective: Option<T>, source: Source }` and `enum Source { Local, Folder { id: NodeId, name: String }, Default }` (serde tag `kind`, `camelCase`)
  - `struct ResolvedNode { id, kind: NodeType, name, description, protocol: Option<Protocol>, host: Option<String>, username: Field<String>, port: Field<u16>, keepalive_secs: Field<u32> }` (serde `camelCase`; `NodeType` is `Folder | Connection`, serde `lowercase`)
  - `fn resolve(ancestors: &[&Node], node: &Node) -> ResolvedNode` where `ancestors` runs from the top-level folder down to the direct parent
  - Constants `DEFAULT_KEEPALIVE_SECS: u32 = 30`, `DEFAULT_SSH_PORT: u16 = 22`, `DEFAULT_RDP_PORT: u16 = 3389`

- [ ] **Step 1: Set M2 `in-progress`** in `Docs/roadmap.json` (status and the overview), regenerate `Docs/ROADMAP.md`, run `dotnet run tools/roadmap.cs -- check` (valid).
- [ ] **Step 2: Add dependencies** (`serde` with `derive`, `serde_json`, `uuid` with `v4` and `serde`, `thiserror`, `directories`; dev: `tempfile`) via `[workspace.dependencies]` and `crates/fernjo-core/Cargo.toml`. `cargo build -p fernjo-core` passes.
- [ ] **Step 3: Write failing tests** in `resolve.rs`:
  - `ssh_port_defaults_to_22_and_rdp_to_3389` (no settings: effective `Some(22)` / `Some(3389)`, source `Default`)
  - `folder_port_has_no_default` (folder without port: effective `None`, source `Default`)
  - `keepalive_defaults_to_30` and `username_has_no_default` (`None`, `Default`)
  - `local_value_wins_even_if_equal_to_inherited` (folder port 2200, connection port 2200 → source `Local`)
  - `nearest_folder_wins_over_three_levels` (A port 2200, B port 2300, connection none → `Some(2300)`, `Source::Folder { id: B, name: "B" }`)
  - `inherits_username_from_grandparent` (A username "ops", B empty → "ops" from A)
  - `json_shape` (a connection serialises with `"type":"connection"`, `"protocol":"ssh"`, and `settings` omitted keys when `None`; round-trips through `serde_json`)
- [ ] **Step 4: Run** `cargo test -p fernjo-core` → fails (items missing).
- [ ] **Step 5: Implement** the types in `model.rs` and `resolve` in `resolve.rs` (walk `ancestors` from nearest to farthest; the node's own setting is `Source::Local`; defaults per Global Constraints, port default depends on the connection's protocol and is `None` for folders). Keep the old `version()` test.
- [ ] **Step 6: Run** `cargo test -p fernjo-core` → PASS; `cargo clippy -p fernjo-core --all-targets -- -D warnings`.
- [ ] **Step 7: Commit** `feat(core): connection model and effective-value resolution`; stage the files above; tick roadmap M2 task 1 (`fernjo-core: tree … effective-value resolution`) only after Task 2.

### Task 2: Tree operations

**Files:**
- Create: `crates/fernjo-core/src/tree.rs`, `crates/fernjo-core/src/error.rs`
- Modify: `crates/fernjo-core/src/lib.rs`

**Interfaces:**
- Consumes: Task 1 types.
- Produces:
  - `struct Tree` (private `children: Vec<Node>`), `Tree::new() -> Tree`, `Tree::from_children(Vec<Node>) -> Tree`, `Tree::children(&self) -> &[Node]`, `Tree::get(&self, NodeId) -> Option<&Node>`, `Tree::descendant_count(&self, NodeId) -> Result<usize, TreeError>` (excludes the node itself)
  - `Tree::create_folder(&mut self, parent: Option<NodeId>, index: Option<usize>, name: &str) -> Result<NodeId, TreeError>`; `Tree::create_connection(&mut self, parent: Option<NodeId>, index: Option<usize>, name: &str, protocol: Protocol, host: &str) -> Result<NodeId, TreeError>` (`None` index appends; name stored trimmed)
  - `Tree::update(&mut self, id: NodeId, patch: NodePatch) -> Result<(), TreeError>`
  - `Tree::move_node(&mut self, id: NodeId, new_parent: Option<NodeId>, index: usize) -> Result<(), TreeError>` (`index` counts among the new parent's children **after** the node was removed; `index == len` appends)
  - `Tree::duplicate(&mut self, id: NodeId) -> Result<NodeId, TreeError>`, `Tree::delete(&mut self, id: NodeId) -> Result<usize, TreeError>` (returns removed count including the node and all descendants)
  - `Tree::resolve_node(&self, id: NodeId) -> Result<ResolvedNode, TreeError>`
  - `struct NodePatch { name: Option<String>, description: Option<String>, host: Option<String>, protocol: Option<Protocol>, username: Patch<String>, port: Patch<u32>, keepalive_secs: Patch<u32> }` (`Default`, serde `camelCase`, `#[serde(default)]`) and `enum Patch<T> { Keep, Set(T), Inherit }` which deserialises from absent = `Keep`, `null` = `Inherit`, value = `Set` (use `Option<Option<T>>` with a `deserialize_with` helper)
  - `enum TreeError` (`thiserror`) with `fn code(&self) -> &'static str`: `NotFound` → `not_found`, `NotAFolder` → `not_a_folder`, `NotAConnection` → `not_a_connection`, `EmptyName` → `empty_name`, `InvalidPort` → `invalid_port`, `InvalidMove` → `invalid_move`, `IndexOutOfRange` → `index_out_of_range`

- [ ] **Step 1: Write failing tests** in `tree.rs` (helper builds trees through the API):
  - create: `create_folder_appends_by_default`, `create_folder_at_index_0_inserts_first`, `create_in_unknown_parent_is_not_found`, `create_inside_connection_is_not_a_folder`, `whitespace_only_name_is_empty_name` (`"   "` → `EmptyName`, tree unchanged), `name_is_stored_trimmed`
  - update: `rename_updates_name`, `port_0_and_65536_are_invalid_port`, `port_65535_is_accepted`, `host_patch_on_folder_is_not_a_connection`, `patch_inherit_clears_local_value` (after `Inherit`, `resolve_node` shows `local: None`), `patch_keep_leaves_value`
  - move: `reorder_within_parent` (children `[a,b,c]`, move `a` to index 2 → `[b,c,a]`), `move_into_other_folder_keeps_subtree`, `move_folder_into_itself_is_invalid_move`, `move_folder_into_descendant_is_invalid_move`, `index_beyond_len_after_removal_is_index_out_of_range`, `failed_move_leaves_tree_unchanged`
  - duplicate: `duplicate_connection_lands_right_after_original_named_copy` (`"web-01"` → `"web-01 copy"`), `duplicate_folder_deep_copies_with_fresh_ids` (every id in the whole tree unique afterwards, children order and settings equal)
  - delete: `delete_folder_returns_count_including_descendants` (folder with 3 descendants → 4), `delete_unknown_is_not_found`, `descendant_count_excludes_self`
  - `error_codes_are_stable` (every variant → the string above)
- [ ] **Step 2: Run** `cargo test -p fernjo-core tree` → fails.
- [ ] **Step 3: Implement** with a nested `Vec<Node>`; locate nodes through a private `find_path(&self, NodeId) -> Option<Vec<usize>>` (500 nodes, DFS is enough). `move_node` validates before mutating so a failure changes nothing. Names are validated with `trim().is_empty()`. `resolve_node` builds the ancestor slice for `resolve`.
- [ ] **Step 4: Run** `cargo test -p fernjo-core` → PASS; clippy clean.
- [ ] **Step 5: Commit** `feat(core): tree operations with validation`; tick roadmap M2 task 1, regenerate.

### Task 3: Persistence (file format, atomic write, load fallback, migration)

**Files:**
- Create: `crates/fernjo-core/src/persist.rs`
- Modify: `crates/fernjo-core/src/error.rs`, `crates/fernjo-core/src/lib.rs`

**Interfaces:**
- Consumes: `Tree`, `Node`.
- Produces:
  - `const CURRENT_VERSION: u32 = 1`, `const FILE_NAME: &str = "connections.json"`
  - `fn save(path: &Path, tree: &Tree) -> Result<(), SaveError>` (creates the parent directory; sequence per Global Constraints; first save creates no `.bak`)
  - `fn load(path: &Path) -> Result<Loaded, LoadError>` where `struct Loaded { tree: Tree, notice: Option<Notice> }`, `enum Notice { RestoredFromBackup { bad_file: Option<PathBuf> } }` (serde `camelCase`, tagged `kind`)
  - `enum LoadError { NewerSchema { found: u32, supported: u32, path: PathBuf }, Unreadable { path: PathBuf, reason: String } }`, `struct SaveError(..)` (`thiserror`, carries path and cause)
  - `fn migrate_with(value: serde_json::Value, from: u32, steps: &[fn(serde_json::Value) -> serde_json::Value]) -> serde_json::Value` (step `i` upgrades version `i+1` → `i+2`); `migrate` uses an empty static step list for now

- [ ] **Step 1: Write failing tests** (each in a `tempfile::tempdir()`):
  - `save_then_load_round_trips_nested_tree_with_settings`
  - `file_is_pretty_json_with_schema_version` (parse as `Value`: `["schemaVersion"] == 1`, `["children"][0]["type"] == "folder"`, text contains `"\n  "`)
  - `save_leaves_no_temp_file` (directory holds exactly `connections.json` after the first save)
  - `first_save_creates_no_bak` and `second_save_keeps_previous_content_as_bak` (`.bak` bytes equal the first file's bytes)
  - `load_missing_file_gives_empty_tree_and_writes_nothing`
  - `newer_schema_is_refused_and_files_untouched` (`schemaVersion` 99 → `LoadError::NewerSchema { found: 99, supported: 1, .. }`; directory listing and bytes unchanged)
  - `corrupt_main_falls_back_to_bak` (garbage main + valid `.bak` → tree equals `.bak`, `Notice::RestoredFromBackup { bad_file: Some(..) }`, a `connections.json.corrupt-<digits>` file holds the garbage, `.bak` bytes unchanged)
  - `missing_main_with_bak_restores_with_notice_without_bad_file`
  - `both_unreadable_is_unreadable_error_and_writes_nothing`
  - `migrate_with_applies_steps_in_order` (two test steps that append `"a"` then `"b"` to a field)
  - `save_into_path_below_a_regular_file_fails` (returns `SaveError`)
- [ ] **Step 2: Run** `cargo test -p fernjo-core persist` → fails.
- [ ] **Step 3: Implement.** `load` reads the file, parses to `serde_json::Value`, checks `schemaVersion` first (missing or non-integer counts as unreadable), runs `migrate`, then deserialises `children`. After a restore from `.bak`, the caller (Task 4) re-saves; `load` itself writes nothing except renaming the bad file. Use `File::sync_all` before the rename; copy (not rename) the old file to `.bak` so the main file always exists.
- [ ] **Step 4: Run** `cargo test -p fernjo-core` → PASS; clippy clean.
- [ ] **Step 5: Commit** `feat(core): connections.json persistence with atomic write and backup fallback`.

### Task 4: Store, data directory and sample-tree generator

**Files:**
- Create: `crates/fernjo-core/src/store.rs`, `crates/fernjo-core/src/paths.rs`, `crates/fernjo-core/examples/generate_tree.rs`
- Modify: `crates/fernjo-core/src/lib.rs`, `crates/fernjo-core/src/error.rs`
- Modify: `Docs/roadmap.json`, `Docs/ROADMAP.md`

**Interfaces:**
- Consumes: `Tree`, `persist::{save, load}`.
- Produces:
  - `fn data_dir_from(env: Option<OsString>, config_dir: Option<PathBuf>) -> Option<PathBuf>` (env wins if non-empty, else `config_dir/"fernjo"`), `fn data_dir() -> Option<PathBuf>` (reads `FERNJO_DATA_DIR` and `directories::BaseDirs::config_dir()`)
  - `struct Store`, `Store::open(dir: &Path) -> Result<Opened, LoadError>` with `struct Opened { store: Store, notice: Option<Notice> }` (after a restore from `.bak`, re-saves so `connections.json` exists again), `Store::tree(&self) -> &Tree`, `Store::path(&self) -> &Path`
  - Mutations mirroring `Tree`, each `-> Result<_, StoreError>`: `create_folder`, `create_connection`, `update`, `move_node`, `duplicate`, `delete`, with the same arguments. `enum StoreError { Tree(TreeError), Save(SaveError) }` with `fn code(&self) -> &'static str` (`Tree` → its code, `Save` → `save_failed`). Each mutation applies to a clone, saves, then swaps the clone in, so memory never diverges from disk.

- [ ] **Step 1: Write failing tests:**
  - `data_dir_from_prefers_non_empty_env`, `data_dir_from_appends_fernjo_to_config_dir`, `data_dir_from_empty_env_falls_back`, `data_dir_from_nothing_is_none`
  - `mutation_is_persisted_and_survives_reopen` (create folder + connection, `Store::open` again shows both in order)
  - `failed_save_rolls_back_memory` (open a store whose directory path is a regular file: the mutation returns `StoreError::Save`, `store.tree().children()` unchanged)
  - `tree_error_does_not_touch_disk` (`create_folder` with empty name: error code `empty_name`, file not created)
  - `open_after_restore_recreates_main_file` (garbage main + valid `.bak` → after `open`, `connections.json` parses and equals the `.bak` tree)
- [ ] **Step 2: Run** → fails. **Step 3: Implement.**
- [ ] **Step 4: Write the example** `generate_tree.rs`: `cargo run -p fernjo-core --example generate_tree -- <dir> [count=500]` builds a deterministic nested tree (no randomness: 10 top-level folders × 5 sub-folders, connections spread evenly, names like `host-001`, hosts `host-001.lab.example`, descriptions from a fixed word list, mixed ssh/rdp, some folders with `username`/`port` settings), writes it with `persist::save`, prints `wrote <n> connections in <m> folders to <path>`.
- [ ] **Step 5: Run** `cargo test -p fernjo-core` (PASS); `cargo run -p fernjo-core --example generate_tree -- <scratchpad dir>/gen500 500` prints `wrote 500 connections …`; the file parses (`Store::open` in a scratch test or `jq '.children|length'`).
- [ ] **Step 6: Commit** `feat(core): store with rollback, data directory and sample-tree generator`; tick roadmap M2 task 2 (persistence), regenerate.

### Task 5: Tauri commands, CSP and single instance

**Files:**
- Modify: `src-tauri/Cargo.toml`, `src-tauri/src/lib.rs`, `src-tauri/tauri.conf.json`, `src-tauri/tests/tauri_conf.rs`
- Create: `src-tauri/src/commands.rs`, `src-tauri/src/state.rs`

**Interfaces:**
- Consumes: `fernjo_core::{Store, Opened, Notice, Node, ResolvedNode, NodePatch, Protocol, NodeId, StoreError, LoadError, data_dir}`.
- Produces (commands in `commands.rs`; each has a plain testable function taking `&AppState` and a thin `#[tauri::command]` wrapper; JS argument names are camelCase):
  - `AppState { store: Mutex<Result<Store, StartupFailure>>, notice: Mutex<Option<Notice>> }` (`state.rs`; `AppState::open(dir: Option<PathBuf>) -> AppState`; `StartupFailure { code: String, message: String }` for newer schema, unreadable files or no data dir)
  - `struct Snapshot { children: Vec<Node>, notice: Option<Notice> }`, `struct Mutation { id: Option<NodeId>, snapshot: Snapshot }` (serde `camelCase`)
  - `struct CommandError { code: String, message: String }` (serde; codes from `TreeError::code`, plus `save_failed`, `load_failed`, `internal` for a poisoned lock)
  - `list_tree() -> Snapshot` (returns the startup notice once, then `None`), `get_node(id) -> ResolvedNode`, `create_folder(parent_id: Option<NodeId>, index: Option<usize>, name: String) -> Mutation`, `create_connection(parent_id, index, name, protocol: Protocol, host: String) -> Mutation`, `update_node(id, patch: NodePatch) -> Mutation`, `move_node(id, parent_id: Option<NodeId>, index: usize) -> Mutation`, `duplicate_node(id) -> Mutation` (`id` = the copy), `delete_node(id) -> Mutation` (`id` = `None`)

- [ ] **Step 1: Write failing tests** in `commands.rs` (`#[cfg(test)]`, `tempfile`):
  - `list_tree_on_fresh_dir_is_empty`
  - `create_folder_returns_id_and_snapshot_containing_it`
  - `empty_name_maps_to_code_empty_name`
  - `failed_startup_makes_every_command_return_load_failed_naming_the_file` (e.g. `schemaVersion` 99 file → `code == "load_failed"`, message contains the file name)
  - `startup_notice_is_returned_once` (restored-from-backup notice on the first `list_tree`, `None` on the second)
  - `snapshot_json_uses_camel_case_and_node_tags` (`serde_json::to_value`: `children[0]["type"] == "folder"`; `get_node` JSON has `keepaliveSecs` and `username.source.kind`)
  - Update `src-tauri/tests/tauri_conf.rs`: `csp_is_strict` now requires `connect-src ipc: http://ipc.localhost`, rejects the token `*` and `unsafe-eval`, and requires every `http:`/`https:` token to equal exactly `http://ipc.localhost`; `default_capability_is_core_only` unchanged.
- [ ] **Step 2: Run** `cargo test -p fernjo-app` → fails.
- [ ] **Step 3: Implement** commands, state and `run()`: register `tauri_plugin_single_instance::init` **first** (callback: `get_webview_window("main")` → `unminimize`, `show`, `set_focus`), `manage(AppState::open(data_dir()))`, `invoke_handler` with the eight commands. Add `connect-src ipc: http://ipc.localhost` to the CSP in `tauri.conf.json`. Add dependencies (`fernjo-core` path, `serde`, `serde_json`, `tauri-plugin-single-instance`; dev `tempfile`).
- [ ] **Step 4: Run** `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo deny check` → all green; record any new advisory ignore with a reason in `deny.toml` only if `cargo deny` reports one.
- [ ] **Step 5: Verify single instance:** `pnpm tauri dev`, then run `target\debug\fernjo.exe` a second time; the second process exits at once and the first window comes to the front (minimise it first to see the effect). Stop leftovers per `CLAUDE.md` (port 1420, `fernjo.exe`).
- [ ] **Step 6: Commit** `feat(app): tree commands, IPC CSP and single instance`; tick roadmap M2 tasks 3 and 6, regenerate.

### Task 6: UI foundation – types, API, store, flatten and filter

**Files:**
- Modify: `package.json`, `pnpm-lock.yaml` (`@tauri-apps/api` ^2), `ui/src/App.svelte`, `ui/src/App.test.ts`, `ui/src/styles/tokens.css` (+ `--tree-indent: 14px` in `:root` only), `ui/src/styles/tokens.test.ts` if it enumerates names
- Create: `ui/src/lib/types.ts`, `ui/src/lib/api.ts`, `ui/src/lib/rows.ts`, `ui/src/lib/rows.test.ts`, `ui/src/lib/store.svelte.ts`, `ui/src/lib/store.test.ts`, `ui/src/styles/components-css.test.ts`

**Interfaces:**
- Produces:
  - `types.ts`: `Protocol = "ssh" | "rdp"`; `Settings`; `TreeNode` (discriminated on `type: "folder" | "connection"`, folder has `children: TreeNode[]`, connection has `protocol`, `host`); `FieldInfo<T> { local: T | null; effective: T | null; source: {kind:"local"} | {kind:"folder"; id: string; name: string} | {kind:"default"} }`; `ResolvedNode`; `NodePatch` (absent = keep, `null` = inherit); `Snapshot`, `Mutation`, `ApiError { code: string; message: string }`; `Notice`
  - `api.ts`: `interface Api { listTree(): Promise<Snapshot>; getNode(id): Promise<ResolvedNode>; createFolder(parentId: string|null, index: number|null, name: string): Promise<Mutation>; createConnection(parentId, index, name, protocol, host): Promise<Mutation>; updateNode(id, patch): Promise<Mutation>; moveNode(id, parentId: string|null, index: number): Promise<Mutation>; duplicateNode(id): Promise<Mutation>; deleteNode(id): Promise<Mutation> }` and `tauriApi: Api` (wraps `invoke` from `@tauri-apps/api/core`; rejections normalised to `ApiError`)
  - `rows.ts`: `interface Row { node: TreeNode; depth: number; parentId: string | null; index: number; expanded: boolean; hasChildren: boolean }`; `flattenRows(children: TreeNode[], expanded: ReadonlySet<string>, filter: string): Row[]`; `matchesFilter(node: TreeNode, query: string): boolean`; `countDescendants(node: TreeNode): number`
  - `store.svelte.ts`: `class TreeStore` (constructor `(api: Api)`; `$state` fields `children`, `notice`, `selectedId`, `expanded`, `filter`, `renamingId`, `error: string | null`, `loadError: string | null`; `$derived rows`; methods `init()`, `select(id|null)`, `toggle(id)`, `expand(id)`, `setFilter(q)`, `startRename(id)`, `stopRename()`, `dismissNotice()`, `dismissError()`, and mutation wrappers `createFolder(parentId, index)`, `createConnection(parentId, index)`, `rename(id, name)`, `update(id, patch)`, `move(id, parentId, index)`, `duplicate(id)`, `remove(id)`) — all mutations run through one promise chain so they execute strictly in order; expansion persists under the `localStorage` key `fernjo.expanded.v1` (every access in try/catch)

- [ ] **Step 1: Write failing tests.** `rows.test.ts`: `flatten_hides_children_of_collapsed_folders`, `rows_carry_depth_parent_and_index`, `filter_matches_name_host_and_description_case_insensitively`, `filter_shows_ancestors_of_matches_force_expanded_and_hides_non_matches`, `filter_treats_regex_characters_literally` (`[`, `(`, `.*` against names containing them and not matching others), `filter_with_no_match_gives_no_rows`, `whitespace_only_filter_means_no_filter`, `query_is_trimmed`, `count_descendants_counts_nested_nodes_excluding_self`, `filtering_500_connections_stays_under_100_ms` (loose sanity bound; the real check is manual in Task 11). `store.test.ts` (mock `Api`): `init_loads_snapshot_and_notice`, `selection_is_cleared_when_snapshot_no_longer_contains_it`, `stale_expanded_ids_are_pruned`, `mutations_run_strictly_in_order` (second `move` does not call the API until the first promise resolves), `api_error_sets_error_and_keeps_snapshot`, `load_failed_error_sets_load_error`, `expansion_persists_to_local_storage_and_restores`, `broken_local_storage_does_not_crash`. `components-css.test.ts`: reads every `ui/src/**/*.svelte` via `import.meta.glob("…", { query: "?raw", eager: true })`, extracts `<style>` blocks and asserts no `oklch(`, `color-mix(`, `light-dark(`, `@layer`, `:has(`, `@container`, no CSS nesting (`&`), no `#rrggbb` literals, no `data:`.
- [ ] **Step 2: Run** `pnpm test` → fails.
- [ ] **Step 3: Implement** types, api, rows, store. Selected id survives a snapshot only if the node still exists; `create*` selects the returned id, expands its parent and starts rename.
- [ ] **Step 4: Rework `App.svelte`** into the shell: a CSS grid with the sidebar (`--sidebar-width`, `--color-sidebar`, filter slot, notice slot, tree slot, properties slot, to be filled by Tasks 7–10) and a main area with the quiet empty state "Open a connection from the tree" (tabs come in M4). `App` takes an optional `api` prop (default `tauriApi`) and creates the store. Update `App.test.ts` (replace the `h1` expectation with: mounts with a mock `Api`, calls `listTree` once, renders the sidebar landmark and the main-area text). Add `--tree-indent` to `tokens.css` and keep `tokens.test.ts` green.
- [ ] **Step 5: Run** `pnpm test`, `pnpm lint`, `pnpm ui:build` → green.
- [ ] **Step 6: Commit** `feat(ui): tree store, API wrapper, row flattening and filter`.

### Task 7: Tree view, selection, expansion, keyboard, inline rename

**Files:**
- Create: `ui/src/lib/components/Sidebar.svelte`, `FilterBox.svelte`, `TreeView.svelte`, `TreeRow.svelte`, `Icon.svelte`, `NoticeBar.svelte`, and `ui/src/lib/components/TreeView.test.ts`
- Modify: `ui/src/App.svelte`

**Interfaces:**
- Consumes: `TreeStore`, `Row`.
- Produces: `TreeView` (props `{ store: TreeStore }`) rendering `role="tree"`; rows `role="treeitem"` with `aria-level`, `aria-selected`, `aria-expanded` (folders) and `data-node-id` (Task 9 and Task 8 rely on this attribute); `Icon` (`name: "folder-chevron" | "ssh" | "rdp" | "search" | "alert"`, 16 px, 1.5 px stroke, `currentColor`, inline SVG); `Sidebar` composes `FilterBox`, `NoticeBar`, `TreeView` and a slot for the properties panel.

- [ ] **Step 1: Write failing component tests** in `TreeView.test.ts` (mount with a mock-`Api` store holding a folder `Homelab` with `pve-01` (ssh) and `DC01` (rdp), plus a top-level `web-01`):
  - `renders_treeitems_with_level_and_expanded_state` (`aria-level` 1/2, `aria-expanded="true"` only on folders)
  - `click_selects_and_chevron_click_toggles`
  - `arrow_keys_move_selection_and_expand_collapse` (Down/Up; Right expands a collapsed folder, Left collapses an open one, Left on a connection selects its parent)
  - `f2_starts_rename_enter_commits_escape_cancels` (Enter → `api.updateNode(id, { name })`; Escape → no API call)
  - `rename_to_whitespace_reverts_and_shows_error` (API rejects `empty_name` → `store.error` set, row shows the old name)
  - `markup_in_names_is_shown_literally_and_long_names_keep_full_title` (name `<b>x</b>` → row `textContent` equals it, no `b` element; `title` attribute equals the full name)
  - `empty_tree_offers_new_folder_and_new_connection` and `filter_without_matches_says_no_matches` (text contains the query)
  - `filter_box_filters_rows_and_clears_with_escape`
- [ ] **Step 2: Run** `pnpm test` → fails.
- [ ] **Step 3: Implement** the components per `DESIGN.md` (Tree row, Filter box, selection bar, hover, focus ring, `--tree-indent` × depth, ellipsis for long names, protocol icon only on connections, no lamp, no host column). One roving `tabindex` for the tree. The filter box is 28 px, placeholder "Filter connections", `Ctrl/Cmd+F` is **not** bound (M4 owns app shortcuts). Rename input is inline in the row, selects the text on start, Enter/blur commits.
- [ ] **Step 4: Run** `pnpm test`, `pnpm lint`, `pnpm ui:build` → green. Visual check: `pnpm tauri dev` with `FERNJO_DATA_DIR=<scratchpad>\gen500` (generated in Task 4) renders the tree in light and dark at 1280×800; count visible rows is at least 23 once the properties panel exists (re-checked in Task 10).
- [ ] **Step 5: Commit** `feat(ui): tree view with selection, keyboard navigation and inline rename`.

### Task 8: Context menu, create/duplicate flows and delete confirmation

**Files:**
- Create: `ui/src/lib/components/ContextMenu.svelte`, `ConfirmDialog.svelte`, `ui/src/lib/components/ContextMenu.test.ts`
- Modify: `TreeView.svelte`, `TreeRow.svelte`, `Sidebar.svelte`, `DESIGN.md` (Components: "Confirm dialog": modal on `--color-surface-raised` with `--shadow-raised` and `--color-overlay`, secondary-style confirm button with `--color-danger` text, initial focus on Cancel, no yellow key)

**Interfaces:**
- Consumes: `TreeStore` mutation wrappers, `Row`, `countDescendants`.
- Produces: `ContextMenu` (`role="menu"`, items `role="menuitem"`, props `{ x: number; y: number; items: {label: string; action: () => void}[]; onClose: () => void }`); `ConfirmDialog` (`role="alertdialog"`, props `{ title: string; confirmLabel: string; onConfirm; onCancel }`).

- [ ] **Step 1: Write failing tests** in `ContextMenu.test.ts`:
  - `row_context_menu_lists_items_in_order_and_selects_the_row` (`New folder`, `New connection`, `Duplicate`, `Rename`, `Delete`; no "Connect")
  - `empty_space_menu_only_offers_new_folder_and_new_connection_at_root`
  - `new_folder_on_folder_appends_inside_and_starts_rename` (API: `createFolder(folderId, null, "New folder")`; new row in rename mode)
  - `new_connection_on_connection_lands_after_it_in_the_same_parent` (`createConnection(parentId, index + 1, …)` with name `"New connection"`)
  - `duplicate_selects_the_copy`
  - `delete_asks_with_descendant_count` (folder with 4 items inside → text `Delete “Homelab” and 4 items inside?`; connection → `Delete “web-01”?`)
  - `confirm_calls_delete_and_cancel_or_escape_does_not`, `enter_on_dialog_does_not_confirm_by_default_focus_is_cancel`, `delete_key_on_selected_row_opens_the_dialog`
  - `menu_closes_on_escape_outside_click_and_after_an_action`, `menu_stays_inside_the_viewport`
- [ ] **Step 2: Run** `pnpm test` → fails.
- [ ] **Step 3: Implement.** Menu opens on `contextmenu` and on Shift+F10 / the context-menu key for the focused row; arrows move through items, Enter activates. New-node default names `New folder` / `New connection` (protocol `ssh`, empty host). Dialog traps focus. Text uses `“` `”` as in the tests.
- [ ] **Step 4: Run** `pnpm test`, `pnpm lint` → green.
- [ ] **Step 5: Commit** `feat(ui): context menu, create and duplicate flows, delete confirmation`.

### Task 9: Drag and drop (pointer-based)

**Files:**
- Create: `ui/src/lib/drop-target.ts`, `ui/src/lib/drop-target.test.ts`, `ui/src/lib/drag.ts`, `ui/src/lib/drag.test.ts`
- Modify: `TreeView.svelte`, `TreeRow.svelte`

**Interfaces:**
- Consumes: `Row`, `TreeStore.move`.
- Produces:
  - `type DropPosition = "before" | "after" | "inside"`; `dropPosition(y: number, rect: { top: number; height: number }, isFolder: boolean): DropPosition` (folder: top 25 % `before`, bottom 25 % `after`, middle `inside`; connection: top half `before`, bottom half `after`)
  - `resolveDrop(rows: Row[], dragId: string, targetId: string, position: DropPosition): { parentId: string | null; index: number } | null` — index uses the same convention as `move_node` (counted after the dragged node is removed); returns `null` for a no-op, for dropping on itself or onto a descendant; `after` on an expanded folder that has children means `inside` at index 0
  - `class DragController` (`drag.ts`, no DOM access): `constructor(opts: { threshold: number; onMove: (dragId, parentId, index) => void })`; `pointerDown(id: string, x: number, y: number): void`; `pointerMove(x: number, y: number, hit: { rowId: string; position: DropPosition } | null): void`; `pointerUp(): void`; `cancel(): void`; read-only `state: "idle" | "pending" | "dragging"`, `dragId`, `target` (the resolved drop for the indicator)

- [ ] **Step 1: Write failing tests.** `drop-target.test.ts`: `connection_row_halves`, `folder_row_quarters_and_middle`; `resolveDrop` over rows `A(folder, expanded: a1, a2), B(connection), C(folder, empty)`: `before_B_from_A_is_root_index_1`, `after_C_is_root_end`, `inside_C_is_C_index_0`, `inside_A_appends_after_a2`, `after_expanded_A_means_inside_A_index_0`, `reorder_a1_after_a2_in_A_is_A_index_1`, `dropping_on_itself_is_null`, `dropping_A_inside_a1_is_null`, `dropping_in_place_is_null` (moving `a1` before `a1`'s next sibling position that equals its current spot). `drag.test.ts`: `small_wobble_is_a_click_not_a_drag` (move 3 px with threshold 4 → `pointerUp` calls nothing, state `idle`), `crossing_threshold_starts_dragging`, `release_over_valid_target_calls_onMove_once`, `release_outside_the_tree_cancels` (`hit: null`), `escape_cancel_returns_to_idle_without_call`, `release_over_invalid_target_calls_nothing`.
- [ ] **Step 2: Run** `pnpm test` → fails.
- [ ] **Step 3: Implement** the two pure modules, then the Svelte glue: pointer events on the tree element (`setPointerCapture`), hit-test with `document.elementFromPoint` + `closest("[data-node-id]")` + `dropPosition`, indicator drawn as a 2 px line in `--color-selection-bar` (before/after) or a `--color-hover` row highlight (inside) with no shadow, auto-scroll within 24 px of the tree edge, `Escape` cancels, text selection disabled while dragging (`user-select: none` on the tree), drag disabled while a filter is active or a rename is open, no drag from the chevron. A moved folder's new parent is expanded after the drop. The only API call is `store.move`; Rust rejection shows the standard error.
- [ ] **Step 4: Run** `pnpm test`, `pnpm lint`, `pnpm ui:build` → green. Manual in `pnpm tauri dev`: move a connection into another folder, reorder two siblings, drop a folder onto its own child (nothing happens), press Escape mid-drag.
- [ ] **Step 5: Commit** `feat(ui): pointer-based drag and drop`.

### Task 10: Properties panel

**Files:**
- Create: `ui/src/lib/components/PropertiesPanel.svelte`, `PropertyRow.svelte`, `ui/src/lib/components/PropertiesPanel.test.ts`
- Modify: `Sidebar.svelte`, `ui/src/lib/store.svelte.ts` (`detail: ResolvedNode | null`, refreshed through `api.getNode` whenever the selection or the snapshot changes)

**Interfaces:**
- Consumes: `TreeStore.update`, `ResolvedNode`, `FieldInfo`.
- Produces: `PropertiesPanel` (props `{ store: TreeStore }`): collapsible, header (name in 600, kind label `SSH connection` / `RDP connection` / `Folder` in muted 12 px), 84 px key column.

- [ ] **Step 1: Write failing tests:**
  - `shows_inherited_value_with_from_folder_mark_and_no_reset` (`web-01` under `Homelab` with username `ops` → value `ops`, text `from Homelab`, no `Inherit` button)
  - `editing_commits_on_blur_and_enter_and_reverts_on_escape` (`root` → `api.updateNode(id, { username: "root" })`)
  - `local_value_shows_inherit_button_which_sends_null` (`{ username: null }`)
  - `clearing_a_local_field_resets_to_inherit`
  - `port_out_of_range_shows_rust_error_and_reverts` (API rejects `invalid_port` → inline message, field back to previous value)
  - `non_numeric_port_is_rejected_without_calling_the_api` (message `Port must be a whole number`)
  - `folder_has_no_host_or_protocol_rows_and_rdp_connection_has_no_keepalive_row`
  - `default_sourced_values_show_a_muted_default_mark` and `folder_port_placeholder_says_default_per_protocol`
  - `name_and_description_edit_and_empty_name_is_rejected_inline`
  - `panel_shows_empty_state_when_nothing_is_selected_or_the_selection_vanished`
  - `header_collapse_hides_the_body`
- [ ] **Step 2: Run** `pnpm test` → fails.
- [ ] **Step 3: Implement** per `DESIGN.md` "Properties panel" (rows min 24 px, 84 px key column, outlined `Inherit` reset with reset icon, 12 px muted marks). Fields: name, description, protocol (select) and host (connections only), port, username, keepalive (SSH connections and folders). Validation message text comes from the API error; only the integer check lives in TS.
- [ ] **Step 4: Run** `pnpm test`, `pnpm lint`, `pnpm ui:build` → green. With `FERNJO_DATA_DIR=<scratchpad>\gen500`, at 1280×800 with the panel open at least 23 tree rows are visible (measure `document.querySelectorAll('[role=treeitem]')` rows within the tree's client height via the devtools console, or count in a screenshot).
- [ ] **Step 5: Commit** `feat(ui): properties panel with inheritance marks and reset`; tick roadmap M2 tasks 4 and 5, regenerate.

### Task 11: Verification, documentation and pull request

**Files:**
- Modify: `Docs/DECISIONS.md`, `CHANGELOG.md`, `CLAUDE.md` (Project-specific), `Docs/roadmap.json`, `Docs/ROADMAP.md`, `Docs/superpowers/specs/2026-11-15-m02-connection-tree-design.md` (status → implemented)

- [ ] **Step 1: Full gate** on Windows: `pnpm install --frozen-lockfile`, `pnpm ui:build`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm lint`, `pnpm test`, `cargo deny check`, `dotnet run tools/roadmap.cs -- check`. Record each command and its result (counts of tests) as evidence.
- [ ] **Step 2: Programmatic app checks** with `pnpm tauri dev` and `FERNJO_DATA_DIR=<fresh scratch dir>`: window opens, DevTools console has no CSP or IPC error; after creating nodes through the UI the file `connections.json` exists with `schemaVersion` 1 and no `.tmp` file; restarting shows the same tree (as far as it can be checked without the owner's eyes). With `FERNJO_DATA_DIR=<scratchpad>\gen500` the filter stays responsive. Mark anything a person must judge (visible lag, drag feel, restart behaviour on the real data folder, macOS/Linux) **OPEN** with its command (`pnpm tauri dev`) for the owner.
- [ ] **Step 3: Docs.** `Docs/DECISIONS.md` entries (newest on top): (a) `connections.json` format, location, atomic-write and corrupt/newer-version policy; (b) pointer-based drag and drop, rejected HTML5 and library; (c) IPC CSP `connect-src` and the relaxed test (closing the follow-up in "Webview hardening from day one"); (d) new dependencies with exact versions. `CHANGELOG.md` `[Unreleased]`: connection tree, folders, inheritance, properties panel, single instance. `CLAUDE.md` Project-specific: `FERNJO_DATA_DIR`, the data folder location, the sample-tree generator command, and that the single-instance plugin must stay first in the builder chain.
- [ ] **Step 4: Roadmap.** Tick every M2 task; tick each acceptance criterion only with its evidence text (unit tests: names and counts; manual items: what was checked, plus `OPEN:` remainder for the owner); set status `done` with `completedOn` only if every criterion that can be verified here is verified, otherwise leave `in-progress` and tell the owner what is outstanding. Regenerate `Docs/ROADMAP.md`; `dotnet run tools/roadmap.cs -- check` valid.
- [ ] **Step 5: Commit** `docs: milestone 2 decisions, changelog and roadmap`; `git show --stat HEAD` matches the list above.
- [ ] **Step 6: PR.** Push `feature/m02-connection-tree`, open a PR to `dev` (title `feat: milestone 2 – connection tree & data model`, body: summary, evidence list, OPEN items), wait for CI (`roadmap`, `rust`, `ui`, `tauri-build`, `cargo-deny` green), then squash-merge per `CLAUDE.md`.
