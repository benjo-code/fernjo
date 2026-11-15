# Milestone 3 – Credential vault Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Passwords and SSH keys live in an encrypted vault (master password or OS keychain), can be attached to connections and folders through inheritance, and the data folder is selectable.

**Architecture:** A new `fernjo-vault` crate (no Tauri, no `fernjo-core` dependency) owns the file format, crypto, entries and the keychain trait. `fernjo-core` gains the credential reference (an id), `connections.json` schema 2, `settings.json`, data-folder resolution and the sync-conflict scan. `src-tauri` holds the vault state and the commands; the UI gets a vault store, chip and lock note, a manager view, a credential row and a data-folder dialog.

**Tech Stack:** Rust 1.100 (edition 2024), `argon2`, `chacha20poly1305` (XChaCha20-Poly1305), `secrecy`, `zeroize`, `unicode-normalization`, `keyring` 4.x (Secret Service over `zbus` on Linux), `tauri-plugin-dialog` 2.x (Rust API only), Svelte 5 + TypeScript, Vitest.

**Spec:** [2026-11-15-m03-credential-vault-design.md](../specs/2026-11-15-m03-credential-vault-design.md) (parent: [2026-11-15-fernjo-design.md](../specs/2026-11-15-fernjo-design.md) §4, §5.2, §5.4). Read both; §10 of the M3 spec lists the settled calls.

## Global Constraints

- Secrets stay in Rust: a secret never reaches the webview except through `reveal_credential`, and is never written to `connections.json`, `settings.json`, logs or webview storage. Secret types use `secrecy`/`zeroize`; their `Debug` output is redacted.
- No home-made crypto: Argon2id + XChaCha20-Poly1305 from RustCrypto crates; random 256-bit data key wrapped by the password-derived key; the body is encrypted with the data key; `change_password` re-wraps the data key and re-saves (the body is re-sealed under the same data key because the whole header is its associated data; the data key and entries stay unchanged).
- Files in the data folder are written atomically (temp file + `sync_all` + copy to `.bak` + rename). A newer format version is refused and never overwritten.
- `fernjo-vault` depends on neither Tauri nor `fernjo-core`. Capabilities stay `["core:default"]` and the CSP is unchanged (`src-tauri/tests/tauri_conf.rs` guards both).
- Tauri commands are plain functions over `AppState` (unit-tested) with thin `#[tauri::command]` wrappers in `commands::handlers`; every mutating tree command returns the full snapshot. TypeScript never derives validity or inherited values.
- UI uses only tokens from `ui/src/styles/tokens.css` and follows `DESIGN.md` (vault never modal; one yellow action per context; hex-only CSS, no nesting/`:has()`/`color-mix()`); a new colour token goes into the light block and both dark blocks.
- Single-instance plugin stays the first `.plugin(...)` call in `src-tauri/src/lib.rs`.
- Roadmap changes only via `Docs/roadmap.json` + `dotnet run tools/roadmap.cs -- generate`. Docs, commits, code and comments in English; Conventional Commits; stage files selectively; branch `feature/m03-credential-vault`.
- Dependencies and versions go into `Docs/DECISIONS.md`; `cargo deny check` stays green (new ignores need a reason).
- Work only within M3: no sessions, no import, no full settings dialog, no auto-lock.

## Review Focus

Inputs the spec implies but no task would otherwise test, most likely first. Each has a test in the owning task.

1. **Unicode master passwords:** the same password typed as precomposed (Windows) or decomposed (macOS) `é` must unlock the same synced vault → normalise to NFC before the KDF (Task 1).
2. **A zero-length or truncated `vault.fernjo`** (sync placeholder, interrupted copy) must become "unusable" with the path, never a panic and never overwritten (Tasks 1, 6).
3. **Secrets are stored byte-exact:** a private key with CRLF line endings, a password with leading/trailing spaces — never trimmed or re-encoded (Task 2).
4. **Data folder chosen equal to the current one, a file, a nested path or an unwritable folder** must not change settings or enter "restart required" wrongly (Task 8).
5. **Credential references without a usable vault:** deleting or editing an entry while locked fails with `vault_locked` and clears nothing; a dangling reference resolves without error; duplicating a node keeps its credential (Tasks 4, 7).

## File Structure

- Create `crates/fernjo-vault/` (`Cargo.toml`, `src/lib.rs`, `error.rs`, `format.rs` header and (de)serialisation, `crypto.rs` KDF/wrap/seal, `entry.rs`, `vault.rs`, `keystore.rs`); document the format in `Docs/vault-format.md`.
- Modify `crates/fernjo-core/src/`: `model.rs` (credential), `resolve.rs`, `tree.rs`, `store.rs`, `persist.rs` (schema 2, notices), `paths.rs` (resolution), create `settings.rs` and `conflicts.rs`, `error.rs`, `lib.rs`.
- Modify `src-tauri/`: `Cargo.toml`, `src/state.rs`, `src/lib.rs`, `src/commands.rs` (tree commands, notices), create `src/vault_commands.rs` and `src/data_folder.rs`.
- Modify `ui/src/lib/`: `types.ts`, `api.ts`, `testing.ts`, `store.svelte.ts`, create `vault.svelte.ts`; components `VaultChip`, `VaultNote`, `PasswordField`, `VaultManager`, `DataFolderDialog` (new), `PropertiesPanel`, `NoticeBar`, `Sidebar`, `App.svelte` (changed).

---

