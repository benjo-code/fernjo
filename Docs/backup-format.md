# Backup and connections export format

Two files leave fernjo in Milestone 5: the **encrypted full backup** (`fernjo-backup-YYYY-MM-DD.fjbk`) and the **connections export** (`fernjo-connections-YYYY-MM-DD.json`, no secrets). This page is enough to write a reader without the fernjo source (no lock-in). Implementation: `crates/fernjo-import` (`backup.rs`, `connections.rs`) and `seal_with_password` in `crates/fernjo-vault` (`sealed.rs`).

## Encrypted backup (`.fjbk`, container version 1)

The file is one `seal_with_password` blob with magic `FERNJOBK` and format version `1`. It is the same primitive set as [`vault.fernjo`](vault-format.md), but a separate container: a backup has its own password (never the master password) and its own version number.

- **Key derivation:** Argon2id (version 0x13), 32-byte output, parameters and salt from the header. The password is Unicode-normalised to NFC and used as UTF-8 bytes exactly as typed (no trimming, no case folding), like the master password.
- **AEAD:** XChaCha20-Poly1305 (24-byte nonce, 16-byte tag appended to the ciphertext). The associated data is the **whole 62-byte header**, so changing any header byte fails authentication.
- **Minimum password length:** 8 characters after NFC normalisation, enforced when a backup is written (`password_too_short`), never when one is opened.
- **Parameters written by this build:** the vault's default cost (128 MiB, 6 iterations, 4 lanes).

### Layout

All integers are big endian.

| Offset | Size | Field |
|---:|---:|---|
| 0 | 8 | magic, ASCII `FERNJOBK` |
| 8 | 2 | format version (`u16`), currently `1` |
| 10 | 4 | Argon2id memory in KiB |
| 14 | 4 | Argon2id iterations |
| 18 | 4 | Argon2id parallelism |
| 22 | 16 | Argon2id salt |
| 38 | 24 | nonce |
| 62 | n >= 16 | body: JSON ciphertext + 16 bytes tag |

Bytes 0..62 are the header. A reader checks, in this order: magic (otherwise not a backup), version (a larger one is refused), minimum length 62 + 16, and the KDF parameters against the same bounds the vault applies when it loads a file (so a hostile file cannot demand gigabytes or hours) before it derives any key. Out-of-range parameters count as damaged.

### Body JSON (`schemaVersion` 1)

```json
{
  "schemaVersion": 1,
  "createdAt": "2026-11-16T09:30:00Z",
  "app": "0.0.0",
  "connections": { "schemaVersion": 2, "children": [] },
  "entries": [
    {
      "id": "<uuid>",
      "name": "…",
      "username": "…",
      "domain": "…",
      "secret": { "type": "password", "password": "…" },
      "notes": ""
    }
  ]
}
```

- `createdAt`: UTC, RFC 3339 with a `Z`. `app`: the fernjo version that wrote the file.
- `connections` is the `connections.json` document exactly as the app stores it (same `schemaVersion`, same node rules, credential references included). It is parsed and validated by the same code that loads `connections.json`.
- `entries` are the vault entries with their secrets (`username` and `domain` are omitted when unset, `notes` defaults to empty). The `secret` object has the shape of the vault body (see `Docs/vault-format.md`). Entry ids are kept, so the credential references in `connections` stay valid and a restore is identical.
- A body `schemaVersion` above 1 is refused as `newer_version`; any other value than 1 is damaged.

### Limits (the decrypted body)

- at most 64 MiB of JSON,
- at most 100 000 entries,
- at most 100 000 folders and connections in `connections` (counted without recursion),
- nesting of at most 60 levels (`MAX_TREE_DEPTH` of `fernjo-core`, the top level is level 1; `connections.json` itself loads down to level 62, see `Docs/DECISIONS.md` "M5 final review fixes"). A deeper tree makes the backup `damaged`.

A backup made without a vault has `"entries": []`.

### Read caps (before any parsing)

`src-tauri/src/import_commands.rs` checks the file size from the metadata before it reads anything and answers `too_large` above the cap: **32 MiB** for an mRemoteNG file, **64 MiB** (`MAX_OTHER_BYTES`) for a connections export or a backup. An encrypted backup above 64 MiB therefore never reaches `open`. The connections importer has its own limit of 32 MiB, so a connections file between 32 and 64 MiB passes the reader and is refused by the importer as `invalid`.

### Error codes

| Code | Meaning |
|---|---|
| `password_too_short` | writing: the password has fewer than 8 characters |
| `not_a_backup` | the magic is not `FERNJOBK` (or the file is shorter than the 10 bytes up to the version) |
| `newer_version` | container or body version above 1, or a `connections` document with a newer `schemaVersion` than this build understands (the message names the found number) |
| `wrong_password` | authentication failed. A wrong password cannot be told from a damaged or modified file, so the message says "wrong password or the file is damaged" |
| `damaged` | truncated file, KDF parameters out of range, body not valid, too large, duplicate or nameless entry, invalid connections, a tree nested deeper than 60 levels |

Opening a backup never reveals why authentication failed: both a wrong password and a modified ciphertext give `wrong_password`.

## Command-level errors

Besides the format codes above, the backup, restore and import commands (`src-tauri/src/import_commands.rs`; importer codes in `crates/fernjo-import/src/error.rs`) answer with these codes. `import_inspect` keeps the picked file for a retry after `key_needed`, `wrong_key`, `wrong_password` and `password_needed`; any other error leaves the pending slot as it was.

