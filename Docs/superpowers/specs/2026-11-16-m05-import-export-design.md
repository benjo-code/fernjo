# fernjo – Milestone 5 design: mRemoteNG import, export and backup

**Status:** approved by the owner on 2026-11-16 (including the builder calls; the owner runs stable mRemoteNG 1.76, so ConfVersion 2.6 is the target) · **Date:** 2026-11-16 · **Parent spec:** [2026-11-15-fernjo-design.md](2026-11-15-fernjo-design.md) (§5.5, §6, §7) · **Visual language:** `DESIGN.md` · **Roadmap:** M5

This document records the decisions for M5 only. Intent, goals, non-goals and anti-goals are those of the parent spec. Decisions the owner made in conversation are marked *(owner)*; everything else is the builder's call with its reason.

## 1. Intent

After M5 the owner points fernjo at the existing mRemoteNG `confCons.xml`, sees what will happen, and gets the whole tree and every saved password into fernjo in one step. fernjo data can also leave the app in two ways (connections as JSON, everything as an encrypted backup) and come back, so the data format is no lock-in (parent spec §2, anti-goal "Lock-in").

## 2. Scope

**In:** the new crate `fernjo-import` (mRemoteNG parser and decryptor, mapping, import plan and report, fernjo JSON export and import, encrypted backup); small additions to `fernjo-core` (graft a subtree, replace the tree) and `fernjo-vault` (a password-sealed blob, export and replace all entries); the Tauri commands and a pending-operation slot in `src-tauri`; an "Import & backup" dialog and an import offer on the empty tree; format documentation (`Docs/backup-format.md`, a mapping table in `Docs/DECISIONS.md`).

**Out (backlog, unchanged):** export to mRemoteNG format; legacy mRemoteNG files (ConfVersion below 2.6, AES-CBC); mRemoteNG's pre-release ConfVersion 2.7 and 2.8 (they keep credentials in a separate credential repository and have no `Password` attribute; the importer says so and stops); protocols other than SSH2 and RDP; anything RDP-specific beyond host, port and credentials (the model has no RDP settings before M8); private keys and PuTTY sessions (mRemoteNG SSH uses PuTTY; there is nothing to import into the vault except passwords); built-in sync.

**Anti-goals:** a password, the file key or the export password in a log, an IPC response, an error text, `Debug` output, `connections.json` or webview storage; a half-applied import or restore (a failure leaves tree and vault as they were); a restore that wipes data without a confirmation that names what will be lost; an import that silently drops something (every skipped or partly mapped item is in the report).

## 3. Approaches