### Task 1: Vault crate – format, crypto, create/open/unlock/lock

**Files:**
- Create: `crates/fernjo-vault/{Cargo.toml,src/lib.rs,src/error.rs,src/format.rs,src/crypto.rs,src/vault.rs}`, `Docs/vault-format.md`
- Modify: `Cargo.toml` (workspace deps), `Docs/DECISIONS.md`

**Interfaces:**
- Produces (in `fernjo_vault`): `FILE_NAME = "vault.fernjo"`; `VaultId = uuid::Uuid`; `struct DataKey` (32 bytes, zeroised on drop, redacted `Debug`, `from_bytes([u8;32])`, `as_bytes()`); `struct KdfParams { memory_kib: u32, iterations: u32, parallelism: u32 }` with `Default` (tuned to about 0.5 s, constants documented) and `KdfParams::TEST` (minimal, documented insecure, accepted by the load bounds); `enum VaultError` with `code() -> &'static str` for: `wrong_password`, `weak_password`, `vault_locked`, `not_found`, `tampered`, `newer_format`, `unreadable`, `invalid_kdf_params`, `already_exists`, `empty_name`, `save_failed`; `struct Vault` with `create(path:&Path, password:&SecretString) -> Result<Vault,VaultError>` and `create_with(path, password, KdfParams)` (both leave the vault unlocked and write the file; refuse if the file exists), `open(path:&Path) -> Result<Option<Vault>,VaultError>` (`None` if missing; locked), `id()`, `path()`, `is_unlocked()`, `unlock(&mut self,&SecretString)`, `lock(&mut self)`.
- Master password rules: at least 8 characters (`weak_password`), no composition rules; normalised to NFC, otherwise used byte-exact (no trimming).

- [ ] **Step 1: Add the crate and dependencies.** Workspace member via `crates/*` already; add workspace deps (`argon2`, `chacha20poly1305`, `secrecy`, `zeroize`, `unicode-normalization`, a CSPRNG source such as `rand` or `getrandom`) at the versions `cargo add` resolves; record versions and the KDF parameter reasoning in `Docs/DECISIONS.md` (entry "M3 dependencies and vault parameters"). Run `cargo deny check` and add ignores only with a reason.
- [ ] **Step 2: Write failing tests** in `vault.rs`/`format.rs` using `KdfParams::TEST` and `tempfile`:
  - `create_lock_unlock_round_trip` (file exists, `lock()` then `unlock()` with the same password works, `is_unlocked` flips).
  - `wrong_password_is_rejected_with_wrong_password_code`.
  - `short_password_is_rejected_with_weak_password` (7 characters).
  - `nfc_and_nfd_passwords_unlock_the_same_vault` (`"caf\u{e9}-pass"` vs `"cafe\u{301}-pass"`) — Review Focus 1.
  - `flipping_any_byte_of_header_wrapped_key_or_body_is_rejected` (loop over every offset of a small vault; each result is `wrong_password`, `tampered` or `unreadable`, never success, never panic).
  - `empty_and_truncated_files_are_unreadable_not_a_panic` (0 bytes, 5 bytes, header only) — Review Focus 2.
  - `newer_format_version_is_refused_and_file_untouched` (bytes compared before and after).
  - `absurd_kdf_parameters_in_header_are_rejected_before_deriving` (e.g. 64 GiB memory → `invalid_kdf_params`, returns instantly).
  - `create_refuses_an_existing_file`.
  - `save_leaves_no_temp_file_and_keeps_bak` (second save copies the previous file to `vault.fernjo.bak`).
  - `debug_output_never_contains_the_data_key_bytes`.
- [ ] **Step 3: Run `cargo test -p fernjo-vault`** — expected: FAIL (crate does not compile / tests missing).
- [ ] **Step 4: Implement** `format.rs` (magic, `u16` format version, vault id, KDF params and salt, wrap nonce + wrapped key, body nonce, ciphertext; the whole header authenticated as associated data of both AEAD operations; fresh 24-byte nonces on every save), `crypto.rs` (Argon2id → 32-byte key; XChaCha20-Poly1305 seal/open), `vault.rs` per the Interfaces block. Load bounds for the header: memory 8·p KiB … 1 GiB, iterations 1 … 20, parallelism 1 … 16. Writes follow the atomic sequence in Global Constraints. A body that fails authentication after a successful unwrap is `tampered`; a failed unwrap is `wrong_password`. The body is JSON (entries added in Task 2; an empty list for now).
- [ ] **Step 5: Document the byte layout** in `Docs/vault-format.md` (what is authenticated, what is encrypted, version rules, how to read the header by hand).
- [ ] **Step 6: Run** `cargo test -p fernjo-vault`, `cargo clippy -p fernjo-vault --all-targets -- -D warnings`, `cargo fmt --all --check` — expected: all PASS.
- [ ] **Step 7: Commit** `feat(vault): vault file format, Argon2id key wrapping and unlock`.

### Task 2: Vault crate – entries and master password change

**Files:**
- Create: `crates/fernjo-vault/src/entry.rs`
- Modify: `crates/fernjo-vault/src/{vault.rs,lib.rs,error.rs}`

