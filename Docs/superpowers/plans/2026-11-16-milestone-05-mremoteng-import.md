# Milestone 5 – mRemoteNG import, export and backup: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The owner's mRemoteNG `confCons.xml` (stable 1.76, ConfVersion 2.6), passwords included, imports into fernjo in one reviewed step, and fernjo data can be exported (connections JSON, encrypted full backup) and restored.

**Architecture:** A new Tauri-free crate `fernjo-import` parses and decrypts files and returns pure *plans* (tree, vault entries, report); `fernjo-core` and `fernjo-vault` gain the few primitives plans need (graft/replace tree, password-sealed blob, add/replace entries). `src-tauri` keeps the decrypted plan in one pending slot in Rust between "inspect" (preview) and "apply", opens all native file dialogs itself, and applies vault entries first and the tree second with rollback. The UI only shows steps, counts and report lines.

**Tech Stack:** Rust (`roxmltree`, `aes-gcm`, `pbkdf2` + `sha1`, `base64`, existing RustCrypto in the vault), Tauri 2 + `tauri-plugin-dialog` (Rust API), Svelte 5 + TypeScript, Vitest, a file-based .NET script (BouncyCastle) for independent test vectors.

**Spec:** [Docs/superpowers/specs/2026-11-16-m05-import-export-design.md](../specs/2026-11-16-m05-import-export-design.md) (parent: [2026-11-15-fernjo-design.md](../specs/2026-11-15-fernjo-design.md) §5.5). Executors read both, `DESIGN.md` and the `CLAUDE.md` section "Project-specific".

