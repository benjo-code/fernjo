# Sync formats and rules

Format version **1** of the multi-device sync (milestone M9). This page is enough to write a compatible client or hub without the fernjo source (no lock-in), like `vault-format.md` and `backup-format.md`. Design and rationale: `Docs/superpowers/specs/2026-11-22-m09-multi-device-sync-design.md`. How to run the hub: `Docs/hub.md`. The code is `crates/fernjo-sync` (client, envelope, merge) and `crates/fernjo-hub` (hub); the hub deliberately does not depend on `fernjo-sync` and has its own copy of the header parser.

What a device syncs: the connection tree (`connections.json` content) and the `vault.fernjo` bytes, in one snapshot. What stays per device: `known_hosts` and `known_hosts.confirmed`, `rdp_certs.confirmed`, `vault-marks.json`, `settings.json`, window layout, the keychain auto-unlock item and the sync state files described below.

## Overview

- One **hub** stores one **envelope**, the head, plus the ten envelopes it replaced. It cannot read an envelope.
- An envelope is the **snapshot** (JSON) encrypted with XChaCha20-Poly1305 under a key derived from the **sync key**, a random 32-byte value that only the devices hold (OS keychain, pairing code).
- A device keeps a **base**: the last snapshot it was in step with. A sync run merges three states (local, head, base) and pushes the result with a compare-and-swap on a rising sequence number `seq`.

## Envelope

All integers are big endian. Maximum size of an envelope: **64 MiB** (67 108 864 bytes), header and tag included.

| Offset | Size | Field |
|---:|---:|---|
| 0 | 8 | magic, ASCII `FERNJOSY` |
| 8 | 2 | format version (`u16`), currently `1` |
| 10 | 16 | group id (UUID bytes) |
| 26 | 8 | `seq` (`u64`) |
| 34 | 24 | nonce (random per envelope) |
| 58 | n >= 16 | ciphertext of the snapshot JSON + 16 bytes tag |

Bytes 0..58 are the header. The smallest valid envelope is 74 bytes. That minimum is the envelope rule of a client; a hub only needs the first 34 bytes (magic `FERNJOSY`, version `1`, group id, `seq`) to decide, and accepts a body of exactly 34 bytes as long as the size limit holds, because it cannot verify the tag.

- **Key:** `HKDF-SHA256(salt = none, ikm = sync key (32 bytes), info = "fernjo-sync v1 snapshot")`, 32 bytes of output. The sync key itself is never used as a cipher key.
- **Encryption:** `ciphertext = XChaCha20-Poly1305(key, nonce, aad = bytes 0..34, plaintext = snapshot JSON)`. The AAD is magic, version, group id and `seq` (not the nonce, which is authenticated by being the nonce), so an envelope cannot be moved to another group or sequence number without failing authentication.
- **Version rules for a reader:** shorter than 74 bytes, longer than 64 MiB, a wrong magic or version `0` is "not an envelope". A version above `1` is "newer format" (refuse, tell the user to update). A tag that does not verify is "wrong key or damaged"; it cannot be told apart. After decrypting, the group id and `seq` of the header must equal those of the snapshot inside, and the group must be the device's own.
- **The hub** reads only group id and `seq` from the unauthenticated header (bytes 0..34) and accepts exactly version `1`.

## Snapshot (the plaintext)

UTF-8 JSON, camelCase, one object. A reader must reject a `formatVersion` above `1`.

```json
{
  "formatVersion": 1,
  "groupId": "0b0c0f4e-7a0e-4c7a-9a58-2e1d3f7c9b10",
  "seq": 7,
  "writer": { "id": "6f3c1b52-1c33-4a43-8b0e-0c8f2f5f1c11", "name": "gaming-pc" },
  "writtenAt": "2026-11-22",
  "connections": { "schemaVersion": 3, "children": [ ... ] },
  "vault": "RkVSTkpPVlQ..."
}
```

- `groupId`, `seq`: same as the envelope header.
- `writer`: the device that wrote it (`id` is a random UUID made when the device joined or set up, `name` the name the owner typed). Informational.
- `writtenAt`: informational label only. This build writes the writer's local calendar date (`YYYY-MM-DD`), not a timestamp; no rule reads it, and a reader must not parse it as a time.
- `connections`: the `connections.json` document as the writer's `connections.json` has it (see `Docs/DECISIONS.md`, "`connections.json`: format, location, write strategy"; schema 3 today). A newer schema, an invalid document or a tree deeper than 60 levels makes the snapshot invalid.
- `vault`: the `vault.fernjo` file exactly as on disk (`vault-format.md`), standard base64 with padding, or `null` when the writer has no vault. It stays encrypted by the master password's key; the envelope adds a second layer around it, not a replacement.