**Interfaces:**
- Consumes: Task 1 `Vault`, `DataKey`, `VaultError`.
- Produces: `EntryId = uuid::Uuid`; `enum Secret { Password(SecretString), PrivateKey { key: SecretString, passphrase: Option<SecretString> } }`; `enum SecretKind { Password, PrivateKey }` (serde `password` / `privateKey`); `struct NewEntry { name: String, username: Option<String>, domain: Option<String>, secret: Secret, notes: String }`; `struct EntryUpdate { name: Option<String>, username: Option<String>, domain: Option<String>, secret: Option<Secret>, notes: Option<String> }` (`None` keeps; for `username`/`domain` an empty string clears; `secret: Some` replaces the whole secret, `None` keeps it); `struct EntrySummary { id, name, kind: SecretKind, username: Option<String>, domain: Option<String>, notes: String }` (`Serialize`, never holds a secret); on `Vault`: `add(NewEntry) -> Result<EntryId,_>`, `update(EntryId, EntryUpdate)`, `delete(EntryId)`, `list() -> Result<Vec<EntrySummary>,_>`, `get(EntryId) -> Result<EntrySummary,_>`, `reveal(EntryId) -> Result<Secret,_>` (a clone), `data_key() -> Result<DataKey,_>`, `unlock_with_key(&mut self,&DataKey)`, `change_password(&mut self, old:&SecretString, new:&SecretString)`. Every mutation saves before returning and leaves memory unchanged if the save fails; every accessor on a locked vault returns `vault_locked`.

- [ ] **Step 1: Write failing tests:**
  - `entries_survive_lock_and_reopen` (password and private-key entries, all fields).
  - `update_without_secret_keeps_the_stored_secret` (reveal equals the original after a rename).
  - `secrets_are_stored_byte_exact` — password `"  pa ss  "` and key with `\r\n` line endings and no trailing newline reveal identically — Review Focus 3.
  - `list_and_get_expose_no_secret` (serialise `EntrySummary` to JSON, assert the sentinel secret is absent).
  - `locked_vault_refuses_every_entry_operation_with_vault_locked`.
  - `delete_unknown_entry_is_not_found`; `empty_name_is_rejected`.
  - `failed_save_leaves_entries_unchanged` (target folder replaced by a file).
  - `change_password_keeps_entries_and_data_key` (old data key still `unlock_with_key`s the file, new password unlocks, old password gets `wrong_password`, the decrypted entries are identical, and the salt, wrap nonce and body nonce are all fresh).
  - `change_password_with_wrong_old_password_changes_nothing`.
- [ ] **Step 2: Run** `cargo test -p fernjo-vault` — expected: FAIL.
- [ ] **Step 3: Implement** the API above. `change_password` verifies the old password, then uses a new salt and wrap nonce, re-wraps the unchanged data key and saves once (the save re-seals the body under the unchanged data key with a fresh nonce; there is no wrap-only path).
- [ ] **Step 4: Run** tests, clippy, fmt — expected: PASS.
- [ ] **Step 5: Commit** `feat(vault): entries, reveal and master password change`.

### Task 3: Vault crate – keychain trait and auto-unlock

**Files:**
- Create: `crates/fernjo-vault/src/keystore.rs`
- Modify: `crates/fernjo-vault/{Cargo.toml,src/lib.rs}`

**Interfaces:**
- Consumes: `DataKey`, `VaultId`, `Vault::{unlock_with_key,data_key,id}`.
- Produces: `trait KeyStore: Send + Sync { fn get(&self, vault:VaultId)->Result<Option<DataKey>,KeyStoreError>; fn set(&self, vault:VaultId, key:&DataKey)->Result<(),KeyStoreError>; fn delete(&self, vault:VaultId)->Result<(),KeyStoreError>; }`; `struct KeyStoreError(String)` (message fit for display); `struct MemoryKeyStore` (tests, `Default`); `struct OsKeyStore` (the `keyring` crate: service `fernjo`, user = the vault id string, secret = the 32 key bytes; Windows Credential Manager, macOS Keychain, Secret Service via `zbus` on Linux, no system C library); `enum AutoUnlock { Unlocked, NoKey, StaleKeyRemoved, Unavailable(String) }`; `fn try_auto_unlock(vault:&mut Vault, store:&dyn KeyStore) -> AutoUnlock`.