| Question | Chosen | Rejected |
|---|---|---|
| Restore semantics *(owner: accepted the recommendation)* | **Full backup replaces** the tree and all vault entries after a confirmation (master password and data key stay; `.bak` files stay). **Connections-only JSON merges** into a new top-level folder with fresh ids. | Merging a full backup (ids change, no identical round trip); replacing from the JSON (it carries no secrets and its credential ids mean nothing on another device). |
| How import reaches the data *(owner: accepted)* | **Pure plan, then apply.** `fernjo-import` does no I/O on app data: it parses and returns a plan (folder tree with fresh ids, vault entries to create, report). `src-tauri` applies it: vault entries first, then the tree, rolling the entries back if the tree save fails. | Parsing straight into the store (a failure half-way leaves partial data; parser tests need a data folder). |
| Backup container *(owner: accepted)* | **Own format `FJBK`:** Argon2id and XChaCha20-Poly1305 with a random salt, through a small new public function in `fernjo-vault` (the existing audited code), body is versioned JSON. The export password is separate from the master password. | Reusing the vault file format (couples the two versions; an export password that is secretly another vault's master password). |
| Where secrets wait between steps | **In Rust**, in one pending slot in `AppState` (see §6). The UI only gets counts and report lines. | Sending the decrypted plan to the UI and back (breaks "secrets stay in Rust"). |
| Choosing files | **Rust opens the native dialog** (`tauri-plugin-dialog`, Rust API, like the data-folder dialog). The webview never passes a path, so a compromised page cannot make Rust read or write an arbitrary file. | A path typed or sent by the UI. |
| XML and crypto crates | `roxmltree` (read-only DOM, no external entities), RustCrypto `aes-gcm`, `pbkdf2` with `sha1`, `base64`. Exact versions go to `Docs/DECISIONS.md` when chosen; `cargo deny check` must stay green. | A hand-written XML scanner; BouncyCastle-style bindings. |

## 4. `fernjo-import` crate

Depends on `fernjo-core` and `fernjo-vault` (for `NewEntry` and `SecretString`), not on Tauri. Created in this milestone.

### 4.1 Reading `confCons.xml`

- **File checks first:** at most 32 MiB; UTF-8 (BOM allowed); the root must be `Connections` with `ConfVersion` and `Protected`. Container nesting is limited to 64 levels and the node count to 100 000, so a hostile file cannot exhaust the stack or memory. (Corrected after the final review: containers are limited to 59 levels so the import folder fits into the core limit of 60 tree levels, and the total PBKDF2 work is capped at 20 000 000 iterations, see `Docs/DECISIONS.md` "M5 final review fixes".) A file that is not XML (the pre-2.6 whole-file encrypted form) or has `ConfVersion` below 2.6 or above 2.6 gives the typed error `unsupported_format` with a message naming what was found and where legacy files stand (backlog).
- **Encryption (verified against mRemoteNG v1.76.20 `AeadCryptographyProvider`, `Pkcs5S2KeyGenerator`, `XmlConnectionsDecryptor`):** attribute `EncryptionEngine` must be `AES` and `BlockCipherMode` `GCM` (anything else is `unsupported_format`). A secret is `base64( salt[16] ‖ nonce[16] ‖ ciphertext ‖ tag[16] )`. Key: PBKDF2-HMAC-SHA1 over the password's UTF-8 bytes, the salt, `KdfIterations` iterations (root attribute, at least 1000 as mRemoteNG enforces; the importer also caps it at 1 000 000), 32 bytes. AES-256-GCM with the 16-byte nonce and the 16-byte salt as associated data. The default password is `mR3m`.
- **Which key:** the root `Protected` attribute decrypts to `ThisIsNotProtected` with the right key. The importer tries `mR3m`; on failure it reports `key_needed`, and with a typed key that also fails, `wrong_key`. A decryption error on a single node's `Password` is not fatal: that node imports without a password and the report names it.
- **Full-file encryption:** when `FullFileEncryption="true"` the root element's text is one encrypted secret whose plaintext is the inner XML (the `Node` elements); decrypt it, then parse as usual.
- **Which version the owner's file has (assumption to verify):** the parent spec says the owner runs mRemoteNG 1.76 or newer; the stable 1.76 line writes `ConfVersion` 2.6, so 2.6 is the supported target. The fixture task is a gate for the acceptance criteria, not for the parser: the parser is built and tested on the synthetic vectors first. When the fixture arrives its first check is the version; if it is not 2.6, work stops and the milestone gets a scope change in the roadmap (support that version, or move it to the backlog) before anything else is built *(builder's call, from the review)*.
- **Fixtures:** (a) the owner's sanitised file *(owner provides it, the first task of the milestone)*; (b) synthetic files for default key, custom key, full-file encryption and each error, whose known-answer ciphertexts are produced by an **independent** implementation, a small file-based .NET script `tools/mremoteng-vectors.cs` using BouncyCastle exactly as mRemoteNG does, so the Rust decryptor is not only tested against itself. The XSD `mremoteng_confcons_v2_6.xsd` from mRemoteNG is used to check that synthetic files have the real shape (read, not copied).

### 4.2 Mapping

| mRemoteNG | fernjo |
|---|---|
| Root | not imported; its children go into the new top-level folder **"Imported from mRemoteNG (YYYY-MM-DD)"** (local date; a second import on the same day gets " (2)", " (3)"…) |
| `Type="Container"` | folder (kept even if every child is skipped) |
| `Type="Connection"`, `Protocol="SSH2"` / `"RDP"` | SSH / RDP connection |
| `Name`, `Descr`, `Hostname`, document order | name, description, host, manual order. `Id`, `Expanded`, icons, panels are not kept (new ids) |
| `Port`, `Inherit Port` | `settings.port`: `None` when the flag is true, else the value |
| `Username`, `InheritUsername` | `settings.username`: `None` when the flag is true or the value is empty, else the value |
| `Password`, `InheritPassword`, `Domain`, `InheritDomain` | when `InheritPassword` is false and the decrypted password is not empty: one **vault entry** (kind password; `username` and `domain` of the entry are those of the node when their inherit flag is false and the value is not empty) referenced as `settings.credential`; entries with the same (username, domain, password) are **one shared entry**. Inherited password: nothing; fernjo inheritance resolves it through the folder |
| Other protocols (`SSH1`, `VNC`, `Telnet`, `Rlogin`, `RAW`, `HTTP(S)`, `ICA`, `IntApp`) | **skipped**, listed with path and protocol |