Limits a device applies to everything it takes from the hub, before anything is written locally: the envelope at most 64 MiB, at most **100 000** nodes, depth at most **60** (`MAX_TREE_DEPTH`), a vault part that parses (header, KDF cost within four times the defaults, vault file at most 64 MiB). A device that would have to push more than 64 MiB stops with `snapshot_too_large` before applying anything (the base64 growth of the vault counts).

## Hub HTTP API (protocol 1)

Every request carries `Authorization: Bearer <hub secret>`. The hub stores only the SHA-256 of the secret and compares in constant time. A missing or wrong secret is `401` (with `WWW-Authenticate: Bearer`) on both endpoints (the secret is checked inside the two handlers: another path answers `404`, another method `405`). The hub speaks plain HTTP and is meant to sit behind a TLS-terminating proxy; the client only talks HTTPS (plain `http://` only to the literal `127.0.0.1` and `[::1]`, for tests), accepts no user name, query (`?`) or fragment (`#`) in the address, never follows redirects, and trusts either the system roots or the certificates of one pinned PEM text from the pairing code (every certificate in it is trusted).

| Request | Answer |
|---|---|
| `GET /v1/info` | `200` `{"protocol":1,"maxBytes":67108864}` (JSON). A client should check `protocol`; this build only parses it. |
| `GET /v1/head` | `200` with the envelope (`application/octet-stream`) and `ETag: "<seq>"`, or `404` while the hub is empty. The `ETag` must equal the `seq` in the envelope header; a client refuses a head where they disagree. |
| `PUT /v1/head`, `If-None-Match: *` | first write into an empty hub: `201`. |
| `PUT /v1/head`, `If-Match: "<seq>"` | replaces the head if it is the one the writer saw: `204`. |

`PUT` errors (the precondition headers are checked before the body is read):

| Status | Meaning |
|---|---|
| `401` | bad or missing secret |
| `428` | neither `If-Match` nor `If-None-Match` sent |
| `400` | both sent, a malformed value (`If-None-Match` must be `*`, `If-Match` a quoted decimal), a body that is not an envelope of version 1, or an unreadable body |
| `413` | `Content-Length` above 64 MiB, or a body that grows past it (the read is aborted, the rest is never read) |
| `412` | the head is not the one expected (`If-Match` another `seq`, `If-None-Match` on a non-empty hub, `If-Match` on an empty hub). Nothing is written. The answer carries `ETag: "<current seq>"` when there is a head. |
| `409` | the head matches but the new `seq` is not higher than the current one, or the group differs from the head's |
| `500` | disk error; nothing is partly written |

Semantics a hub must keep: compare-and-swap under one lock (check and write are one step; run exactly one hub process per data directory); the new head is written to a random temp file, fsynced and renamed; the replaced envelope is first copied to `history/<seq>.env`; only the newest ten history files stay; a failed write leaves the head as it was. `seq` rises strictly; the first envelope of a group is `seq` 1 in this client. Nothing in a request body, header or secret may be logged.

On a `412` a client starts the run over from the new head; on `401` it shows a notice; on a connection or timeout failure (3 s connect, 20 s total) it is quietly "offline". A response is read in bounded steps: more than 64 MiB (or 4 KiB for `/v1/info`) is refused without buffering it.

## Pairing code

One pasteable string that holds everything a second device needs. It contains the hub secret and the sync key; it is shown only on explicit request and never logged.

```
fernjo-sync-1:<base64url>
```

- `fernjo-sync-` then the payload version as a decimal number without leading zeros (`1`), `:`, then base64url without padding of `JSON ++ checksum`.
- `checksum` = the first 4 bytes of SHA-256 of the JSON bytes, appended raw after the JSON.
- JSON (compact): `{"v":1,"url":"https://hub.example.lan","secret":"...","key":"<base64url of the 32-byte sync key>","group":"<uuid>","ca":"<PEM text>"}`. `ca` is left out when there is none. `url`, `secret` and `key` must not be empty.
- A reader ignores all whitespace and line breaks anywhere, refuses an input of more than 64 KiB (measured on the raw text, before whitespace is removed), a missing prefix, a version of `0` or with leading zeros, bad base64, a short or wrong checksum, JSON that does not parse, and a `url` that is not allowed (see the client rules above). A payload version above `1` is a code from a newer version (`newer_pairing_code`), everything else broken is `invalid_pairing_code`. Nothing is stored before every check passed.
- Size: a few hundred bytes without a CA, about 1.5 KB with Caddy's root certificate.

