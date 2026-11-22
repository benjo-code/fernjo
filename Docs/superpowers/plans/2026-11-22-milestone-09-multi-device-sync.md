# Milestone 9: Multi-device sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Connections and vault entries made on one device appear on the owner's other devices through a small self-hosted hub, saving never waits for the hub, and concurrent edits are merged by fixed rules without silent loss.

**Architecture:** A new `fernjo-sync` library holds the snapshot envelope, the pure merge, the hub client, the device-local sync state and a sync engine that talks to the app through two small ports. A new `fernjo-hub` binary stores one encrypted head (plus the last 10) behind compare-and-swap. `src-tauri` implements the ports over `Store` and `Vault`, runs the triggers and exposes the commands; the UI gets a Settings section and a status line. `fernjo-vault` gets the minimal API to adopt a file and to merge entries.

**Tech Stack:** Rust 2024 (MSRV 1.90), RustCrypto (`chacha20poly1305`, `sha2`, `hkdf`), rustls with ring through an HTTP client and server crate chosen against `cargo deny`, tokio, Tauri 2, Svelte 5 + TypeScript, rootless Podman with Caddy for the hub.

**Spec:** `Docs/superpowers/specs/2026-11-22-m09-multi-device-sync-design.md` (decisions, rules C1 to C10 and V1 to V5, threat model). Milestone scope: `Docs/roadmap.json`, milestone "9".

## Global Constraints