**Refinements of the approved spec (decided while planning, same intent):**
- *Wrong password vs damaged file:* an AEAD cannot tell them apart after authentication fails, so an authentication failure is `wrong_password` (message "wrong password or the file is damaged", the vault's own behaviour); `damaged` is only for structural problems (too short, bad lengths, KDF parameters out of bounds, unparsable body). `not_a_backup` (wrong magic) and `newer_version` stay separate.
- *Connections JSON shape:* the file is the `connections.json` document (`schemaVersion`, `children`) plus `format: "fernjo-connections"` and `exportedAt`, minus every `credential`, so the existing migration and validation apply unchanged (no second version number).
- *Debug-only dialog override:* in debug builds the environment variable `FERNJO_DIALOG_FILE` replaces the native file dialog, so the flows can be driven through the DevTools port without OS input (`CLAUDE.md`: never drive real input). Release builds do not contain it.

## Global Constraints

- **Secrets stay in Rust:** no password, file key, export password or backup content in an IPC response, error text, `Debug` output, log, `connections.json` or webview storage. Typed secrets arrive as `String` at the command boundary and become `SecretString` at once. Secrets in the pending slot are `SecretString`/`Zeroizing`. Every new command joins the no-secret test.
- No home-made crypto: backup = Argon2id + XChaCha20-Poly1305 through the vault crate's code; mRemoteNG decryption = AES-256-GCM (16-byte nonce, 16-byte salt as associated data) with PBKDF2-HMAC-SHA1 (32-byte key, `KdfIterations` from the root attribute, 1000 ≤ n ≤ 1 000 000), default key `mR3m`. Secret layout: `base64(salt[16] ‖ nonce[16] ‖ ciphertext ‖ tag[16])`.
- Limits (hostile files): confCons at most 32 MiB, container depth at most 64, at most 100 000 nodes; backup KDF parameters bounded like the vault's.
- `fernjo-import` depends on `fernjo-core` and `fernjo-vault` only (no Tauri); `fernjo-core` and `fernjo-vault` stay free of each other.
- Supported mRemoteNG input: ConfVersion 2.6, `EncryptionEngine="AES"`, `BlockCipherMode="GCM"`. Everything else is `unsupported_format` with a message naming what was found (ConfVersion below 2.6 = legacy CBC, backlog; 2.7 and 2.8 = pre-release credential repository, not supported).
- Imported nodes get fresh ids; folder name `Imported from mRemoteNG (YYYY-MM-DD)` (local date), a repeat on the same day gets ` (2)`, ` (3)`; connections JSON uses `Imported from fernjo export (YYYY-MM-DD)`; file names `fernjo-connections-YYYY-MM-DD.json`, `fernjo-backup-YYYY-MM-DD.fjbk`.
- Export password: at least 8 characters, NFC-normalised, typed twice in the UI, never stored.
- Every new `#[tauri::command]` follows the `CLAUDE.md` checklist: function plus `handlers` wrapper, entry in `table()` (or `APP_HANDLE_EXEMPT` with a reason), `generate_handler!` in `lib.rs`, the new source file in the test's `sources`; plain functions the table calls that are not commands go into `NOT_A_COMMAND`. Argon2 commands are `#[tauri::command(async)]`.
- UI: only tokens from `ui/src/styles/tokens.css`, hex-only CSS and none of `oklch()`, `color-mix()`, `light-dark()`, nesting, `@layer`, `:has()`, container queries; one yellow key per context, the restore confirm is not yellow; UI text in English; TypeScript holds view state only.
- `Docs/DECISIONS.md` entries for the dependencies and the format facts; `CLAUDE.md` "Project-specific" gets the lasting knowledge; `CHANGELOG.md` `[Unreleased]` gets a user-facing entry.
- Work on `feature/m05-mremoteng-import`; Conventional Commits; stage files by name (no `git add .`); `git show --stat HEAD` after every commit and compare with the task's file list; no attribution lines; never `--no-verify`.
- "Done" = build, test, format and lint green (commands in the final task) plus `dotnet run tools/roadmap.cs -- check`. Tick roadmap criteria only after verification, with proof as evidence; anything unverifiable (the owner's real file, Linux, macOS) is marked open with the command that verifies it.

## Review Focus

Input classes the spec implies but no happy-path test covers, most likely first (each has a test in the named task):

1. A hostile or odd `confCons.xml`: huge, deeply nested, ten thousand siblings, missing attributes, `KdfIterations` of 1 or 4 billion, a `Port` of 0 or 70000, empty names, a file that is XML but not `Connections`, a backup or JSON picked as the mRemoteNG file: typed error or report line, never a panic or a hang (Tasks 4, 5, 8).
2. A node with its own *empty* password or username under a folder that has one: mRemoteNG uses none, fernjo would inherit the folder's; the report must name it (Task 5 `empty_own_password_under_folder_with_credential_is_reported`).
3. A failure half-way: vault entries written but the tree save fails; restore replaced the entries but the tree step fails: everything is as before and no stray entry remains (Task 8 `import_rolls_entries_back_when_graft_fails`, `restore_puts_entries_back_when_tree_replace_fails`).
4. The export password with non-ASCII text typed composed on one machine and decomposed on another, and a wrong-kind file picked for a flow (Task 2 `password_is_nfc_normalised`, Task 8 `wrong_kind_of_file_is_reported`).
5. Importing twice in one day, and restoring or importing while tabs are open or after a page reload with a plan waiting (Task 8 `second_import_same_day_gets_a_numbered_folder`, `restore_closes_sessions`; Task 9 `page_load_clears_pending`).

---

### Task 1: Core primitives (document parse, graft, replace, exact write)

**Files:**
- Modify: `crates/fernjo-core/src/persist.rs`, `tree.rs`, `store.rs`, `error.rs`, `lib.rs`

**Interfaces:**
- Produces (all `pub`, re-exported from `lib.rs`):
  - `persist::document(tree: &Tree) -> serde_json::Value` (exactly what `save` writes: `schemaVersion` and `children`).
  - `persist::parse_document(value: serde_json::Value) -> Result<Tree, DocumentError>`; `error::DocumentError { NewerSchema { found: u32, supported: u32 }, Invalid(String) }` with `code()` (`newer_schema`, `invalid_document`). `read_file` is refactored to call it, so migration and `sanitize` exist once.
  - `persist::write_exact(path: &Path, bytes: &[u8]) -> Result<(), SaveError>`: temp file, `sync_all`, rename; no trailing newline, no `.bak`; the temp file is removed on failure.
  - `Tree::graft(&mut self, parent: Option<NodeId>, node: Node) -> Result<NodeId, TreeError>`; new `TreeError::DuplicateId(NodeId)` (code `duplicate_id`) when any id of the subtree already exists in the tree or twice inside the subtree.
  - `Store::graft(&mut self, parent: Option<NodeId>, node: Node) -> Result<NodeId, StoreError>` and `Store::replace_tree(&mut self, tree: Tree) -> Result<(), StoreError>`, both through `mutate`-style save-then-swap so memory never diverges from disk.

- [ ] **Step 1: Write failing tests** (in the three files' `tests` modules): `persist::tests::document_round_trips_through_parse_document`; `parse_document_refuses_newer_schema_with_found_and_supported`; `parse_document_gives_repeated_ids_fresh_ids` (existing `sanitize` behaviour); `write_exact_has_no_newline_no_bak_and_no_temp_file`; `tree::tests::graft_adds_subtree_at_end_of_parent`, `graft_into_a_connection_is_not_a_folder`, `graft_with_an_existing_id_is_duplicate_id`, `graft_with_a_repeated_id_inside_is_duplicate_id`; `store::tests::graft_is_persisted`, `failed_save_rolls_back_graft`, `replace_tree_is_persisted_and_keeps_bak`, `failed_save_rolls_back_replace_tree`.
- [ ] **Step 2: Run** `cargo test -p fernjo-core` — expect the new tests to fail to compile/fail.
- [ ] **Step 3: Implement** the functions above; `read_file` keeps its current behaviour and error mapping (`NewerSchema` → `LoadError::NewerSchema`).
- [ ] **Step 4: Run** `cargo test -p fernjo-core` — all pass (the existing 98+ tests unchanged); `cargo clippy -p fernjo-core --all-targets -- -D warnings`.
- [ ] **Step 5: Commit** `feat(core): document parsing, subtree graft and tree replace for import` — files: the five above.

### Task 2: Vault primitives (sealed blob, add with id, export and replace entries)

**Files:**
- Create: `crates/fernjo-vault/src/sealed.rs`
- Modify: `crates/fernjo-vault/src/vault.rs`, `error.rs`, `lib.rs`, `entry_tests.rs` (or a new `sealed_tests.rs`)

**Interfaces:**
- Produces:
  - `pub fn seal_with_password(magic: &[u8; 8], version: u16, password: &SecretString, kdf: KdfParams, plaintext: &[u8]) -> Result<Vec<u8>, VaultError>` — layout `magic ‖ version u16 BE ‖ memory u32 ‖ iterations u32 ‖ parallelism u32 ‖ salt[16] ‖ nonce[24] ‖ ciphertext+tag`, the whole header as associated data, password NFC-normalised exactly like the master password (reuse that function).
  - `pub fn open_with_password(magic: &[u8; 8], max_version: u16, password: &SecretString, bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>, SealedError>`; `pub enum SealedError { NotThisFormat, NewerVersion { found: u16 }, Damaged, WrongPassword }` (`Damaged` = too short or KDF parameters outside the vault's load bounds; authentication failure = `WrongPassword`).
  - `Vault::add_with_id(&mut self, id: EntryId, new: NewEntry) -> Result<(), VaultError>` (new error `DuplicateEntry`, code `duplicate_entry`), `Vault::export_entries(&self) -> Result<Vec<(EntryId, NewEntry)>, VaultError>` (unlocked only), `Vault::replace_entries(&mut self, entries: Vec<(EntryId, NewEntry)>) -> Result<(), VaultError>` (unlocked only, ids unique, one `save`, memory unchanged if the save fails).
  - Re-exports `seal_with_password`, `open_with_password`, `SealedError` from `lib.rs`.
- Consumes: `KdfParams`, `derive_key`, `seal`, `open`, the vault's save path and password normalisation.

- [ ] **Step 1: Failing tests:** `sealed::tests::round_trip`, `wrong_password_is_wrong_password`, `flipping_any_header_byte_is_rejected`, `truncated_blob_is_damaged`, `other_magic_is_not_this_format`, `newer_version_is_reported_with_found`, `kdf_parameters_out_of_bounds_are_damaged_without_deriving`, `password_is_nfc_normalised` (seal with composed "é", open with decomposed "e\u{301}" succeeds); `vault::tests::add_with_id_keeps_the_id_and_rejects_a_duplicate`, `export_entries_requires_unlocked`, `replace_entries_swaps_all_in_one_save_and_keeps_bak`, `replace_entries_with_duplicate_ids_changes_nothing`, `failed_save_in_replace_entries_keeps_old_entries` (reuse the existing failing-save technique of `failed_save_during_change_password_keeps_the_old_password_working`).
- [ ] **Step 2: Run** `cargo test -p fernjo-vault` — fail.
- [ ] **Step 3: Implement** per the signatures; `sealed.rs` uses the existing `crypto` helpers (no new crypto dependency).
- [ ] **Step 4: Run** `cargo test -p fernjo-vault` (existing 43 + 2 ignored stay green) and clippy for the crate.
- [ ] **Step 5: Commit** `feat(vault): password-sealed blobs and entry export/replace for backup` — the files above.

### Task 3: `fernjo-import` crate and the mRemoteNG decryptor, with independent vectors

**Files:**
- Create: `crates/fernjo-import/Cargo.toml`, `src/lib.rs`, `src/error.rs`, `src/mremoteng/mod.rs`, `src/mremoteng/crypto.rs`, `tools/mremoteng-vectors.cs`, `crates/fernjo-import/tests/fixtures/vectors.json`
- Modify: root `Cargo.toml` (workspace deps `roxmltree`, `aes-gcm`, `pbkdf2`, `sha1`, `base64`; versions chosen at implementation time and recorded in `Docs/DECISIONS.md` in Task 12), `deny.toml` only if `cargo deny check` reports a licence

**Interfaces:**
- Produces: `fernjo_import::error::ImportError { KeyNeeded, WrongKey, UnsupportedFormat(String), TooLarge, TooComplex, Damaged(String) }` with `code() -> &'static str` (`key_needed`, `wrong_key`, `unsupported_format`, `too_large`, `too_complex`, `damaged`); `mremoteng::crypto::decrypt(secret_b64: &str, key: &SecretString, iterations: u32) -> Result<Zeroizing<String>, CryptoError>` (`CryptoError::{BadBase64, TooShort, Authentication}`); `mremoteng::crypto::DEFAULT_KEY: &str = "mR3m"`; `MAX_ITERATIONS: u32 = 1_000_000`, `MIN_ITERATIONS: u32 = 1000`.

- [ ] **Step 1: Write `tools/mremoteng-vectors.cs`** (`#:package BouncyCastle.Cryptography@<pinned>`, same style as `tools/roadmap.cs`): a port of mRemoteNG's `AeadCryptographyProvider` encrypt (`Pkcs5S2KeyGenerator` over UTF-8 bytes, `GcmBlockCipher(AesEngine)`, 16-byte nonce, salt as AAD) that writes `vectors.json`: an array of `{ "password", "iterations", "plaintext", "secret" }` for the default key with 1000 iterations, a custom key with 5000 iterations, a non-ASCII password and plaintext, and a 2000-byte plaintext. Run `dotnet run tools/mremoteng-vectors.cs -- crates/fernjo-import/tests/fixtures` and commit the output (CI never needs dotnet).
- [ ] **Step 2: Failing tests** in `mremoteng/crypto.rs`: `decrypts_every_independent_vector` (loads `vectors.json`, asserts each plaintext), `wrong_key_is_authentication_error`, `flipped_ciphertext_byte_is_authentication_error`, `too_short_input_is_too_short`, `bad_base64_is_bad_base64`, `iterations_below_1000_or_above_1_000_000_are_refused_before_deriving`.
- [ ] **Step 3: Implement** the crate skeleton (workspace member via `crates/*` glob) and `decrypt`: `base64` decode, split salt/nonce/rest, PBKDF2-HMAC-SHA1 into a `Zeroizing<[u8; 32]>`, `AesGcm<Aes256, U16>` with `Payload { msg, aad: salt }`, UTF-8 plaintext in `Zeroizing<String>`.
- [ ] **Step 4: Run** `cargo test -p fernjo-import`, `cargo clippy -p fernjo-import --all-targets -- -D warnings`, `cargo deny check` (new crates pass the allow-list).
- [ ] **Step 5: Commit** `feat(import): mRemoteNG AES-GCM decryption checked against independent vectors` — the files above plus `Cargo.lock`.

### Task 4: Reading `confCons.xml` (raw document, limits, key handling, full-file encryption)

**Files:**
- Create: `crates/fernjo-import/src/mremoteng/file.rs`, `tests/read.rs`, fixtures `tests/fixtures/{default-key,custom-key,full-file}.xml` and `*.expected.json` (generated by extending `tools/mremoteng-vectors.cs` with a writer that emits files with the real 2.6 attribute set and validates each against `mremoteng_confcons_v2_6.xsd` when `--xsd <path>` is passed; hand-written small files for the unsupported cases: `legacy-2.5.xml`, `v2_7.xml`, `not-connections.xml`)
- Modify: `src/mremoteng/mod.rs`

**Interfaces:**
- Produces:
  - `mremoteng::read(bytes: &[u8], key: Option<&SecretString>) -> Result<RawDocument, ImportError>`: size check, BOM strip, `roxmltree` parse (DTD not expanded beyond roxmltree's limits), root `Connections` check, `ConfVersion` exactly 2.6, `EncryptionEngine`/`BlockCipherMode` check, key check through `Protected` == `ThisIsNotProtected` (no key given: try `mR3m`, failure → `KeyNeeded`; key given, failure → `WrongKey`), full-file decryption, then a depth-first walk with the limits.
  - `RawDocument { nodes: Vec<RawNode> }`, `RawNode { kind: RawKind, name: String, description: String, hostname: String, protocol: String, port: Option<u32>, username: String, domain: String, password: Option<Zeroizing<String>>, password_error: bool, inherit: Inherit, putty_session: String, children: Vec<RawNode> }`, `Inherit { username, password, domain, port: bool }`, `RawKind::{Container, Connection}`. A missing optional attribute reads as empty/false; an undecryptable `Password` sets `password_error` and leaves `password` `None`. `Debug` of `RawNode` prints no password.

- [ ] **Step 1: Failing tests** (`tests/read.rs`): `default_key_file_reads_names_hosts_and_passwords`, `custom_key_file_needs_the_key` (`KeyNeeded` without, `WrongKey` with a wrong one, success with the right one), `full_file_encryption_reads_the_same_tree`, `undecryptable_password_sets_password_error_and_keeps_going`, `legacy_and_prerelease_versions_are_unsupported_with_their_version_in_the_message`, `non_aes_gcm_engine_is_unsupported`, `xml_that_is_not_connections_is_unsupported`, `a_json_or_binary_file_is_unsupported_not_a_panic`, `file_over_32_mib_is_too_large`, `depth_65_is_too_complex_and_depth_64_is_fine`, `more_than_100_000_nodes_is_too_complex`, `missing_attributes_read_as_defaults`, `raw_node_debug_hides_the_password`.
- [ ] **Step 2: Run** `cargo test -p fernjo-import --test read` — fail.
- [ ] **Step 3: Implement** `read` per the interface; the walk is iterative or depth-limited so a 64-level file cannot overflow the stack.
- [ ] **Step 4: Run** the crate's tests, clippy, `cargo fmt --all --check`.
- [ ] **Step 5: Commit** `feat(import): read mRemoteNG 2.6 files with key handling and limits` — the files above.

### Task 5: Mapping to an import plan and report

**Files:**
- Create: `crates/fernjo-import/src/report.rs`, `src/mremoteng/map.rs`, `tests/map.rs`
- Modify: `src/lib.rs`, `src/mremoteng/mod.rs`

**Interfaces:**
- Consumes: `RawDocument` (Task 4), `fernjo_core::{Node, NodeKind, Protocol, Settings, Tree, NodeId, CredentialId}`, `fernjo_vault::{NewEntry, Secret, EntryId}`.
- Produces:
  - `pub struct ImportPlan { pub folder: Node, pub entries: Vec<PlannedEntry>, pub report: Report }`, `pub struct PlannedEntry { pub id: EntryId, pub entry: NewEntry }` (the tree's `settings.credential` values equal these ids).
  - `pub struct Report { pub counts: Counts, pub skipped: Vec<ReportLine>, pub partial: Vec<ReportLine>, pub problems: Vec<ReportLine> }`, `Counts { folders, ssh, rdp, credentials, shared_credentials, skipped, partial, problems: usize }`, `ReportLine { path: String, text: String }`; every list capped at 200 lines, `Counts` always exact, a final line `and N more` when capped; `Report` is `Serialize` (camelCase) and contains no secret.
  - `mremoteng::plan(doc: &RawDocument, today: &str, taken_names: &[String]) -> ImportPlan` (`today` is `YYYY-MM-DD`, injected so tests are deterministic; `taken_names` are the existing top-level names for the ` (2)` suffix).

- [ ] **Step 1: Failing tests** (`tests/map.rs`, building `RawDocument`s directly plus one test through the default-key fixture):
  - structure: `containers_become_folders_and_ssh2_rdp_become_connections`, `document_order_is_kept`, `folder_is_kept_when_every_child_is_skipped`, `second_import_same_day_gets_a_numbered_folder`, `fresh_ids_are_unique`;
  - skipped: `vnc_telnet_ssh1_http_ica_are_skipped_and_listed_with_path_and_protocol`;
  - settings: `inherit_flag_true_leaves_port_and_username_unset`, `inherit_flag_false_sets_local_values`, `port_zero_or_above_65535_is_unset_and_reported_as_a_problem`, `empty_name_becomes_unnamed_and_is_reported`, `connection_without_host_is_kept_and_reported`;
  - credentials: `own_password_creates_an_entry_referenced_by_the_node`, `identical_username_domain_password_share_one_entry`, `entry_takes_username_and_domain_only_when_their_flags_are_false`, `inherited_password_creates_nothing`, `password_own_username_inherited_gives_entry_without_username_and_no_node_username`, `entry_names_are_username_or_user_at_domain_with_numbering`, `undecryptable_password_is_reported_and_the_node_imports_without_one`;
  - report: `domain_without_password_is_partly_mapped`, `putty_session_other_than_default_is_partly_mapped`, `rdp_display_settings_are_one_aggregate_line`, `empty_own_password_under_folder_with_credential_is_reported` (naming the folder it now inherits from; same for username), `lists_are_capped_at_200_with_exact_counts`, `report_serialises_without_any_password` (sentinel in the plan's entries, absent in `serde_json::to_string(&plan.report)`).
- [ ] **Step 2: Run** `cargo test -p fernjo-import --test map` — fail.
- [ ] **Step 3: Implement** `plan` following spec §4.2: faithful flag mapping per setting; entries deduplicated by (entry username, domain, password); after building the tree, run `Tree::from_children(vec![folder.clone()]).resolve_node` for nodes with an own empty password or username to produce the "inherits" lines.
- [ ] **Step 4: Run** the crate's tests and clippy.
- [ ] **Step 5: Commit** `feat(import): map mRemoteNG nodes to a plan with entries and report` — the files above.

### Task 6: fernjo connections JSON export and import

**Files:**
- Create: `crates/fernjo-import/src/connections.rs`, `tests/connections.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `fernjo_core::persist::{document, parse_document}`, `Report` (Task 5).
- Produces: `connections::export(tree: &Tree, exported_at: &str) -> ConnectionsExport { bytes: Vec<u8>, dropped_credentials: usize }` (pretty JSON; `document` plus `format` and `exportedAt`; every `settings.credential` removed); `connections::plan(bytes: &[u8], today: &str, taken_names: &[String]) -> Result<ConnectionsPlan, ConnectionsError>` with `ConnectionsPlan { folder: Node, report: Report }` (fresh ids on every node, credentials stripped and counted as a `partial` line, folder name `Imported from fernjo export (YYYY-MM-DD)` numbered like Task 5); `ConnectionsError::{NotThisFormat, NewerSchema { found, supported }, Invalid(String)}` with `code()`.

- [ ] **Step 1: Failing tests:** `export_contains_no_credential_and_counts_the_dropped`, `export_round_trips_through_plan_with_fresh_ids_and_same_shape`, `plan_refuses_a_newer_schema`, `plan_refuses_json_without_the_format_marker` (a plain `connections.json` is `NotThisFormat`), `plan_refuses_a_confcons_xml_and_a_backup_as_not_this_format`, `plan_reports_invalid_ports_through_the_core_rules`.
- [ ] **Step 2: Run** — fail. **Step 3: Implement.** **Step 4: Run** tests, clippy.
- [ ] **Step 5: Commit** `feat(import): export and import connections as fernjo JSON`.

### Task 7: Encrypted backup container

**Files:**
- Create: `crates/fernjo-import/src/backup.rs`, `tests/backup.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `fernjo_vault::{seal_with_password, open_with_password, SealedError, KdfParams, NewEntry, Secret, EntryId}`, `fernjo_core::persist::{document, parse_document}`.
- Produces: `backup::MAGIC: &[u8; 8] = b"FERNJOBK"`, `backup::VERSION: u16 = 1`; `BackupContents { created_at: String, app: String, tree: Tree, entries: Vec<(EntryId, NewEntry)> }`; `backup::create(contents: &BackupContents, password: &SecretString, kdf: KdfParams) -> Result<Vec<u8>, BackupError>`; `backup::open(bytes: &[u8], password: &SecretString) -> Result<BackupContents, BackupError>`; `BackupError::{PasswordTooShort, NotABackup, NewerVersion { found: u16 }, WrongPassword, Damaged(String)}` with `code()` (`password_too_short`, `not_a_backup`, `newer_version`, `wrong_password`, `damaged`). Body JSON is `{ schemaVersion: 1, createdAt, app, connections: <document>, entries: [{ id, name, username?, domain?, secret, notes }] }` (a private serde struct using the vault's `Secret` serde).

- [ ] **Step 1: Failing tests:** `round_trip_gives_identical_tree_and_entries` (nested tree with credentials, two entries including a key with passphrase, compare tree equality and every entry field incl. secrets), `password_shorter_than_8_chars_is_refused_before_deriving`, `wrong_password_is_wrong_password`, `damaged_body_json_is_damaged` (seal valid ciphertext containing invalid JSON), `truncated_file_is_damaged`, `a_vault_file_or_connections_json_is_not_a_backup`, `newer_version_is_reported`, `hostile_kdf_parameters_are_damaged`, `entries_reference_ids_that_exist_in_the_tree_are_kept_verbatim`, `debug_of_contents_hides_secrets`.
- [ ] **Step 2: Run** — fail. **Step 3: Implement.** **Step 4: Run** tests, clippy.
- [ ] **Step 5: Commit** `feat(import): encrypted full backup container`.

### Task 8: Pending slot, inspect, apply, export (plain functions over `AppState`)

**Files:**
- Create: `src-tauri/src/import_commands.rs` (plain functions and types only in this task; wrappers in Task 9)
- Modify: `src-tauri/src/state.rs` (`pub pending: Mutex<Option<Pending>>`, initialised `None` in both constructors), `src-tauri/src/lib.rs` (`mod import_commands;`), `src-tauri/Cargo.toml` (`fernjo-import` dependency)

**Interfaces:**
- Consumes: Tasks 1–7; `commands::{CommandError, Mutation, Snapshot, snapshot, ensure_writable, with_store}`, `vault_commands::with_vault` and the vault status view, `state.sessions.close_all()`. Lock order is vault first, then store (as `delete_credential`).
- Produces:
  - `#[derive(Deserialize)] #[serde(rename_all = "lowercase")] enum ImportKind { Mremoteng, Connections, Backup }`.
  - `enum Pending { Picked { kind: ImportKind, path: PathBuf }, MRemoteNg(ImportPlan), Connections(ConnectionsPlan), Backup(BackupContents) }`.
  - `fn import_begin(state: &AppState, kind: ImportKind, path: PathBuf) -> PickedFile` (`{ file_name }`, replaces any pending value).
  - `fn import_inspect(state: &AppState, secret: Option<String>) -> Result<Preview, CommandError>`; `Preview { kind, file_name, report: Option<Report>, backup: Option<BackupSummary { created_at, app, folders, connections, credentials }>, needs_vault: bool, vault: VaultStatus }`. Reads the file (≤ 32 MiB for confCons, ≤ 64 MiB otherwise), wraps `secret` in `SecretString`, maps `ImportError`/`ConnectionsError`/`BackupError` to `CommandError` with their codes; a key/password error leaves `Picked` in place so the UI can retry; success replaces it with the plan.
  - `fn import_apply(state: &AppState, options: ApplyOptions { without_passwords: bool }) -> Result<ApplyResult, CommandError>`; `ApplyResult { snapshot: Snapshot, report: Option<Report>, restored: Option<RestoreSummary> }`. Import with entries needs an unlocked vault, else `vault_needed` (message names the vault state) with the plan kept; `without_passwords` strips entries and every credential reference of the plan and adds the count to the report. Order: `add_with_id` each entry, then `Store::graft` at top level (end); on graft failure delete the entries just added. Restore: `vault_needed` when `entries` is non-empty and the vault is not unlocked, `vault_unusable` refused, then `sessions.close_all()`, snapshot via `export_entries`, `replace_entries`, `Store::replace_tree`, on tree failure `replace_entries(snapshot)`. Always `ensure_writable` first; the pending value is cleared only on success.
  - `fn import_cancel(state: &AppState)`; `fn export_connections_to(state: &AppState, path: &Path) -> Result<ExportInfo { file_name, dropped_credentials }, CommandError>` (today's date in the file name is the handler's job; this takes the final path); `fn export_backup_to(state: &AppState, path: &Path, password: String) -> Result<ExportInfo, CommandError>` (vault `none` → no entries; `locked` → `vault_locked`; `unusable` → `vault_unusable`; written with `persist::write_exact`).

- [ ] **Step 1: Failing tests** (`import_commands::tests`, using `AppState::open` with `KdfParams::TEST` and fixtures from `fernjo-import`): `inspect_default_key_file_returns_report_and_no_secret`, `inspect_custom_key_file_asks_for_the_key_then_accepts_it`, `wrong_kind_of_file_is_reported` (a backup picked as mRemoteNG gives `unsupported_format`, and the reverse gives `not_a_backup`), `apply_import_adds_folder_entries_and_returns_snapshot`, `apply_import_without_unlocked_vault_is_vault_needed_and_plan_survives`, `apply_without_passwords_drops_entries_and_references`, `second_import_same_day_gets_a_numbered_folder`, `import_rolls_entries_back_when_graft_fails` (make the store's save fail as in `failed_save_rolls_back_memory`), `apply_clears_pending_and_a_second_apply_is_no_pending_import`, `cancel_clears_pending`, `connections_json_import_merges_into_new_folder_without_touching_vault`, `restore_replaces_tree_and_entries_exactly`, `restore_closes_sessions`, `restore_with_entries_needs_unlocked_vault`, `restore_refuses_an_unusable_vault`, `restore_puts_entries_back_when_tree_replace_fails`, `restore_of_backup_without_entries_clears_entries_and_summary_says_zero`, `export_backup_round_trips_through_inspect_and_apply`, `export_backup_needs_unlocked_vault_when_one_exists`, `export_connections_drops_credentials_and_reports_the_count`, `import_apply_honours_restart_required` (`ensure_writable` refuses with `restart_required`, the plan stays pending).
- [ ] **Step 2: Run** `cargo test -p fernjo-app import_commands` — fail.
- [ ] **Step 3: Implement** per the interfaces; no `tauri` types in this file except in Task 9's wrappers.
- [ ] **Step 4: Run** `cargo test -p fernjo-app`, clippy.
- [ ] **Step 5: Commit** `feat(app): inspect and apply imports, restore and exports with rollback`.

### Task 9: Tauri commands, native dialogs, page-load clearing, no-secret test

**Files:**
- Modify: `src-tauri/src/import_commands.rs` (a `handlers` module), `src-tauri/src/lib.rs` (register, clear `pending` in `on_page_load` next to `close_all`), `src-tauri/src/vault_commands.rs` (test only: `table()`, `APP_HANDLE_EXEMPT`, `NOT_A_COMMAND`, `sources`)

**Interfaces:**
- Produces commands: `import_pick(app, state, kind) -> Result<Option<PickedFile>, CommandError>` (`spawn_blocking` + `DialogExt::file().blocking_pick_file()` with a filter per kind: `xml`, `json`, `fjbk`; debug builds read `FERNJO_DIALOG_FILE` instead of showing a dialog), `import_inspect(state, secret: Option<String>) -> Preview` (`#[tauri::command(async)]`), `import_apply(state, options) -> ApplyResult`, `import_cancel(state)`, `export_connections(app, state) -> Result<Option<ExportInfo>, CommandError>` and `export_backup(app, state, password: String) -> Result<Option<ExportInfo>, CommandError>` (`async`; `blocking_save_file` with the default file names; `FERNJO_DIALOG_FILE` in debug builds). Capabilities stay `["core:default"]`.
- `table()` gets entries for `import_inspect`, `import_apply`, `import_cancel`; `APP_HANDLE_EXEMPT` gets `import_pick`, `export_connections`, `export_backup` (reason: native dialog needs an `AppHandle`); `NOT_A_COMMAND` gets `import_begin`, `export_connections_to`, `export_backup_to`; `sources` gets `include_str!("import_commands.rs")`.

- [ ] **Step 1: Failing tests:** extend `no_response_contains_a_secret` so its sentinel secrets also sit in a synthetic mRemoteNG file (passwords), a custom file key, an export password and a backup's entries, run inspect → apply → export_backup_to → inspect(backup) → apply, and assert no sentinel appears in any response, error, `Debug` output or `connections.json`; `command_table_covers_every_tauri_command` must fail until the six commands are registered; `page_load_clears_pending` (the clearing is a function `AppState::clear_pending()` called from `on_page_load`; test the function).
- [ ] **Step 2: Run** `cargo test -p fernjo-app` — fail.
- [ ] **Step 3: Implement** the handlers, registration and `clear_pending`.
- [ ] **Step 4: Run** `cargo test --workspace` (no SSH variables needed: those tests skip locally), `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] **Step 5: Commit** `feat(app): import and export commands with native file dialogs`.

### Task 10: UI api, types, mock and `ImportStore`

**Files:**
- Create: `ui/src/lib/import.svelte.ts`, `ui/src/lib/import.test.ts`
- Modify: `ui/src/lib/types.ts` (types `ImportKind`, `PickedFile`, `ReportLine`, `Report`, `Counts`, `BackupSummary`, `ImportPreview`, `ApplyResult`, `ExportInfo`, and the six `Api` methods `importPick(kind)`, `importInspect(secret)`, `importApply(options)`, `importCancel()`, `exportConnections()`, `exportBackup(password)`), `ui/src/lib/api.ts`, `ui/src/lib/testing.ts` (`mockApi` defaults and factories `preview()`, `report()`)

**Interfaces:**
- Produces `class ImportStore` (`$state`): `step: "idle" | "key" | "preview" | "vault" | "applying" | "done" | "error"`, `kind`, `fileName`, `preview`, `result`, `error: ApiError | null`; methods `start(kind)` (pick, inspect without secret; code `key_needed`/`wrong_password` → step `key`), `submitSecret(secret)` (wrong secret keeps the step and sets `error`), `apply({ withoutPasswords })` (code `vault_needed` → step `vault`; the dialog then uses the existing `VaultStore` and calls `apply` again), `cancel()`, `exportConnections()`, `exportBackup(password)`, `reset()`. It never validates anything itself and never keeps a secret after sending it. The tree snapshot of `ApplyResult` is handed to the existing `TreeStore` through a callback given to the constructor.

- [ ] **Step 1: Failing tests** (`import.test.ts`, `mockApi`): `start_with_cancelled_dialog_stays_idle`, `default_key_file_goes_straight_to_preview`, `key_needed_then_wrong_key_keeps_key_step_and_shows_the_error_then_right_key_goes_to_preview`, `backup_asks_for_its_password`, `apply_vault_needed_goes_to_vault_step_and_retry_applies`, `apply_success_hands_the_snapshot_to_the_tree_and_ends_in_done`, `cancel_calls_import_cancel_and_resets`, `export_backup_passes_the_password_once_and_clears_it`, `an_api_error_ends_in_error_with_its_message`.
- [ ] **Step 2: Run** `pnpm test -- import` — fail. **Step 3: Implement.** **Step 4: Run** `pnpm test` and `pnpm lint`.
- [ ] **Step 5: Commit** `feat(ui): import store and api types`.

### Task 11: UI dialog, sidebar button, empty-tree offer

**Files:**
- Create: `ui/src/lib/components/ImportDialog.svelte`, `ImportReport.svelte`, `ImportDialog.test.ts`
- Modify: `ui/src/lib/components/Sidebar.svelte` (second footer button next to the gear, prop `onimport`), `TreeView.svelte` (empty-tree offer gets "Import from mRemoteNG…" through a prop `onimport`), `Icon.svelte` (an `import` icon if no existing one fits), `ui/src/App.svelte` (state `importOpen`, the dialog, `onimport`, modal key handling like `folderOpen`), `DESIGN.md` only if a new component pattern appears

**Interfaces:**
- Consumes: `ImportStore` (Task 10), `VaultStore` (unlock/create inline, as `SessionPrompt`/`NeedsVaultPrompt` do), `PasswordField`, `ConfirmDialog`, modelled on `DataFolderDialog` (focus trap, Escape, `aria-modal="true"`).
- Produces: dialog "Import & backup" with three groups (Import: mRemoteNG file, fernjo connections file; Export: connections JSON, full backup; Restore: backup file). Steps inline: pick, key or password field (inline error, field kept, show/hide), preview (counts plus collapsible Skipped, Partly mapped, Problems lists from `ImportReport`), confirm, result (same report). Restore confirmation text: "This replaces N folders and connections and M credentials with the backup from <date> (K and L). Open sessions are closed." with Cancel focused and a non-yellow confirm. Backup export asks the password twice and disables the button until both match and have at least 8 characters (Rust enforces it too).

- [ ] **Step 1: Failing tests** (`ImportDialog.test.ts`): `dialog_lists_the_six_actions`, `mremoteng_flow_shows_preview_counts_then_result`, `custom_key_prompt_keeps_the_field_and_shows_the_wrong_key_message`, `skipped_and_partial_lists_are_collapsed_until_opened`, `restore_confirmation_names_counts_and_has_cancel_focused`, `export_backup_needs_matching_passwords_of_8_chars`, `vault_needed_step_offers_unlock_and_resumes_the_apply`, `escape_cancels_a_pending_import_via_import_cancel`, `empty_tree_offer_opens_the_dialog`, `focus_returns_to_the_opener_on_close`.
- [ ] **Step 2: Run** `pnpm test` — fail. **Step 3: Implement** with tokens only; check the styling rules with `pnpm test` (`tokens.test.ts` guards them). **Step 4: Run** `pnpm lint`, `pnpm test`, `pnpm ui:build`.
- [ ] **Step 5: Commit** `feat(ui): import and backup dialog`.

### Task 12: Documentation and decisions

**Files:**
- Create: `Docs/backup-format.md` (the `FJBK` byte layout table like `Docs/vault-format.md`, the body JSON, the connections JSON, bounds, error codes)
- Modify: `Docs/DECISIONS.md` (three entries on top: "M5 dependencies" with the chosen versions; "M5 mRemoteNG format facts and mapping" with the verified source facts of the spec §4.1 and the mapping table, the 2.6-only decision and why 2.7/2.8 and CBC are out; "M5 restore semantics and backup container"), `Docs/vault-format.md` (pointer), `CLAUDE.md` ("Project-specific": the pending slot, the six commands and the `NOT_A_COMMAND` additions, `FERNJO_DIALOG_FILE`, `tools/mremoteng-vectors.cs` and how to regenerate fixtures, the rule that mRemoteNG's GPL source is read, never copied), `README.md` (feature list: import, export, backup), `CHANGELOG.md` (`[Unreleased]`: "Import connections and passwords from mRemoteNG; export connections as JSON; encrypted full backup and restore")

- [ ] **Step 1: Write the documents** (facts only; no guesses recorded as facts). **Step 2: Run** `pnpm lint` (Prettier covers Markdown if configured) and `dotnet run tools/roadmap.cs -- check`.
- [ ] **Step 3: Commit** `docs: M5 formats, decisions and project notes`.

### Task 13: The owner's fixture (gate for the acceptance criteria)

**Files:**
- Create (when delivered): `crates/fernjo-import/tests/fixtures/owner-confCons.xml`, `owner-confCons.expected.json`, `crates/fernjo-import/tests/owner.rs`

- [ ] **Step 1: Ask the owner for the file** with these instructions: in mRemoteNG 1.76, File → Save Connection File As… into a copy, then in that copy rename hosts, usernames and descriptions to fake values and set throwaway passwords (keep the structure: at least one folder, a nested folder, a connection that inherits username/password from its folder, one with its own, one RDP, one with another protocol such as VNC or Telnet, and the file's real encryption setup: default key or custom key, full-file encryption on or off); send the file plus the key if custom and a list of the fake passwords per connection. Never send the real file.
- [ ] **Step 2: First check, before anything else:** `ConfVersion` in the file's first line. If it is not `2.6`, **stop**: record a scope change in `Docs/roadmap.json` (support that version, or move it to the backlog) and tell the owner before building anything further.
- [ ] **Step 3: Failing test** `owner_fixture_imports_with_the_expected_tree_and_passwords`: reads the file with its key, plans it, asserts the folder/connection counts, skipped list, every connection's host, port, username and the entry set (name, username, domain and password) from `owner-confCons.expected.json`.
- [ ] **Step 4: Run** `cargo test -p fernjo-import --test owner` — fix the mapping only through a new failing synthetic test first if the real file shows a case the synthetic ones missed.
- [ ] **Step 5: Commit** `test(import): owner fixture`.

### Task 14: Real-app checks, roadmap and the gate

**Files:**
- Modify: `Docs/roadmap.json`, `Docs/ROADMAP.md` (generated)

- [ ] **Step 1: Real-app flows on Windows** with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`, a scratch `FERNJO_DATA_DIR`, a debug build and `FERNJO_DIALOG_FILE` pointing at a fixture, driven over the DevTools protocol (no OS mouse or keyboard): import the default-key fixture (preview counts, skipped list, folder appears, passwords in the vault via the vault manager, `grep` of the data folder for the sentinel passwords finds nothing outside `vault.fernjo`), custom key (wrong key message, then success), vault locked then unlock then apply, "Import without passwords", export connections, export backup, restore into a second scratch folder and compare `connections.json` and the vault entries, light and dark screenshots, 300 px sidebar footer. Record the results as evidence. The native dialog itself cannot be driven this way: OPEN for the owner (`pnpm tauri dev` run once without `FERNJO_DATA_DIR` and with `APPDATA` pointed at a scratch folder; use each of the six actions through the real dialogs).
- [ ] **Step 2: Roadmap:** link the plan, status `in-progress` while working, tick tasks as they finish, tick acceptance criteria only with evidence; add a "Scope changes" entry (M5 supports ConfVersion 2.6 only; legacy CBC stays in the backlog; 2.7/2.8 unsupported). `dotnet run tools/roadmap.cs -- generate` then `-- check`.
- [ ] **Step 3: The gate**, all green with output quoted as evidence: `pnpm install --frozen-lockfile`; `pnpm ui:build`; `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace` (with `eval "$(tools/ssh-test-server.sh start)"` before and `tools/ssh-test-server.sh stop` after, as in M4); `pnpm lint`; `pnpm test`; `cargo deny check`; `pnpm tauri build --no-bundle` (release binary starts, CSP unchanged, no console message); `dotnet run tools/roadmap.cs -- check`.
- [ ] **Step 4: Open items stay open:** the owner's real file and five spot-checked connections, Linux and macOS (add to "Open manual checks (owner)").
- [ ] **Step 5: Commit** `docs: milestone 5 done – evidence` and open the PR to `dev` (Claude merges once all required checks are green; CodeQL test-password alerts are the owner's to dismiss).