## Device-local files

Both live in the device folder (`<OS local data dir>/fernjo-device`, or `<FERNJO_DATA_DIR>/.device` for scratch runs), never in the synced data folder. They hold no secret. Writes are atomic (random `<name>.<hex>.tmp` opened with `create_new`, fsync, rename, directory fsync on Unix) and keep no `.bak`: both are rebuilt by the next sync. A read is capped at 64 MiB. A file that cannot be read, does not parse, is too large or has a `schema` above `1` is "state unusable" and is never overwritten.

`sync.json`:

```json
{
  "schema": 1,
  "url": "https://hub.example.lan",
  "caPem": "-----BEGIN CERTIFICATE-----...",
  "device": { "id": "<uuid>", "name": "gaming-pc" },
  "group": "<uuid>",
  "lastSeq": 7
}
```

`caPem` may be absent. `lastSeq` is the highest `seq` this device has seen or written; a save never lowers it for the same group. A head with a lower `seq` is refused (rollback protection, below).

`sync-base.json`: `{"schema":1,"snapshot":{ ...a snapshot as above... }}`. The base is the last state the device was in step with, in full (connections and vault file bytes). After a successful run it is the pushed or adopted snapshot. Order of writes after a push: `sync-base.json` first, then `sync.json`, so a crash in between never leaves a `lastSeq` ahead of the stored base.

## Keychain item

The hub secret and the sync key of a group are one item in the OS keychain (Windows Credential Manager with `Local` persistence, macOS Keychain, Secret Service): service `fernjo`, user `sync-<group id>`, content a small JSON object `{"hubSecret": <string>, "key": <base64 of the 32-byte sync key>}`. They are never written to a file, a log or an error text. Without a keychain, sync cannot be set up.

## Sync run

1. Read the local state (tree, vault file bytes, and while the vault is unlocked its entries and data key). No lock is held afterwards.
2. `GET /v1/head`. Unreachable: stop, "offline". Empty or a lower `seq` than `lastSeq`: stop with "hub rolled back" (below). The group must be the device's own.
3. Decrypt and validate the head (limits above), merge tree and vault.
4. Seal the result as `seq` = head `seq` + 1 (size rule decided here, before anything local changes).
5. Apply the merge locally. The local data must still be exactly what was merged (tree and vault file bytes compared under the locks) or the run starts over; a refusal of the local guards (`changed_on_disk`, `rolled_back`) ends the run.
6. `PUT` with `If-Match: "<head seq>"`. `412` starts over. After at most five attempts the run gives up until the next trigger.
7. Store `sync-base.json`, then `sync.json`.

A run is idempotent: if the app stops between steps 5 and 6, the next run merges again, finds the head's changes already present, and creates no second conflict copy and no duplicate node.

**Rollback protection.** The hub holds no key, so it can only withhold data or show an old head. A head with a `seq` below `lastSeq` (or an empty hub for a device that has synced before) is refused: nothing is merged or written. The owner can re-seed: the device pushes its local state with a `seq` above both `lastSeq` and the head's, after a confirmation. Honest limit: a hub that shows different devices different histories is not detected.

**Setup, join, leave.** Setup requires an empty hub (`hub_not_empty` otherwise), generates the sync key and the group id, stores the keychain item first, uploads `seq` 1, then writes the state. Join takes the pairing code, replaces the local connections (and the vault, when the head has one) with the head after the owner confirmed, sets the base to the head and `lastSeq` to its `seq`; a head without a vault leaves the local vault, which becomes the shared one with the next run. Leave removes the keychain item, `sync.json` and `sync-base.json`; local data stays. When the OS keychain refuses to delete the item, the state and the base are removed anyway and the app reports `keychain_unavailable`, naming the item (`fernjo` / `sync-<group id>`) to remove by hand. A lost device is handled by `fernjo-hub rotate-secret`; the sync key is not rotated in version 1.

## Merge rules

The merge is a pure function of its three inputs (base, local, head). Nodes and vault entries are matched by their UUIDs. No rule reads a clock: `writtenAt` and the date in a conflict copy's name are labels. "Touched" means "differs from the base".

### Connection tree

A node's fields are: name, description, host and protocol (connections), username, port, keepalive, credential reference, domain and the RDP settings, and its parent.

