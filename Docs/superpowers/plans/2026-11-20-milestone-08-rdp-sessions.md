# Milestone 8: RDP sessions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A connection with protocol `rdp` opens as a tab next to SSH sessions: NLA login from the vault, keyboard and mouse, text clipboard both ways, fit-to-tab resize and fullscreen, at least 30 frames/s while scrolling and dragging.

**Architecture:** A new `fernjo-rdp` crate wraps IronRDP behind an API shaped like `fernjo-ssh` (`Session::connect` returning a session plus an event receiver, an `Interaction` trait for prompts) and owns a shared RGBA framebuffer. The session manager in `src-tauri` gets a second kind of session; the forwarder sends changed regions of the framebuffer as binary messages on the existing per-session channel, coalescing while the UI is behind, and a canvas component paints them. The frame transport and canvas tasks sit behind a measured gate (Phase 2); everything else does not depend on its outcome.

**Tech Stack:** Rust 2024 (workspace MSRV 1.90), IronRDP (`ironrdp` 0.17 family, all MIT OR Apache-2.0, MSRV 1.89, versions pinned in the workspace `Cargo.toml`), tokio, Tauri 2 channels, Svelte 5 + TypeScript, HTML canvas.

**Spec:** `Docs/superpowers/specs/2026-11-20-m08-rdp-sessions-design.md` (decisions) and `Docs/superpowers/specs/2026-11-15-fernjo-design.md` §5.6 (rendering path, bar). Milestone scope: `Docs/roadmap.json`, milestone "8".

## Global Constraints

- Secrets stay in Rust: the RDP password, and the domain together with it, never reach the webview, logs or `connections.json`; `RdpPlan` and every new type holding a password have a redacted `Debug` (`SecretString`).
- Adding a `#[tauri::command]` updates three places: the function plus its `handlers` wrapper, its entry in `table()` of `no_response_contains_a_secret` in `src-tauri/src/vault_commands.rs` (or `APP_HANDLE_EXEMPT` with a reason), and the `generate_handler!` list in `src-tauri/src/lib.rs`; `command_table_covers_every_tauri_command` must stay green. A new source file with commands goes into that test's `sources`.
- `fernjo-rdp` never depends on Tauri and is created in this milestone only. Cargo, clippy and tests run without a UI build.
- A new reader of a file in the data folder or device folder uses a capped read (`persist::read_capped` pattern, 1 MiB cap for the certificate file); atomic writes use the random `create_new` temp name, keep the `.bak`, rename, fsync the directory on Unix (copy of `write_atomic` in `crates/fernjo-ssh/src/known_hosts.rs`, because `fernjo-rdp` must not depend on `fernjo-vault`).
- `connections.json` moves to `schemaVersion` 3 with a no-op migration; a version 4 file is refused as a newer schema.
- Defaults: resolution `fit`, colour depth 32, clipboard on, port 3389 (`DEFAULT_RDP_PORT`). Valid colour depths 16, 24, 32. Fixed resolution width and height each 200 to 8192. Domain at most 256 characters, no control characters. At most 256 input events per `rdp_input` call. Clipboard text at most 1 MiB (`MAX_CLIPBOARD_BYTES`). A desktop larger than 8192x8192 pixels in either dimension, or zero in either, is refused (`desktop_too_large`).
- UI styling uses only tokens from `ui/src/styles/tokens.css` and follows `DESIGN.md`; shipped CSS stays hex-only without `oklch()`, `color-mix()`, `light-dark()`, nesting, `@layer`, `:has()` or container queries. UI text is English.
- RDP never reports `endedCleanly: true` (a logoff or server disconnect keeps the tab with the reason and Reconnect).
- CI uses GitHub-hosted runners only, actions pinned to SHAs. A new dependency licence outside the `deny.toml` allow-list goes to the owner, never into the allow-list unasked.
- "Done" per task: the commands named in the task are green. At the end: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm lint`, `pnpm test`, `cargo deny check`, `dotnet run tools/roadmap.cs -- check`, `bash tools/test-release-scripts.sh`.
- Commits follow Conventional Commits, no attribution lines, selective `git add <file>`; check `git show --stat HEAD` after each. Work stays on `feature/m08-rdp-sessions`.

## Review Focus

Failure modes the spec implies that no milestone task states; each has a test in the task named in brackets.

1. A server announces a zero or enormous desktop size (memory exhaustion, division by zero when scaling): refused with `desktop_too_large`, nothing allocated. [Task 3]
2. The tab is hidden, zero-sized or dragged through many sizes: the UI never sends a width or height below 200 or a zero size, and a drag sends one resize after it settles, not one per frame. [Task 12]
3. A key is held while the canvas loses focus, the tab is switched or the session drops: every pressed key is released, so nothing stays stuck on the remote desktop. [Task 11]
4. Clipboard: text set from the remote side is not announced back (echo loop), oversize text is dropped not truncated, line endings are converted (LF locally, CRLF on the wire), and with the setting off nothing flows either way. [Tasks 3 and 13]
5. Credentials: a private-key credential on an RDP connection, a username typed as `DOMAIN\user` or `user@domain` together with a domain, a blank domain, and the credential's domain winning over the node's. [Task 5]

---

## Phase 1: groundwork that does not depend on the spike

### Task 1: RDP settings in the core model, schema 3

**Files:**
- Modify: `crates/fernjo-core/src/model.rs`, `crates/fernjo-core/src/resolve.rs`, `crates/fernjo-core/src/tree.rs` (`NodePatch`, `update`), `crates/fernjo-core/src/persist.rs` (`CURRENT_VERSION`, `MIGRATIONS`, `sanitize`), `crates/fernjo-core/src/error.rs`, `crates/fernjo-core/src/lib.rs` (re-exports)
- Modify: `src-tauri/src/commands.rs` (test pins on the node JSON only if they list every field)
- Test: unit tests next to each module, as the existing ones

**Interfaces:**
- Produces in `model.rs`: `pub const DEFAULT_RDP_COLOR_DEPTH: u8 = 32; pub const DEFAULT_RDP_CLIPBOARD: bool = true; pub const MIN_RDP_DIMENSION: u16 = 200; pub const MAX_RDP_DIMENSION: u16 = 8192; pub const MAX_DOMAIN_CHARS: usize = 256;`
- `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)] #[serde(tag = "mode", rename_all = "camelCase")] pub enum RdpResolution { Fit, Fixed { width: u16, height: u16 } }` (JSON `{"mode":"fit"}`, `{"mode":"fixed","width":1920,"height":1080}`)
- `Settings` gains `domain: Option<String>`, `rdp_resolution: Option<RdpResolution>`, `rdp_color_depth: Option<u8>`, `rdp_clipboard: Option<bool>` (serde `domain`, `rdpResolution`, `rdpColorDepth`, `rdpClipboard`, `default`, `skip_serializing_if = "Option::is_none"`).
- `ResolvedNode` gains `domain: Field<String>`, `rdp_resolution: Field<RdpResolution>`, `rdp_color_depth: Field<u8>`, `rdp_clipboard: Field<bool>`; `NodePatch` gains the same four as `Patch<_>` with `patch_from_json`.
- `TreeError` gains `InvalidResolution` (`invalid_resolution`), `InvalidColorDepth` (`invalid_color_depth`), `InvalidDomain` (`invalid_domain`).

