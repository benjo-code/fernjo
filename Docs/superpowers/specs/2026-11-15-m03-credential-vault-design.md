# fernjo – Milestone 3 design: credential vault

**Status:** approved by the owner on 2026-11-15 (including the builder calls in §10) · **Date:** 2026-11-15 · **Parent spec:** [2026-11-15-fernjo-design.md](2026-11-15-fernjo-design.md) (§4, §5.2, §5.4) · **Roadmap:** M3

This document records the decisions for M3 only. Intent, goals, non-goals and anti-goals are those of the parent spec; the visual language is `DESIGN.md`. Decisions the owner made in conversation are marked *(owner)*; everything else is the builder's call with its reason.

## 1. Intent

After M3 the owner can store passwords and SSH private keys in an encrypted vault, unlock it with a master password (or automatically through the OS keychain), and attach a stored credential to a connection or a folder. Nothing connects yet; M4 (SSH) and M5 (import) are the first consumers.

## 2. Scope

**In:** the `fernjo-vault` crate; the credential reference as a fourth inheritable setting; vault Tauri commands; vault state, lock note and unlock in the sidebar; vault manager view; credential row in the properties panel; a minimal `settings.json` and a data-folder dialog; the sync-conflict warning.

**Out:** opening sessions and the inline unlock prompt in a tab (M4); import of mRemoteNG passwords (M5); the full settings dialog (theme, terminal font; M6); auto-lock after idle time (backlog); master-password recovery (there is none, and the create dialog says so).

**Anti-goals:** a secret in `connections.json`, `settings.json`, logs, webview storage or any IPC response other than the explicit reveal; a modal vault; home-made crypto; an entry that silently disappears.

## 3. Approaches *(owner chose each)*