- **Inheritance is mapped faithfully, not simplified:** a flag decides local or inherit per setting, folders included. mRemoteNG writes defaults on every container (for example port 3389); a child that inherits the port gets exactly what it got in mRemoteNG. The mixed case "password own, username inherited" falls out naturally (entry without username, node username unset).
- **Empty own values (builder's call, from the review):** mRemoteNG with `InheritPassword="false"` and an empty password means "no password" (it prompts). fernjo cannot say that: an unset setting always inherits, so the connection would pick up a password from a folder above it. The importer therefore resolves the plan after building it and lists every connection whose own empty password or username now inherits a different value than mRemoteNG used, naming the folder it inherits from; the owner can fix those few by hand. No empty-secret entries are invented.
- **Partly mapped (listed in the report):** a domain that has no password to travel with (fernjo has no domain setting before M8); a non-empty `PuttySession` other than `Default Settings`; for RDP connections, a summary line that display, redirection and gateway settings are not carried. Per-file aggregate lines are used instead of one line per node for things that apply to most nodes, so the report stays readable.
- **Entry names** (builder's call): `<username>` or `<username>@<domain>`, with ` (2)` etc. when a name is taken; entries with no username are named after the first node that uses them.
- **Report:** counts (folders, SSH, RDP, credentials created, credentials shared), then three lists (skipped, partly mapped, problems), each capped at 200 lines with "and N more". The report contains names, paths and protocols, never a password.
- **Plan shape (not signatures):** `ImportPlan { folder: Node, entries: Vec<PlannedEntry>, report: ImportReport }` where the tree's `settings.credential` values are ids chosen at plan time and each `PlannedEntry` carries that id and a `NewEntry`, so apply needs no remapping (this needs `Vault::add_with_id`, §5).

### 4.3 fernjo connections JSON

- File `fernjo-connections-YYYY-MM-DD.json`: `{ "format": "fernjo-connections", "exportedAt", "schemaVersion", "children": [ …nodes as in connections.json… ] }`. **No secrets and no credential references** (the export drops every `credential`; the count of dropped references comes back after the file is written and is shown in the done step). Human-readable, documented in `Docs/backup-format.md`.
- Import validates through the same node rules as loading `connections.json` (no rule is duplicated), assigns fresh ids and grafts the result into **"Imported from fernjo export (YYYY-MM-DD)"**. A newer `schemaVersion` is refused with a message. (Corrected after implementation: the first draft said `"version": 1`; the file carries the `connections.json` `schemaVersion`, so there is one version number, not two.)

### 4.4 Encrypted backup

- File `fernjo-backup-YYYY-MM-DD.fjbk`: `magic "FERNJOBK" ‖ version u16 BE ‖ KDF params ‖ salt ‖ nonce ‖ ciphertext(JSON) ‖ tag`, key from the export password by Argon2id with `KdfParams::DEFAULT`, the header authenticated as associated data. Parameters read from a file are bounded like the vault's (a hostile file cannot demand gigabytes).
- Body JSON: `{ "schemaVersion": 1, "createdAt", "app": "<version>", "connections": <the connections.json document>, "entries": [ { id, name, username, domain, secret, notes } ] }`. Entry ids and credential references are preserved, so a restore is identical.
- The export password is at least 8 characters (enforced in Rust, NFC-normalised like the master password, typed twice in the UI) and never stored. A failed authentication cannot be told from a modified file (code `wrong_password`, message "wrong password or the file is damaged"); `damaged` is only for structural problems (truncated file, KDF parameters out of range, invalid body). (Corrected after implementation: the first draft promised two distinct errors for a wrong password and a damaged file; an AEAD cannot distinguish them.)
- Without a vault file, a backup holds the tree and no entries; with a vault, it must be unlocked (the error says so).

## 5. Additions to existing crates

- **`fernjo-core`:** `Tree::graft(parent: Option<NodeId>, node: Node)` (adds a ready subtree; a duplicate id is an error), `Store::graft` (persists, rolls the memory back if the save fails, like the other mutations), `Store::replace_tree(Tree)` (same rollback). `Tree` validation of imported nodes reuses the existing rules.
- **`fernjo-vault`:** `seal_with_password` and `open_with_password` (generic password-sealed blob with a caller-chosen magic and version; the vault file itself is untouched); `Vault::add_with_id(id, NewEntry)`; `Vault::export_entries() -> Vec<(EntryId, NewEntry)>` (unlocked only) and `Vault::replace_entries(Vec<(EntryId, NewEntry)>)` (one atomic save, the previous file kept as `.bak` by the existing write path). No existing format changes; `Docs/vault-format.md` gets a pointer to the backup document.

## 6. Tauri layer

- **Pending slot:** `AppState.pending: Mutex<Option<Pending>>` with `Pending::MRemoteNg { plan }`, `Pending::Connections { folder }`, `Pending::Backup { payload }`. One at a time: a new inspect replaces it; apply, cancel and every main-window page load clear it (like `SessionManager::close_all`). Secrets in it are `SecretString`/`Zeroizing`.
- **Commands (names the builder may adjust):** `import_pick(kind)` opens the native open dialog (`kind`: `mremoteng` | `connections` | `backup`), remembers the path, returns `{ fileName }` or nothing when cancelled; `import_inspect(secret?)` reads the remembered file with the optional typed secret (file key or backup password; wrapped in `SecretString` on arrival), fills the pending slot and returns a **preview** (counts and report lines, or the typed error `key_needed` / `wrong_key` / `wrong_password` / `unsupported_format` / `damaged`); `import_apply()` applies the pending slot; `import_cancel()`; `export_connections()` and `export_backup(password)` open the native save dialog and write atomically (the second is `#[tauri::command(async)]`, Argon2). `import_inspect` for a backup is async for the same reason. Each follows the `CLAUDE.md` checklist; the no-secret test gets sentinel secrets in a synthetic mRemoteNG file, in a backup and in the typed keys, and asserts none appears in any response, error, `Debug` output or `connections.json`.
- **Apply rules:** an mRemoteNG import with planned entries, or a backup with entries, needs an unlocked vault. Without one `import_apply` returns `vault_needed` with the vault status (`none`, `locked`, `unusable`); for `none` and `locked` the UI offers create or unlock (the existing vault commands) and calls `import_apply` again, the pending slot waits in Rust. The restore never creates a vault by itself and never reuses the export password as a master password *(builder's call, from the review)*; with `unusable` it refuses, because that vault is never overwritten. For an mRemoteNG import the owner can also choose "Import without passwords": entries and their references are dropped and the report says how many connections lost a password. Order for import: add all entries with one save (`add_entries_with_ids`; first written as `add_with_id` per entry, changed after the final review), then `Store::graft`; if the graft fails, remove the entries again with one save (`remove_entries`). Order for restore: close all sessions, snapshot the current entries (zeroising), `replace_entries`, then `replace_tree`; if the tree step fails, put the snapshot back. Restoring a backup that has no entries (made without a vault) onto a vault that has some removes them; that is the meaning of "replace" and the confirmation says "0 credentials" in the same sentence. After any apply the full snapshot is returned like every other mutating command.
- **Restore confirmation names what is lost:** "This replaces N folders and connections and M credentials with the backup from <date> (K and L). Open sessions are closed." Cancel has focus (like the delete dialog).

## 7. UI

- A button in the sidebar footer (next to the settings gear) opens the dialog **"Import & backup"**; the empty tree's offer gains "Import from mRemoteNG…". The dialog has three groups: **Import** (mRemoteNG file; fernjo connections file), **Export** (connections JSON; full backup), **Restore** (backup file). Each flow runs inside the dialog in steps: pick file, key or password if needed (a secret field with show/hide like the existing ones, inline error on a wrong key with the field kept), preview (counts, collapsible skipped and partly mapped lists), confirm, result. The preview shows the report before anything is written; after applying, the same report is the result.
- Only tokens from `ui/src/styles/tokens.css`, hex-only CSS (CLAUDE.md rules); a new colour token only if needed, in all three blocks. UI text in English. One yellow key per context: the confirm button of a restore is not yellow (it is destructive); Import and Export use the normal primary.
- TypeScript holds only view state (step, counts, lines); validity, counts, names and report text come from Rust.

## 8. Tests that matter

Decryption against the independent vectors (default key, custom key, full-file, tampered ciphertext, wrong key, KDF iterations out of bounds); the owner's fixture; mapping (inherit flags mixed per setting, shared entries, skipped protocols listed, folder kept when children skipped, depth and size limits, a file that is XML but not a `Connections` document); report caps; plan apply rollback in both directions (graft fails, entry add fails); backup round trip gives an identical tree and entry set, wrong password, damaged file, hostile KDF parameters; connections JSON round trip and refusal of a newer version; the no-secret test; the pending slot cleared by page load and apply.

## 9. Left to the builder

Crate versions, exact entry-naming details, report wording, dialog layout within `DESIGN.md`, the Argon2 cost of the backup (default is the vault's), file extension spelling, whether `fernjo-import` gets one module per concern or fewer.