| Code | Meaning |
|---|---|
| `no_pending_import` | nothing was picked or is waiting, or the picked file was replaced or cancelled while it was being read |
| `already_inspected` | `import_inspect` on a file that was already inspected |
| `not_inspected` | `import_apply` on a file that was picked but not inspected |
| `file_unreadable` | the file cannot be read, is not a regular file or the dialog returned a non-local path |
| `too_large` | the file is above the read cap (see above), or the importer's own size limit for an mRemoteNG file |
| `too_complex` | mRemoteNG file nested deeper than 59 levels (the new import folder is level 1 of 60), holding more than 100 000 folders and connections, or costing more than 20 000 000 PBKDF2 iterations in all: the key check, a full-file body and one derivation per stored password, each of `KdfIterations` iterations. The check runs before any password is decrypted and the message names the numbers; hundreds of passwords at 1000 iterations use about 2 % of the budget |
| `too_deep` | an import or restore would nest folders deeper than 60 levels (`Store::graft`, `Store::replace_tree`); nothing is written. Readers refuse such files earlier (`too_complex`, `invalid`, `damaged`), so this only shows when the plan itself is too deep |
| `export_target_not_allowed` | the chosen export file lies inside fernjo's data folder (where it could replace `vault.fernjo` or `connections.json`); choose another folder. Links and `..` are resolved before comparing |
| `key_needed` | mRemoteNG file encrypted with a custom key and none was given |
| `wrong_key` | the typed mRemoteNG key does not decrypt the file |
| `unsupported_format` | not a ConfVersion 2.6 mRemoteNG file (message names what was found) |
| `password_needed` | a backup was inspected without a password |
| `vault_needed` | applying needs a vault that does not exist yet or is locked: create or unlock it, the pending slot waits |
| `vault_locked` | the vault was locked when its entries were read or written (for example `export_backup`) |
| `vault_unusable` | `vault.fernjo` is unreadable or from a newer version; it is never overwritten |
| `import_incomplete` | an import failed and the entries it had added (all of them, in one save) could not be removed again |
| `restore_incomplete` | a restore failed and the vault could not be put back: it may hold the backup's credentials while the connections are the old ones. The previous content may still be in `vault.fernjo.bak` (not if the failed save got as far as replacing it), so the message tells you to look there before changing the vault again |
| `save_failed` | an export file or `connections.json` could not be written |
| `restart_required` | a changed data folder waits for a restart; mutations are refused |
| `internal` | the connection data is unavailable after an internal error |

`damaged` (see above) is also what an inconsistent backup yields at inspect time (a duplicate or nameless entry), and `damaged` from the mRemoteNG reader covers malformed values (`KdfIterations` out of range, an undecryptable `Protected`). Errors from the vault (`changed_on_disk`, `duplicate_entry`, `empty_name`) and from the store (`duplicate_id` and the other tree codes, a store that failed to load) pass through with their own codes.

## Restoring

- **A full backup replaces.** The whole tree and all vault entries are replaced after a confirmation that names what is lost (folders and connections, credentials, the backup's date). The master password and the data key stay; the `.bak` files are kept and afterwards hold the state from before the restore. Open sessions are closed.
- **A vault that exists must be unlocked**, even for a backup without entries (replacing then empties the vault); without a vault file there is nothing to replace, and a backup with entries first needs a vault to be created. A vault that cannot be read (`vault_unusable`) is never overwritten.
- Entries are written first, then the tree; when the tree step fails the previous entries are put back. If that fails too the error is `restore_incomplete`; the previous vault content may still be in `vault.fernjo.bak`.
- An mRemoteNG import adds its credentials with one vault save, so `vault.fernjo.bak` afterwards holds the vault as it was before the import (after a rolled-back import it holds the vault with the imported entries in it, because the rollback is a second save).

## Connections export (`.json`)

Human-readable, pretty-printed JSON without any secret and without credential references (every `credential` is dropped; the count comes back after the file is written, as `ExportInfo.droppedCredentials`, and is shown in the done step):

```json
{
  "format": "fernjo-connections",
  "exportedAt": "2026-11-16T09:30:00Z",
  "schemaVersion": 2,
  "children": [ { "id": "…", "name": "…", "type": "folder", "children": [] } ]
}
```

- `schemaVersion` is the version of `connections.json` (currently 2); there is no second version number. `children` are the nodes exactly as in `connections.json`.
- Import checks `format`, then validates through the `connections.json` loader (migration and node rules, no rule is duplicated). Every node gets a fresh id and the nodes go into a new top-level folder **"Imported from fernjo export (YYYY-MM-DD)"** (local date, a second one the same day gets " (2)", " (3)", …). Importing **merges**; it never replaces.
- Limits: at most 32 MiB, 100 000 nodes, nesting depth 59 (the new import folder is level 1 and the tree may be 60 levels deep; JSON nesting itself is limited by the parser to 128 levels). A UTF-8 BOM is accepted.

| Code | Meaning |
|---|---|
| `not_this_format` | not JSON, or `format` is not `fernjo-connections` (a plain `connections.json`, a `confCons.xml`, a backup) |
| `newer_schema` | `schemaVersion` above what this version understands |
| `invalid` | the nodes are not valid, too large, too many or nested too deeply |
