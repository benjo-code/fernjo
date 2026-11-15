# `vault.fernjo` file format

Format version **1**. The vault is one binary file in the data folder (`vault.fernjo`, plus `vault.fernjo.bak` from the previous save). This page is enough to write a reader without the fernjo source (no lock-in). Implementation: `crates/fernjo-vault`.

## Primitives

- **Key derivation:** Argon2id (version 0x13), 32-byte output, parameters and salt from the header.
- **AEAD:** XChaCha20-Poly1305 (24-byte nonce, 16-byte tag appended to the ciphertext).
- **Master password:** Unicode-normalised to NFC, then used as UTF-8 bytes exactly as typed (no trimming, no case folding). Minimum length 8 characters (after normalisation), checked when a vault is created or its password is changed, never when unlocking.
- **Data key:** 32 random bytes generated once when the vault is created. It encrypts the body. The password-derived key only wraps the data key, so a password change re-wraps 32 bytes.

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

UTF-8 JSON, `{ "entries": [ ... ] }`. Entry fields are described when entries land (milestone 3, task 2); a reader must ignore unknown fields.

## Version and bounds rules

- A file whose version is **greater** than the reader's is refused and never overwritten; version `0` and a wrong magic are "unreadable". The version is checked before the length, so a future, longer header is still reported as "newer format".
- The file must be at least 150 + 16 bytes.
- KDF parameters are bounded when read, **before** any derivation, so a hostile synced file cannot demand gigabytes or minutes: parallelism 1..16, iterations 1..20, memory from `8 * parallelism` KiB up to 1 GiB (1 048 576 KiB).
- Writes are atomic: temporary file in the same folder, `sync_all`, copy of the current file to `.bak`, rename.

## Reading the header by hand

```sh
xxd -l 150 vault.fernjo
# bytes 0-7 "FERNJOVT", 8-9 version (0001), 10-25 vault id,
# 26-29 memory KiB (00020000 = 131072), 30-33 iterations (00000006), 34-37 parallelism (00000004)
```

To decrypt: take the password, NFC-normalise it, derive the key with Argon2id from the parameters and salt at offsets 26..54, open the 48 bytes at 78 with the wrap nonce at 54 and AAD `header[0..78]` to get the data key, then open the bytes from offset 150 to the end with the body nonce at 126 and AAD `header[0..150]`.
