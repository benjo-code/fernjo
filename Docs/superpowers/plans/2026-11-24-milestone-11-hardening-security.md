# Milestone 11: Full hardening and security check Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring everything in v0.3.0 through an independent security review and a hardening pass, close the parked hardening items, and write a go or no-go verdict for v0.3.0 (no release).

**Architecture:** Known items first, reviews second (spec, approach 1). Phase A (Tasks 1 to 14) writes the threat model, consolidates the hardened file helpers into a new crate `fernjo-fs` and fixes every parked item and hardening leftover; phase B (Tasks 15 to 17) runs eight independent area reviewers, `/security-review` and the dependency checks on that code and triages the findings; phase C (Tasks 18 to 21) fixes the findings, collects the owner's decisions in one batch, writes the readiness verdict and opens the PR; Task 22 is the owner checks that close the milestone.

**Tech Stack:** Rust 2024 workspace, Tauri 2, IronRDP crates pinned exactly (`ironrdp-session` 0.13.0, `ironrdp-svc` and `ironrdp-dvc` of the same generation, `ironrdp-tls` 0.3.0), rustls 0.23 with the ring provider, russh 0.65.2, `getrandom` 0.4, Svelte 5 + TypeScript, vitest, bash for `tools/`.

**Spec:** [Docs/superpowers/specs/2026-11-24-m11-hardening-security-design.md](../specs/2026-11-24-m11-hardening-security-design.md). Executors read both. Branch: `feature/m11-hardening-security` (already created from `dev`).

## Global Constraints