- **C1** Different nodes, or different fields of one node, changed on the two sides: both changes are kept.
- **C2** A field changed on one side only: that change is kept.
- **C3** A field changed to the same value on both sides: no conflict.
- **C4** A field changed to different values: the head's values stay on the node. The local version of that node becomes a new sibling right after it (fresh id), named `"<name> (conflict <device> <date>)"` where `<name>` is the local name, `<device>` the name of the device that merged and `<date>` a local calendar date `YYYY-MM-DD`, carrying all the local own fields of the node. A conflicted folder's copy is empty; its children stay where they are. One copy per conflicted node. The copy is pushed like any change and a notice names the original.
- **C5** A node deleted on one side and unchanged on the other, together with everything under it: it is deleted.
- **C6** A node deleted on one side but edited, or with something added or edited under it, on the other: it stays (an edit beats a delete), with the folders that lead to it; a folder kept only to hold something that the placement rules moved elsewhere is dropped again. The notice names it.
- **C7** Nodes added on either side are kept.
- **C8** Moves and order. A node moved on one side only is moved. Moved to different parents on both sides: the head's parent wins and the notice names the node. Sibling order changed on one side only: that order is taken, and the other side's additions are placed after their predecessor. Order changed on both sides of one folder: **the head's order wins** and local-only nodes are appended; the notice names the folder (a top-level reorder produces no notice).
- **C9** If the merged tree would contain a cycle or exceed depth 60, the local moves that cause it are undone (for a cycle every local move on it; for a too-deep chain the topmost local move on it first, repeated until it fits; the head's placement wins) and the notice names them. If no local move is to blame (a hostile or corrupt head) the run fails with `snapshot_invalid`, permanently until someone moves nodes.
- **C10** Credential references are plain fields and may dangle, as they do today.

Without a base node (the id is new on both sides) equal values agree and different ones conflict.

### Vault

The vault file counts as changed on a side when its first 150 bytes (the header, which includes the body nonce that is fresh on every save) differ from the base's.

- **V1** Only one side changed the vault file: that file is taken as it is, also while the vault is locked, and an unlocked vault stays unlocked with the new entries. A device without a vault takes the head's. A head without a vault leaves the local one, which is pushed. A local vault file that does not parse stops the run before anything is applied or pushed (`vault_unreadable`), even when the head would replace it.
- **V2** Both sides changed it: the vault must be unlocked; entries are matched by id and merged with the rules C1 to C7 (fields: name, username, domain, notes, and the secret as one unit). A conflict keeps the head's version and adds the local version as a new entry named `"<name> (conflict <device> <date>)"` right after its original; an edit beats a delete; additions are kept. The result is written under the held data key, entries in head order followed by local additions. While the vault is locked the whole run waits ("sync paused"): nothing is pulled or pushed in part.
- **V3** A master password change lives in the header. When both sides changed the vault, the head's header (password wrap, KDF cost) is kept; a local password change in the same window is not applied. The notice (`passwordChangeDropped`, naming the vault, no secret) appears when this device's password wrap (KDF cost, salt, wrapped key) differs from the base's and from the head's: it says that the password set here was replaced by the other device's and to change it again if a different one is wanted. The old password of this device does not work any more once the merged vault is installed.
- **V4** The two sides' vault ids differ (two vaults created independently) while both changed: nothing is merged, applied or pushed; the run stops with `vault_conflict_different_vaults` on every trigger until the owner resolves it. Tree sync is paused meanwhile. (If only one side changed, that side's file wins as in V1 even when the ids differ; a local vault replaced by another vault's file ends locked.)
- **V5** The data key never changes (a password change only re-wraps it), so every copy of one vault opens with the key a device holds. A merged save writes a generation above both the local file's and the head file's and above the device mark; the mark is never lowered and "accept older" is never used by sync. A head file whose generation is below the device mark is refused (`rolled_back`) when the vault is unlocked, and reported by the next unlock otherwise.

Findings (the notices) are a function of the three inputs: merging the same inputs again reports the same findings, and they disappear once the result has become the base.

## Errors and codes

Stable codes used by the client and the app: `bad_envelope`, `newer_format`, `wrong_key_or_damaged`, `wrong_group`, `snapshot_invalid`, `snapshot_too_large`, `hub_unreachable`, `hub_unauthorized`, `hub_tls_untrusted`, `hub_conflict` (412), `hub_rolled_back` (not a `SyncError`: the status code the app reports for an older or empty hub), `random_failed`, `hub_protocol`, `hub_too_large`, `invalid_hub_url`, `invalid_ca`, `invalid_pairing_code`, `newer_pairing_code`, `state_unusable`, `state_write_failed`, `keychain_unavailable`, `not_paired`, `already_paired`, `hub_not_empty`, `hub_empty`, `vault_conflict_different_vaults`, `local_changed`, and `local_failed` which carries the code of the local refusal (`changed_on_disk`, `rolled_back`, `vault_unreadable`, `vault_unusable`, `restart_required`, ...).