- [ ] **Step 1: Write the failing tests**
  - `resolve.rs`: `rdp_defaults_are_fit_32_and_clipboard_on` (source `Default` for all four, `domain.effective` `None`); `rdp_settings_inherit_from_a_folder` (folder sets `rdp_color_depth` 16, connection sets nothing, effective 16, source `Folder`); `a_connection_value_beats_the_folder`.
  - `tree.rs`: `invalid_color_depth_is_refused` (for 8 and 15 and 33, code `invalid_color_depth`); `fixed_resolution_outside_200_to_8192_is_refused` (width 199, 8193, height 0); `domain_with_a_control_character_or_257_characters_is_refused`; `patch_null_clears_an_rdp_setting`; `node_patch_parses_rdp_fields_from_json` (`{"rdpColorDepth":16,"rdpResolution":{"mode":"fixed","width":1920,"height":1080}}`).
  - `persist.rs`: `version_two_file_loads_and_is_saved_as_three`; `version_four_file_is_refused_as_newer_schema`; `rdp_settings_round_trip_through_the_file`; `an_invalid_stored_rdp_value_is_dropped_on_load` (a hand-edited depth 7 or width 5 loads as unset, like port 0 in `sanitize`).
  - Update every existing test that pins `schemaVersion` 2 to 3 and the "version three is refused" test to version four.
- [ ] **Step 2: Run** `cargo test -p fernjo-core` — expect the new tests to fail to compile or fail.
- [ ] **Step 3: Implement** the interface above. `update` validates after applying the patch, before saving, and returns the new errors; the migration `two_to_three` is a no-op with the same doc comment as `one_to_two` (an older build would drop the fields on save).
- [ ] **Step 4: Run** `cargo test -p fernjo-core -p fernjo-import -p fernjo-app` — all green; fix golden or fixture pins of `schemaVersion` in `fernjo-import` tests.
- [ ] **Step 5: Commit** `feat(core): RDP settings (domain, resolution, colour depth, clipboard) and schema 3`

### Task 2: RDP rows in the properties panel

**Files:**
- Modify: `ui/src/lib/types.ts` (`RdpResolution`, `ResolvedNode` fields, `NodePatch`), `ui/src/lib/testing.ts` (mock node), `ui/src/lib/components/PropertiesPanel.svelte`, `ui/src/lib/components/PropertiesPanel.test.ts`
- Modify: `src-tauri/src/commands.rs` test that checks the node JSON (add the four fields)

**Interfaces:**
- Consumes: the four resolved fields and patch fields of Task 1, JSON names `domain`, `rdpResolution`, `rdpColorDepth`, `rdpClipboard`.
- Produces: rows Domain, Resolution (select "Fit to tab" / "Fixed size" plus width and height inputs), Colour depth (select 16, 24, 32), Clipboard (checkbox) shown when `isFolder || detail.protocol === "rdp"` (`showRdp`, next to `showKeepalive`), each with the existing inherit source note and reset; row order puts them after Port.