- Secrets stay in Rust; nothing secret in `connections.json`, `settings.json`, `open-tabs.json`, sync state, logs or webview storage. The only reveals are vault reveal, `sync_pairing_code` and `clipboard_read_text`.
- A new `#[tauri::command]` updates three places (function plus `handlers` wrapper, `table()` entry in `src-tauri/src/vault_commands.rs`, `generate_handler!` in `src-tauri/src/lib.rs`); M11 plans no new command.
- Crate rules: protocol, vault, import, sync, hub and `fernjo-fs` never depend on Tauri; `fernjo-hub` never depends on `fernjo-sync`; `fernjo-fs` depends only on `std` and `getrandom` and may be used by every crate.
- Files in the data folder are written atomically and keep a `.bak`. Callers of `fernjo-fs` keep today's behaviour except the security fix (spec, step 2).
- RDP clipboard default stays **on** for saved connections, **off** for quick connect (owner, 2026-11-24). The RDP handshake signature check stays strict, through rustls's own functions only; no second crypto library (owner, 2026-11-24).
- No new `deny.toml` ignore, no weakened check, no skipped test. A new dependency needs `cargo deny check` green without an ignore.
- UI uses only tokens from `ui/src/styles/tokens.css` and `DESIGN.md`; CSS stays hex-only; a new colour token goes into the light block and both dark blocks.
- Every fix has a test or recorded check that fails without it; the evidence says so.
- Commits are Conventional Commits, no attribution lines; stage files by name; check `git show --stat HEAD` after each commit. Work only inside M11.
- The `src-tauri` package is `fernjo-app` (`cargo test -p fernjo-app`).
- Quality gate (all green): `cargo build --workspace`, `cargo test --workspace`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo deny check`, `pnpm test`, `pnpm lint`, `pnpm audit --prod`, `bash tools/test-release-scripts.sh`, `bash tools/check-workflows.sh`, `dotnet run tools/roadmap.cs -- check`.

## Review Focus

1. Data files written by v0.2.0 and earlier: a `connections.json` with its `.bak`, a leftover `connections.json.tmp`, a `vault.fernjo.bak` that is a symlink planted by a sync tool. Expected: everything loads, the next save works, the symlink target is untouched and the leftover fixed-name temp file is gone (Task 3, Task 2).
2. The largest legitimate clipboard transfer next to the new channel cap: 1 MiB of ASCII text becomes about 2 MiB of UTF-16 on the wire. Expected: it still passes; only a message announced above 4 MiB ends the session (Task 6).
3. A slow but live network during an RDP write. Expected: a peer that keeps reading slowly never trips the write timeout; only a peer that stops reading does (Task 7).
4. A normal Windows 11 certificate (RSA 2048, SHA-256, self-signed) under the new verifier. Expected: it reaches the certificate decision exactly as before (Task 8).
5. A burst of window resizes (dragging the window edge) under the new resize coalescing. Expected: the final size always reaches the session; nothing grows without bound (Task 12).

---

### Task 1: Threat model and the roadmap's scope change

**Files:**
- Modify: `Docs/DECISIONS.md`, `Docs/roadmap.json`, `Docs/ROADMAP.md` (generated)

- [ ] **Step 1: Read** the existing model (`Docs/DECISIONS.md`: "Webview hardening from day one", the M6 security entries, "M9 ..." entries, "M10 ..." entries) and `Docs/sync-format.md`, `Docs/hub.md`.
- [ ] **Step 2: Write** the entry "M11 threat model" per spec step 1: a table of assets (vault secrets, sync key and hub secret, `connections.json`, `known_hosts` and `known_hosts.confirmed`, RDP pins, device folder files, `open-tabs.json`, the webview) by adversaries (hostile SSH server, hostile RDP server, hostile hub or path to it, hostile peer device in the sync group, writer of the synced data folder, hostile import or backup file, script in the webview, another local user), each cell "what it can do / what stops it / accepted". Name the trust assumptions explicitly: a peer in the sync group holds the sync key and is trusted with the data; the hub is not trusted with content; the clipboard text is a by-design reveal.
- [ ] **Step 3: Roadmap** (`Docs/roadmap.json`; the planning commit already linked this plan, set `planned` and wrote the scope-change entry of 2026-11-24): set status `in-progress`. `dotnet run tools/roadmap.cs -- generate`, then `-- check` → "Roadmap is valid and ROADMAP.md is up to date."
- [ ] **Step 4: Commit** `docs(m11): threat model for sync and the M10 features`.

### Task 2: `fernjo-fs` crate

**Files:**
- Create: `crates/fernjo-fs/Cargo.toml`, `crates/fernjo-fs/src/lib.rs`, `crates/fernjo-fs/src/tests.rs`
- Modify: root `Cargo.toml` (`[workspace.dependencies] fernjo-fs = { path = "crates/fernjo-fs" }`, dev profile unchanged)

**Interfaces:**
- Produces (in `fernjo_fs`):
  - `pub fn read_capped(path: &Path, max: u64) -> io::Result<Vec<u8>>`: metadata length checked first, read bounded by `take(max + 1)`; over the cap → `io::ErrorKind::InvalidData` with the text "the file is too large".
  - `pub struct WriteOptions { pub parent: Parent, pub backup: bool, pub file_mode: FileMode }` with `pub enum Parent { MustExist, CreatePrivate, CreateDefault }` and `pub enum FileMode { OwnerOnly, Default }`; a `const fn data_file() -> WriteOptions` (CreatePrivate, backup, OwnerOnly) for the common case.
  - `pub fn write_atomic(path: &Path, bytes: &[u8], options: WriteOptions) -> io::Result<()>`: writes `bytes` exactly (no newline added), through `<file>.<16 hex>.tmp` opened with `create_new` (0600 on Unix for `OwnerOnly`, up to 8 attempts), flush, `sync_all`, then the `.bak` step, then rename, then directory fsync on Unix (best effort; a failed folder sync never fails the write). The temp file is removed on every failure.
  - `.bak` step (when `backup` and `path` exists): copy `path` into a second fresh `create_new` temp file, `sync_all`, rename it onto `<file>.bak`. Never `fs::copy` to the `.bak` name, never open the `.bak` name for writing.
  - `pub fn with_suffix(path: &Path, suffix: &str) -> PathBuf` (moved, so callers stop keeping their own).

- [ ] **Step 1: Write failing tests** in `src/tests.rs`: `read_capped_accepts_up_to_the_cap_and_refuses_one_byte_more`; `write_uses_a_fresh_temp_name_and_leaves_none_behind`; `write_keeps_the_previous_content_as_bak_only_when_asked`; `a_failed_write_leaves_the_target_and_no_temp_file`; `must_exist_parent_is_not_created`; `create_default_parent_creates_missing_folders`; Unix-only (`#[cfg(unix)]`): `owner_only_files_and_created_folders_are_0600_and_0700`, `an_existing_folder_keeps_its_mode`, `a_symlink_at_the_bak_name_is_replaced_not_followed` (plant `x.bak -> victim`, write twice, assert `victim` unchanged and `x.bak` is a regular file with the old content), `a_symlink_at_a_temp_name_is_never_followed`.
- [ ] **Step 2: Run** `cargo test -p fernjo-fs` → FAIL (crate empty).
- [ ] **Step 3: Implement** the interfaces above in `src/lib.rs`; the bodies of `create_temp_file`, `create_private_dir_all`, `owner_only` and `sync_dir` come from `crates/fernjo-vault/src/vault.rs` (the most complete copy).
- [ ] **Step 4: Run** `cargo test -p fernjo-fs`, `cargo clippy -p fernjo-fs --all-targets -- -D warnings`, `cargo deny check` → PASS (the Unix tests run in CI on Linux; say so in the evidence).
- [ ] **Step 5: Docs.** `Docs/DECISIONS.md` entry "M11: one set of hardened file helpers (`fernjo-fs`)" (supersedes "M9 follow-up candidate: one set of hardened file helpers"; the `.bak` rename rule and why). `CLAUDE.md`: add `crates/fernjo-fs` to the architecture list and the crate rule; rewrite the "Hardened atomic writes" paragraph to name `fernjo_fs::write_atomic` and `fernjo_fs::read_capped`, and "A new reader of a data-folder file must use the capped helper" to name it.
- [ ] **Step 6: Commit** `feat(fs): hardened atomic writer and capped reader in one crate`.

### Task 3: `fernjo-core` on `fernjo-fs`

**Files:**
- Modify: `crates/fernjo-core/Cargo.toml`, `crates/fernjo-core/src/persist.rs`

**Interfaces:**
- Consumes: `fernjo_fs::{write_atomic, read_capped, WriteOptions, Parent, FileMode, with_suffix}` (Task 2).
- Produces: unchanged public API (`persist::write_atomic` keeps adding the trailing newline and returning `SaveError`; `write_exact` keeps no newline, no `.bak`, default-mode parent folders; `read_capped(path)` keeps using `MAX_DATA_FILE_LEN`).