- [ ] **Step 1: Write failing tests** (with `MemoryKeyStore`): `auto_unlock_unlocks_when_key_is_stored`; `no_stored_key_stays_locked_with_no_key`; `wrong_stored_key_stays_locked_and_the_item_is_deleted` (→ `StaleKeyRemoved`; password unlock still works afterwards); `store_failure_stays_locked_with_unavailable` (a failing fake); `keychain_key_stays_valid_after_change_password` (set the key, change the password, `try_auto_unlock` still unlocks); `keys_are_per_vault_id` (a second vault does not see the first one's key).
- [ ] **Step 2: Run** `cargo test -p fernjo-vault` — expected: FAIL.
- [ ] **Step 3: Implement** `keystore.rs`. Enable the `keyring` feature set that gives the native stores and the `zbus` Secret Service store and nothing that needs libdbus; check with `cargo tree -p fernjo-vault`. Record the choice and the threat-model statement (spec §10 item 1: with auto-unlock on, anything running as the same OS user can open the vault) in `Docs/DECISIONS.md` (entry "Vault: keychain behaviour"). `OsKeyStore` has no automated test; add `#[test] #[ignore]` `os_keystore_round_trip` (set/get/delete with a random vault id) for the manual Windows check.
- [ ] **Step 4: Run** tests, `cargo deny check`, clippy, fmt — expected: PASS.
- [ ] **Step 5: Commit** `feat(vault): keychain key store and auto-unlock`.

### Task 4: Core – credential reference and schema 2

**Files:**
- Modify: `crates/fernjo-core/src/{model.rs,resolve.rs,tree.rs,store.rs,persist.rs,lib.rs}`

**Interfaces:**
- Produces: `pub type CredentialId = uuid::Uuid;` `Settings.credential: Option<CredentialId>` (serde `credential`, skipped when `None`); `NodePatch.credential: Patch<CredentialId>` (same JSON rule as the others); `ResolvedNode.credential: Field<CredentialId>`; `Tree::nodes_using_credential(&self, id:CredentialId) -> Vec<(NodeId, String)>` (nodes with the id set *locally*, depth-first, name included); `Tree::clear_credential(&mut self, id) -> usize`; `Store::clear_credential(&mut self, id) -> Result<usize,StoreError>` (saves only if something changed); `CURRENT_VERSION = 2`.

- [ ] **Step 1: Write failing tests:**
  - `resolve::tests`: `credential_inherits_from_grandparent_with_source`, `local_credential_wins_over_folder`, `credential_has_no_default`.
  - `tree::tests`: `patch_credential_set_then_inherit_clears_local_value`, `duplicate_keeps_the_credential` (Review Focus 5), `nodes_using_credential_lists_local_users_only`, `clear_credential_removes_every_local_reference_and_counts_them`, `dangling_credential_id_resolves_without_error`.
  - `persist::tests`: `version_one_file_loads_and_is_written_as_version_two`, `credential_round_trips`, `version_three_file_is_refused_as_newer_schema`, `migration_chain_has_a_one_to_two_step` (via `migrate_with`).
  - `store::tests`: `clear_credential_is_persisted`, `clear_credential_with_no_users_writes_nothing`.
- [ ] **Step 2: Run** `cargo test -p fernjo-core` — expected: FAIL.
- [ ] **Step 3: Implement** per the Interfaces block. Step 1 → 2 in `persist.rs` is a no-op function in the `migrate` chain (reason in a comment: an older build would drop `credential` and overwrite it). Update `crates/fernjo-core/examples/generate_tree.rs` only if it breaks.
- [ ] **Step 4: Run** `cargo test --workspace`, clippy, fmt — expected: PASS (src-tauri tests still compile; fix literal `Settings {..}` constructions if any).
- [ ] **Step 5: Commit** `feat(core): inheritable credential reference, connections.json schema 2`.

### Task 5: Core – settings.json, data folder resolution, sync-conflict scan

**Files:**
- Create: `crates/fernjo-core/src/{settings.rs,conflicts.rs}`
- Modify: `crates/fernjo-core/src/{paths.rs,persist.rs,error.rs,lib.rs}`

**Interfaces:**
- Produces: `struct AppSettings { data_dir: Option<PathBuf> }` (serde `{ "schemaVersion": 1, "dataDir"?: string }`, camelCase); `settings::FILE_NAME = "settings.json"`; `settings::load(path:&Path) -> Result<AppSettings,LoadError>` (missing = defaults; newer schema refused; unreadable JSON falls back to `.bak`, else `LoadError::Unreadable`); `settings::save(path:&Path, &AppSettings) -> Result<(),SaveError>` (same atomic + `.bak` sequence as `connections.json`; share the write helper in `persist.rs` rather than copying it); `enum DataDirSource { Env, Settings, Default }`; `struct Location { data_dir: PathBuf, config_dir: PathBuf, source: DataDirSource }`; `fn locate() -> Option<Location>` (precedence: `FERNJO_DATA_DIR` non-empty > `settings.json` `dataDir` > `<OS config dir>/fernjo`; `config_dir` is always `<OS config dir>/fernjo`, where `settings.json` lives; when the env var wins, `settings.json` is not read); the testable form `locate_from(env:Option<OsString>, config_base:Option<PathBuf>) -> Option<Location>`; `fn find_conflicts(dir:&Path) -> Vec<String>` (file names in `dir`, sorted, containing `sync-conflict` (Syncthing), `conflicted copy` (Dropbox) or `_conflict-` (Nextcloud), case-insensitive; the folder missing yields an empty list); `Notice::SyncConflicts { files: Vec<String> }` (serde like the existing variant). Keep `data_dir_from`/`data_dir` only if still used, otherwise remove them and their tests.

- [ ] **Step 1: Write failing tests:** `settings::tests` — `missing_file_is_defaults`, `round_trip_keeps_data_dir`, `save_keeps_bak_and_leaves_no_temp_file`, `newer_schema_is_refused_and_untouched`, `corrupt_file_falls_back_to_bak`; `paths::tests` — `env_wins_over_settings_and_settings_is_not_read`, `settings_data_dir_wins_over_default`, `default_is_config_dir_fernjo`, `no_config_dir_and_no_env_is_none`; `conflicts::tests` — `finds_syncthing_dropbox_and_nextcloud_copies_sorted`, `ignores_normal_files`, `missing_dir_is_empty`.
- [ ] **Step 2: Run** `cargo test -p fernjo-core` — expected: FAIL.
- [ ] **Step 3: Implement** per the Interfaces block.
- [ ] **Step 4: Run** `cargo test -p fernjo-core`, clippy, fmt — expected: PASS.
- [ ] **Step 5: Commit** `feat(core): settings.json, data folder precedence and sync-conflict scan`.

### Task 6: App state, startup notices and restart guard (Rust and snapshot type)

**Files:**
- Modify: `src-tauri/src/{state.rs,lib.rs,commands.rs}`, `src-tauri/Cargo.toml`, `ui/src/lib/{types.ts,testing.ts,store.svelte.ts}`, `ui/src/lib/components/NoticeBar.svelte` (+ their tests)

**Interfaces:**
- Consumes: Tasks 1–5.
- Produces: `Snapshot.notices: Vec<Notice>` (was `notice: Option<Notice>`; TS `notices: Notice[]`, `TreeStore.notices`, `dismissNotice(index)`); `AppState` fields: `store`, `notices: Mutex<Vec<Notice>>`, `vault: Mutex<VaultState>`, `keystore: Box<dyn KeyStore>`, `data_dir: Option<PathBuf>`, `config_dir: Option<PathBuf>`, `source: DataDirSource`, `restart_required: AtomicBool`; `kdf: KdfParams` (production default; tests set `KdfParams::TEST`); `enum VaultState { None, Open(Vault), Unusable { reason: String, path: PathBuf } }` (`Vault::is_unlocked()` says locked or unlocked; `AppState::vault_status()` is the single reader that commands use); `AppState::open(dir: Option<PathBuf>)` stays as the test constructor (in-memory key store, no settings file) and `AppState::open_for_app(location: Option<Location>)` is the production one (`OsKeyStore`); `fn ensure_writable(state:&AppState) -> Result<(), CommandError>` returning code `restart_required` while `restart_required` is set; every mutating tree command calls it first.
- At startup: open the store as before; open the vault (`Vault::open`): missing → `None`, structural/IO error or newer format → `Unusable` (never blocks the tree), otherwise `Locked` then `try_auto_unlock`; `find_conflicts(data_dir)` adds `Notice::SyncConflicts` when non-empty.

- [ ] **Step 1: Write failing Rust tests** in `commands.rs`/`state.rs`: `startup_with_conflict_files_delivers_a_sync_conflicts_notice_once` (second `list_tree` has none); `restored_from_backup_notice_still_arrives_in_the_notices_list`; `vault_missing_is_none_and_zero_length_file_is_unusable_with_path` (Review Focus 2); `unusable_vault_does_not_break_list_tree`; `startup_auto_unlocks_when_the_key_store_has_the_key`; `mutating_commands_return_restart_required_when_flagged_and_reads_still_work`.
- [ ] **Step 2: Write failing UI tests:** `store.test.ts` — `notices_from_the_snapshot_are_kept_and_dismissed_by_index`; `NoticeBar` — renders a sync-conflict notice naming each file and keeps the restored-from-backup text.
- [ ] **Step 3: Run** `cargo test -p fernjo-app` and `pnpm test` — expected: FAIL.
- [ ] **Step 4: Implement** the Interfaces block; register `AppState::open_for_app(fernjo_core::locate())` in `lib.rs`; update `types.ts`, `testing.ts` (`snapshot(children, notices = [])`), `store.svelte.ts`, `NoticeBar.svelte`; add the `fernjo-vault` path dependency to `src-tauri/Cargo.toml`.
- [ ] **Step 5: Run** `cargo test --workspace`, `pnpm lint`, `pnpm test` — expected: PASS.
- [ ] **Step 6: Commit** `feat(app): vault state at startup, multiple startup notices, restart guard`.

### Task 7: Vault and credential commands, credential setting over IPC

**Files:**
- Create: `src-tauri/src/vault_commands.rs`
- Modify: `src-tauri/src/{lib.rs,commands.rs,state.rs}`

**Interfaces:**
- Consumes: Task 6 `AppState`, Task 2–3 vault API, Task 4 `Store::clear_credential`, `Tree::nodes_using_credential`.
- Produces (plain functions over `&AppState`, wrappers in `vault_commands::handlers`, registered in `run()`; JSON camelCase):
  - `vault_status() -> VaultStatus { state: "none"|"locked"|"unlocked"|"unusable", reason: Option<String>, path: Option<String>, autoUnlock: bool }` (`autoUnlock` = the key store holds this vault's key).
  - `vault_create(password: String, remember: bool)`, `vault_unlock(password: String, remember: bool)`, `vault_lock()`, `vault_change_password(old: String, new: String)`, `vault_set_auto_unlock(enabled: bool)` (needs an unlocked vault; failure message from `KeyStoreError`) — each returns `VaultStatus`. `vault_create` is refused (`already_exists`) whenever any `vault.fernjo` exists, including an unusable one. Passwords are converted to `SecretString` at the boundary.
  - `list_credentials() -> Vec<CredentialInfo>` and `get_credential(id) -> CredentialInfo` where `CredentialInfo { id, name, kind: "password"|"privateKey", username, domain, notes }` (no secret field); `create_credential(draft: CredentialDraft) -> CredentialInfo`, `update_credential(id, update: CredentialUpdate) -> CredentialInfo` with `CredentialDraft { name, kind, username?, domain?, notes, password?, privateKey?, passphrase? }` and `CredentialUpdate` (all optional; a secret field present replaces the secret); `credential_usage(id) -> Vec<UsageItem { id, name, kind }>`; `delete_credential(id) -> Mutation` (clears references in the tree first, then deletes the entry; returns the new snapshot); `reveal_credential(id) -> RevealedSecret { password?, privateKey?, passphrase? }` — the only response type with a secret field.
  - `NodePatch.credential` already flows through `update_node`; `get_node` returns `credential` in `ResolvedNode`.
  - All vault/credential mutations call `ensure_writable`; every command on a locked vault returns `vault_locked`; `VaultError` maps to `{code, message}`.

- [ ] **Step 1: Write failing tests** (`AppState::open`, which uses `MemoryKeyStore` and `KdfParams::TEST`):
  - Lifecycle: `create_then_status_is_unlocked`, `lock_then_wrong_password_then_right_password`, `remember_stores_the_key_and_status_reports_auto_unlock`, `set_auto_unlock_false_deletes_the_key`, `change_password_keeps_auto_unlock_working`, `create_is_refused_when_a_vault_file_exists_even_if_unusable`, `short_password_returns_weak_password`.
  - Credentials: `create_list_get_update_round_trip`, `reveal_returns_the_secret_only_via_reveal`, `delete_clears_references_then_removes_the_entry` (snapshot has no reference; usage list before delete names the users), `delete_while_locked_returns_vault_locked_and_clears_nothing` (Review Focus 5), `update_node_with_credential_patch_sets_and_resets`, `locked_vault_still_resolves_credential_ids`.
  - **No secrets over IPC:** `no_response_contains_a_secret` — sentinel password, private key and passphrase in the vault; call every command function in a table (tree commands, vault commands, credential commands), `serde_json::to_string` each response, assert none contains a sentinel except `reveal_credential`; also assert the saved `connections.json` text has none. `command_table_covers_every_tauri_command` — read `src/commands.rs` and `src/vault_commands.rs` (and `src/data_folder.rs` once it exists) with `include_str!`, count `#[tauri::command]` occurrences, assert equal to the table length.
- [ ] **Step 2: Run** `cargo test -p fernjo-app` — expected: FAIL.
- [ ] **Step 3: Implement** the Interfaces block. Convert incoming `String` secrets to `SecretString` immediately and keep response types free of secrets; `RevealedSecret` is the single exception and is documented as such.
- [ ] **Step 4: Run** `cargo test --workspace`, clippy, fmt — expected: PASS.
- [ ] **Step 5: Commit** `feat(app): vault and credential commands`.

### Task 8: Data folder commands

**Files:**
- Create: `src-tauri/src/data_folder.rs`
- Modify: `src-tauri/{Cargo.toml,src/lib.rs}`

**Interfaces:**
- Consumes: Task 5 `settings`, `Location`, `DataDirSource`; Task 6 `AppState`.
- Produces: `get_data_folder() -> DataFolderInfo { path: String, source: "env"|"settings"|"default", changeable: bool, restartRequired: bool }` (`changeable` is false while the env var controls it); `apply_data_folder(state:&AppState, target:&Path) -> Result<DataFolderInfo, CommandError>` (testable core of the command); `choose_data_folder(app, state) -> Result<Option<DataFolderInfo>, CommandError>` (async command; native picker through `tauri_plugin_dialog` Rust API on a worker thread, `None` if cancelled); `restart_app(app)`. `tauri_plugin_dialog::init()` is registered after the single-instance plugin; no JS permission is added.
- Rules for `apply_data_folder` (error codes in parentheses): refuse while the env var controls it (`data_dir_locked_by_env`); the target must exist as a folder and accept a test write (`folder_not_usable`); target equal to the current folder (compared after canonicalising) is a no-op that does not set `restart_required`; target inside the current folder or the current folder inside the target is refused (`folder_nested`); if the target contains neither `connections.json` nor `vault.fernjo`, copy the current ones (not `.bak`) there first; existing files in the target are never overwritten; write `settings.json` last (atomic); only then set `restart_required`.

- [ ] **Step 1: Write failing tests:** `choosing_an_empty_folder_copies_files_writes_settings_and_sets_restart_required`; `existing_files_in_the_target_are_kept_and_not_overwritten`; `same_folder_is_a_no_op_without_restart_required`, `file_instead_of_folder_is_folder_not_usable`, `read_only_folder_is_folder_not_usable` (where the OS allows; otherwise a path that cannot be written), `nested_folders_are_refused` (Review Focus 4); `env_override_refuses_with_data_dir_locked_by_env`; `failed_settings_write_leaves_restart_required_unset` (settings path unwritable); `after_success_mutating_commands_return_restart_required`; `get_data_folder_reports_source_and_changeable`.
- [ ] **Step 2: Run** `cargo test -p fernjo-app` — expected: FAIL.
- [ ] **Step 3: Implement.** Add `tauri-plugin-dialog = "2"` (record version in `Docs/DECISIONS.md`); `restart_app` calls the app handle's restart; extend the command-count test list to this file.
- [ ] **Step 4: Run** `cargo test --workspace` (includes `src-tauri/tests/tauri_conf.rs`: capabilities must still be `core:default`), `cargo deny check`, clippy, fmt — expected: PASS.
- [ ] **Step 5: Commit** `feat(app): selectable data folder with restart guard`.

### Task 9: UI – vault store, chip, lock note, create/unlock/change-password

**Files:**
- Create: `ui/src/lib/vault.svelte.ts`, `ui/src/lib/components/{VaultChip,VaultNote,PasswordField}.svelte` and tests
- Modify: `ui/src/lib/{types.ts,api.ts,testing.ts}`, `ui/src/lib/components/{Sidebar,FilterBox}.svelte` (chip placement), `ui/src/App.svelte`, `ui/src/lib/store.svelte.ts` (surface `restart_required`)

**Interfaces:**
- Produces: TS types `VaultStatus`, `CredentialInfo`, `CredentialDraft`, `CredentialUpdate`, `UsageItem`, `RevealedSecret`, `DataFolderInfo` mirroring Tasks 7–8; the `Api` interface and `tauriApi` gain camelCase methods for every vault, credential and data-folder command (`vaultStatus`, `vaultCreate`, `vaultUnlock`, `vaultLock`, `vaultChangePassword`, `vaultSetAutoUnlock`, `listCredentials`, `getCredential`, `createCredential`, `updateCredential`, `credentialUsage`, `deleteCredential`, `revealCredential`, `getDataFolder`, `chooseDataFolder`, `restartApp`); `mockApi` in `testing.ts` records and overrides them like the existing ones; `class VaultStore { status; credentials; error; init(); create(password, remember); unlock(password, remember); lock(); changePassword(old, next); setAutoUnlock(on); refreshCredentials() }` (`credentials` empty unless unlocked; every call goes through the same serial queue idea as `TreeStore`, errors shown inline by the caller); `PasswordField` (masked input with show/hide toggle, `label`, `error`, `autocomplete="off"`); `App.svelte` owns `view: "empty" | "vault"`.
- UI contract (`DESIGN.md` "Vault state and lock note"): chip beside the filter reading "Unlocked"/"Locked" (and "No vault"); lock note under the filter when locked; no vault → note "No vault yet" with "Create vault"; unlock form inline in the note with "Unlock automatically on this device" and the sentence that Lock lasts until the next start while that is on; wrong password shows the `DESIGN.md` danger state ("Wrong master password. Check Caps Lock and try again."); create asks twice and states there is no recovery; a `restart_required` error shows in the notice bar.

- [ ] **Step 1: Write failing tests** (Vitest, same style as `PropertiesPanel.test.ts`): `vault.test.ts` — `init_loads_status_and_credentials_only_when_unlocked`, `unlock_error_keeps_status_locked_and_returns_the_error`, `lock_clears_the_credential_list`; `VaultChip.test.ts` — texts for none/locked/unlocked/unusable, click while unlocked opens the manager, click while locked focuses the unlock form; `VaultNote.test.ts` — create requires matching passwords and shows the no-recovery sentence, wrong password shows the danger message, remember checkbox value is passed to `vaultUnlock`, unusable vault shows the reason and path and no "Create vault" button.
- [ ] **Step 2: Run** `pnpm test` — expected: FAIL.
- [ ] **Step 3: Implement** the Interfaces block; reuse `TextField`/`Icon`/existing button styles, no new tokens unless a test in `tokens.test.ts` forces it.
- [ ] **Step 4: Run** `pnpm lint`, `pnpm test` — expected: PASS.
- [ ] **Step 5: Commit** `feat(ui): vault chip, lock note and unlock flow`.

### Task 10: UI – vault manager view

**Files:**
- Create: `ui/src/lib/components/VaultManager.svelte` and `VaultManager.test.ts`
- Modify: `ui/src/App.svelte`, `ui/src/lib/vault.svelte.ts`

**Interfaces:**
- Consumes: Task 9 `VaultStore`, `Api` credential methods, `ConfirmDialog`, `PasswordField`.
- Produces: `VaultManager` props `{ vault: VaultStore; onclose: () => void }`; main-area view with entry list (name, kind, username), "+ New", edit form (name, type password/private key, username, domain, secret field(s), notes), header actions Lock, Change password, auto-unlock toggle, Close. On edit the secret fields start empty with the hint "unchanged"; "Reveal" calls `revealCredential`, shows the value and hides it after 10 seconds, when the vault locks or when the view closes (timer injectable through a prop for tests); delete opens `ConfirmDialog` listing the usage items from `credentialUsage` ("Used by: …", "They will be set back to inherit").

- [ ] **Step 1: Write failing tests:** `lists_entries_and_selects_one_for_editing`; `editing_without_typing_a_secret_sends_no_secret_field`; `creating_a_private_key_entry_sends_key_and_passphrase_once`; `reveal_shows_the_secret_then_hides_it_after_ten_seconds` (fake timers); `reveal_is_hidden_on_lock_and_on_close`; `delete_dialog_lists_usage_and_cancel_has_focus`; `delete_confirm_calls_delete_credential_and_refreshes_the_list`; `no_secret_is_rendered_before_reveal` (DOM text contains no sentinel).
- [ ] **Step 2: Run** `pnpm test` — expected: FAIL.
- [ ] **Step 3: Implement**; `App.svelte` shows `VaultManager` when `view === "vault"` and the existing empty text otherwise.
- [ ] **Step 4: Run** `pnpm lint`, `pnpm test` — expected: PASS.
- [ ] **Step 5: Commit** `feat(ui): vault manager view`.

### Task 11: UI – credential row in the properties panel

**Files:**
- Modify: `ui/src/lib/components/PropertiesPanel.svelte` (+ test), `ui/src/App.svelte`, `ui/src/lib/types.ts` (`ResolvedNode.credential`, `NodePatch.credential`), `ui/src/lib/testing.ts` (`resolved()` default `credential`)

**Interfaces:**
- Consumes: `store.detail.credential: FieldInfo<string>`, `VaultStore.status`/`credentials`.
- Produces: `PropertiesPanel` takes a second prop `vault: VaultStore`; a "Credential" `PropertyRow` (`name="credential"`) shown for folders and connections: a `<select>` of entry names when unlocked (first option "None"), effective value shown with "from <folder>" / "Inherit" like the other rows; when locked the value reads "Vault locked" (read-only, the id is kept); when the effective id is not in the unlocked list it reads "Missing credential".

- [ ] **Step 1: Write failing tests:** `credential_row_lists_entry_names_when_unlocked`; `choosing_an_entry_sends_a_credential_patch`; `inherited_credential_shows_from_folder_and_inherit_resets_it` (patch `credential: null`); `locked_vault_shows_vault_locked_and_offers_no_choice`; `unknown_credential_id_shows_missing_credential`; `no_secret_or_id_text_is_shown_for_the_credential_value` (only the name).
- [ ] **Step 2: Run** `pnpm test` — expected: FAIL.
- [ ] **Step 3: Implement** and pass `vault` from `App.svelte`.
- [ ] **Step 4: Run** `pnpm lint`, `pnpm test` — expected: PASS.
- [ ] **Step 5: Commit** `feat(ui): credential row in the properties panel`.

### Task 12: UI – data folder dialog

**Files:**
- Create: `ui/src/lib/components/DataFolderDialog.svelte` and test
- Modify: `ui/src/lib/components/Sidebar.svelte` (gear button), `ui/src/App.svelte`, `ui/src/lib/components/NoticeBar.svelte` (restart-required notice)

**Interfaces:**
- Consumes: `Api.getDataFolder`, `chooseDataFolder`, `restartApp`; `Icon` (add a `gear` glyph in the existing 16 px line style if absent).
- Produces: gear button at the bottom of the sidebar opening a modal dialog (same shell as `ConfirmDialog`: Escape closes, focus kept inside) showing the current path and its source, "Choose folder…", and — after a successful choice — "Restart to use the new folder" with a secondary "Restart now"; when `changeable` is false it shows "Set by FERNJO_DATA_DIR" and disables choosing; errors from Rust (`folder_not_usable`, `folder_nested`) show inline. After a choice a persistent bar in the notice area says a restart is required.

- [ ] **Step 1: Write failing tests:** `shows_current_path_and_source`; `choose_calls_the_api_and_offers_restart`; `cancelled_picker_changes_nothing`; `env_controlled_folder_disables_choosing`; `folder_errors_show_inline`; `restart_now_calls_restart_app`; `escape_closes_the_dialog`.
- [ ] **Step 2: Run** `pnpm test` — expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `pnpm lint`, `pnpm test` — expected: PASS.
- [ ] **Step 5: Commit** `feat(ui): data folder dialog`.

### Task 13: Docs, verification in the real app, gate, roadmap

**Files:**
- Modify: `Docs/DECISIONS.md`, `CLAUDE.md` ("Project-specific"), `CHANGELOG.md` (`[Unreleased]`), `README.md` only if it describes data files, `Docs/roadmap.json` + generated `Docs/ROADMAP.md`

- [ ] **Step 1: Docs.** `Docs/DECISIONS.md` entries (newest on top): vault file format and parameters (point to `Docs/vault-format.md`), `connections.json` schema 2, `settings.json` and data-folder precedence, keychain behaviour and threat model, data-folder change and restart guard, NFC normalisation of the master password, dependency versions. `CLAUDE.md`: the vault state and `AppState::open` versus `open_for_app`, "`FERNJO_DATA_DIR` also disables settings.json and the data-folder dialog; use it for every manual run", the no-secret command-table test. `CHANGELOG.md` `[Unreleased]`: vault, credential reference, data folder, sync-conflict warning.
- [ ] **Step 2: Manual check in the real app (Windows).** Start `pnpm tauri dev` with `FERNJO_DATA_DIR=<scratch dir>` and `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`, drive it through the DevTools protocol as described in `CLAUDE.md` (never the user's mouse or keyboard): create the vault, add a password and a key entry, reveal and watch it hide, attach the credential to a folder and see the connection inherit it, lock, see "Vault locked" in the panel, unlock with a wrong then the right password, delete the entry and read the usage warning, check `connections.json` and the console for secrets and CSP warnings (also in `pnpm tauri build --no-bundle` with the served CSP). Auto-unlock through Credential Manager: the real `OsKeyStore` cannot run under the env override alone, so run `cargo test -p fernjo-vault os_keystore_round_trip -- --ignored` and then the app check with "Unlock automatically on this device" ticked, a graceful restart, and the vault opening unlocked; untick and confirm it stays locked after another restart. The data-folder dialog needs the env var unset: with the user's consent, or with `APPDATA` pointed at a scratch folder for that one run, choose a folder, restart, see the data. Sync-conflict: put a `connections.sync-conflict-20261115-000000-ABC.json` next to the files and read the notice. Record each result with the exact command or observation; mark anything not run as OPEN with its command.
- [ ] **Step 3: Gate.** Run and record results: `pnpm install --frozen-lockfile`, `pnpm ui:build`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm lint`, `pnpm test`, `cargo deny check`, `dotnet run tools/roadmap.cs -- check` — all green.
- [ ] **Step 4: Roadmap.** In `Docs/roadmap.json`: tick every M3 task (`done: true`), tick each acceptance criterion with evidence (test names, command results, the manual checks), add macOS and Linux keychain checks to the "Open manual checks (owner)" block, set M3 `done` with `completedOn`; `dotnet run tools/roadmap.cs -- generate`, then `check`.
- [ ] **Step 5: Commit** `docs: milestone 3 – decisions, changelog and roadmap evidence` and open the PR to `dev` (see CLAUDE.md "Git & branches").
