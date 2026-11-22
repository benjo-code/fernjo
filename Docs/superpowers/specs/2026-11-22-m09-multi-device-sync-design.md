# M9 Multi-device sync: design

Status: draft for owner approval, 2026-11-22. Scope, tasks and acceptance criteria are those of Milestone 9 in `Docs/roadmap.json`. This note answers what that milestone leaves open: what syncs, the mechanism, the conflict rules and the threat model. The plan follows from it. The research behind the mechanism choice is summarised under "Rejected alternatives".

## Intent

The owner uses one device at a time (gaming PC, CachyOS laptop, macOS laptop) and wants the connection tree and the vault entries to follow them, without copying files and without a sync tool in between. fernjo does the sync itself. The owner runs one small program, the **hub**, on an always-on Ubuntu or Debian VM under rootless Podman, behind the Caddy they already run (`tls internal`, domain `benjo.loc`). The hub is reachable only inside the home network. Away from home every device keeps working on its own data and catches up the next time it runs inside the network.

## Goals, non-goals, anti-goals

- Goals: the five tasks and five acceptance criteria of M9 as written in the roadmap; offline-first behaviour (saving never waits for the hub); no edit lost silently; end-to-end encryption so that the hub VM, its backups and the Caddy logs hold nothing readable.
- Non-goals (v1): reaching the hub from outside the home network (the URL is configurable, so a WireGuard or Tailscale address would work without code changes, but it is neither built nor tested); per-device revocation (a lost device is handled by rotating the hub secret); rotating the sync key; syncing `known_hosts`, `settings.json`, theme, terminal preferences or window layout; a UI for restoring older hub versions; padding snapshot sizes; detecting a hub that shows different devices different histories; publishing a container image; background sync while fernjo is not running; hub and sync for more than one owner.
- Anti-goals: a sync that resolves a conflict by silently dropping one side; a hub that has to be trusted with plaintext; a second, parallel store of secrets outside the vault's own encryption; weakening any existing guard to make sync easier.

## Constraints

- Secrets stay in Rust. The hub secret and the sync key live in the OS keychain (`KeyStore`, new item kind); neither appears in a response, a log or `connections.json`. The one exception is the explicit "Show pairing code" action, which is a reveal (see decision 5).
- No home-made crypto: XChaCha20-Poly1305 for the snapshot envelope, HKDF-SHA256 from RustCrypto to derive its key from the sync key, TLS through rustls. The vault's own encryption is not touched.
- `fernjo-sync` and `fernjo-hub` never depend on Tauri and are testable without the UI. Each crate is created in this milestone.
- The existing guards keep working and keep their tests: the `connections.json` fingerprint check, `Vault::save` header guard, the vault `generation` rollback protection, `known_hosts.confirmed` and `rdp_certs.confirmed`. Sync writes through the same `Store` and `Vault` APIs as every other writer, so the guards see it.
- New dependencies (HTTP client and server, HKDF) must pass `cargo deny check`. The client uses rustls with ring, no OpenSSL.
- Format and load limits already in force apply to everything adopted from the hub: 64 MiB, 100 000 nodes, depth 60 (`MAX_TREE_DEPTH`), the vault's KDF bounds.
- This machine has no second device and no Caddy. The unit and integration tests are mine; the two-device run is the owner's, on named devices.

## Approaches

- **A. Snapshots plus a small `fernjo-hub` (picked, owner said "your recommendation").** One encrypted snapshot of connections and vault is the shared state; devices merge by comparing three states.
- B. Operation log with the same hub. Finer merges, but log compaction, per-operation checks and a harder rollback story; not worth it for KB-sized data used by one device at a time.
- C. Snapshots on an existing S3 server (Garage, MinIO) through OpenDAL. No hub to write, but a large dependency, access keys to manage, and conditional-write behaviour I could not verify.

*Change from the sketch the owner saw:* it said one blob per device. Because the hub is our own program and the owner runs it, a single shared head with compare-and-swap is simpler and gives the same safety for this threat model (see decision 3). The roadmap needs one test per conflict rule, and this keeps the number of states small.

## Decisions

### 1. What syncs and what stays on the device (my call, except where marked)

- Syncs: the whole connection tree (names, descriptions, settings including inherited ones, credential references) and the vault file with all entries. Both travel in one snapshot.
- Stays per device: `known_hosts.confirmed`, `rdp_certs.confirmed`, `vault-marks.json`, the sync state itself, the keychain auto-unlock item, the data folder choice, `settings.json` (theme, terminal preferences), window layout, and the data folder's `known_hosts` file. Every host therefore still asks once per device (first use on that device), exactly as today.
- fernjo's sync and a file-sync tool on the same data folder must not be combined. The existing `find_conflicts` warning stays for people who still use one.