- [ ] **Step 1: Write failing tests** in `persist.rs` `mod tests`: `a_leftover_fixed_name_temp_file_is_removed_after_a_save` (create `connections.json.tmp` as a regular file, save, assert it is gone); `write_exact_uses_a_random_temp_name` (no `<file>.tmp` ever exists: plant a directory named `<file>.tmp`, the write still succeeds); Unix: `a_symlink_at_the_bak_name_is_not_followed_by_a_save`.
- [ ] **Step 2: Run** `cargo test -p fernjo-core persist` → the new tests FAIL.
- [ ] **Step 3: Implement.** `write_atomic` builds `contents + b"\n"` and calls `fernjo_fs::write_atomic(path, …, WriteOptions::data_file())`; `write_exact` calls it with `{ parent: CreateDefault, backup: false, file_mode: Default }` (export files keep today's default mode); after a successful `write_atomic` remove `with_suffix(path, ".tmp")` when `symlink_metadata` says it is a regular file. `read_capped` delegates to `fernjo_fs::read_capped(path, MAX_DATA_FILE_LEN)`. Delete the now-unused private helpers.
- [ ] **Step 4: Run** `cargo test -p fernjo-core` (all existing persist and store tests unchanged) and `cargo test -p fernjo-app` → PASS.
- [ ] **Step 5: Commit** `refactor(core): write data files through fernjo-fs`.

### Task 4: Vault, SSH and RDP pin file on `fernjo-fs`

**Files:**
- Modify: `crates/fernjo-vault/{Cargo.toml,src/vault.rs,src/format.rs,src/generation.rs}`, `crates/fernjo-ssh/{Cargo.toml,src/known_hosts.rs}`, `crates/fernjo-rdp/{Cargo.toml,src/cert_store.rs}`

**Interfaces:**
- Consumes: Task 2. Produces: unchanged crate APIs; `fernjo_vault::MAX_FILE_LEN`, `STALE_TEMP_AGE` and `sweep_stale_temp_files` stay in the vault; `known_hosts` keeps `WRITE_LOCK`.

- [ ] **Step 1: Write failing tests** (Unix, one per crate): `a_symlink_at_the_vault_bak_is_not_followed`, `a_symlink_at_the_known_hosts_bak_is_not_followed`, `a_symlink_at_the_pin_file_bak_is_not_followed`.
- [ ] **Step 2: Run** them in CI or note they are Unix-only; on Windows confirm they compile out (`cargo test -p fernjo-vault -p fernjo-ssh -p fernjo-rdp` green).
- [ ] **Step 3: Implement.** Each `write_atomic` becomes a call with `WriteOptions::data_file()`, mapping the `io::Error` to the crate's error as today (`SshError::Io`, `RdpError::Io`, the vault's `io::Result`); each `read_capped` becomes `fernjo_fs::read_capped(path, <crate cap>)`. Delete the duplicated helpers and the tests that moved to `fernjo-fs` only where `fernjo-fs` now has the same test; keep every behaviour test (temp sweep, rollback, golden fixture `v1-no-generation.fernjo`).
- [ ] **Step 4: Run** `cargo test -p fernjo-vault -p fernjo-ssh -p fernjo-rdp -p fernjo-app` → PASS.
- [ ] **Step 5: Commit** `refactor(vault,ssh,rdp): write through fernjo-fs; the .bak no longer follows a symlink`.

### Task 5: Sync, hub, AppImage menu and open tabs on `fernjo-fs`

**Files:**
- Modify: `crates/fernjo-sync/{Cargo.toml,src/files.rs}`, `crates/fernjo-hub/{Cargo.toml,src/store.rs}`, `src-tauri/{Cargo.toml,src/app_menu.rs,src/open_tabs.rs}`