- Secrets stay in Rust: the hub secret and the sync key live in the OS keychain only; no response, error, log or `Debug` output contains them. The one exception is `sync_pairing_code` (an explicit reveal, like vault reveal). `SyncKey` and every type holding the hub secret have a redacted `Debug` and zeroize on drop.
- No home-made crypto: XChaCha20-Poly1305 for the envelope, HKDF-SHA256 for its key, rustls for TLS. The vault's own encryption is not changed; the vault part of a snapshot is the `vault.fernjo` bytes exactly as on disk.
- `fernjo-sync` and `fernjo-hub` never depend on Tauri; both are created in this milestone only and testable without the UI. `cargo build/clippy/test --workspace` needs no UI build.
- Adding a `#[tauri::command]` updates three places: the function plus its `handlers` wrapper, its entry in `table()` of `no_response_contains_a_secret` in `src-tauri/src/vault_commands.rs` (or `APP_HANDLE_EXEMPT` with a reason), and the `generate_handler!` list in `src-tauri/src/lib.rs`; `command_table_covers_every_tauri_command` stays green; a new source file with commands goes into that test's `sources`. A command that runs Argon2 or does network I/O is `#[tauri::command(async)]` or an `async fn` that moves the work into `spawn_blocking`.
- Existing guards keep their tests unchanged and green: the `Store` fingerprint check (`changed_on_disk`), the `Vault::save` header guard, vault generation and `rolled_back`, `known_hosts.confirmed`, `rdp_certs.confirmed`. Sync writes only through `Store` and `Vault`. Lock order is pending, then vault, then store; network I/O never happens while a lock is held.
- Limits on everything adopted from the hub: envelope at most 64 MiB (`MAX_ENVELOPE_LEN`), 100 000 nodes, depth 60 (`MAX_TREE_DEPTH`), the vault's KDF bounds and `MAX_FILE_LEN`. A new reader of a data-folder or device-folder file uses a capped read; atomic writes use the random `create_new` temp name, keep the `.bak`, rename, fsync the directory on Unix.
- Wire facts: envelope magic `FERNJOSY`, version 1, header = magic 8, version 2, group id 16, `seq` 8 (u64 big endian), nonce 24 (58 bytes), AAD = header bytes 0..34; hub paths `GET|PUT /v1/head`, `GET /v1/info`; `Authorization: Bearer <hub secret>`; `If-Match: "<seq>"` and `If-None-Match: *`; 412 on a stale write; hub keeps the last 10 envelopes; size limit 64 MiB; client HTTPS only (plain `http://` only for loopback).
- Device-local files live in the device folder (`AppState.device_dir`, `<FERNJO_DATA_DIR>/.device` for scratch runs): `sync.json` (hub URL, CA PEM, device id and name, group id, last `seq`), `sync-base.json` (the base snapshot). Neither is ever in the data folder.
- UI styling uses only tokens from `ui/src/styles/tokens.css` and follows `DESIGN.md`; shipped CSS stays hex-only without `oklch()`, `color-mix()`, `light-dark()`, nesting, `@layer`, `:has()` or container queries. UI text is English.
- CI uses GitHub-hosted runners only, actions pinned to SHAs. A dependency licence outside the `deny.toml` allow-list goes to the owner, never into the list unasked.
- "Done" per task: the commands named in the task are green. At the end: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm lint`, `pnpm test`, `cargo deny check`, `dotnet run tools/roadmap.cs -- check`, `bash tools/test-release-scripts.sh`.
- Commits follow Conventional Commits, no attribution lines, selective `git add <file>`; check `git show --stat HEAD` after each. Work stays on `feature/m09-multi-device-sync`.

## Review Focus

Failure modes the spec implies that no task states; each has a test in the task named in brackets.

1. A truncated or garbled pairing code is pasted: refused with `invalid_pairing_code` before anything is stored, and a code made by a newer version is refused as `newer_pairing_code`. [Task 3]
2. The hub answers with a body larger than 64 MiB, a wrong content length, a redirect to another host, or a 200 that is not an envelope: refused without reading it all into memory, nothing written locally. [Task 3]
3. fernjo is closed or crashes between "apply the merge locally" and "push": the next run merges again and changes nothing (no second conflict copy, no duplicate nodes). [Task 8]
4. The vault is locked at start and the head only changed the connections, or only the vault file: the connections merge and the vault file is adopted without unlock; only a both-sides vault change waits for unlock. [Task 8]
5. The tree is edited by the user while a sync run is between "read local" and "apply": the run notices, starts over and the edit survives. [Task 9]

---

## Phase 1: Protocol and hub

### Task 1: `fernjo-sync` crate: key, envelope, snapshot

**Files:**
- Create: `crates/fernjo-sync/Cargo.toml`, `src/lib.rs`, `src/error.rs`, `src/envelope.rs`, `src/snapshot.rs`
- Modify: root `Cargo.toml` (workspace dependency `hkdf`; `crates/*` already a member)
- Test: unit tests next to each module

**Interfaces:**
- Produces in `envelope.rs`: `pub struct SyncKey` (32 bytes; `fn generate() -> Result<SyncKey, SyncError>`, `fn from_bytes([u8; 32]) -> SyncKey`, `fn as_bytes(&self) -> &[u8; 32]`; redacted `Debug`); `pub type GroupId = uuid::Uuid`; `pub const MAGIC: &[u8; 8] = b"FERNJOSY"`, `pub const FORMAT_VERSION: u16 = 1`, `pub const MAX_ENVELOPE_LEN: usize = 64 * 1024 * 1024`; `pub fn seal(key: &SyncKey, group: GroupId, seq: u64, plaintext: &[u8]) -> Result<Vec<u8>, SyncError>`; `pub struct Opened { pub group: GroupId, pub seq: u64, pub plaintext: Zeroizing<Vec<u8>> }`; `pub fn open(key: &SyncKey, bytes: &[u8]) -> Result<Opened, SyncError>`; `pub fn peek_header(bytes: &[u8]) -> Result<(GroupId, u64), SyncError>` (unauthenticated, for the hub and for rollback checks before decryption).
- Produces in `snapshot.rs`: `pub struct DeviceInfo { pub id: Uuid, pub name: String }`; `pub struct Snapshot { pub format_version: u32, pub group_id: GroupId, pub seq: u64, pub writer: DeviceInfo, pub written_at: String, pub connections: fernjo_core::Tree, pub vault: Option<Vec<u8>> }` with `fn to_plaintext(&self) -> Result<Vec<u8>, SyncError>` and `fn from_plaintext(&[u8]) -> Result<Snapshot, SyncError>` (JSON; `connections` is the `connections.json` document via `fernjo_core::persist::document` / `parse_document`, `vault` is base64; field names camelCase).
- Produces in `error.rs`: `SyncError` with `code() -> &'static str` for: `bad_envelope`, `newer_format`, `wrong_key_or_damaged`, `wrong_group`, `snapshot_invalid`, `snapshot_too_large`, `random_failed`.

- [ ] **Step 1: Write the failing tests**
  - `envelope.rs`: `seal_then_open_round_trips`; `open_with_another_key_fails_as_wrong_key_or_damaged`; `flipping_any_header_byte_fails_authentication` (loop over bytes 0..34 except where the failure is `bad_envelope` or `newer_format`; assert `wrong_key_or_damaged` or those codes, never success); `flipping_a_ciphertext_byte_fails`; `a_head_moved_to_another_seq_fails` (rewrite bytes 26..34, expect `wrong_key_or_damaged`); `newer_version_is_refused_as_newer_format`; `short_or_wrong_magic_is_bad_envelope`; `nonce_is_fresh_per_seal` (two seals of equal input differ); `a_envelope_above_64_MiB_is_refused_before_decryption`; `debug_of_sync_key_is_redacted`.
  - `snapshot.rs`: `snapshot_round_trips_with_and_without_a_vault`; `a_tree_deeper_than_60_levels_is_snapshot_invalid`; `unknown_fields_are_ignored`; `format_version_above_1_is_newer_format`.
- [ ] **Step 2: Run** `cargo test -p fernjo-sync` — expect FAIL (crate missing, then failing tests).
- [ ] **Step 3: Implement** the interfaces. Key derivation: HKDF-SHA256 over the sync key with the fixed info string `fernjo-sync v1 snapshot`, output 32 bytes, used as the AEAD key; nonce from `getrandom`.
- [ ] **Step 4: Run** `cargo test -p fernjo-sync && cargo clippy -p fernjo-sync --all-targets -- -D warnings && cargo deny check` — green.
- [ ] **Step 5: Commit** `feat(sync): snapshot envelope with HKDF-derived XChaCha20-Poly1305`

### Task 2: `fernjo-hub`: the blob store with compare-and-swap, the server and the deploy files

**Files:**
- Create: `crates/fernjo-hub/Cargo.toml`, `src/main.rs` (CLI), `src/lib.rs`, `src/store.rs` (head and history on disk), `src/server.rs` (HTTP), `tests/http.rs`, `Containerfile` (in `crates/fernjo-hub/`), `deploy/fernjo-hub.container` (Quadlet unit), `deploy/Caddyfile.example`, `Docs/hub.md`
- Modify: `deny.toml` only if a licence is outside the allow-list (then ask the owner)

**Interfaces:**
- Consumes: nothing from `fernjo-sync`: the hub keeps its own parser for the 34 unauthenticated header bytes (`fn peek_header(&[u8]) -> Option<(uuid::Uuid, u64)>` over magic, version, group id, `seq`) and its own 64 MiB constant, so the two crates stay independent and the hub never holds the sync key. Task 3 pins both parsers against each other.
- Produces in `store.rs`: `pub struct HubStore` with `fn open(dir: &Path) -> io::Result<HubStore>`, `fn head(&self) -> io::Result<Option<(u64, Vec<u8>)>>`, `fn put(&self, expected: Expected, seq: u64, group: GroupId, bytes: &[u8]) -> Result<(), PutError>` where `enum Expected { Empty, Seq(u64) }` and `enum PutError { Stale { current: Option<u64> }, SeqNotHigher, WrongGroup, TooLarge, Io(io::Error) }`; `fn history(&self) -> io::Result<Vec<u64>>`. One process-wide lock around compare-and-swap; the first write fixes the group id and a later write for another group is `WrongGroup`; atomic write (random temp name, fsync, rename), the old head is kept as `history/<seq>.env`, only the newest 10 are kept.
- Produces in `server.rs`: `pub fn router(store: Arc<HubStore>, secret_sha256: [u8; 32]) -> <server type>`; routes as in Global Constraints; constant-time comparison of the SHA-256 of the presented bearer token; `PUT` answers 201/204 on success, 412 on `Stale`, 409 on `SeqNotHigher` or `WrongGroup`, 413 on `TooLarge`, 401 without a valid secret; `GET /v1/head` answers `ETag: "<seq>"`; no secret and no body in logs.
- CLI: `fernjo-hub init --data <dir>` creates the folder and a `config.json` holding only the SHA-256 of a new random secret, prints the secret once; `fernjo-hub serve --data <dir> --listen 127.0.0.1:<port>`; `fernjo-hub rotate-secret --data <dir>` prints a new secret and replaces the stored hash. Refuses to `serve` without `init`.

- [ ] **Step 1: Write the failing tests**
  - `store.rs`: `first_write_needs_empty`; `second_write_with_the_current_seq_succeeds`; `a_stale_expected_seq_is_refused_and_nothing_changes`; `a_seq_not_higher_is_refused`; `the_first_write_fixes_the_group`; `only_the_newest_10_envelopes_are_kept`; `a_reopened_store_keeps_head_and_history`; `an_oversize_body_is_too_large`; `two_threads_writing_the_same_expected_seq_one_wins`.
  - `tests/http.rs` (server on a loopback port): `no_token_or_wrong_token_is_401`; `get_head_of_an_empty_hub_is_404`; `put_then_get_returns_the_bytes_and_the_etag`; `put_with_a_stale_if_match_is_412`; `put_over_the_limit_is_413_without_buffering_it_all` (stream a body above 64 MiB, assert 413 and memory use stays bounded by checking the server did not accept the body); `info_reports_protocol_1`; `rotate_secret_invalidates_the_old_one` (CLI function level).
- [ ] **Step 2: Run** `cargo test -p fernjo-hub` — expect FAIL.
- [ ] **Step 3: Implement** the interfaces. Server and client HTTP crates are the builder's choice if `cargo deny check` accepts them (rustls with ring, no OpenSSL). `Docs/hub.md` covers: build the image (`podman build -f crates/fernjo-hub/Containerfile .`), static musl binary, non-root user, read-only root filesystem with one data volume, the Quadlet unit, the Caddy snippet `benjo.loc { tls internal; reverse_proxy 127.0.0.1:<port> }`, how to get Caddy's root certificate, `init`, `rotate-secret`, how to rebuild an empty hub from a device (Re-seed), the last-10 history files and what the hub can and cannot see.
- [ ] **Step 4: Run** `cargo test -p fernjo-hub && cargo clippy -p fernjo-hub --all-targets -- -D warnings && cargo deny check` — green. Where `podman` exists, also `podman build -f crates/fernjo-hub/Containerfile .` and a `curl` against the running container; where it does not, record that as an open owner check with this command.
- [ ] **Step 5: Commit** `feat(hub): fernjo-hub blob store with compare-and-swap, server and deploy files`

### Task 3: Hub client and pairing code

**Files:**
- Create: `crates/fernjo-sync/src/client.rs`, `src/pairing.rs`
- Modify: `crates/fernjo-sync/Cargo.toml`, `src/lib.rs`
- Test: unit tests next to each module and `crates/fernjo-sync/tests/client_hub.rs` (in-process hub from `fernjo-hub` as a dev-dependency, loopback)

**Interfaces:**
- Consumes: `fernjo-hub` router (dev-dependency only), `SyncKey`, `GroupId`, `MAX_ENVELOPE_LEN`.
- Produces in `client.rs`: `pub struct HubConfig { pub url: String, pub secret: secrecy::SecretString, pub ca_pem: Option<String> }`; `pub struct HubClient` with `fn new(cfg: &HubConfig) -> Result<HubClient, SyncError>` (HTTPS only; `http://` only for `127.0.0.1` and `::1`; with `ca_pem` only that CA is trusted, otherwise the system roots; redirects are refused; connect timeout 3 s, total timeout 20 s), `fn info(&self) -> Result<HubInfo, SyncError>`, `fn get_head(&self) -> Result<Option<(u64, Vec<u8>)>, SyncError>` (body read bounded by `MAX_ENVELOPE_LEN`), `fn put_head(&self, expected: Expected, seq: u64, bytes: &[u8]) -> Result<(), SyncError>`; `pub enum Expected { Empty, Seq(u64) }`. New `SyncError` codes: `hub_unreachable` (connect or timeout, the quiet offline state), `hub_unauthorized`, `hub_tls_untrusted`, `hub_conflict` (412), `hub_protocol` (anything else unexpected), `hub_too_large`.
- Produces in `pairing.rs`: `pub struct PairingCode { pub url: String, pub secret: SecretString, pub key: SyncKey, pub group: GroupId, pub ca_pem: Option<String> }` with `fn encode(&self) -> String` (base64url of a versioned JSON plus a 4-byte SHA-256 checksum, prefix `fernjo-sync-1:`) and `fn decode(&str) -> Result<PairingCode, SyncError>` (whitespace and line breaks inside are ignored; codes `invalid_pairing_code`, `newer_pairing_code`); redacted `Debug`.

- [ ] **Step 1: Write the failing tests**
  - `pairing.rs`: `encode_decode_round_trips_with_and_without_a_ca`; `a_truncated_code_is_invalid_pairing_code`; `one_changed_character_is_invalid_pairing_code`; `a_code_with_line_breaks_and_spaces_still_decodes`; `prefix_with_a_higher_version_is_newer_pairing_code`; `debug_is_redacted`.
  - `client.rs` with a stub server: `plain_http_to_a_non_loopback_host_is_refused`; `a_redirect_is_refused`; `a_body_above_64_MiB_is_hub_too_large_and_is_not_buffered`; `a_200_for_head_without_an_etag_is_hub_protocol`; `connection_refused_is_hub_unreachable`; `401_is_hub_unauthorized`.
  - `tests/client_hub.rs`: `put_get_round_trip_against_the_real_hub`; `a_stale_put_is_hub_conflict`; `a_wrong_secret_is_hub_unauthorized`; `hub_and_sync_header_parsers_agree` (an envelope from `seal` is read identically by the hub's parser and `peek_header`, and both refuse the same malformed inputs).
- [ ] **Step 2: Run** `cargo test -p fernjo-sync` — expect FAIL.
- [ ] **Step 3: Implement** the interfaces. The TLS check against a private CA is covered by a test that starts a TLS stub with a throwaway CA when the chosen crates make that cheap; if not, record "pinned CA against Caddy `tls internal`" as an owner check in Task 12.
- [ ] **Step 4: Run** `cargo test -p fernjo-sync -p fernjo-hub && cargo clippy --workspace --all-targets -- -D warnings && cargo deny check` — green.
- [ ] **Step 5: Commit** `feat(sync): hub client (HTTPS, pinned CA, bounded reads) and pairing code`

---

## Phase 2: The merge

### Task 4: Merge the connection tree (rules C1 to C10)

**Files:**
- Create: `crates/fernjo-sync/src/merge/mod.rs`, `src/merge/tree.rs`, `src/merge/report.rs`
- Test: `crates/fernjo-sync/src/merge/tree_tests.rs`

**Interfaces:**
- Produces: `pub struct MergeContext { pub device_name: String, pub date: String }` (labels for conflict copies; the date comes from the caller, never read inside the merge); `pub enum Finding { ConflictCopy { original: NodeId, copy: NodeId, name: String }, KeptEdit { node: NodeId, name: String }, PlacementKept { node: NodeId, name: String }, MoveUndone { node: NodeId, name: String } }`; `pub struct TreeMerge { pub tree: fernjo_core::Tree, pub findings: Vec<Finding> }`; `pub fn merge_tree(base: &Tree, local: &Tree, head: &Tree, ctx: &MergeContext) -> Result<TreeMerge, SyncError>` (pure and deterministic apart from the new random ids of conflict copies, which come from an injectable `fn() -> NodeId` parameter in an inner function for tests).
- Behaviour: exactly spec rules C1 to C10; the head's value stays on the original node and the local version becomes the copy (name `"<name> (conflict <device> <date>)"`, a folder copy is empty); the result never exceeds depth 60 and has no cycle; a result that cannot be built is `snapshot_invalid`.

- [ ] **Step 1: Write the failing tests**, one per rule, each building `base`, `local`, `head` from a small helper and asserting the merged tree and the findings:
  `c1_different_nodes_and_different_fields_are_both_kept`; `c2_a_field_changed_on_one_side_is_kept` (once local-only, once head-only); `c3_same_value_on_both_sides_is_no_conflict`; `c4_different_values_keep_the_head_and_copy_the_local_version` (also: a conflicted folder copy is empty and its children stay); `c5_deleted_on_one_side_and_unchanged_on_the_other_is_deleted` (with a subtree); `c6_delete_versus_edit_keeps_the_node_and_its_folders` (edit, and a node added under a deleted folder); `c7_nodes_added_on_either_side_are_kept`; `c8_one_sided_move_is_applied`; `c8_move_to_different_parents_keeps_the_heads_parent`; `c8_sibling_order_one_side_and_both_sides`; `c9_a_cycle_from_two_moves_undoes_the_local_move`; `c9_depth_above_60_undoes_the_local_move`; `c10_a_dangling_credential_reference_is_left_alone`; plus `identical_inputs_change_nothing` and `merging_twice_is_idempotent` (merge the result against the same head and base again: tree equal, no new findings).
- [ ] **Step 2: Run** `cargo test -p fernjo-sync merge::tree` — expect FAIL.
- [ ] **Step 3: Implement** `merge_tree` over a flattened view (id to parent, index and own fields) with one generic field-merge helper that Task 5 reuses (`fn merge_field<T: PartialEq + Clone>(base: &T, local: &T, head: &T) -> FieldOutcome<T>` with `Same`, `Take(T)`, `Conflict { local, head }`). "Touched" for C6 means "differs from base", also for descendants.
- [ ] **Step 4: Run** `cargo test -p fernjo-sync && cargo clippy -p fernjo-sync --all-targets -- -D warnings` — green.
- [ ] **Step 5: Commit** `feat(sync): three-way merge of the connection tree`

### Task 5: Merge vault entries (rules V2 to V4)

**Files:**
- Create: `crates/fernjo-sync/src/merge/entries.rs`
- Test: `crates/fernjo-sync/src/merge/entries_tests.rs`

**Interfaces:**
- Consumes: `merge_field`, `MergeContext`, `Finding` from Task 4 (extend `Finding` with `EntryConflictCopy { original: EntryId, copy: EntryId, name: String }`, `EntryKeptEdit { entry: EntryId, name: String }`); `fernjo_vault::{EntryId, NewEntry}`.
- Produces: `pub struct EntriesMerge { pub entries: Vec<(EntryId, NewEntry)>, pub findings: Vec<Finding> }`; `pub fn merge_entries(base: &[(EntryId, NewEntry)], local: &[(EntryId, NewEntry)], head: &[(EntryId, NewEntry)], ctx: &MergeContext) -> EntriesMerge`. Fields are name, username, domain, notes and the secret as one unit; the head's version keeps the id, the local version becomes a new entry named `"<name> (conflict <device> <date>)"` with a fresh id.

- [ ] **Step 1: Write the failing tests:** `v2_different_entries_and_fields_merge`; `v2_the_secret_changed_on_both_sides_copies_the_local_entry`; `v2_notes_and_password_changed_on_different_sides_both_kept` (notes and secret are different fields); `v2_deleted_versus_edited_keeps_the_entry`; `v2_deleted_and_unchanged_is_deleted`; `v2_added_on_either_side_is_kept`; `merging_twice_is_idempotent`; `findings_never_contain_a_secret` (sentinel secrets, `format!("{:?}")` of the findings and entries' names only).
- [ ] **Step 2: Run** `cargo test -p fernjo-sync merge::entries` — expect FAIL.
- [ ] **Step 3: Implement** with the shared `merge_field`; compare the secret by value (a `Secret` needs a value comparison that exposes nothing outside the function).
- [ ] **Step 4: Run** `cargo test -p fernjo-sync && cargo clippy -p fernjo-sync --all-targets -- -D warnings` — green.
- [ ] **Step 5: Commit** `feat(sync): three-way merge of vault entries`

---

## Phase 3: Vault support

### Task 6: `fernjo-vault`: adopt a file, merge into a file, sweep stale temp files

**Files:**
- Modify: `crates/fernjo-vault/src/vault.rs`, `src/lib.rs`, `src/format.rs` only if a header accessor is missing
- Test: tests next to `vault.rs` (existing style) and `crates/fernjo-vault/tests/`; golden fixture `v1-no-generation.fernjo` must keep opening

**Interfaces:**
- Produces: `pub struct FileInfo { pub id: VaultId, pub version_header: Vec<u8> }` (the 150 header bytes) and `pub fn inspect_file(bytes: &[u8]) -> Result<FileInfo, VaultError>` (`Header::parse`, so KDF bounds and the newer-format rule apply); `pub struct Contents { pub generation: u64, pub entries: Vec<(EntryId, NewEntry)> }` and `pub fn read_entries(bytes: &[u8], key: &DataKey) -> Result<Contents, VaultError>`; `Vault::file_bytes(&self) -> Result<Vec<u8>, VaultError>` (capped read of the current file); `Vault::adopt_file(&mut self, bytes: &[u8]) -> Result<(), VaultError>`; `Vault::adopt_merged(&mut self, head_bytes: &[u8], entries: Vec<(EntryId, NewEntry)>) -> Result<(), VaultError>` (needs an unlocked vault); constant `STALE_TEMP_AGE: Duration = 24 h`; the sweep runs inside `Vault::open`.
- Behaviour of `adopt_file`: refuses `bytes` above `MAX_FILE_LEN`, with another vault id (`unreadable`), or a newer format; runs the header guard (`changed_on_disk` when the file on disk is not the one this handle knows); writes atomically with the `.bak`; sets the handle's header; when unlocked, re-opens the body with the held key and refuses with `rolled_back` (before writing anything) when its generation is below the floor; when locked, stays locked; never lowers the floor or the device mark. Behaviour of `adopt_merged`: same guards except the rollback check, takes the head's header (password wrap), sets the floor to `max(floor, head generation)`, replaces the entries and saves, so the new generation is above both the local and the head's.

- [ ] **Step 1: Write the failing tests:** `inspect_file_reports_the_vault_id_and_changes_with_every_save`; `inspect_file_refuses_a_kdf_cost_above_the_cap`; `read_entries_with_the_held_key_opens_a_file_after_a_password_change` (same data key); `adopt_file_while_locked_stays_locked_and_the_next_unlock_sees_the_new_entries`; `adopt_file_while_unlocked_keeps_the_vault_unlocked_and_the_key_valid`; `adopt_file_of_another_vault_is_unreadable_and_writes_nothing`; `adopt_file_refuses_when_the_file_changed_on_disk`; `adopt_file_with_a_lower_generation_is_rolled_back_and_writes_nothing`; `adopt_file_keeps_a_bak_of_the_previous_file`; `adopt_merged_writes_a_generation_above_both_sides`; `adopt_merged_takes_the_heads_password_wrap` (unlock with the head's password afterwards); `open_removes_vault_temp_files_older_than_a_day_and_keeps_newer_and_foreign_files`.
- [ ] **Step 2: Run** `cargo test -p fernjo-vault` — expect FAIL.
- [ ] **Step 3: Implement** the interfaces; reuse `write_atomic`, `check_unchanged_on_disk`, `open_body` and `read_capped`. The sweep only touches names matching `vault.fernjo.<16 hex>.tmp` that are regular files older than a day, ignores errors, and runs once per open.
- [ ] **Step 4: Run** `cargo test -p fernjo-vault && cargo clippy -p fernjo-vault --all-targets -- -D warnings` — green; the golden fixture test passes.
- [ ] **Step 5: Commit** `feat(vault): adopt and merge files for sync, sweep stale temp files`

---

## Phase 4: Engine, app, UI

### Task 7: Device-local sync state and the keychain items

**Files:**
- Create: `crates/fernjo-sync/src/state.rs`, `src/secrets.rs`
- Test: unit tests next to each module

**Interfaces:**
- Produces in `state.rs`: `pub struct SyncState { pub url: String, pub ca_pem: Option<String>, pub device: DeviceInfo, pub group: GroupId, pub last_seq: u64 }` and `pub struct StateStore` with `fn new(device_dir: PathBuf) -> StateStore`, `fn load(&self) -> Result<Option<SyncState>, SyncError>`, `fn save(&self, &SyncState) -> Result<(), SyncError>`, `fn load_base(&self) -> Result<Option<Snapshot>, SyncError>`, `fn save_base(&self, &Snapshot) -> Result<(), SyncError>`, `fn clear(&self) -> Result<(), SyncError>`; capped reads (64 MiB), atomic writes, `last_seq` only ever rises, an unreadable or newer-schema file is `state_unusable` and never overwritten.
- Produces in `secrets.rs`: `pub trait SyncSecrets: Send + Sync { fn get(&self, group: GroupId) -> Result<Option<(SecretString, SyncKey)>, SyncError>; fn set(&self, group: GroupId, hub_secret: &SecretString, key: &SyncKey) -> Result<(), SyncError>; fn delete(&self, group: GroupId) -> Result<(), SyncError>; }`, `MemorySyncSecrets` for tests, `OsSyncSecrets` on the `keyring` crate (service `fernjo`, user `sync-<group id>`, one item holding both values); an unavailable keychain is `keychain_unavailable`.

- [ ] **Step 1: Write the failing tests:** `state_round_trips_and_last_seq_never_lowers`; `a_corrupt_state_file_is_state_unusable_and_is_not_overwritten`; `a_newer_schema_state_file_is_state_unusable`; `base_round_trips_with_a_vault_and_without`; `clear_removes_state_and_base`; `memory_secrets_round_trip_and_delete`; `debug_of_the_secret_stores_is_redacted`.
- [ ] **Step 2: Run** `cargo test -p fernjo-sync state secrets` — expect FAIL.
- [ ] **Step 3: Implement** the interfaces (the atomic-write helper is the same copy used in `fernjo-ssh` and `fernjo-rdp`, because `fernjo-sync` may depend on `fernjo-vault` but should not grow a third divergent copy: use `fernjo_core::persist::write_exact` if it fits, else a local copy with a comment).
- [ ] **Step 4: Run** `cargo test -p fernjo-sync && cargo clippy -p fernjo-sync --all-targets -- -D warnings` — green.
- [ ] **Step 5: Commit** `feat(sync): device-local sync state and keychain secrets`

### Task 8: The sync engine (setup, join, run, re-seed)

**Files:**
- Create: `crates/fernjo-sync/src/engine.rs`, `src/ports.rs`
- Test: `crates/fernjo-sync/tests/engine.rs` (in-memory ports, real hub from `fernjo-hub` on loopback)

**Interfaces:**
- Consumes: Tasks 1, 3, 4, 5, 7.
- Produces in `ports.rs`: `pub trait LocalPort { fn read(&self) -> Result<LocalState, SyncError>; fn apply(&self, expected: &LocalState, next: LocalChange) -> Result<(), SyncError>; fn vault_unlocked(&self) -> bool; }` where `pub struct LocalState { pub tree: Tree, pub vault: Option<VaultSide> }`, `pub struct VaultSide { pub file: Vec<u8>, pub entries: Option<Vec<(EntryId, NewEntry)>>, pub key: Option<DataKey> }` (entries and key only while unlocked), and `pub struct LocalChange { pub tree: Option<Tree>, pub vault: VaultChange }` with `pub enum VaultChange { Keep, Adopt(Vec<u8>), Merged { head: Vec<u8>, entries: Vec<(EntryId, NewEntry)> } }`. `apply` fails with `local_changed` when the current local state no longer equals `expected` (the port compares).
- Produces in `engine.rs`: `pub struct Engine<'a> { pub hub: &'a HubClient, pub state: &'a StateStore, pub secrets: &'a dyn SyncSecrets, pub local: &'a dyn LocalPort, pub ctx: MergeContext }`; `pub enum Outcome { Synced { findings: Vec<Finding>, seq: u64 }, NothingToDo, Offline { pending_changes: usize }, WaitingForUnlock, HubRolledBack { found: u64, seen: u64 } }`; `fn run_once(&self) -> Result<Outcome, SyncError>`; `fn setup(&self, cfg: &HubConfig, device_name: &str) -> Result<PairingCode, SyncError>` (refuses `hub_not_empty`); `fn join(&self, code: &PairingCode, device_name: &str) -> Result<(), SyncError>` (replaces local data through `apply`, base := head); `fn reseed(&self) -> Result<(), SyncError>` (pushes local state with `seq` above the seen one); `fn pending_changes(&self) -> Result<usize, SyncError>` (comparison with the base: tree nodes plus 1 for a changed vault file).
- Behaviour of `run_once`: read local, fetch head (outside any lock), refuse `hub_rolled_back` for a head `seq` below `last_seq` or an empty hub with `last_seq > 0`, open the envelope (group must match), merge (tree always; vault by V1, V2, V3, V4, V5), apply with `expected`, restart (at most 5 times) on `local_changed`, push with the right `Expected`, restart on `hub_conflict`, then store the new base and `last_seq`; an unreachable hub gives `Offline`, a both-sides vault change while locked gives `WaitingForUnlock` without any partial apply or push; a merged result larger than the hub limit is `snapshot_too_large`.

- [ ] **Step 1: Write the failing tests** (two engines, each with in-memory ports and its own state folder, one real hub): `setup_uploads_seq_1_and_join_copies_it_to_the_second_device`; `setup_on_a_hub_with_a_head_is_hub_not_empty`; `join_when_the_head_has_no_vault_keeps_the_local_vault_and_the_next_run_shares_it`; `a_local_edit_reaches_the_other_device`; `a_vault_entry_added_on_one_device_reaches_the_other` (the second device keeps its vault locked: V1); `both_sides_changed_the_vault_while_locked_waits_for_unlock_and_changes_nothing`; `both_sides_changed_the_vault_unlocked_merges_and_the_generation_is_above_both`; `a_conflict_makes_one_copy_and_a_second_run_changes_nothing`; `a_crash_between_apply_and_push_is_repaired_by_the_next_run_without_a_duplicate` (run with a client that fails the PUT once, run again); `a_stale_head_below_last_seq_is_hub_rolled_back_and_nothing_is_written`; `an_empty_hub_after_sync_is_hub_rolled_back_and_reseed_rebuilds_it`; `an_unreachable_hub_is_offline_with_the_pending_count`; `a_hub_that_serves_a_different_group_is_wrong_group`; `a_local_change_during_the_run_restarts_it_and_the_edit_survives` (port that changes local state once between read and apply); `a_merged_result_over_64_MiB_is_snapshot_too_large`; `a_head_with_a_tree_deeper_than_60_is_refused_and_writes_nothing`.
- [ ] **Step 2: Run** `cargo test -p fernjo-sync --test engine` — expect FAIL.
- [ ] **Step 3: Implement** the interfaces. The engine itself holds no lock and no `AppState`; the Tauri side supplies the `LocalPort`.
- [ ] **Step 4: Run** `cargo test -p fernjo-sync && cargo clippy -p fernjo-sync --all-targets -- -D warnings` — green.
- [ ] **Step 5: Commit** `feat(sync): sync engine with rollback protection, join and re-seed`

### Task 9: App integration: state, port, scheduler, commands

**Files:**
- Create: `src-tauri/src/sync_commands.rs`, `src-tauri/src/sync_port.rs`, `src-tauri/src/sync_runner.rs`
- Modify: `src-tauri/Cargo.toml`, `src-tauri/src/state.rs` (`AppState.sync`), `src-tauri/src/lib.rs` (modules, `generate_handler!`, window events, startup trigger), `src-tauri/src/vault_commands.rs` (`table()`, `sources`, `NOT_A_COMMAND` where needed), `src-tauri/src/commands.rs` if the snapshot type needs a sync field
- Test: unit tests in each new file; the existing `no_response_contains_a_secret` and `command_table_covers_every_tauri_command`

**Interfaces:**
- Consumes: `Engine`, `LocalPort`, `StateStore`, `SyncSecrets` (`OsSyncSecrets` in `open_for_app`, `MemorySyncSecrets` in `AppState::open`), `Store::replace_tree`, `Vault::{file_bytes, adopt_file, adopt_merged, export_entries, data_key}`.
- Produces: `AppState.sync: Mutex<SyncRuntime>` (`SyncRuntime` holds the last `Outcome` summary, the pending findings for the notice bar, and a running flag); `sync_port.rs`: `pub struct AppPort<'a>(&'a AppState)` implementing `LocalPort` (reads under pending, vault, store lock order, releases; `apply` re-locks, compares with `expected`, then `replace_tree` and the vault change, so the fingerprint and header guards run; a `changed_on_disk` from a foreign writer ends the run with that code); `sync_runner.rs`: `pub fn spawn_triggers(app: &AppHandle)` for the triggers (after start once auto-unlock was tried, 5 minutes after the last run, window focus, a debounce of 5 s after a local save, "Sync now", and a best-effort push of at most 3 s on close) with no overlapping runs.
- Commands (each with `table()` entry and `generate_handler!`): `sync_status() -> SyncStatus { state: "off" | "ok" | "offline" | "waiting_for_unlock" | "rolled_back" | "error", lastSyncedAt: Option<String>, pendingChanges: usize, deviceName: Option<String>, hubUrl: Option<String>, findings: Vec<FindingView>, error: Option<{code, message}> }`; `sync_setup(url, secret, caPem: Option<String>, deviceName) -> SyncStatus`; `sync_pairing_code() -> String` (the reveal exception); `sync_join(code, deviceName, confirmReplace: bool) -> SyncStatus` (without `confirmReplace` it returns the counts that would be lost as `needs_confirmation`); `sync_now() -> SyncStatus`; `sync_reseed() -> SyncStatus`; `sync_edit_secret(secret) -> SyncStatus`; `sync_leave() -> SyncStatus`; `sync_dismiss_findings()`. Findings view carries ids and names only, never a secret.

- [ ] **Step 1: Write the failing tests:** `sync_status_is_off_without_setup`; `setup_stores_the_secrets_in_the_keychain_and_not_in_any_response` (memory secrets, sentinel hub secret and key, assert absent from every command response, error and `Debug`, and from `sync.json`); `pairing_code_is_the_only_response_with_the_secret`; `save_never_waits_for_the_hub` (hub unreachable, a tree edit returns at once and `pendingChanges` is 1); `join_without_confirmation_reports_what_would_be_lost_and_changes_nothing`; `join_replaces_tree_and_vault_and_closes_sessions`; `a_local_edit_between_read_and_apply_restarts_the_run` (Review Focus 5); `a_changed_on_disk_during_apply_ends_the_run_and_the_next_run_succeeds`; `locked_vault_with_a_head_that_changed_only_the_connections_still_syncs` (Review Focus 4); `leave_removes_state_base_and_keychain_items_and_keeps_local_data`; `the_existing_rollback_and_fingerprint_tests_still_pass` (no new test: run `cargo test -p fernjo-app`). Update `table()` for all new commands.
- [ ] **Step 2: Run** `cargo test -p fernjo-app sync` — expect FAIL.
- [ ] **Step 3: Implement.** Network I/O runs on a blocking thread outside every lock. The sidebar's "changes waiting" and the status come from `Engine::pending_changes`. Without a device folder (`device_dir` is `None`) setup is refused with `no_device_folder`; with an unavailable keychain `keychain_unavailable`.
- [ ] **Step 4: Run** `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings` — green.
- [ ] **Step 5: Commit** `feat(app): sync state, local port, triggers and commands`

### Task 10: UI: Settings section, status line, notices, reload bar

**Files:**
- Create: `ui/src/lib/sync.svelte.ts` (`SyncStore`, view state only), `ui/src/lib/components/SyncSettings.svelte`, `ui/src/lib/components/SyncStatus.svelte`, tests next to each
- Modify: `ui/src/lib/types.ts`, `ui/src/lib/api.ts`, `ui/src/lib/testing.ts`, `ui/src/lib/components/SettingsDialog.svelte`, `ui/src/lib/components/Sidebar.svelte` (footer line), `ui/src/lib/components/NoticeBar.svelte` (findings), `ui/src/lib/store.svelte.ts` (`#run` already sets `changedOnDisk`; see below), `ui/src/lib/components/VaultManager.svelte` and `ImportDialog.svelte` (route `changed_on_disk` to the flag)

**Interfaces:**
- Consumes: the commands of Task 9 and the wire types above.
- Produces: `SyncStore` with `status`, `busy`, `refresh()`, `setup(...)`, `join(...)`, `pairingCode()`, `syncNow()`, `reseed()`, `editSecret(...)`, `leave()`; ignores a new action while `busy`; the Settings "Sync" section (not set up: two choices "Set up this device" and "Join with a pairing code"; set up: status, Sync now, Show pairing code (secret hidden until clicked and cleared on close), Edit hub secret, Re-seed hub (only shown for `rolled_back`), Leave sync, each destructive step confirmed through `ConfirmDialog`; join shows the counts from `needs_confirmation` and offers the backup export first); the footer line "Synced 12:03", "Offline, 3 changes waiting" (offline is a calm state, not an error), "Sync paused: unlock the vault", "Sync: hub older than this device" with the Re-seed action; findings in the notice bar naming each conflict copy, kept edit, placement and undone move with a Dismiss button.
- Reload bar: a vault `delete_credential` and an import `apply_backup` that fail with `changed_on_disk` set `TreeStore.changedOnDisk` so the existing bar and Reload button appear (roadmap task 3).

- [ ] **Step 1: Write the failing tests:** `SyncSettings.test.ts`: `not_set_up_offers_setup_and_join`; `setup_sends_url_secret_ca_and_device_name_and_clears_the_secret_field`; `show_pairing_code_reveals_once_and_hides_on_close`; `join_asks_for_confirmation_with_the_counts_before_replacing`; `reseed_is_only_offered_when_rolled_back`; `leave_asks_first`; `SyncStatus.test.ts`: `offline_shows_the_pending_count_and_no_error_bar`; `waiting_for_unlock_shows_its_text`; `synced_shows_the_time`; `NoticeBar.test.ts`: `findings_are_listed_and_dismissable`; `VaultManager.test.ts` and `ImportDialog.test.ts`: `a_changed_on_disk_from_delete_credential_shows_the_reload_bar`; `a_changed_on_disk_from_apply_backup_shows_the_reload_bar`; `store.test.ts`: `sync_store_ignores_a_new_action_while_busy`.
- [ ] **Step 2: Run** `pnpm test -- Sync NoticeBar VaultManager ImportDialog store` — expect FAIL.
- [ ] **Step 3: Implement** with existing components and tokens only. The UI never derives sync state; it shows what `sync_status` says.
- [ ] **Step 4: Run** `pnpm test && pnpm lint` — green. Check the section and the footer line at 1280x800 and in both themes in a Chromium harness with the mocked API (state it as harness-only evidence).
- [ ] **Step 5: Commit** `feat(ui): sync settings, status line, notices and the reload bar for vault and import`

---

## Phase 5: Documents, proof, roadmap

### Task 11: Documents and changelog

**Files:**
- Create: `Docs/sync-format.md` (envelope layout, snapshot JSON, hub API, pairing code format, `sync.json` and `sync-base.json`)
- Modify: `Docs/hub.md` (final check against the shipped CLI), `Docs/superpowers/specs/2026-11-15-fernjo-design.md` §5.4 (one line: built-in sync exists, see the M9 design), `CLAUDE.md` ("Project-specific": new crates, device-local files, the new commands and the three-places rule, the reveal exception, the hub, the engine's ports, the lock rule), `CHANGELOG.md` (`[Unreleased]`: multi-device sync, the reload bar for vault and import, the temp-file sweep), `Docs/ROADMAP.md` via the generator

- [ ] **Step 1: Write** the documents so that someone can write a compatible reader or hub without the source (no lock-in), as `vault-format.md` does.
- [ ] **Step 2: Run** `dotnet run tools/roadmap.cs -- check` and `cargo test --workspace` — green.
- [ ] **Step 3: Commit** `docs: sync format, hub guide, changelog and project notes`

### Task 12: Verification, owner two-device run, roadmap close

**Files:**
- Modify: `Docs/roadmap.json` (tick tasks and acceptance with evidence), `Docs/ROADMAP.md` (generated)

- [ ] **Step 1: Run the full gate** and record each result: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm lint`, `pnpm test`, `cargo deny check`, `dotnet run tools/roadmap.cs -- check`, `bash tools/test-release-scripts.sh`.
- [ ] **Step 2: Owner run (not doable on this machine).** On two of {gaming PC, CachyOS laptop, macOS 15 laptop}, against the hub VM (rootless Podman, Caddy `tls internal`, `benjo.loc`): (a) set up device 1, join device 2 with the pairing code; (b) create and edit a connection on one and see it on the other; (c) add a vault entry on one and use it on the other; (d) take one device off the network, edit the same connection field on both and a different one on each, reconnect: one conflict copy and no loss; (e) delete a connection on one while editing it on the other: it stays; (f) restart the hub container and Caddy: nothing lost; (g) restore the hub from an older VM snapshot: devices refuse with the rolled-back notice and Re-seed rebuilds it; (h) `fernjo-hub rotate-secret`: the old devices report `hub_unauthorized` until the secret is edited. The evidence names the devices and the hub setup.
- [ ] **Step 3: Update the roadmap:** tick the finished tasks immediately, tick an acceptance criterion only with its proof (test name or command and result, or the owner's run), keep the owner run open until it happened, set the milestone `done` with `completedOn` only after that, then `dotnet run tools/roadmap.cs -- generate` and `-- check`.
- [ ] **Step 4: Open the PR to `dev`** with every check green; merge it myself only when CI is green (CLAUDE.md, "Git & branches").