| Question | Chosen | Rejected |
|---|---|---|
| Username when a connection has a credential | **The connection's own (or inherited) username wins; the entry's username is the fallback.** One key entry can serve hosts with different logins, and the properties panel already shows where the username comes from. | Entry always wins (a shared credential could never be used with another login); entry is secret-only (the entry's username field would be pointless, and the import in M5 maps username with the password). |
| Where the vault manager lives | **A view in the main area** (empty until M4), opened from the vault chip: entry list on the left, edit form on the right. Non-modal, as `DESIGN.md` requires. | Modal dialog (breaks "Don't make the vault modal"); a sidebar section (too cramped for a key editor). |
| Folder picker for the data folder | **Native folder picker**, opened by a Rust command through `tauri-plugin-dialog`'s Rust API. *(Builder refinement: calling it from Rust means no JS capability is granted, so capabilities stay `core:default`.)* | A typed path (error-prone). |

## 4. Vault crate (`fernjo-vault`)

No dependency on Tauri or on `fernjo-core`. Created in this milestone.

- **File `vault.fernjo`** in the data folder. Header (authenticated as associated data): magic, format version, vault id (random UUID), KDF parameters (Argon2id memory, iterations, parallelism, salt), the data key wrapped by the password-derived key (XChaCha20-Poly1305, own nonce). Body: the entries (serialised JSON) encrypted with the data key, fresh random 24-byte nonce on every save. Argon2id parameters target about 0.5 s unlock on a typical machine and are stored in the header, so they can be raised later; on load they are bounded (sane minimum and maximum) so a hostile synced file cannot demand gigabytes of memory.
- **Writes** as for `connections.json`: temp file, `sync_all`, copy to `vault.fernjo.bak`, rename. A format version newer than the app is refused and the file is never overwritten. The format is documented in `Docs/vault-format.md` (anti-goal: lock-in).
- **Entry:** `id` (UUID v4), `name`, optional `username`, optional `domain`, `secret` = `Password(password)` or `PrivateKey { key, passphrase? }` (OpenSSH or PEM text), `notes`. Names need not be unique. Password, key and passphrase are `secrecy::SecretString`; the unlocked data key is zeroised on lock and drop.
- **API (shape, not signatures):** `Vault::create(password)`, `Vault::open(path)` (locked), `unlock(password)`, `unlock_with_key(data_key)`, `lock()`, entry add/update/delete/list/get/reveal, `change_password(old, new)` (verifies the old password, re-wraps the data key under a new salt and saves once; the data key and the entries stay unchanged, but the body is re-sealed with a fresh nonce because the whole header is its associated data), `data_key()` for the keychain. An update that leaves the secret fields `None` keeps the stored secret, so the UI never needs to know it.
- **Master password:** at least 8 characters, no composition rules *(builder's call)*. Wrong password and a tampered file are distinct errors (wrong password cannot be told from a corrupt wrapped key; a body that fails authentication after a successful unlock is "tampered or corrupt").
- **OS keychain:** behind a small `KeyStore` trait (get, set, delete the data key for a vault id) so everything is testable with an in-memory fake; the real implementation uses the `keyring` crate (item keyed by the vault id, so a synced vault maps to the same item on every device). Auto-unlock at start tries the keychain; if the key is missing or fails authentication the vault stays locked and a stale item is deleted (*Superseded in M6: auto-unlock never deletes the keychain item, see `DECISIONS.md` "Vault: keychain behaviour"*). If the keychain is unavailable (for example no Secret Service on Linux), enabling the option reports why and the vault keeps working with the password. Prefer the keyring backend that needs no system C library on Linux (the project already depends on `zbus`).

## 5. Core changes (`fernjo-core`)

- **`Settings.credential: Option<Uuid>`**, serialised as `credential`, inherited like the others; `ResolvedNode.credential: Field<Uuid>` with the usual local/effective/source. The reference is an id; names live in the vault.
- **`schemaVersion` 2** for `connections.json`, with the first real migration step (1 → 2: no change). Reason: an older build reading a version-1 file would drop `credential` and overwrite it on its next save; a bump makes older builds refuse the file instead *(builder's call)*.
- `Tree` gains: all references to a credential id (nodes with the id set locally), and clearing them. A dangling reference (entry missing, for example from a vault synced from another device) is allowed and shown as "Missing credential".
- **Sync-conflict scan** (`paths`/new module): lists files in the data folder whose names mark a sync conflict (Syncthing `sync-conflict`, Dropbox `conflicted copy`, Nextcloud `_conflict-`). `Notice` gains a `SyncConflicts { files }` variant; the startup notice mechanism must carry more than one notice.
- **`settings.json`** (new `settings` module): `{ "schemaVersion": 1, "dataDir"?: string }` in `<OS config dir>/fernjo/`, atomic write with `.bak`, same load rules as `connections.json` (missing = defaults, newer refused). Data folder precedence: `FERNJO_DATA_DIR` (env, wins and disables changing it in the UI) > `settings.json` `dataDir` > `<OS config dir>/fernjo`. M6 adds theme and terminal fields to this file.

## 6. Tauri layer

- **State:** `AppState` gains the vault state (no vault yet / locked / unlocked / unusable with the reason) behind a lock. A broken or newer vault never blocks the tree; only vault commands report it.
- **Commands (names the builder may adjust):** `vault_status`, `vault_create`, `vault_unlock` (password, remember-on-this-device), `vault_lock`, `vault_change_password`, `vault_set_auto_unlock`, `list_credentials`, `get_credential`, `create_credential`, `update_credential`, `delete_credential`, `credential_usage`, `reveal_credential`, `get_data_folder`, `choose_data_folder`, `restart_app`. Pattern as in M2: plain functions over `AppState` (unit-tested), thin `#[tauri::command]` wrappers.
- **What crosses IPC:** status, and per entry id, name, kind, username, domain, notes. The secret is returned by `reveal_credential` and by nothing else; incoming secrets (create, update, unlock, change password) travel UI → Rust only. Response types for entries have no secret field, so a leak needs a deliberate new type.
- **Delete:** `credential_usage(id)` lists the nodes using the entry (names and ids); the UI warns with that list; `delete_credential` first clears the references in the tree (atomic save), then removes the entry. If the vault write fails the references are already gone and the entry remains, which loses no secret.
- **Data folder:** `choose_data_folder` opens the native picker and validates the choice (exists, writable). If the target contains neither `connections.json` nor `vault.fernjo`, the current files (not the `.bak`) are copied there; existing files in the target are never overwritten and the originals are left in place. It then writes `settings.json` and reports "restart required"; `restart_app` restarts the app. While `FERNJO_DATA_DIR` is set the command refuses.
- Dependencies are added to `Docs/DECISIONS.md` with versions when chosen (expected: `argon2`, `chacha20poly1305`, `secrecy`, `zeroize`, `keyring`, `tauri-plugin-dialog`, plus a CSPRNG source); `cargo deny check` must stay green, with new ignores only with a reason.

## 7. UI

- **Vault chip** beside the filter ("Unlocked" or "Locked", per `DESIGN.md`); clicking it opens the vault manager view (or, while locked, starts unlock). **Lock note** under the filter while locked, with "Unlock"; in M3 the password field appears inline inside the note (the tab prompt arrives in M4 and reuses the same command). No vault yet: the note says "No vault yet" with a "Create vault" button; the vault is created lazily, never forced at launch.
- **Create / unlock / change password:** the master-password field is masked with a show toggle; "Unlock automatically on this device" checkbox; the wrong-password state follows `DESIGN.md` (danger border and message). Change password asks for the current and the new password twice.
- **Vault manager view:** entry list (name, kind, username), "+ New", an edit form (name, type password or private key, username, domain, secret field(s), notes). Secret fields start empty on edit with the hint "unchanged"; "Reveal" fetches the secret, shows it, and hides it again after 10 seconds, when the vault locks or when the view closes. Delete uses the confirm dialog, showing the usage list. Lock, change password and the auto-unlock toggle sit in the view's header.
- **Properties panel:** a "Credential" row for folders and connections: a choice of entry names (when unlocked), "from <folder>" and "Inherit" reset as for the other rows. While locked it reads "Vault locked" (the id is kept; names are in the vault). A dangling id reads "Missing credential".
- **Data folder:** a small gear button at the bottom of the sidebar opens a dialog with the current path, "Choose folder…", the restart prompt, and a note when `FERNJO_DATA_DIR` controls it. M6 moves this into the full settings dialog.
- **Sync-conflict warning:** shown in the existing notice bar at startup, naming the files, never merged.
- New UI uses only existing tokens; a missing token goes into the light block and both dark blocks.

## 8. Verification (feeds the roadmap acceptance criteria)

- **Vault tests:** create → lock → unlock round trip with every entry type; wrong password rejected; flipped bytes in header, wrapped key and body each rejected; newer format refused and untouched; absurd KDF parameters rejected; `change_password` keeps all entries and the old data key (the in-memory keychain key still unlocks) and the old password stops working; update without secret keeps the secret; atomic write leaves no temp file and keeps `.bak`; a locked vault holds no readable entries.
- **No-secret tests:** with sentinel secrets in the vault, serialise `connections.json` and the JSON of every command response except `reveal_credential`; assert no sentinel appears. `reveal_credential` is the only one that contains it.
- **Core tests:** credential inherits across levels with source; reset to inherit; usage list and clearing; migration 1 → 2; a version-2 file is refused by version-1 logic (via `migrate_with`/schema check); settings load rules; data-folder precedence; conflict scan names.
- **UI tests (Vitest):** vault chip and note states, unlock error state, credential row (inherited, local, locked, missing), reveal hides after 10 s.
- **Manual (Windows):** auto-unlock via Credential Manager: unlock with "Unlock automatically", restart, vault opens unlocked; turn it off, restart, locked. Data folder: choose a new folder, restart, data present. Sync-conflict warning with a hand-made `connections.sync-conflict-…json`. macOS and Linux go to "Open manual checks (owner)".
- **Gate:** the commands in `CLAUDE.md` (build, tests, lint, `cargo deny check`, roadmap check) green.

## 9. Docs to update during implementation

`Docs/vault-format.md` (new), `Docs/DECISIONS.md` (vault format and parameters, `connections.json` schema 2, `settings.json` and data-folder precedence, dependencies, keychain behaviour), `CHANGELOG.md` `[Unreleased]`, `CLAUDE.md` "Project-specific" (the vault state, the `FERNJO_DATA_DIR` rule for settings), `Docs/roadmap.json` (+ regenerated `ROADMAP.md`).

## 10. Calls settled after the builder review *(builder's calls, listed so the owner can check them)*

1. **Keychain threat model.** With "Unlock automatically on this device" on, the data key sits in the OS keychain, so any process running as the same OS user can open the vault without the master password. That is the accepted trade of the option (it is opt-in, per device, and the vault stays encrypted at rest and when synced). It goes into `Docs/DECISIONS.md` as an explicit statement. No biometric gate in M3.
2. **Changing the data folder is safe by construction.** `choose_data_folder` copies first and writes `settings.json` last; if the copy or the write fails nothing has changed and the error is shown (a half-copied target holds only extra files). After a successful choice `AppState` enters a "restart required" state: every mutating tree and vault command refuses with the code `restart_required`, so no edit lands in the old folder after the choice, and the UI shows a persistent notice with "Restart now". Every mutation already saves before it returns, so `restart_app` has nothing to flush.
3. **Vault edge states.** Deleting or editing an entry needs an unlocked vault. "Create vault" is offered only when no `vault.fernjo` exists; a present but unusable file (newer format, tampered, unreadable) is shown as "Vault unusable" with the reason and the file's path, never replaced, and the message names `vault.fernjo.bak` for a manual restore. There is no automatic fallback to the `.bak` (it can lack recent entries, and a vault must never be silently swapped for an older one).
4. **Schema 2 and `.bak`.** The existing `migrate_with` chain and the "newer schema refused" rule in `persist.rs` are enough; M3 adds step 1 → 2 and sets `CURRENT_VERSION` to 2. A version-1 `.bak` is read and migrated by the new build as before; an older build refuses any version-2 file.
5. **Keychain backend.** `keyring` with the Secret Service backend over `zbus` on Linux (no system C library), the native stores on Windows and macOS. No file-based fallback: a key file next to the vault would defeat it. On a Linux system without Secret Service, enabling the option reports why. CI tests use the in-memory `KeyStore`; the real store is a manual check.
6. **Lock and the keychain item.** "Lock" locks the running session only and never touches the keychain item, so the next start unlocks automatically again (that is what the option means; the vault chip and the manager say so beside the checkbox). The item is removed when the option is turned off, when the stored key fails authentication, and never by `change_password` (the data key does not change). Turning the option on needs an unlocked vault.
7. **No-secret test coverage.** A table in the test calls every vault and tree command function with sentinel secrets in the vault and checks the JSON of each response. A second test counts the `#[tauri::command]` wrappers in `commands.rs` and fails if the count differs from the table, so a new command cannot slip past it.

## 11. Left to the builder

Crate versions and exact feature flags, the binary layout of the vault header within §4, Argon2id numbers that hit about 0.5 s, error code names, Svelte component split, where the gear button and chip sit pixel-wise, the exact Notice plumbing for several startup notices, and the file-name patterns of the conflict scan beyond the three named.