- [ ] **Step 1: Write failing tests:** `app_menu.rs` `the_appimage_path_must_be_absolute` (`should_offer(Some("fernjo.AppImage"), home, false)` is false; `should_offer(Some("/opt/fernjo.AppImage"), …)` keeps today's answer) — the `APPIMAGE` Low item; `app_menu.rs` `install_leaves_no_pid_named_temp_file` (after `install`, no `*.tmp` in `applications/` or `icons/`).
- [ ] **Step 2: Run** `cargo test -p fernjo-app app_menu` → FAIL.
- [ ] **Step 3: Implement.** `fernjo-sync`: `{ CreatePrivate, backup: false, OwnerOnly }`, `read_capped(path, MAX_DATA_FILE_LEN)`; `fernjo-hub` store: `{ MustExist, backup: false, OwnerOnly }` (the store creates its folder elsewhere, unchanged); `app_menu.rs`: `{ CreateDefault, backup: false, Default }` and `should_offer`/`offer` refuse a relative `APPIMAGE`; `open_tabs.rs`: its private bounded reader becomes `fernjo_fs::read_capped(path, MAX_FILE_LEN)` with the same "reads as empty" mapping. The hub's HTTP body reader in `server.rs` stays.
- [ ] **Step 4: Run** `cargo test -p fernjo-sync -p fernjo-hub -p fernjo-app` → PASS. If Docker or Podman is available: `podman build -f crates/fernjo-hub/Containerfile -t fernjo-hub .` (or `docker build`) succeeds; otherwise record it as an open check (the CI build of `fernjo-hub` covers the compile).
- [ ] **Step 5: Docs.** (The backlog item "Consolidate the hardened file helpers" was already removed by the planning commit.) `CLAUDE.md` loses the "a copy because fernjo-ssh must not depend on fernjo-vault" and "a private bounded read because core's `read_capped` is `pub(crate)`" remarks.
- [ ] **Step 6: Commit** `refactor(sync,hub,app): write and read through fernjo-fs; APPIMAGE must be absolute`.

### Task 6: RDP channel message cap

**Files:**
- Create: `crates/fernjo-rdp/src/session/channel_cap.rs`
- Modify: `crates/fernjo-rdp/src/session.rs`, `crates/fernjo-rdp/src/error.rs`, `src-tauri/src/sessions/rdp.rs` (end text)

**Interfaces:**
- Produces: `pub(crate) const MAX_CHANNEL_MESSAGE: u32 = 4 * 1024 * 1024;` and `pub(crate) fn check_frame(action: ironrdp_pdu::Action, payload: &[u8]) -> Result<(), ChannelTooLarge>`; `CloseReason::ChannelTooLarge` (new variant), end text "The server sent a channel message that is too large." in `sessions/rdp.rs`.

- [ ] **Step 1: Confirm in source.** Read `ironrdp-svc` (the `ChunkProcessor` / reassembly) and `ironrdp-dvc` (`DataFirst` handling) of the versions in `Cargo.lock`, and note in the task report: which header field sizes the buffer, whether a "first" chunk's total length is trusted before data arrives, whether DVC reassembles the same way. If DVC does not, the DVC half of this task is dropped and recorded.
- [ ] **Step 2: Write failing tests** in `channel_cap.rs` with frames built by `ironrdp-pdu` encoders: `a_static_channel_message_announced_above_the_cap_is_refused` (X.224 action, MCS send-data indication on a static channel, channel PDU header with the first flag and total length `MAX_CHANNEL_MESSAGE + 1`); `a_two_mebibyte_clipboard_message_passes` (total length `2 * 1024 * 1024 + 64`); `fast_path_frames_and_non_channel_pdus_pass`; `a_dynamic_channel_data_first_above_the_cap_is_refused` (only if Step 1 says DVC needs it); `a_truncated_frame_is_left_to_ironrdp` (garbage returns `Ok`, the stage reports its own error).
- [ ] **Step 3: Run** `cargo test -p fernjo-rdp channel_cap` → FAIL.
- [ ] **Step 4: Implement** `check_frame` (decode only the headers it needs; anything it cannot decode is `Ok`) and call it in the `reader.read_pdu()` arm of the session loop before `stage.process`; `Err` returns `CloseReason::ChannelTooLarge`.
- [ ] **Step 5: Run** `cargo test -p fernjo-rdp -p fernjo-app` → PASS.
- [ ] **Step 6: Commit** `fix(rdp): end the session when a channel message is announced above 4 MiB`.

### Task 7: RDP write timeout

**Files:**
- Modify: `crates/fernjo-rdp/src/session.rs`

**Interfaces:**
- Produces: `pub(crate) const WRITE_LIMIT: Duration = Duration::from_secs(15);` and `async fn write_bounded<W: ironrdp_tokio::FramedWrite>(writer: &mut W, frame: &[u8]) -> Result<(), ()>`.

- [ ] **Step 1: Write failing tests** (tokio, paused time, a `tokio::io::duplex` with a small buffer): `a_peer_that_stops_reading_ends_the_write_after_the_limit`; `a_peer_that_reads_slowly_never_trips_the_limit` (the reader drains 1 KiB every second for 30 s of virtual time; the write of 16 KiB completes).
- [ ] **Step 2: Run** `cargo test -p fernjo-rdp write_bounded` → FAIL.
- [ ] **Step 3: Implement** `write_bounded` with `tokio::time::timeout(WRITE_LIMIT, writer.write_all(frame))` and use it for every `writer.write_all` in the session loop, including the reactivation step; a timeout returns `CloseReason::ConnectionLost`. The limit applies per frame, so a slowly reading peer that makes progress is never cut.
- [ ] **Step 4: Run** `cargo test -p fernjo-rdp` → PASS.
- [ ] **Step 5: Commit** `fix(rdp): bound every write to the server`.

### Task 8: RDP TLS with an own certificate verifier

**Files:**
- Create: `crates/fernjo-rdp/src/tls.rs`, `crates/fernjo-rdp/tests/fixtures/` (`rsa2048-sha256.{crt,key}`, `rsa2048-sha1.{crt,key}`, `critical-ext.{crt,key}`, a `README.md` with the generating commands)
- Modify: `crates/fernjo-rdp/{Cargo.toml,src/connect.rs,src/error.rs,src/lib.rs}`, `crates/fernjo-rdp/tests/support/mod.rs`, `crates/fernjo-rdp/tests/connect.rs`, `src-tauri/src/sessions/rdp.rs` (error text)

**Interfaces:**
- Produces: `RdpError::CertificateUnsupported` with code `certificate_unsupported`; `pub(crate) async fn upgrade(stream: TcpStream, host: &str) -> Result<(TlsStream<TcpStream>, Vec<u8> /* leaf DER */), RdpError>` in `tls.rs`; `struct PinVerifier { provider: Arc<CryptoProvider>, unsupported: Arc<AtomicBool> }` implementing `rustls::client::danger::ServerCertVerifier`.

- [ ] **Step 1: Confirm in source** (rustls and `ironrdp-tls` versions in `Cargo.lock`): `rustls::crypto::verify_tls12_signature` and `verify_tls13_signature` verify the handshake signature with the leaf's key and parse the leaf with webpki; which parse and key errors they return (`InvalidCertificate(BadEncoding | UnsupportedCertVersion | UnsupportedCriticalExtension …)`, `PeerIncompatible`/`UnsupportedSignatureAlgorithm…`); whether `ironrdp-tls` accepts a custom verifier. If it does not, `tls.rs` builds the `rustls::ClientConfig` and the `tokio_rustls::TlsConnector` itself (add `tokio-rustls` and `rustls` as direct dependencies at the versions already in `Cargo.lock`, ring provider, `cargo deny check` green). Record the exact list of still-refused certificates for Step 7.
- [ ] **Step 2: Fixtures.** Generate with OpenSSL (commands in the fixtures `README.md`): a self-signed RSA 2048 SHA-256 certificate for `127.0.0.1` (Windows-like), the same signed with SHA-1 (`-sha1`), and an RSA 2048 SHA-256 one with an unknown critical extension (`-addext "1.3.6.1.4.1.55555.1=critical,ASN1:NULL"`). Test-only, committed; CodeQL may flag the keys as test data.
- [ ] **Step 3: Write failing tests** in `tests/connect.rs` with a new `Behaviour::TlsWithCert(Fixture)` in `tests/support`: `a_windows_like_rsa_certificate_reaches_the_certificate_decision`; `a_sha1_signed_certificate_reaches_the_certificate_decision`; `a_certificate_webpki_cannot_parse_fails_with_certificate_unsupported_before_any_prompt`; unit test in `tls.rs` `the_verifier_needs_no_root_store` (a `ClientConfig` built by `upgrade` holds no root store; a self-signed leaf passes `verify_server_cert`). Keep `a_server_that_cannot_sign_for_its_certificate_fails_the_handshake_before_any_prompt` unchanged.
- [ ] **Step 4: Run** `cargo test -p fernjo-rdp --test connect` and `cargo test -p fernjo-rdp tls` → the new tests FAIL.
- [ ] **Step 5: Implement.** `verify_server_cert` returns `ServerCertVerified::assertion()` for any chain (the pin decides after the handshake, as today); `verify_tls12_signature`/`verify_tls13_signature` delegate to the rustls functions with `provider.signature_verification_algorithms` and set `unsupported` when the error is a certificate parse or unsupported-key error; `supported_verify_schemes` from the provider. `connect.rs` uses `tls::upgrade`, maps a handshake failure with `unsupported` set to `CertificateUnsupported`, otherwise `TlsFailed`, and logs the rustls error at `debug` (no secrets in it). The certificate and public key extraction for the pin and CredSSP stays as it is.
- [ ] **Step 6: Run** `cargo test -p fernjo-rdp -p fernjo-app` → PASS. The live tests (`FERNJO_TEST_RDP_ADDR`) skip; they are Task 22.
- [ ] **Step 7: Docs.** `Docs/DECISIONS.md` "M11 RDP TLS: own verifier, strict handshake signature" (supersedes the verifier part of "M8 RDP TLS handshake signature check"; the confirmed list of still-refused certificates, the owner's choice of 2026-11-24, rejected option B). README known limits: hosts with such certificates fail with "certificate not supported". `CLAUDE.md` RDP paragraph: `tls::upgrade` replaces `upgrade_with_certificate_validation_callback_for_endpoint`. `CHANGELOG.md` `[Unreleased]`: RDP hosts without root certificates or with SHA-1-signed certificates connect again; unsupported certificates get their own message.
- [ ] **Step 8: Commit** `fix(rdp): own certificate verifier with a strict handshake signature check`.

### Task 9: RDP Low items (desktop area, guard sentinels)

**Files:**
- Modify: `src-tauri/src/sessions/rdp.rs`, `src-tauri/src/vault_commands.rs` (`table()` and its fixture)

- [ ] **Step 1: Write failing tests:** `desktop_size_scales_a_too_large_area_down_keeping_the_aspect` (`desktop_size(8192, 8192)` → both sides even, `w * h <= MAX_DESKTOP_PIXELS`, `|w - h| <= 2`; `desktop_size(3840, 2160)` unchanged); in `no_response_contains_a_secret`, the RDP connection of the fixture gets sentinel user and domain values (`SENTINEL-RDP-USER`, `SENTINEL-RDP-DOMAIN`) that must not appear in any response, error or `Debug` output.
- [ ] **Step 2: Run** `cargo test -p fernjo-app desktop_size no_response_contains_a_secret` → the first FAILS (today it clamps sides only); the guard may already pass (then it is a new pin, say so).
- [ ] **Step 3: Implement** the area clamp in `desktop_size` (scale both sides by `sqrt(MAX_DESKTOP_PIXELS / area)` when the area is too large, then the existing side clamp and even rounding). For `rdp_input`/`rdp_clipboard` reaching past `session_not_connected`: check whether `src-tauri/src/sessions/rdp/tests.rs` can stand up a connected session without a server; if not, record in the task report why the guard only reaches the not-connected path (needs a live RDP server) — that becomes a triaged Low in Task 17.
- [ ] **Step 4: Run** `cargo test -p fernjo-app` → PASS.
- [ ] **Step 5: Commit** `fix(rdp): scale an oversized desktop down; guard RDP user and domain`.

### Task 10: Import username cleaning and `clear_pending`

**Files:**
- Modify: `crates/fernjo-import/src/limits.rs`, `src-tauri/src/state.rs`, `src-tauri/src/import_commands.rs`

**Interfaces:**
- Produces: `AppState.page_reloaded: AtomicBool` (set by `clear_pending` when it has to skip because an apply holds the lock); `import_apply` clears its pending value on failure when the flag is set, and resets the flag.

- [ ] **Step 1: Write failing tests:** `sanitize_nodes_cleans_and_caps_the_username` (control characters removed, cut to `MAX_NAME_CHARS`; an empty result becomes `None`; counted in `changed`); `a_failed_apply_after_a_skipped_page_load_drops_the_pending_value` (hold the lock as `clear_pending_does_not_block_while_an_apply_holds_the_lock` does, call `clear_pending`, release, make the apply fail, assert `pending` is `None`); `a_failed_apply_without_a_page_load_keeps_the_pending_value` (today's behaviour).
- [ ] **Step 2: Run** `cargo test -p fernjo-import sanitize` and `cargo test -p fernjo-app pending` → FAIL.
- [ ] **Step 3: Implement** both as described (the username uses `clean(…, MAX_NAME_CHARS, false)` like the name).
- [ ] **Step 4: Run** `cargo test -p fernjo-import -p fernjo-app` → PASS.
- [ ] **Step 5: Commit** `fix(import): clean imported user names; drop a pending import after a reload`.

### Task 11: `check-no-build-paths.sh` false-clean

**Files:**
- Modify: `tools/check-no-build-paths.sh`, `tools/test-release-scripts.sh`

- [ ] **Step 1: Write the failing test** in `test-release-scripts.sh`: `no_build_paths_fails_when_grep_cannot_read_the_file` — an unreadable file (`chmod 000`; skip with a message when running as root or when `chmod` has no effect, as on Git Bash) must exit 2.
- [ ] **Step 2: Run** `bash tools/test-release-scripts.sh` → the new test FAILS (exit 0 today), or is skipped on Git Bash; then its first real run is the CI job that runs this script on Linux (find it in `.github/workflows/ci.yml` and name it in the evidence).
- [ ] **Step 3: Implement:** capture grep's status (`n="$(grep …)"; status=$?`); status 0 or 1 is a count, anything above 1 prints `error: cannot read $bin` and exits 2.
- [ ] **Step 4: Run** `bash tools/test-release-scripts.sh` → all pass.
- [ ] **Step 5: Commit** `fix(tools): check-no-build-paths fails when grep cannot read the binary`.

### Task 12: Session queue: minimum weight and resize coalescing; russh task leak

**Files:**
- Modify: `src-tauri/src/sessions.rs`, `src-tauri/src/session_flow.rs`, `src-tauri/src/sessions/rdp.rs`, `crates/fernjo-ssh/src/session.rs`
- Test: `src-tauri/src/sessions/tests.rs`, `crates/fernjo-ssh/src/session.rs` tests or `crates/fernjo-ssh/tests/timeout.rs`

**Interfaces:**
- Produces: `pub const MIN_OP_WEIGHT: usize = 64;` in `session_flow.rs` (every `Data`, `Rdp` and `Clipboard` op weighs at least this); `Shared.resize_queued: AtomicBool` — `resize` stores the size in `shared.size` and sends `Op::Resize` only when no resize is queued; the writer clears the flag and applies the **latest** `shared.size`.

- [ ] **Step 1: Write failing tests:** `a_flood_of_one_byte_writes_hits_the_cap_by_weight` (`MAX_QUEUED_INPUT / MIN_OP_WEIGHT + 1` one-byte inputs without a running writer → the last is `input_queue_full`); `a_resize_burst_queues_one_op_and_applies_the_last_size` (1 000 resizes while the writer is paused → one queued op; after it runs, the session saw only the final size). SSH: `a_pre_key_exchange_timeout_leaves_no_session_task` — first confirm in russh 0.65.2 whether `client::connect_stream` spawns the session task before the key exchange finishes and what owns the stream; the test asserts the TCP peer sees the connection closed within one second after `SshError::Timeout` (a `TcpListener` that accepts, sends a banner and stays silent).
- [ ] **Step 2: Run** the three → FAIL (or the SSH one passes: then the leak is not real in 0.65.2; record it and drop that part).
- [ ] **Step 3: Implement** the weight and coalescing for both protocols (RDP resizes use the same flag in `sessions/rdp.rs`). SSH: make the timeout drop everything that holds the stream (keep a handle to abort the spawned task or shut the socket through a clone taken before `connect_stream`); the exact means follows from Step 1.
- [ ] **Step 4: Run** `cargo test -p fernjo-app -p fernjo-ssh` → PASS; SSH integration tests with the container (`eval "$(tools/ssh-test-server.sh start)"`, `env -u CI cargo test -p fernjo-ssh --no-fail-fast`, `tools/ssh-test-server.sh stop`) or mark open.
- [ ] **Step 5: Docs.** `Docs/DECISIONS.md` "M11 session queue: minimum weight and resize coalescing" (why this bounds the `Op` queue of SSH and RDP without turning the channel into a bounded one that could block the UI thread; this is how the spec's "bounded `ops` channel" is met) and, if the russh leak was real, its fix.
- [ ] **Step 6: Commit** `fix(sessions): weigh every queued op, coalesce resizes; close the socket on a handshake timeout`.

### Task 13: App shell leftovers (clipboard timeout, idle thread, opener, Cmd+Q)

**Files:**
- Modify: `src-tauri/src/clipboard.rs`, `src-tauri/src/idle_lock.rs`, `src-tauri/src/lib.rs`

**Interfaces:**
- Produces: `pub const CLIPBOARD_READ_LIMIT: Duration = Duration::from_secs(2);` and `pub async fn read_with_limit(clip: Arc<dyn ClipboardText + Send + Sync>, limit: Duration) -> Result<String, CommandError>` in `clipboard.rs` (timeout → `clipboard_unavailable`, the fixed error text, no content).

- [ ] **Step 1: Write failing tests:** `a_clipboard_read_that_hangs_ends_with_clipboard_unavailable` (fake whose `read_text` sleeps longer than the limit, paused time); `a_panicking_tick_does_not_end_the_idle_loop` (extract the loop body into `fn run_tick(state, now, wall) -> Option<VaultStatus>` wrapped in `catch_unwind`; a tick that panics returns `None` and the next tick runs).
- [ ] **Step 2: Run** `cargo test -p fernjo-app clipboard idle_lock` → FAIL.
- [ ] **Step 3: Implement.** `handlers::clipboard_read_text` uses `read_with_limit(OsClipboard, CLIPBOARD_READ_LIMIT)`; the idle thread calls the guarded tick. `lib.rs`: replace `tauri_plugin_opener::init()` with `tauri_plugin_opener::Builder::new().open_js_links_on_click(false).build()` (check the method exists in the locked plugin version; the single-instance plugin stays first and `nav_guard` second). Cmd+Q with no main window: the code already lets the exit go ahead (comment "with no main window there is nothing to ask in"); record "no change needed" with that reference for Task 17.
- [ ] **Step 4: Run** `cargo test -p fernjo-app` → PASS; `cargo test -p fernjo-app --test tauri_conf` still PASS.
- [ ] **Step 5: Commit** `fix(app): time-limit the clipboard read, keep the idle thread alive, no opener init script`.

### Task 14: Clipboard marker and restore wording (UI)

**Files:**
- Modify: `ui/src/lib/components/TabStrip.svelte`, `ui/src/lib/components/ImportDialog.svelte`
- Test: `ui/src/lib/components/TabStrip.test.ts`, `ui/src/lib/components/ImportDialog.test.ts`

**Interfaces:**
- Consumes: `Tab.clipboard: boolean` (already in `ui/src/lib/sessions.svelte.ts`, set from Rust's resolved setting); the sync state from `SyncStore` (`ui/src/lib/sync.svelte.ts`).

- [ ] **Step 1: Write failing tests:** `an_rdp_tab_sharing_the_clipboard_shows_the_marker_with_the_host_in_its_tooltip` (marker present, `title` and `aria-label` "Clipboard shared with <host>"); `no_marker_on_an_ssh_tab_or_an_rdp_tab_with_the_clipboard_off`; `with_sync_on_the_restore_confirmation_says_it_reaches_every_device` (text contains "With sync on, the restore also replaces the data on every device in the sync group."); `without_sync_the_restore_confirmation_is_unchanged`.
- [ ] **Step 2: Run** `pnpm vitest run TabStrip ImportDialog` → FAIL.
- [ ] **Step 3: Implement.** A 12 px clipboard icon in the tab's existing icon slot style, colour `--color-text-muted` (no new token), shown when `tab.protocol === "rdp" && tab.clipboard`; the restore sentence appended to `restoreText` when sync is set up.
- [ ] **Step 4: Run** `pnpm test` and `pnpm lint` → PASS.
- [ ] **Step 5: Docs.** `Docs/DECISIONS.md` "M11 RDP clipboard default: stays on, accepted by the owner" (the v0.2.0 Medium, remaining risk: the local clipboard reaches the host when the desktop gets focus, and the host can overwrite the local clipboard; mitigations: off per connection or folder, quick connect off, the marker). `CHANGELOG.md` `[Unreleased]`: the marker and the restore sentence.
- [ ] **Step 6: Commit** `feat(ui): mark RDP tabs that share the clipboard; restore wording names sync`.

### Task 15: Phase A gate

- [ ] **Step 1: Run** every command of the quality gate (Global Constraints) and record each result.
- [ ] **Step 2:** Push the branch only on the user's say-so and read `gh pr checks` of a draft PR for the Linux and macOS compile and the Unix-only tests of Tasks 2 to 4 and 11; until then those stay "not yet seen".
- [ ] **Step 3: Commit** any formatting fix as `chore: format`.

### Task 16: Independent reviews and checks

Run by the controller (the session executing this plan), not by an implementer subagent.

- [ ] **Step 1: Known-items list.** Write `.superpowers/m11/known-items.md`: every item fixed in Tasks 2 to 14 and every finding of the earlier reviews in `Docs/release-checklist.md` (v0.1.0, v0.1.1, v0.2.0) with its disposition only ("fixed in M11 Task 6", "accepted by the owner 2026-11-22"), no reasoning.
- [ ] **Step 2: Eight reviewers in parallel**, each a fresh `general-purpose` subagent on the strongest available model, read-only, probes only in a scratch folder, given: the "M11 threat model" entry, its area's files (spec step 4, list 1 to 8, plus `crates/fernjo-fs` and `crates/fernjo-core/src/persist.rs` where its crate uses them), the severity scale (High: secret disclosure, code execution, auth bypass or data loss reachable by an adversary of the threat model; Medium: denial of service or integrity loss needing an unusual position, or a defence layer missing; Low: hardening, needs local write access or a user mistake), the known-items list, and the instruction to report each finding with file, scenario, severity and a suggested check. Never the other reviewers' output, never expected conclusions. Each writes `.superpowers/m11/review-<area>.md`.
- [ ] **Step 3: `/security-review`** of `git diff v0.2.0...HEAD`; save as `.superpowers/m11/review-diff.md`.
- [ ] **Step 4: Checks**, results recorded: `cargo deny check`; `pnpm audit --prod`; `gh api "repos/benjo-code/fernjo/dependabot/alerts?state=open"`; `gh api "repos/benjo-code/fernjo/code-scanning/alerts?state=open"`; `bash tools/check-workflows.sh`; gitleaks v8.30.1 via Docker with `.gitleaks.toml` over the history (start Docker Desktop first, or mark open).
- [ ] **Step 5: Secrets check.** Confirm that `table()` in `vault_commands.rs` lists every command added since v0.2.0 (`git diff v0.2.0 -- src-tauri/src/lib.rs` for the `generate_handler!` additions against the table), that sync state and base files carry no secret field (read `crates/fernjo-sync/src/state.rs`, `snapshot.rs`), that the envelope's clear part is the 34-byte header only (`envelope.rs` test), and that no `log::`/`tracing::` call or `console.*` in `ui/` prints a secret-carrying value (grep). Each gap is a finding.

### Task 17: Triage

**Files:**
- Modify: `Docs/release-checklist.md`

- [ ] **Step 1:** New section "v0.3.0 security review" (after "v0.2.0 result"): scope line (commit, date, the eight areas, the diff review, the checks), then the findings table `| # | area | finding | severity | decision | fixed in |` continuing the numbering after the last earlier finding; reviewer severity and controller ruling both shown where they differ; duplicates across reviewers merged with all sources named; the Task 9 guard limit and the Task 12/13 "not real / no change" results entered as findings with their disposition.
- [ ] **Step 2:** Mark each finding: `fix in M11` (all High and Medium unless impossible), `owner: accept?` (Low proposed for acceptance, or a Medium that cannot be fixed), `backlog` (only with the owner's later yes).
- [ ] **Step 3:** If a High cannot be fixed in M11, stop and tell the owner at once (spec stop rules).
- [ ] **Step 4: Commit** `docs(m11): security review findings and triage`.

### Task 18: Fix waves

For each finding marked `fix in M11`, one implementer task in the usual shape (failing test or recorded check, fix, green, commit `fix(<area>): …`), the finding number in the commit body, and the `fixed in` column updated.

- [ ] **Step 1:** Fix all High, then all Medium, then the Lows the controller chose to fix.
- [ ] **Step 2: Second read** for every High and Medium fix: a fresh reviewer gets the finding text and `git show <fix commits>` and answers "closes the finding: yes/no, new problem: …". A "no" goes back to Step 1.
- [ ] **Step 3:** Run the quality gate again; record.

### Task 19: Owner decisions (one batch)

- [ ] **Step 1:** One message to the owner listing every `owner: accept?` row (finding, severity, remaining risk, why not fixed, recommendation), any stop-rule item, and any CodeQL alert that would need dismissing (with the read line for each). Wait for the answer.
- [ ] **Step 2:** Record each decision: `Docs/DECISIONS.md` (accepted Mediums with the remaining risk, accepted Lows with the reason), the `decision` column, backlog entries for "later" items in `Docs/roadmap.json`. CodeQL dismissals only for alerts the owner named.

### Task 20: Readiness gate and verdict

**Files:**
- Modify: `Docs/release-checklist.md`, `Docs/roadmap.json`, `Docs/ROADMAP.md`, `CHANGELOG.md`, `README.md`, `CLAUDE.md`

- [ ] **Step 1:** Section "v0.3.0 readiness" in `Docs/release-checklist.md`: the reusable checklist steps that apply before a release branch (2, 3, 4, 5, 7) on the branch head, the command table with results, the known gaps table (open owner checks of M9, M10 and Task 22, accepted findings, unverified claims), one go or no-go line per area (security review, installers: "not built in M11", docs, performance: "not re-run", repo settings) and an overall line. No release step is performed.
- [ ] **Step 2:** README, `SECURITY.md` and `CHANGELOG.md` `[Unreleased]` match the code (new error texts, the marker, the TLS limits); `CLAUDE.md` "Project-specific" records the lasting knowledge of M11 (`fernjo-fs`, the channel cap, `tls::upgrade`, the resize flag, `MIN_OP_WEIGHT`).
- [ ] **Step 3: Roadmap:** tick the M11 tasks done; tick acceptance criteria with evidence (review files summarised by finding numbers, test names from Tasks 2 to 14 and 18, gate results); leave "Manual, owner" open; status stays `in-progress` (owner decision). Backlog: "M4 review follow-ups (remaining)" and "M10 review leftovers" keep only their polish parts. `dotnet run tools/roadmap.cs -- generate`, `-- check`.
- [ ] **Step 4: Full gate** again on the final head; results into the readiness section.
- [ ] **Step 5: Commit** `docs(m11): readiness verdict for v0.3.0`.

### Task 21: Pull request to `dev`

- [ ] **Step 1:** On the user's say-so: `git push -u origin feature/m11-hardening-security`, `gh pr create --base dev` with the tasks, the findings summary, the gate results and the open owner checks.
- [ ] **Step 2:** `gh pr checks` until green (Linux and macOS compile, Unix-only tests, SSH container tests in the `rust` job, CodeQL). A red check is fixed on the branch, never bypassed. Merge (squash allowed) when all are green; the milestone stays `in-progress`.

### Task 22: Owner checks and closing the milestone

- [ ] **Step 1: RDP** on the owner's Windows 11 host from `dev` after the merge: `FERNJO_TEST_RDP_ADDR=<host:port> FERNJO_TEST_RDP_USER=… FERNJO_TEST_RDP_PASSWORD=… cargo test -p fernjo-rdp --test connect`, and one connection from `pnpm tauri dev` on a scratch `FERNJO_DATA_DIR`: certificate prompt, typing, resize, clipboard both ways, the clipboard marker visible.
- [ ] **Step 2: Sync** on two real devices against the hub: pair, edit a connection on each and see it on the other, add a vault entry on one and connect with it on the other (Linux laptop if at hand).
- [ ] **Step 3:** Record the results as evidence on the M11 "Manual, owner" criterion (or the owner's explicit waiver with the date), update the known gaps and the verdict, set M11 `done` with `completedOn`, generate and check the roadmap, PR to `dev`.

---

## Self-review notes

- Spec coverage: step 1 → T1; step 2 → T2 to T5; step 3 → T6 (channel cap), T7 (write timeout; the "bounded ops channel" is met by T12's weight plus resize coalescing on the shared `Op` queue, recorded in DECISIONS by T12 Step 5), T8 (TLS), T9 (RDP Lows; the plain-`String` password goes to the owner batch in T19), T10 (username, `clear_pending`), T11 (script), T12 and T13 (leftovers), T14 (marker, restore wording); step 4 → T16; step 5 → T17 to T19; step 6 → T20; step 7 → T22; roadmap and docs → T1, T5, T20. Owner decisions (clipboard on, strict TLS, in-progress until owner checks) appear in Global Constraints and T1, T8, T20.
- Names used across tasks: `fernjo_fs::{read_capped, write_atomic, WriteOptions, Parent, FileMode, with_suffix}`, `WriteOptions::data_file`, `MAX_CHANNEL_MESSAGE`, `check_frame`, `CloseReason::ChannelTooLarge`, `WRITE_LIMIT`, `write_bounded`, `RdpError::CertificateUnsupported`, `tls::upgrade`, `PinVerifier`, `MIN_OP_WEIGHT`, `resize_queued`, `CLIPBOARD_READ_LIMIT`, `read_with_limit`, `page_reloaded`; each is defined in the task that produces it.
- Order: T3 to T5 depend on T2; T6 and T7 both edit `session.rs` (run in order); T12 touches `sessions/rdp.rs` after T9; T16 needs T15 green.
