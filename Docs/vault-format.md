# `vault.fernjo` file format

Format version **1**. The vault is one binary file in the data folder (`vault.fernjo`, plus `vault.fernjo.bak` from the previous save). This page is enough to write a reader without the fernjo source (no lock-in). Implementation: `crates/fernjo-vault`. The encrypted backup of Milestone 5 uses the same primitives in a separate, simpler container (`seal_with_password` in `crates/fernjo-vault`, no wrapped data key); it is described in [backup-format.md](backup-format.md).

## Primitives

- **Key derivation:** Argon2id (version 0x13), 32-byte output, parameters and salt from the header.
- **AEAD:** XChaCha20-Poly1305 (24-byte nonce, 16-byte tag appended to the ciphertext).
- **Master password:** Unicode-normalised to NFC, then used as UTF-8 bytes exactly as typed (no trimming, no case folding). Minimum length 8 characters (after normalisation), checked when a vault is created or its password is changed, never when unlocking.
- **Data key:** 32 random bytes generated once when the vault is created. It encrypts the body. The password-derived key only wraps the data key, so a password change never changes the data key or the entries. A password change picks a new salt and wrap nonce, re-wraps the unchanged data key and saves once; because the header changes and the whole header is the body's AAD, that save re-seals the body under the same data key with a fresh nonce.

- **Old copies keep the old password.** The data key never changes (so keychain items on other devices stay valid), and changing the master password does not re-encrypt or delete existing copies of the file. `vault.fernjo.bak` from just before the change, backups and the version history of a sync tool still open with the **previous** password, and the data key they yield also decrypts every later version of the vault. A leaked old password therefore protects older copies only as long as those copies exist. Real re-keying (a new data key, re-encrypting everything, invalidating keychain items) is not part of this format version.

## Layout

All integers are big endian.

| Offset | Size | Field |
|---:|---:|---|
| 0 | 8 | magic, ASCII `FERNJOVT` |
| 8 | 2 | format version (`u16`), currently `1` |
| 10 | 16 | vault id (UUID bytes), keys the OS keychain item |
| 26 | 4 | Argon2id memory in KiB |
| 30 | 4 | Argon2id iterations |
| 34 | 4 | Argon2id parallelism |
| 38 | 16 | Argon2id salt |
| 54 | 24 | nonce of the key wrap |
| 78 | 48 | wrapped data key: 32 bytes ciphertext + 16 bytes tag |
| 126 | 24 | nonce of the body |
| 150 | n >= 16 | body: JSON ciphertext + 16 bytes tag |

Bytes 0..150 are the **header**. The default parameters written by this build are 128 MiB, 6 iterations, 4 lanes.

## What is authenticated, what is encrypted

- **Key wrap:** `wrapped = XChaCha20-Poly1305(key = Argon2id(password, salt, params), nonce = wrap nonce, aad = header[0..78], plaintext = data key)`. The AAD is everything in front of the wrapped key (magic, version, id, KDF parameters, salt, wrap nonce).
- **Body:** `body = XChaCha20-Poly1305(key = data key, nonce = body nonce, aad = header[0..150], plaintext = JSON)`. The AAD is the **whole header**, so every header byte, including the wrapped key and the body nonce, is authenticated: changing any byte anywhere makes the file fail to open.
- **Encrypted:** the data key and the body. Everything in the header is readable without the password (vault id, KDF parameters, salt, nonces) and is not secret.
- **Nonces:** the body nonce is fresh random on every save (a save also re-encrypts the whole body). The wrap nonce is fresh whenever the key is wrapped (creation, password change).

A wrapped key that does not authenticate is reported as "wrong password" (it cannot be told apart from a damaged key). A body that does not authenticate after the key unwrapped is "tampered or corrupt".

## Body (after decryption)

UTF-8 JSON, `{ "entries": [ ... ] }`. A reader must ignore unknown fields. Secrets are stored byte-exact (no trimming or normalisation, line endings preserved). One entry:

```json
{
  "id": "5b0f6c1e-0c55-4d6e-9d57-3f1c1c0a7a10",
  "name": "web server",
  "username": "root",
  "domain": "EXAMPLE",
  "secret": { "type": "password", "password": "..." },
  "notes": "free text"
}
```

- `id` is a UUID string; `name` is non-empty. `username` and `domain` are omitted when not set. `notes` may be empty or missing (empty).
- `secret` is tagged by `type`: `"password"` with `password`, or `"privateKey"` with `key` (the key text as stored, for example PEM with its original line endings) and an optional `passphrase` (omitted when there is none).

## Version and bounds rules

- A file whose version is **greater** than the reader's is refused and never overwritten; version `0` and a wrong magic are "unreadable". The version is checked before the length, so a future, longer header is still reported as "newer format".
- The file must be at least 150 + 16 bytes.
- The file must be at most 64 MiB (`MAX_FILE_LEN`). A larger file is refused as `unreadable` without being read into memory (the length is checked and the read itself is bounded), the vault is then unusable and the file is never overwritten.
- KDF parameters are bounded when read, **before** any derivation, so a hostile synced file cannot demand gigabytes or minutes: parallelism 1..16, iterations 1..20, memory from `8 * parallelism` KiB up to 1 GiB (1 048 576 KiB).
- Writes are atomic: temporary file in the same folder, `sync_all`, copy of the current file to `.bak`, rename.
- **Lost-update guard:** a handle remembers the header bytes (0..150) it last read or wrote. The body nonce changes on every save, so the header identifies the version of the file. Before a save the file on disk is read again: if it is missing the save recreates it; otherwise it must start with exactly those header bytes. A file that cannot be parsed, is empty or shorter than a header, is a newer format or has any other header was written by someone else (another device, a sync tool) and is **not** overwritten: the save fails with `changed_on_disk` and the in-memory entries stay as they were. Locking and unlocking re-reads the file and picks the other version up. This is a check followed by a write, not a lock: a sync tool can still win the race in the instant between the check and the rename.

## Reading the header by hand

```sh
xxd -l 150 vault.fernjo
# bytes 0-7 "FERNJOVT", 8-9 version (0001), 10-25 vault id,
# 26-29 memory KiB (00020000 = 131072), 30-33 iterations (00000006), 34-37 parallelism (00000004)
```

To decrypt: take the password, NFC-normalise it, derive the key with Argon2id from the parameters and salt at offsets 26..54, open the 48 bytes at 78 with the wrap nonce at 54 and AAD `header[0..78]` to get the data key, then open the bytes from offset 150 to the end with the body nonce at 126 and AAD `header[0..150]`.