### 2. Snapshot, envelope and the hub API (my call)

- **Plaintext snapshot (JSON):** `formatVersion`, `groupId`, `seq`, the writer's device name and id, `writtenAt`, `connections` (the `connections.json` document, current schema), `vault` (absent, or the `vault.fernjo` bytes exactly as on disk, base64). The vault part stays encrypted by the master password's key even inside the envelope, so the hub copy is not weaker than the vault file.
- **Envelope:** magic `FERNJOSY`, version, group id, `seq`, a 24-byte random nonce, ciphertext. Key: HKDF-SHA256 of the 32-byte sync key with a fixed info string. AAD: the header fields up to the nonce, so a head cannot be moved to another group or sequence number. The byte layout is written down in `Docs/sync-format.md` (no lock-in, like the vault and backup formats).
- **Hub API (HTTP behind Caddy, `Authorization: Bearer <hub secret>`):** `GET /v1/head` returns the newest envelope with its `seq` as the ETag, or 404 when empty. `PUT /v1/head` with `If-Match: "<seq>"` (or `If-None-Match: *` for the first write) stores a new envelope whose `seq` must be higher than the current one; a mismatch is 412 and nothing is written. `GET /v1/info` returns the protocol version and the size limit. The hub keeps the last 10 envelopes next to the head, for the owner's own rescue; there is no UI for them. The size limit is 64 MiB.
- **Hub program:** `fernjo-hub init` creates the data folder and prints the hub secret once (only its SHA-256 is stored); `serve` listens on a loopback address and port; `rotate-secret` prints a new one. Writes are atomic with fsync, one process-wide lock around compare-and-swap, constant-time secret comparison, no request body or secret in logs. It runs as a non-root user from a Containerfile and a Quadlet unit; a Caddy snippet (`reverse_proxy 127.0.0.1:<port>`) is part of `Docs/hub.md`. The hub never sees the sync key.
- **Client transport:** HTTPS only. Plain `http://` is refused except for loopback (tests). TLS trusts the system roots, or only the pinned CA certificate when the pairing code carries one (Caddy's root certificate for `tls internal`), so the CA need not be installed in each operating system.

### 3. Sync run and merge (my call)

- **Base:** each device keeps the last snapshot it synced in full, not as hashes (connections as JSON, vault file bytes), and the `seq` it was at, in the device-local folder. Merging compares local, head and base. The "changes waiting" count and the "synced" status are comparisons with the base. A merge never runs without a base: first setup refuses a hub that already has a head (`hub_not_empty`, use Join), Join sets the base to the head it took, and a re-seed sets it to what it pushed.
- **Identity and comparison:** nodes and vault entries are matched by their UUIDs, which are stable (Duplicate and import assign fresh ids, so a copy never collides). No rule reads a timestamp: `writtenAt` and the date in a conflict copy's name are labels from the local clock and never decide anything. "Touched" in C6 means "differs from the base". A vault file counts as changed when its header bytes (offsets 0 to 150) differ from the base's; the header includes the body nonce, which is fresh on every save, so every save, including an entry edit, changes it.
- **Size:** the hub limit applies to the envelope as sent. The client checks the size before the upload and stops with `snapshot_too_large` and a notice; the base64 growth of the vault bytes counts.
- **Run:** read local state under the locks, release them, fetch the head (outside the locks), merge, then take the locks again, check that local state is still what was merged (otherwise start over, at most a few times), apply through `Store` and `Vault` (so the fingerprint and header guards run), then `PUT` with `If-Match`. A 412 starts over. A `changed_on_disk` from a foreign writer ends the run and retries later.
- **Triggers:** after start (once the vault auto-unlock was tried), five minutes after the last run, window focus, a debounced few seconds after a local save, "Sync now", and a short best-effort push when the window closes. An unreachable hub (connect timeout of a few seconds) is the quiet state "offline, N changes waiting", never an error bar. Real errors (wrong secret, certificate not trusted, unknown protocol, rollback, an unreadable head) show a notice.
- **Rollback and replay of the head:** the device-local sync state keeps the highest `seq` seen. A head with a lower `seq` (for example the hub VM was restored from an older snapshot, or the hub folder is empty) is refused with `hub_rolled_back`; nothing is merged or written. The notice offers "Re-seed hub from this device", which pushes the local state with a higher `seq` after the owner confirms. This also rebuilds a lost hub from any device. A forged or damaged envelope fails authentication and is refused like a damaged backup.
- **Honest limit:** a hub could show different devices different histories or hold back the newest head. The first is not detected and the second only delays. The hub is the owner's own VM; the design does not defend against it withholding data, only against it reading or forging data.
- **Convergence:** the merged result becomes the new head, so devices converge; the head's version keeps the original id and place whenever the rules below make a choice.

**Rules for the connection tree** (nodes are matched by id; a field is name, description, host, protocol, parent, or one setting):

- C1 Different nodes or different fields changed on the two sides: both changes are kept.
- C2 A field changed on one side only: that change is kept.
- C3 A field changed to the same value on both sides: no conflict.
- C4 A field changed to different values: the head's value stays on the node; the local version becomes a new sibling item named "<name> (conflict <device> <date>)" carrying the local values of that node's own fields (a conflicted folder copy is empty; its children stay where they are). The copy is pushed like any other change, and a notice names the items.
- C5 A node deleted on one side and unchanged on the other (including everything under it): it is deleted.
- C6 A node deleted on one side but edited, or with something added or edited under it, on the other: it stays (an edit beats a delete), with the folders that lead to it, and the notice says so.
- C7 Nodes added on either side are kept.
- C8 Moves and order: a node moved on one side only is moved; moved to different parents on both sides, the head's parent wins and the notice names the node; sibling order changed on one side is taken, on both sides the head's order wins and local-only nodes are appended.
- C9 If the merged tree would contain a cycle or exceed depth 60, the local moves that cause it are undone (the head's placement wins) and the notice names them.
- C10 Credential references are plain fields and may dangle, as they do today.

**Rules for the vault** (entries matched by id; a field is name, username, domain, notes, and the secret as one unit):

- V1 Only one side changed the vault file (its header differs from the base): that file is taken as it is. No unlock is needed, so a locked vault still syncs in this case. The existing rollback check on the next unlock still applies to the adopted file.
- V2 Both sides changed it: the vault must be unlocked on this device; entries are merged with rules C1 to C7, with "(conflict …)" copies as new entries. If the vault is locked, the whole run waits ("Sync paused: unlock the vault to merge"); nothing is pushed or pulled in part. The merged vault is saved through the normal `Vault` path, which raises the generation.
- V3 A master password change is part of the file's header. When both sides changed the vault, the head's header (password wrap) is kept; a local password change in the same window is not applied and the notice says to change the password again.
- V4 The two sides' vault ids differ (two vaults created independently): not merged; see decision 4.
- V5 Generation and rollback: the data key never changes (a password change only re-wraps it), so every copy of one vault opens with the key a device already holds, and an unlocked vault stays unlocked and its keychain item stays valid when a file is adopted. Adoption while unlocked re-reads the entries with the held key; while locked the vault stays locked. A merged save (V2) writes a generation above both the local and the head file's, so the result is newer than either; this also closes the documented gap where two devices each save once from the same base. The device mark is never lowered and "accept older" is never used by sync. In ordinary use the head's generation is not below the mark (the head descends from the base this device saw); if it ever is, the next unlock reports `rolled_back` as it does today, and a head older than what the device saw is normally caught first by the `seq` check.

### 4. Pairing, first setup and leaving (my call)

- **Set up the first device:** Settings, Sync: hub URL, hub secret (from `fernjo-hub init`), optional CA certificate (PEM), a device name (default: host name). fernjo generates the sync key and a group id, checks the connection, and uploads the first snapshot. Setup needs the OS keychain; without one fernjo says sync is unavailable (`keychain_unavailable`) and does not store the secrets anywhere else.
- **Add a device:** on a synced device, "Show pairing code" produces one copy-paste string (URL, hub secret, sync key, group id, optional CA certificate; about 1.5 KB, base64url, with a checksum so a truncated paste is caught). On the new device, "Join" takes the code and a device name. A short human code or a QR scan is a possible later improvement, not v1.
- **Joining replaces.** Local connections and vault on the joining device are replaced by the head, after a confirmation that names what is lost (counts) and offers the encrypted backup first (the `FJBK` export that exists). Merging two histories that never shared a base is not attempted. If the head has no vault yet the device keeps its own vault and it becomes the shared one with the next run.
- **Lost or stolen device:** `fernjo-hub rotate-secret` locks that device out of the hub; the other devices are re-paired with a new code ("Edit hub secret" on each, or leave and join again). The sync key stays; rotating it means leaving sync everywhere and setting it up again.
- **Leave sync:** removes the sync state, keychain items and base; local data stays as it is.
- **Recovery:** the sync key lives only in the keychains and pairing codes, never on the hub. If every device is lost, the hub data cannot be read. The encrypted backup (`FJBK`) and the master password remain the independent recovery path.

### 5. What changes in the app (my call)

- New commands (each counts in the three-places rule and has a `table()` entry): `sync_status`, `sync_setup`, `sync_pairing_code`, `sync_join`, `sync_now`, `sync_reseed`, `sync_leave`, `sync_edit_secret`. `sync_pairing_code` returns a secret by design, like vault reveal, and is the one deliberate exception in the no-secret test; no other sync response, error or `Debug` output may contain the hub secret or the sync key.
- `fernjo-vault` gains the smallest API sync needs: read the file bytes, adopt validated bytes (same header guard, same atomic write, KDF bounds checked first, vault id must match), and open a second handle from bytes with the data key for a merge.
- UI: a "Sync" section in Settings (set up, join, show pairing code, status, sync now, edit secret, re-seed, leave) and a small status line in the sidebar footer ("Synced 12:03", "Offline, 3 changes waiting", "Sync paused: unlock the vault"). The `changed_on_disk` bar and its Reload button also appear for a vault `delete_credential` and an import `apply_backup` (backlog item, task 3), and the vault open sweeps `vault.fernjo.<hex>.tmp` files older than a day (task 4). Notices about conflict copies and kept edits use the existing notice bar.
- Documents: `Docs/sync-format.md`, `Docs/hub.md` (set-up, Quadlet, Caddy, rotate, rebuild), a `Docs/DECISIONS.md` entry for the mechanism and the rejected approaches, a line in main spec section 5.4 that built-in sync now exists, and the `CLAUDE.md` pointers (device-local files, new commands, new crates).

### 6. Threat model (my call)

Assets: connection data (host names, user names), vault secrets, the hub secret and the sync key.

| Adversary | What stops them |
|---|---|
| Another host on the home network | TLS with the pinned CA, hub secret; the envelope is useless without the sync key |
| Someone with the hub VM, its disk or its backups | Sees sizes, timing and `seq` only; vault entries are still under the master password's key; cannot forge a head (authenticated); an older head is refused by devices that have seen a newer one |
| A stolen powered-off device | Out of scope for sync (its local data was already there); vault secrets need the master password; `rotate-secret` locks it out |
| A hostile or corrupt head | Authentication, then the same caps and validators as restore and import before anything is written; `Store`/`Vault` guards still apply |
| A compromised paired device | Can write a bad head; the hub's last 10 versions and each device's `.bak` are the rescue |
| Hub withholds or splits history | Not defended (see "Honest limit") |

## Rejected alternatives (mechanism research, 2026-11-22)

- **No stored intermediary** (iroh, BitTorrent DHT, Hypercore, p2panda, Tailscale Taildrop, magic wormhole, QR or USB): every one needs two devices online together or a holder someone runs; the owner is never on two devices at once.
- **Free public networks as the only copy** (Nostr relays, Veilid, Blossom): no retention guarantee for a credential vault.
- **Hosted local-first services and other products** (Anytype, Jazz, Evolu, Etebase, CloudKit): lock-in, account or licence problems, or no fit for our merge needs.
- **A user-owned cloud bucket or drive folder** (R2/S3, Dropbox, OneDrive, GitHub): viable and close to this design, but the owner chose to self-host, and a stale-write refusal could not be verified without a spike. The snapshot format would carry over unchanged.
- **A sync tool on the data folder** (Syncthing, Nextcloud, iCloud): excluded by the owner; it stays possible but unsupported.
- **CRDTs and timestamp-wins merges:** the first is overkill for this data, the second resolves conflicts silently.

## Verification

- One automated test per rule C1 to C10 and V1 to V4 (merge as a pure function over snapshots), plus tests for: envelope round trip and tamper rejection, `seq` rollback refusal and re-seed, compare-and-swap in the hub (stale `If-Match`, first write, size limit, wrong secret, restart keeps the head and history), an end-to-end run of two clients against a hub on loopback (including offline then online), a repeated merge after a crash between "apply" and "push" that changes nothing (no second conflict copy), `snapshot_too_large`, the no-secret table for the new commands, and the existing guard tests unchanged.
- Manual, owner: two real devices from {gaming PC, CachyOS laptop, macOS 15 laptop} against the hub VM behind Caddy `tls internal` on `benjo.loc`. Create and edit a connection on one and see it on the other; add a vault entry and use it on the other; disconnect one device from the network, edit on both, reconnect, and see the rules play out (one conflict copy, one delete-versus-edit case); check that the hub restart and a Caddy restart lose nothing. The evidence names the devices and the hub setup.

## Left to the builder

- Crate choice for the hub server and the client (kept to what `cargo deny` accepts), the HKDF crate, the exact envelope byte layout, JSON field names of the snapshot.
- How the merge reads the tree and entries (a pure function in `fernjo-sync` over `fernjo-core` and `fernjo-vault` types is expected) and how "touched" is computed for C6.
- Retry limits, debounce times, the offline retry cadence, and whether notices are kept across a restart.
- How a second vault handle is built for V2, and the exact adoption API in `fernjo-vault`.
- The Containerfile base image, the Quadlet unit and the Caddy snippet, as long as the hub runs rootless and read-only except its data volume.
