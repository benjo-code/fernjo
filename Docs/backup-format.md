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
- at most 100 000 folders and connections in `connections` (counted without recursion).

A backup made without a vault has `"entries": []`.

### Error codes

| Code | Meaning |
|---|---|
| `password_too_short` | writing: the password has fewer than 8 characters |
| `not_a_backup` | the magic is not `FERNJOBK` (or the file is shorter than the 10 bytes up to the version) |
| `newer_version` | container or body version above 1 |
| `wrong_password` | authentication failed. A wrong password cannot be told from a damaged or modified file, so the message says "wrong password or the file is damaged" |
| `damaged` | truncated file, KDF parameters out of range, body not valid, too large, duplicate or nameless entry, invalid connections |

Opening a backup never reveals why authentication failed: both a wrong password and a modified ciphertext give `wrong_password`.

## Restoring

- **A full backup replaces.** The whole tree and all vault entries are replaced after a confirmation that names what is lost (folders and connections, credentials, the backup's date). The master password, the data key and the `.bak` files stay. Open sessions are closed.
- **A vault that exists must be unlocked**, even for a backup without entries (replacing then empties the vault); without a vault file there is nothing to replace, and a backup with entries first needs a vault to be created. A vault that cannot be read (`vault_unusable`) is never overwritten.
- Entries are written first, then the tree; when the tree step fails the previous entries are put back. If that fails too the error is `restore_incomplete`, and the previous vault content is in `vault.fernjo.bak`.

## Connections export (`.json`)

Human-readable, pretty-printed JSON without any secret and without credential references (every `credential` is dropped; the preview says how many):

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
- Limits: at most 32 MiB, 100 000 nodes, nesting depth 64 (JSON nesting itself is limited by the parser to 128 levels). A UTF-8 BOM is accepted.

| Code | Meaning |
|---|---|
| `not_this_format` | not JSON, or `format` is not `fernjo-connections` (a plain `connections.json`, a `confCons.xml`, a backup) |
| `newer_schema` | `schemaVersion` above what this version understands |
| `invalid` | the nodes are not valid, too large, too many or nested too deeply |