- [ ] **Step 1: Write the failing tests** in `PropertiesPanel.test.ts`: `rdp_connection_shows_the_rdp_rows_and_no_keepalive`; `ssh_connection_shows_no_rdp_rows`; `folder_shows_both_groups`; `changing_the_colour_depth_saves_rdpColorDepth` (api mock called with `{ rdpColorDepth: 16 }`); `choosing_fixed_size_saves_rdpResolution_with_width_and_height`; `an_invalid_width_shows_the_error_and_does_not_save`; `reset_sends_null`.
- [ ] **Step 2: Run** `pnpm test -- PropertiesPanel` — expect FAIL.
- [ ] **Step 3: Implement** with existing `PropertyRow`, `TextField` and tokens; validation of the number inputs mirrors Rust (the UI shows Rust's `invalid_resolution` message and never derives validity itself, so only parse numbers locally).
- [ ] **Step 4: Run** `pnpm test && pnpm lint` — green. Check the Credential row stays visible at 1280x800 in a Chromium harness with the mocked API (state it as harness-only evidence).
- [ ] **Step 5: Commit** `feat(ui): RDP settings rows in the properties panel`

### Task 3: `fernjo-rdp` crate: pure parts

**Files:**
- Create: `crates/fernjo-rdp/Cargo.toml`, `crates/fernjo-rdp/src/lib.rs`, `error.rs`, `framebuffer.rs`, `input.rs`, `clipboard.rs`, `cert_store.rs`, `interaction.rs`
- Modify: root `Cargo.toml` (`serde` already a workspace dependency; no IronRDP yet)

**Interfaces (all `pub`, re-exported from `lib.rs`):**
- `struct Rect { x: u16, y: u16, width: u16, height: u16 }` with `fn union(self, other: Rect) -> Rect`, `fn clamp_to(self, w: u16, h: u16) -> Option<Rect>`.
- `struct Framebuffer` (clone is cheap, `Arc<Mutex<_>>` inside): `fn new(width: u16, height: u16) -> Result<Framebuffer, RdpError>` (`desktop_too_large` above 8192 in either dimension or zero), `fn resize(&self, w: u16, h: u16) -> Result<(), RdpError>`, `fn size(&self) -> (u16, u16)`, `fn write_region(&self, r: Rect, rgba: &[u8])`, `fn copy_region(&self, r: Rect) -> Vec<u8>` (tightly packed RGBA, clipped to the buffer).
- `enum InputEvent` (serde `tag = "kind"`, camelCase): `Key { scancode: u8, extended: bool, pressed: bool }`, `MouseMove { x: u16, y: u16 }`, `MouseButton { button: MouseButton, pressed: bool }`, `Wheel { vertical: bool, delta: i16 }`; `enum MouseButton { Left, Middle, Right, X1, X2 }`; `const MAX_INPUT_EVENTS: usize = 256`.
- `clipboard`: `const MAX_CLIPBOARD_BYTES: usize = 1024 * 1024`; `fn to_remote(text: &str) -> Option<String>` (LF to CRLF, `None` above the cap), `fn from_remote(text: &str) -> Option<String>` (CRLF to LF, trailing NUL removed, `None` above the cap).
- `CertStore`: `fn open(path: PathBuf) -> CertStore`, `fn in_memory() -> CertStore`, `fn check(&self, host: &str, port: u16, fingerprint: &str) -> Result<CertCheck, RdpError>`, `fn pin(&self, host: &str, port: u16, fingerprint: &str) -> Result<(), RdpError>`, `fn replace(...)` same signature; `enum CertCheck { Unknown, Match, Changed { old: String } }`. File format one line per pin `host:port sha256:<hex>`; a host with whitespace or control characters is `invalid_host`.
- `trait Interaction: Send + Sync + 'static` (async fns via `impl Future`, as `fernjo_ssh::Interaction` does): `certificate(&self, CertPrompt) -> CertDecision`, `certificate_changed(&self, CertChangedPrompt) -> CertChangedDecision`, `username(&self, host: &str) -> Option<String>`, `password(&self, user: &str, host: &str) -> Option<SecretString>`; `struct CertPrompt { host: String, port: u16, fingerprint: String }`, `struct CertChangedPrompt { host, port, old: String, new: String }`, `enum CertDecision { TrustOnce, TrustAlways, Reject }`, `enum CertChangedDecision { Replace, Reject }`.
- `RdpError` (thiserror) with `fn code(&self) -> &'static str`: `invalid_host`, `connect_failed`, `timeout`, `auth_failed`, `tls_failed`, `certificate_rejected`, `desktop_too_large`, `protocol`, `lost`.

- [ ] **Step 1: Write the failing tests** (unit tests in each file):
  - framebuffer: `refuses_zero_and_oversize_desktops` (0x600, 8193x600, 100000x1), `write_then_copy_round_trips_a_region`, `copy_region_clips_to_the_buffer`, `resize_keeps_a_valid_size_and_refuses_a_bad_one`, `rect_union_is_the_bounding_box`.
  - clipboard: `lf_becomes_crlf_on_the_way_out_and_back`, `text_over_one_mebibyte_is_dropped_not_truncated`, `a_trailing_nul_from_the_remote_is_removed`.
  - input: `events_parse_from_json` (one of each kind), `an_unknown_kind_is_an_error`.
  - cert store: `unknown_then_pinned_then_match`, `a_different_fingerprint_is_changed_with_the_old_one`, `replace_overwrites_the_pin`, `a_file_larger_than_the_cap_is_unreadable_and_untouched`, `a_host_with_a_space_is_invalid_host`, `the_pin_survives_reopening_the_file`, `in_memory_keeps_pins_for_the_run_only`.
- [ ] **Step 2: Run** `cargo test -p fernjo-rdp` — expect FAIL.
- [ ] **Step 3: Implement.** RGBA storage `Vec<u8>` of `w*h*4`; the cert file writer is the hardened `write_atomic` copy named in Global Constraints.
- [ ] **Step 4: Run** `cargo test -p fernjo-rdp && cargo clippy -p fernjo-rdp --all-targets -- -D warnings && cargo fmt --all --check` — green.
- [ ] **Step 5: Commit** `feat(rdp): fernjo-rdp crate with framebuffer, input, clipboard text and certificate store`

### Task 4: `fernjo-rdp`: IronRDP session (TLS, NLA, input, resize, clipboard)

**Files:**
- Modify: root `Cargo.toml` (workspace dependencies for the IronRDP crates this task needs, exact versions, only the features used), `crates/fernjo-rdp/Cargo.toml`, `deny.toml` only if the owner approves a licence
- Create: `crates/fernjo-rdp/src/session.rs`, `connect.rs`, `crates/fernjo-rdp/tests/support/mod.rs`, `crates/fernjo-rdp/tests/connect.rs`

**Interfaces:**
- Consumes: Task 3 types.
- Produces: `struct ConnectParams { host: String, port: u16, username: String, password: SecretString, domain: Option<String>, width: u16, height: u16, color_depth: u8, clipboard: bool, dynamic_resize: bool, certs: CertStore, connect_timeout: Duration }` with `fn new(host, port, username, password) -> ConnectParams` (defaults: 1280x800, depth 32, clipboard on, dynamic resize on, in-memory certs, 15 s) and a redacted `Debug`.
- `struct Session` with `async fn connect<I: Interaction>(params: ConnectParams, interaction: Arc<I>) -> Result<(Session, tokio::sync::mpsc::Receiver<SessionEvent>), RdpError>`, `fn framebuffer(&self) -> Framebuffer`, `async fn input(&self, events: Vec<InputEvent>) -> Result<(), RdpError>`, `async fn resize(&self, width: u16, height: u16) -> Result<(), RdpError>` (display-control resize when the server supports it, else `Ok` and the size stays), `async fn set_clipboard_text(&self, text: String) -> Result<(), RdpError>` (no-op when the clipboard is off), `fn close(&self)`.
- `enum SessionEvent { Damage(Rect), DesktopSize { width: u16, height: u16 }, Clipboard(String), Closed(CloseReason) }`; `enum CloseReason { ServerClosed, ConnectionLost, ClosedByUser }`. The event queue is bounded (8) and `Damage` events are merged when the queue is full, so a slow consumer never grows memory.

- [ ] **Step 1: Find the test host.** First check whether `ironrdp-server` (0.13) can serve an in-process test server with TLS and CredSSP in `tests/support/mod.rs`. If it can, the tests below run against it in CI and never skip. If it cannot within a day's effort, the tests read `FERNJO_TEST_RDP_ADDR`, `FERNJO_TEST_RDP_USER`, `FERNJO_TEST_RDP_PASSWORD`, `FERNJO_TEST_RDP_DOMAIN` and skip with a message when unset, **also in CI** (there is no CI RDP host; that is an owner check, recorded as open with the command `FERNJO_TEST_RDP_ADDR=... cargo test -p fernjo-rdp --test connect`). Record the outcome in `Docs/DECISIONS.md` "M8 RDP test host".
- [ ] **Step 2: Write the failing tests** in `tests/connect.rs`: `connects_with_nla_and_receives_damage`, `wrong_password_is_auth_failed`, `a_new_certificate_asks_and_reject_ends_with_certificate_rejected`, `a_pinned_certificate_connects_without_asking`, `a_changed_certificate_asks_and_replace_pins_the_new_one`, `unreachable_host_is_connect_failed_within_the_timeout`, `input_events_reach_the_server` (if the host is the in-process server), `close_ends_with_closed_by_user`, `clipboard_text_round_trips_with_the_setting_on_and_not_with_it_off`.
- [ ] **Step 3: Run** `cargo test -p fernjo-rdp --test connect` — expect FAIL.
- [ ] **Step 4: Implement** over `ironrdp` connector, session and tokio framing with `ironrdp-tls` for TLS; the certificate fingerprint is the SHA-256 of the server's DER certificate (`sha2` is a workspace dependency); a certificate prompt is raised before credentials are sent (CredSSP starts after the certificate decision). Map IronRDP errors to the `RdpError` codes; never put the password into an error string. Cap the desktop size through `Framebuffer::new` and `resize` on every `DesktopSize` change from the server.
- [ ] **Step 5: Run** `cargo test -p fernjo-rdp && cargo clippy --workspace --all-targets -- -D warnings && cargo deny check` — green; `cargo deny check` output for the new crates stated in the commit body.
- [ ] **Step 6: Commit** `feat(rdp): IronRDP session with TLS, NLA, input, resize and clipboard`

### Task 5: Connect plan for RDP

**Files:**
- Modify: `src-tauri/src/connect_plan.rs`, `src-tauri/Cargo.toml` (depend on `fernjo-rdp`), `src-tauri/src/sessions.rs` (call sites of `resolve`)
- Test: `connect_plan.rs` tests module

**Interfaces:**
- Consumes: `fernjo_core::{DEFAULT_RDP_PORT, DEFAULT_RDP_COLOR_DEPTH, DEFAULT_RDP_CLIPBOARD, RdpResolution}`, `fernjo_vault` entry `domain`.
- Produces: rename `resolve` to `resolve_ssh` (same behaviour, an RDP node is refused with `unsupported_protocol` and the text "this connection is not an SSH connection"); `pub enum Plan { Ssh(ConnectPlan), Rdp(RdpPlan) }`; `pub fn resolve_session(state: &AppState, node_id: NodeId, skip_vault: bool) -> Result<Plan, CommandError>`; `pub struct RdpPlan { pub host: String, pub port: u16, pub username: Option<String>, pub domain: Option<String>, pub resolution: RdpResolution, pub color_depth: u8, pub clipboard: bool, pub credential: RdpCredentialPlan }`; `pub enum RdpCredentialPlan { None, Missing(MissingReason), NeedsVault, Ready(SecretString) }`; `MissingReason` gains `WrongKind` ("The credential is not a password").
- Rules: host validation as for SSH (`fernjo_ssh::validate_host`, `invalid_host`); the node's or inherited username beats the entry's; the entry's domain beats the node's domain; blank values count as unset; a `DOMAIN\user` or `user@domain` username keeps the domain `None` unless a domain was configured, in which case the typed form is kept as is and the domain is still passed (no prefixing).

- [ ] **Step 1: Write the failing tests:** replace `rdp_is_refused` with `resolve_ssh_refuses_an_rdp_node`; add `resolve_session_returns_the_rdp_plan_with_defaults` (port 3389, `Fit`, 32, clipboard on), `rdp_settings_are_inherited_from_a_folder`, `credential_domain_beats_node_domain`, `blank_domain_is_unset`, `private_key_credential_is_missing_wrong_kind`, `typed_domain_backslash_user_is_kept`, `locked_vault_is_needs_vault`, `skip_vault_is_missing_skipped`, `invalid_host_is_refused_for_rdp`, `rdp_plan_debug_shows_no_password` (sentinel password).
- [ ] **Step 2: Run** `cargo test -p fernjo-app connect_plan` — expect FAIL.
- [ ] **Step 3: Implement.** `RdpPlan` has a manual `Debug` or only redacting field types.
- [ ] **Step 4: Run** `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings` — green (the `no_response_contains_a_secret` test still passes).
- [ ] **Step 5: Commit** `feat(app): resolve a connect plan for RDP connections`

### Task 6: mRemoteNG import maps resolution and colour depth

**Files:**
- Modify: `crates/fernjo-import/src/mremoteng/file.rs` (read `Resolution` and `Colors`, cut at 64 characters like `protocol`), `crates/fernjo-import/src/mremoteng/map.rs`, `crates/fernjo-import/src/report.rs` if a partial-mapping note is needed
- Test: `crates/fernjo-import/tests/map.rs`; a hand-written fixture only if the existing tests have no in-code builder (never regenerate the .NET fixtures)

**Interfaces:**
- Consumes: Task 1 settings. Produces: an RDP connection with `Colors="Colors16Bit"|"Colors24Bit"|"Colors32Bit"` gets `rdp_color_depth` 16, 24, 32; `Resolution="FitToWindow"|"Fullscreen"` maps to `Fit`, `Res1920x1080`-style values to `Fixed`, anything else is left unset and counted as partially mapped in the report. Domain stays on the vault entry as today (no node domain).

- [ ] **Step 1: Write the failing tests:** `colors_map_to_color_depth`, `fit_to_window_maps_to_fit`, `fixed_resolution_maps_to_width_and_height`, `unknown_resolution_is_unset_and_reported`, `an_overlong_resolution_attribute_is_cut`, `ssh_connections_get_no_rdp_settings`.
- [ ] **Step 2: Run** `cargo test -p fernjo-import` — expect FAIL.
- [ ] **Step 3: Implement** with the existing bounded-attribute helpers (`attr`, `MAX_*_CHARS`); no copying of mRemoteNG source.
- [ ] **Step 4: Run** `cargo test -p fernjo-import` — green, including `owner.rs`.
- [ ] **Step 5: Commit** `feat(import): map mRemoteNG RDP colour depth and resolution`

---

## Phase 2: spike and gate

### Task 7: Throwaway spike (own branch)

**Files (all on branch `spike/m08-rdp-canvas`, created from the current feature branch, never merged, not deleted without the owner's say):**
- Create: `crates/fernjo-rdp/examples/spike_bench.rs`, a debug-only command `spike_stream` in `src-tauri/src/spike.rs` registered only on that branch, `ui/src/spike/SpikePage.svelte` reachable by the URL hash `#spike`

**Interfaces:** none kept; nothing from the spike is merged.

- [ ] **Step 1: Headless benchmark.** `spike_bench` connects with `fernjo_rdp::Session` to `FERNJO_TEST_RDP_*`, runs for 30 s while the owner scrolls and drags a window (it prints a countdown), and prints: damage events per second, merged-region frames per second at the consumer, mean and p95 bytes per message, mean decode-to-damage latency. Run: `cargo run -p fernjo-rdp --release --example spike_bench`.
- [ ] **Step 2: Canvas page.** `spike_stream(channel, width, height, mode)` sends synthetic RGBA region messages (header `u16 x, y, w, h` little endian, then pixels): mode `full` sends 1920x1080 every frame, mode `scroll` sends a 1920x900 region moved each frame, mode `drag` sends a 400x300 region moved each frame, always as fast as acknowledged (one message in flight). `SpikePage.svelte` paints with `putImageData`, acknowledges after paint, and shows painted frames per second and mean ms per message for 10 s per mode.
- [ ] **Step 3: Build the spike** for `pnpm tauri build --no-bundle` (Windows, macOS, Linux release binaries). State what was built here and what only compiles in CI. Do not run the app on this machine until the WebView2 runtime is installed (`winget install Microsoft.EdgeWebView2Runtime`); until then the page is verified only by `svelte-check` and a Chromium harness.
- [ ] **Step 4: Commit on the spike branch** `spike: RDP canvas path measurement (throwaway)`; push is the owner's call.

### Task 8: Owner measures; record; gate (STOP)

**Files:** Modify `Docs/DECISIONS.md` (new entry "M8 RDP render path spike"), `Docs/roadmap.json` (task and criterion), regenerate `Docs/ROADMAP.md`.

- [ ] **Step 1: Hand over to the owner** with this exact list: Windows 11 PC, macOS laptop and CachyOS laptop each run the spike page in all three modes at 1920x1080 and note painted frames per second per mode; one of them runs `spike_bench` against the Windows 11 or Windows Server host (NLA on) and types in a real session through the benchmark build for a latency judgment. Stop here and wait; do not start Phase 3.
- [ ] **Step 2: Record the numbers** per OS and mode, the date, the machines, the IronRDP versions and what was optimised, in `Docs/DECISIONS.md`.
- [ ] **Step 3: Apply the gate.** Pass means at least 30 frames/s while scrolling and dragging on every OS and the owner calls latency acceptable. If a number misses, try these in order, re-measuring after each, before declaring a miss: (a) coalesce damage to one region per painted frame, (b) send only the dirty region instead of the full frame, (c) keep two messages in flight, (d) paint via `createImageBitmap` or a WebGL texture upload instead of `putImageData`, (e) lower the colour depth sent to the UI. The first step that passes is the transport for Task 10 (its constant for messages in flight comes from here).
- [ ] **Step 4: Tick** the roadmap task "Throwaway spike" and the criterion "Spike measured ≥ 30 frames/s while scrolling and dragging; owner judges keystroke latency acceptable" with the numbers as evidence only if they pass; regenerate with `dotnet run tools/roadmap.cs -- generate` and check with `-- check`.
- [ ] **Step 5: Commit** `docs: record the M8 spike result`. On a pass continue with Task 10; on a miss go to Task 9.

### Task 9: Only if the gate fails: scope change

- [ ] **Step 1:** Add the scope change to the roadmap ("Scope changes"): a native-surface milestone (wgpu or softbuffer over the tab area) inserted before v0.2.0 and M8 reduced to the groundwork (Tasks 1 to 6) plus the numbers; record the decision and the numbers in `Docs/DECISIONS.md`.
- [ ] **Step 2:** Regenerate and check the roadmap, commit `docs(roadmap): native render surface milestone before v0.2.0`, stop and report. Tasks 10 to 14 are replaced by the new milestone's plan.

---

## Phase 3: build on the canvas path (only after a passed gate)

### Task 10: Session wiring and frame transport (Rust)

**Files:**
- Create: `src-tauri/src/rdp_session.rs` (connect task, interaction, forwarder), `src-tauri/src/rdp_commands.rs`
- Modify: `src-tauri/src/sessions.rs` (`Op`, `UiEvent`, `SessionManager::open/begin/start`, wire-contract doc comment), `src-tauri/src/lib.rs` (`generate_handler!`, module list), `src-tauri/src/vault_commands.rs` (`table()`, `sources`), `src-tauri/src/session_commands.rs` only where `open_session` text mentions terminals
- Test: `src-tauri/src/sessions/tests.rs`, `rdp_session.rs` tests, `rdp_commands.rs` tests

**Interfaces:**
- Consumes: `connect_plan::{resolve_session, Plan, RdpPlan, RdpCredentialPlan}`, `fernjo_rdp::{Session, SessionEvent, CloseReason, Rect, InputEvent, CertStore, Interaction}`, `FlowControl::new(high, low)`.
- Produces on the wire (documented at the top of `sessions.rs`, extended not forked): for an RDP session the raw message is a region `u16 x, u16 y, u16 width, u16 height` (little endian) followed by `width*height*4` RGBA bytes; new JSON events `{kind:"desktop", width, height}` (sent before the first region and on every desktop size change) and `{kind:"clipboard", text}`; `session_ack(id, bytes)` acknowledges region messages by their byte length; `open_session`, `session_resize` and `reconnect_session` carry pixel width and height in their `cols` and `rows` arguments for RDP (documented, not renamed).
- `PromptView` gains `Certificate { host, port, fingerprint }` and `CertificateChanged { host, port, old, new }`; answers reuse `PromptAnswer::HostKey { decision }` for the first and `PromptAnswer::HostKeyChanged { decision }` for the second; a cancel or wrong shape is the safe answer (reject). `Username` and `Password` prompts are reused for a missing credential.
- New commands (three-places rule): `rdp_input(id: String, events: Vec<InputEvent>) -> Result<(), CommandError>` (`session_not_connected` without a live session, `too_many_events` above 256, coordinates clamped to the desktop size), `rdp_clipboard(id: String, text: String) -> Result<(), CommandError>`; both `table()` entries use a sentinel text and the no-secret assertion.
- Manager: the `RdpPlan` path starts `run_rdp`; `Op` gains `Rdp(Vec<InputEvent>)` and `Clipboard(String)`; `Op::Resize(w, h)` carries pixels; `connected_count`, `close_all`, `reconnect`, `resume` and the close confirmation work unchanged for RDP tabs. The certificate file is `<device_dir>/rdp_certs.confirmed`, `CertStore::in_memory()` held by the manager without a device folder, exactly like `confirmed_in_memory`.
- Forwarder: keeps the union of `Damage` rectangles while no credit is available, then sends one region message copied from the framebuffer (latest pixels win); credit is `FlowControl::new(RDP_MAX_IN_FLIGHT_BYTES, 0)` where the constant comes from Task 8's result.
- `classify_rdp_end(reason) -> Ended` never sets `clean`; texts: "The remote desktop session ended.", "The connection was lost.", "The session was closed."

- [ ] **Step 1: Write the failing tests:** `open_session_for_an_rdp_node_starts_an_rdp_session` (with the test host or a stub `Session` seam), `damage_is_coalesced_while_the_ui_is_behind` (many `Damage` events, one unacked message, exactly one message after the ack and it covers the union), `region_message_header_and_length_are_exact`, `desktop_event_precedes_the_first_region`, `rdp_end_is_never_clean` (server closed after 60 s), `rdp_input_without_a_live_session_is_session_not_connected`, `rdp_input_over_256_events_is_refused`, `rdp_input_coordinates_are_clamped`, `certificate_prompt_reject_ends_the_attempt`, `closing_a_connecting_rdp_session_answers_open_prompts_safely`, `connected_count_counts_rdp_tabs`, `command_table_covers_every_tauri_command`, `no_response_contains_a_secret` (RDP plan with sentinel password in the vault).
- [ ] **Step 2: Run** `cargo test -p fernjo-app` — expect FAIL.
- [ ] **Step 3: Implement** in `rdp_session.rs`; keep `sessions.rs` changes to dispatch, `Op`/`UiEvent` additions and the doc comment.
- [ ] **Step 4: Run** `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check` — green.
- [ ] **Step 5: Commit** `feat(app): RDP sessions in the session manager with region streaming`

### Task 11: RDP tab in the UI: store, controller, key and pointer mapping, canvas view

**Files:**
- Create: `ui/src/lib/rdp/keymap.ts`, `pointer.ts`, `controller.ts`, `RdpView.svelte`, and a `.test.ts` for each
- Modify: `ui/src/lib/types.ts` (`RdpInputEvent`, `SessionEvent` variants `desktop` and `clipboard`, `Prompt` variants `certificate` and `certificateChanged`, `Api.rdpInput`, `Api.rdpClipboard`), `ui/src/lib/api.ts`, `ui/src/lib/testing.ts`, `ui/src/lib/sessions.svelte.ts` (`Tab.protocol`, `protocolOf` constructor argument, `attachRemote`, message handling by protocol), `ui/src/lib/components/SessionArea.svelte` (render `RdpView` for RDP tabs), `ui/src/App.svelte` (pass `protocolOf`)

**Interfaces:**
- Produces: `scancodeFor(code: string): { scancode: number; extended: boolean } | null` (PC set 1 from `KeyboardEvent.code`, not `key`); `toDesktop(clientX: number, clientY: number, box: { left: number; top: number; width: number; height: number }, desktop: { width: number; height: number }): { x: number; y: number }` (clamped, rounded); `buttonOf(button: number): "left" | "middle" | "right" | "x1" | "x2" | null`.
- `class RdpController` constructed with `{ canvas: HTMLCanvasElement, send(events: RdpInputEvent[]): void, requestAnimationFrame?, now? }`: `setDesktop(width, height)`, `paint(rect, rgba: Uint8Array)`, `keyDown(e)`, `keyUp(e)`, `pointerMove(e)`, `pointerButton(e, pressed)`, `wheel(e)`, `releaseAll()`, `blur()`; input is batched to at most one `send` per animation frame, key events flush immediately.
- `SessionStore.attachRemote(id, bridge: { paint(rect, rgba): void; setDesktop(w, h): void; clipboard(text): void; size(): { width: number; height: number } })`; the region parser reads the 8-byte header and passes `new Uint8Array(buffer, 8)`; `Tab.protocol: Protocol`; `input` for RDP goes through `api.rdpInput` only while `state === "connected"`.

- [ ] **Step 1: Write the failing tests:** keymap `letters_digits_arrows_and_numpad_enter_map_to_set_1` (`KeyA` 0x1E, `ArrowUp` 0x48 extended, `NumpadEnter` 0x1C extended, `ControlRight` 0x1D extended), `unknown_code_is_null`, `no_two_codes_share_a_scancode_and_extended_flag`; pointer `maps_client_to_desktop_pixels_when_scaled`, `clamps_to_the_desktop`; controller `blur_releases_pressed_keys` (Review Focus 3, also on tab switch via `releaseAll` and on state `disconnected`), `mouse_moves_are_coalesced_per_frame`, `key_events_are_sent_at_once`, `paint_writes_pixels_at_the_rect`; store `rdp_region_messages_go_to_the_remote_bridge_and_not_the_terminal`, `desktop_event_sizes_the_canvas`, `input_is_dropped_unless_connected`, `an_rdp_tab_has_protocol_rdp`; `SessionArea` `an_rdp_tab_renders_a_canvas_not_a_terminal`.
- [ ] **Step 2: Run** `pnpm test` — expect FAIL.
- [ ] **Step 3: Implement.** Pixel scaling uses the canvas CSS box against the desktop size; the canvas gets focus on pointer down, `tabindex="0"`, `contextmenu` suppressed, tokens only (`--term-bg` behind it).
- [ ] **Step 4: Run** `pnpm test && pnpm lint` — green. State the Chromium-harness-only evidence for anything not run in the real app.
- [ ] **Step 5: Commit** `feat(ui): RDP tab with canvas, keyboard and pointer input`

### Task 12: Fit to tab, dynamic resize, fullscreen

**Files:**
- Modify: `ui/src/lib/rdp/controller.ts` (resize observer), `ui/src/lib/rdp/RdpView.svelte`, `ui/src/lib/sessions.svelte.ts` (`remoteFullscreen`), `ui/src/App.svelte` (hide sidebar and tab strip while `remoteFullscreen`), `ui/src/lib/shortcuts.ts` (`fullscreen` action, `routeKey` gets a focus kind), `ui/src/lib/shortcuts.test.ts`, `ui/src/lib/components/SettingsDialog.svelte` (list entry from `SHORTCUTS`)
- Modify (Rust, only if the call needs it): `SessionManager::resize` for RDP is already the pixel resize of Task 10.

**Interfaces:**
- Produces: `routeKey(e: Key, mac: boolean, focus: "none" | "terminal" | "remote"): AppAction | null` replacing the `inTerminal` boolean (update every caller and test); in `remote` focus only `fullscreen` (Ctrl+Shift+F11 on every platform, `AppAction` `"fullscreen"`) and `nextTab`/`prevTab` (Ctrl+Tab, Ctrl+Shift+Tab) are intercepted, everything else is forwarded; `SessionStore.remoteFullscreen: boolean`, `toggleFullscreen()`; the resolution mode comes from the effective `rdpResolution`: `fit` observes the box, `fixed` sends nothing and scales the canvas to fit with the aspect ratio kept.
- Resize: debounced 150 ms after the last observed change, width and height rounded down to even numbers, clamped to 200..=8192, never zero (Review Focus 2); when the server cannot resize the canvas scales instead.

- [ ] **Step 1: Write the failing tests:** `a_drag_through_many_sizes_sends_one_resize_after_it_settles`, `zero_or_hidden_box_sends_no_resize`, `sizes_are_even_and_clamped`, `fixed_mode_never_sends_a_resize_and_scales`; shortcuts `remote_focus_forwards_everything_except_fullscreen_and_tab_chords` (Ctrl+Shift+W and Ctrl+Shift+K reach the remote), `terminal_focus_is_unchanged`, `settings_list_and_routeKey_agree_for_fullscreen`; app `fullscreen_hides_the_sidebar_and_tab_strip_and_the_chord_brings_them_back`, `a_dropped_session_leaves_fullscreen`.
- [ ] **Step 2: Run** `pnpm test` — expect FAIL.
- [ ] **Step 3: Implement.** A short hint ("Press Ctrl+Shift+F11 to leave full screen") shows for 3 s on entering fullscreen, styled with tokens.
- [ ] **Step 4: Run** `pnpm test && pnpm lint` — green.
- [ ] **Step 5: Commit** `feat(ui): fit-to-tab resize and in-app fullscreen for RDP tabs`

### Task 13: Clipboard both ways

**Files:**
- Create: `ui/src/lib/rdp/clipboard.ts`, `ui/src/lib/rdp/clipboard.test.ts`
- Modify: `ui/src/lib/rdp/RdpView.svelte`, `ui/src/lib/sessions.svelte.ts`
- Modify (Rust): `src-tauri/src/rdp_session.rs` only if the event wiring of Task 10 needs a fix

**Interfaces:**
- Produces: `class ClipboardSync` constructed with `{ read(): Promise<string>; write(text: string): Promise<void>; send(text: string): void; timeoutMs?: number }`: `onRemote(text)` writes locally and remembers it as last seen, `syncLocal()` reads with a timeout (default 1500 ms, the terminal paste precedent in `Docs/DECISIONS.md` "M4 clipboard path"), and sends only text that differs from the last seen one and is at most 1 MiB; a failed read or write reports `blocked` once through an `onBlocked` callback (the tab shows one line "The clipboard is blocked by the web view"). `syncLocal` runs on canvas focus and on the paste chord before the key is forwarded. The effective `rdpClipboard` false disables the sync object entirely.

- [ ] **Step 1: Write the failing tests:** `text_from_the_remote_is_written_and_not_sent_back` (Review Focus 4), `local_text_that_changed_is_sent_once`, `unchanged_local_text_is_not_sent_again`, `text_over_one_mebibyte_is_not_sent`, `a_slow_read_times_out_and_reports_blocked_once`, `a_failed_write_reports_blocked`, `with_the_setting_off_nothing_is_sent_or_written`. Rust (Task 3 already covers CRLF and caps): `remote_clipboard_event_reaches_the_ui_channel` in the session tests with the setting on and none with it off.
- [ ] **Step 2: Run** `pnpm test` — expect FAIL.
- [ ] **Step 3: Implement.** Record in `Docs/DECISIONS.md` "M8 clipboard path" which mechanism each OS' web view allows, from the owner's check (open until run); the backlog item "Native clipboard commands" stays and is cited as the fix if `blocked` shows up on a platform.
- [ ] **Step 4: Run** `pnpm test && pnpm lint && cargo test -p fernjo-app` — green.
- [ ] **Step 5: Commit** `feat: text clipboard for RDP tabs in both directions`

### Task 14: Certificate prompts in the UI

**Files:**
- Create: `ui/src/lib/components/CertificatePrompt.svelte`, `CertificateChangedPrompt.svelte` with `.test.ts`
- Modify: `ui/src/lib/components/SessionPrompt.svelte` (route the two new prompt kinds)

**Interfaces:**
- Consumes: `Prompt` variants `certificate` and `certificateChanged` (Task 11 types), answers `{ kind: "hostKey", decision }` and `{ kind: "hostKeyChanged", decision }`.
- Produces: components built on `PromptFrame`, with the wording "Trust this server certificate?" (choices "Trust once", "Trust always", "Cancel", each explained as in `HostKeyPrompt`) and "The server certificate has changed" (blocking, "Replace and connect" and "Cancel"), fingerprint shown as `sha256:<hex>`.

- [ ] **Step 1: Write the failing tests** mirroring `HostKeyPrompt.test.ts` and `HostKeyChangedPrompt.test.ts`: `shows_host_port_and_fingerprint`, `choices_answer_with_the_decision`, `escape_cancels`, `changed_shows_old_and_new_and_defaults_to_cancel`, `session_prompt_routes_certificate_kinds`.
- [ ] **Step 2: Run** `pnpm test -- Certificate` — expect FAIL.
- [ ] **Step 3: Implement** reusing `PromptFrame` focus handling and tokens.
- [ ] **Step 4: Run** `pnpm test && pnpm lint` — green.
- [ ] **Step 5: Commit** `feat(ui): server certificate prompts for RDP`

---

## Phase 4: documentation, verification, hand-over

### Task 15: Docs, roadmap, changelog, checks

**Files:** Modify `Docs/DECISIONS.md` ("M8 session IPC", "M8 RDP certificate pinning", "M8 clipboard path", "M8 RDP test host", "M8 settings and schema 3"), `CLAUDE.md` (Project-specific: `fernjo-rdp`, wire contract, certificate file, commands in the three-places rule, no-secret table, test-host rule), `CHANGELOG.md` ([Unreleased], user-facing), `README.md` (RDP in the feature list, known limits found in the checks), `Docs/roadmap.json`, regenerate `Docs/ROADMAP.md`.

- [ ] **Step 1:** Write the DECISIONS entries and the CLAUDE.md additions (names of files and types, no line numbers).
- [ ] **Step 2:** Tick the roadmap tasks that are done, with evidence for acceptance criteria only where verified: "Build, test and lint are green" with the commands and results; leave "Manual: connect to Windows 11 / Windows Server with NLA; clipboard both ways, resize and fullscreen work" open with the owner steps below.
- [ ] **Step 3: Run the whole set** and state each result: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (SSH tests skip locally without the container), `pnpm lint`, `pnpm test`, `cargo deny check`, `dotnet run tools/roadmap.cs -- check`, `bash tools/test-release-scripts.sh`.
- [ ] **Step 4: Owner checks to list in the roadmap block "Open manual checks (owner)":** (a) Windows 11 and Windows Server with NLA: connect from the tree with a vault credential, certificate prompt once then silent, type in a real session, scroll and drag, resize the window and the sidebar, fullscreen in and out, clipboard both ways with text from Notepad and from fernjo's own terminal tab, logoff on the server ends the tab with the reason; (b) the same on macOS and on the CachyOS laptop under Wayland; (c) the real-app rerun on a machine with a working WebView2 runtime of everything the Chromium harness covered; (d) latency judgment against the OS's own RDP client; (e) `FERNJO_TEST_RDP_ADDR=... cargo test -p fernjo-rdp --test connect` if the in-process test host did not work.
- [ ] **Step 5: Commit** `docs: M8 decisions, project notes, changelog and roadmap`; open the PR to `dev` and merge it only once CI is green (squash allowed), per `CLAUDE.md`. The release v0.2.0 is a separate, explicitly instructed procedure.
