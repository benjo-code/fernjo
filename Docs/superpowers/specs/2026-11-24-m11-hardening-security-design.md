# Milestone 11: full hardening and security check — design

Date: 2026-11-24 · Status: awaiting the owner's approval · Roadmap: milestone 11 in `Docs/ROADMAP.md` · Branch: `feature/m11-hardening-security`

## Intent

v0.3.0 will ship device sync (M9) and the daily-use features (M10): since v0.2.0 that is 138 changed files and about 21,700 added lines, none of which has had an independent security review. The owner asked for a full review before the release, not only the parked items. M11 brings every area of the app through an independent review and a hardening pass, closes the hardening items parked in the backlog, and ends with a go or no-go verdict for v0.3.0. It prepares the release and does not perform it.

## Goals

- Every area in the roadmap task list has a written, independent review, and every finding sits in `Docs/release-checklist.md` with severity, disposition and evidence.
- No High finding is open. Every Medium is fixed, or its remaining risk is recorded in `Docs/DECISIONS.md` with the owner's acceptance. Every Low is fixed or accepted by the owner with the reason recorded.
- The parked hardening items (roadmap milestone 11, "Source:" blocks) are fixed or consciously accepted.
- The hardened file writer and the capped reader exist once, in a new crate `fernjo-fs`, and every crate uses it.
- A go or no-go verdict for v0.3.0 is written.

## Non-goals

- No release, no version bump, no PR to `main` (that is the release procedure, on the owner's instruction).
- No new features. The clipboard marker (below) is the only new UI and exists to make an accepted risk visible.
- UI polish leftovers of M4 and M10 (tab ARIA, focus targets, font weights, search bar size, the quick connect overlay and similar) stay in the backlog.
- No code signing, auto-update, or other backlog features.
- No upstream patch or fork of IronRDP.

## Anti-goals

- A finding "fixed" without a test or recorded check that fails without the fix.
- A High or Medium fix that nobody but its author has read (v0.2.0's TLS fix went out without a second review; not again).
- Reviewers that saw each other's findings or were handed the expected conclusions.
- Weakening a check, skipping a test or adding a `deny.toml` ignore just to get green.
- Changing behaviour of the file writers beyond the security fix while consolidating them (callers must not notice except where stated).

## Constraints

- Everything in `CLAUDE.md`, in particular: secrets stay in Rust; the three-places rule for commands; protocol, vault, import, sync and hub crates never depend on Tauri; `fernjo-hub` never depends on `fernjo-sync`; files in the data folder are written atomically and keep a `.bak`; UI follows `DESIGN.md` and the tokens; work on the branch, PR to `dev`.
- The development machine may lack a usable WebView2 runtime, so real-app checks may be impossible here; the owner's machines are the fallback, and an unrun check stays marked as open.
- The live RDP tests need the owner's Windows 11 host (`FERNJO_TEST_RDP_ADDR`); the live SSH tests need the throwaway container (`tools/ssh-test-server.sh`).
- CodeQL alerts are dismissed only on the owner's word.
- Review reports are working files under `.superpowers/` (gitignored); the findings themselves go into `Docs/release-checklist.md`, which is committed.

## Decisions made with the owner (2026-11-24)

1. **RDP clipboard default: stays on for saved connections.** Quick connect sessions keep it off, as built in M10. The v0.2.0 Medium finding "RDP clipboard on by default" is accepted by the owner; the reason and remaining risk go into `Docs/DECISIONS.md`. *Builder's addition, proposed to the owner without objection:* a small clipboard marker on an RDP tab while clipboard sharing is on (tooltip naming the host), so the accepted risk is visible.
2. **Scope of the leftovers.** The hardening-relevant leftovers of the M4 and M10 reviews go into the triage like any finding (list under "Design, step 3"); UI polish stays in the backlog.
3. **File helpers: consolidate first.** A new crate `fernjo-fs` holds the hardened atomic writer and the capped reader; the `.bak` symlink fix is made once there. The backlog item "Consolidate the hardened file helpers" moves into M11.
4. **Order: fix the known items first, then review everything** (approach 1 below).
5. **RDP TLS: the handshake signature check stays strict** (rustls's own functions only); old hosts webpki cannot handle fail with a distinct error instead of being accepted through a second crypto library (step 3).
6. **M11 stays in progress until the owner checks ran or were waived** (step 7).

## Approaches

**Order of the work.**

1. *Fix known items first, then independent reviews of the fixed code, then triage and fixes, then the gate (chosen).* Reviewers spend no time re-finding known items, and every line M11 changes (the consolidation above all) is itself reviewed.
2. *Review first, then fix everything in one batch:* one triage list, but known items are re-reported and the M11 changes go unreviewed unless a second round runs.
3. *Area by area (fix and review RDP, then sync, …):* tidy per area, but the consolidation spans six crates and fits no area, and reviewer independence needs extra coordination.

**The `.bak` symlink fix.**

- A. *Patch each of the seven writer copies:* small and low-risk, but the copies stay and drift (the reason the gap survived M7).
- B. *Consolidate into `fernjo-fs` first, fix once (chosen).* Touches the file I/O of six crates plus `src-tauri`, so it is done as a separate, behaviour-preserving change before the reviews, which then read it.

**RDP static channel cap** (builder's call, stated to the owner): pre-check in fernjo's own read loop rather than patching `ironrdp-svc`. **RDP TLS** (builder's call, stated to the owner): own certificate verifier rather than the strict webpki one. Within it, A. strict signature check through rustls only, unusual old certificates fail with a distinct error (chosen by the owner), or B. accept those too by verifying through a second crypto library (rejected: more crypto surface in the most sensitive spot for rare hosts).

## Design

### Step 1: threat model update

A new `Docs/DECISIONS.md` entry "M11 threat model" that updates the existing model for sync and the M10 features. Per asset (vault secrets, sync key and hub secret, `connections.json`, `known_hosts`, RDP pins, device folder, the webview), it states what touches it and what each adversary can do: a hostile SSH or RDP server, a hostile hub or network path to it, a hostile peer device in the sync group, a writer of the synced data folder (sync tool), a hostile import or backup file, a script in the webview, another local user. It names the accepted trust assumptions (for example: a peer device in the sync group holds the sync key and is trusted with the data; the hub is not). The reviewers get this entry as their brief.

### Step 2: `fernjo-fs`

- New workspace crate `crates/fernjo-fs`, no Tauri, depends only on `std` and `getrandom` (already a workspace dependency). Allowed as a dependency of every crate including `fernjo-hub`, so the "protocol crates never depend on each other" and "hub never depends on sync" rules hold. `CLAUDE.md` (architecture list and the "Hardened atomic writes" paragraph) and `Docs/DECISIONS.md` record it.
- API (exact names left to the builder):
  - `read_capped(path, max) -> io::Result<Vec<u8>>`: length checked before reading and the read itself bounded; one error kind the callers can map to their existing "too large/unreadable" rules.
  - `write_atomic(path, bytes, options)`: parent folder handling exactly as each caller does today (*builder's call after the check*: `persist` and `fernjo-sync` create missing parents with 0700 through `DirBuilder`, which leaves existing folders' modes alone; `app_menu.rs` creates `applications/` and `icons/` with default modes and must keep doing so; the hub store and the vault create no parent in the writer), so the option is "create no parent / create private / create default", never a chmod of an existing folder; random `<file>.<16 hex>.tmp` opened with `create_new` (0600 on Unix), write, flush, fsync, optional `.bak`, rename, directory fsync on Unix (best effort). Options cover what differs today: whether a `.bak` is kept (yes for `connections.json`, `settings.json`, `vault.fernjo`, `known_hosts`, the RDP pin file; no for the sync state files and the hub store) and nothing else unless a caller truly needs it.
  - **The `.bak` is never written through a path that may be a symlink:** the current file is copied into a second random `create_new` temp file and renamed onto `<file>.bak` (a rename replaces a symlink at the target instead of following it).
- Callers moved onto it: `fernjo-core::persist` (`connections.json`, `settings.json`; this also replaces the fixed `<file>.tmp` name with a random one), `fernjo-vault` (`vault.rs` writer, `format.rs` reader), `fernjo-ssh::known_hosts` (keeps its in-process `WRITE_LOCK`), `fernjo-rdp::cert_store`, `fernjo-sync::files`, `fernjo-hub::store`, `src-tauri/src/app_menu.rs` and the private bounded reader in `src-tauri/src/open_tabs.rs`. The hub's HTTP body reader (`server.rs`) is a different thing and stays. Each caller keeps its own error type and cap constant (`MAX_DATA_FILE_LEN`, `fernjo_vault::MAX_FILE_LEN`, …).
- Callers must not notice any other change: the trailing newline of `persist`, `.bak` presence, file modes, the vault's stale temp sweep. Existing writer tests in the crates stay and must pass unchanged; the generic ones (temp name, symlink at the temp name, directory fsync, cap boundaries) move to `fernjo-fs` with new ones for the `.bak` symlink. Unix-only tests run in CI on Linux.
- Old fixed-name leftovers (`connections.json.tmp`, `settings.json.tmp`) from earlier versions: left to the builder whether to sweep them like the vault's stale temp files.
- Hub image and store (*builder's call after the check*): the `Containerfile` copies the whole filtered workspace (`COPY . .`), so a new crate needs no change there; the hub store already uses random temp names and no `.bak`, so its on-disk behaviour does not change and a running hub can be upgraded in place. A `podman build` of the image is part of the step's evidence (or, if Podman or Docker is not available, the CI compile of `fernjo-hub` and an open owner check).

### Step 3: parked items, leftovers and the clipboard marker

Each item gets a test or recorded check that fails without the fix, and a `Docs/DECISIONS.md` note where a decision is involved.

- **RDP static channel cap.** First confirm in the `ironrdp-svc` (and `ironrdp-dvc`) source of the pinned versions how reassembly sizes its buffer. Then, in the read loop of `crates/fernjo-rdp/src/session.rs`, before a frame reaches `stage.process`, parse slow-path MCS send-data indications for static channels: when a channel PDU with the "first" flag announces a total length above a cap (builder picks; well above the 1 MiB clipboard cap, around 4 MiB), end the session with a distinct error code. The same check covers dynamic channel "data first" PDUs if `ironrdp-dvc` reassembles the same way. Test: the fake server announces an oversized channel message; the session ends with the new code and memory stays bounded.
- **RDP write timeout.** Every `writer.write_all` in the session loop is bounded by a timeout, and the `ops` channel in `src-tauri/src/sessions/rdp.rs` is bounded, so a server that stops reading cannot keep a closing session alive. Test with a fake server that stops reading.
- **RDP TLS: own verifier.** Replace the strict webpki verifier with a `ServerCertVerifier` that accepts the presented chain without building a path to a root (the pin decides trust, as today) but verifies the TLS 1.2 and 1.3 handshake signatures through the crypto provider's verification functions. First confirm in the rustls and `ironrdp-tls` sources that this keeps the handshake signature check; if `ironrdp-tls` does not accept a custom verifier, build the rustls client config in `fernjo-rdp` directly. **Owner decision (2026-11-24): the signature check stays strict and uses only rustls's own verification functions; no second crypto library.** Those functions parse the leaf with webpki, so the expected effect is: an empty platform root store no longer matters, and SHA-1 signatures *on* the certificate, name and expiry mismatches stop mattering (the pin decides); hosts whose certificate webpki cannot parse or whose key it cannot verify with (RSA under 2048 or over 8192 bits, non-v3 certificates, unknown critical extensions) still fail, now with a distinct error code that says the certificate is not supported (name left to the builder), not with the generic `tls_failed`. The builder confirms the exact list against the pinned rustls and webpki sources and records it in `Docs/DECISIONS.md` and the README's known limits. The cause of a TLS failure is logged at debug level without secrets. Tests: the existing `a_server_that_cannot_sign_for_its_certificate_fails_the_handshake_before_any_prompt` stays green; a fake-server case with a SHA-1-signed certificate now reaches the certificate decision; a fake-server case with a certificate webpki cannot use (for example a 1024-bit RSA key) fails with the new code before any prompt.
- **RDP Low items.** Desktop size clamped by area too, scaling down instead of failing with `desktop_too_large`; the no-secret guard gets sentinels for an RDP user name and domain and its `rdp_input` and `rdp_clipboard` entries reach past `session_not_connected` (a connected fake session, if the test harness allows; otherwise recorded why not); `app_menu.rs` requires `APPIMAGE` to be absolute (its temp name and the `cert_store` `.bak` are covered by step 2). The password in IronRDP's plain `String` config is an API limit: proposed for owner acceptance, not fixed.
- **Import username cleaning.** `fernjo_import::limits::sanitize_nodes` applies the same cap and control-character cleaning to `settings.username` as to the other fields, for JSON and backup imports.
- **`clear_pending` residue.** An `AtomicBool` set by the page-load hook; a failing `import_apply` that sees it clears its pending value.
- **`check-no-build-paths.sh`.** Capture grep's exit status and fail on anything above 1; test in `tools/test-release-scripts.sh` (unreadable binary).
- **Hardening leftovers of M4 and M10, entered as known findings** (severity set at triage, fixed or accepted like any finding): `Resize` messages unbounded and the session input cap weighing payload bytes only (minimum weight per message); the `arboard` read has no timeout of its own; the idle-lock thread has no `catch_unwind`; a russh session task and its file descriptor leak on a pre-key-exchange timeout; `tauri_plugin_opener::init()` injects an init script (use `Builder::new().open_js_links_on_click(false).build()`); a sync-enabled restore from a backup is pushed to the other devices without the confirmation saying so (*builder's call*: fixed by wording only, the confirmation says the restore reaches every device in the sync group; pushing it is how sync is meant to work, and a behaviour change would be a feature); the Cmd+Q path with no main window prevents the exit and shows nothing.
- **Clipboard marker.** A small clipboard icon on an RDP tab whose session shares the clipboard, with a tooltip naming the host; tokens and icon style from `DESIGN.md`, no new colour unless the design calls for one (then in all three token blocks). Presentation only: whether sharing is on comes from Rust (the resolved setting the session already carries).

### Step 4: independent reviews and checks

- **Eight area reviewers**, each a fresh read-only subagent that has not seen the others' findings, given: the threat model entry, its area's file list, the severity scale (High, Medium, Low as in "v0.1.0 security review"), and the list of known items already fixed or accepted (so they are not re-counted) — never the expected conclusions. Probes only in a scratch folder. Areas and what each covers:
  1. Vault: `fernjo-vault`, `vault_commands.rs`, keychain, rollback marks, auto-lock (`idle_lock.rs`).
  2. SSH: `fernjo-ssh` (host keys, `known_hosts` and confirmed hosts, agent incl. Pageant, algorithms, input cap).
  3. RDP: `fernjo-rdp`, `sessions/rdp.rs`, `rdp_commands.rs`, including the step 3 changes.
  4. Import and backup: `fernjo-import`, `import_commands.rs`.
  5. Sync: `fernjo-sync`, `fernjo-hub`, `sync_commands.rs`, `sync_port.rs`, `sync_runner.rs`, the hub container files.
  6. Session manager and IPC: `sessions.rs`, `session_flow.rs`, `session_commands.rs`, `connect_plan.rs`, quick connect, open tabs, `clipboard.rs`.
  7. Webview hardening: CSP, `nav_guard.rs`, capabilities, the full command list and the no-secret guard, webview storage, `tauri.conf.json`.
  8. Release workflow and scripts: `.github/workflows/`, `tools/*.sh`, the notices generator.
  *Builder's call on the mechanics:* the session that runs the plan spawns the reviewers itself as fresh subagents on the strongest available model (as for v0.1.0 and v0.2.0), in parallel, each with only its own brief; the controller writes the known-items list from the roadmap blocks of M11 and the findings tables of earlier reviews in `Docs/release-checklist.md` (v0.2.0 included), stating for each item only that it is known and its disposition, not the reasoning.
  `fernjo-fs` and `fernjo-core::persist` belong to reviewer 1's brief (data at rest) and are named in every reviewer's file list where their crate uses them.
- **`/security-review`** of `v0.2.0..HEAD`.
- **Checks:** `cargo deny check`, `pnpm audit --prod`, open Dependabot and CodeQL alerts (`gh api`), `bash tools/check-workflows.sh`, gitleaks over the history.
- **Secrets check:** the no-secret guard covers every command added since v0.2.0 (M9 and M10 commands in `table()`), plus a check that nothing secret reaches `connections.json`, `settings.json`, logs, sync data (the envelope carries only the 34-byte header in the clear) or webview storage. A gap found here is a finding.

### Step 5: triage and fixes

- A new section "v0.3.0 security review" in `Docs/release-checklist.md`: findings table (#, area, finding, severity, decision, fixed in) in the style of the v0.1.0 review, reviewer severity and the controller's ruling both kept where they differ.
- High and Medium: fixed. A Medium that cannot be fixed in M11 goes to the owner for acceptance with the remaining risk written in `Docs/DECISIONS.md`. Lows: fixed, or brought to the owner in one batch for acceptance with reasons.
- Every High and Medium fix gets a second read by a fresh reviewer who sees the finding and the fix diff.
- *Builder's call on owner decisions:* the build runs through steps 1 to 4 and the fixes it can make without the owner, then stops once with a single batch for the owner (Medium and Low acceptances, any stop-rule item, CodeQL dismissals), then finishes the fixes and the gate. A finding that blocks other work (a High that cannot be fixed) is raised at once instead.
- Fixes follow test-first where the code allows; the evidence names the test or check and that it failed before the fix.

### Step 6: release-readiness gate

A "v0.3.0 readiness" section in `Docs/release-checklist.md` following the reusable checklist where it applies before a release branch: gate commands with results, known gaps table (open owner checks, accepted findings, unverified claims), and a go or no-go line per area plus an overall verdict. No release is performed.

### Step 7: owner checks

- RDP on the owner's Windows 11 host after the TLS and channel changes: `cargo test -p fernjo-rdp --test connect` with `FERNJO_TEST_RDP_ADDR`, plus one connection from the app with the clipboard and the new marker.
- The M9 sync run once more on two real devices after the fixes (at least: pair, an edit both ways, a vault entry used on the other device; Linux if the CachyOS laptop is at hand).
- Results go into the roadmap criterion as evidence; unrun checks stay open and appear in the known gaps.
- **Owner decision (2026-11-24): M11 stays `in-progress` until these owner checks have been run or the owner has explicitly waived them** (unlike M10, which closed with its manual criterion open). The rest of the milestone (reviews, fixes, gate) can be finished and merged to `dev` before that; the roadmap status and the verdict say what is still waiting.

### Roadmap and docs

- Scope-change entry dated 2026-11-24: clipboard default stays on (owner), clipboard marker added, `fernjo-fs` consolidation pulled in from the backlog, hardening leftovers of M4 and M10 pulled into the triage. The backlog item "Consolidate the hardened file helpers" is removed; the two leftover items keep only their polish parts once M11 closes.
- `CHANGELOG.md` `[Unreleased]`: the user-visible changes (RDP hosts that failed with `tls_failed` connect again, the clipboard marker, the oversized channel error).

## Stop rules

Stop and ask the owner when: a High finding cannot be fixed within the milestone; a fix needs a change to a file format or a wire contract that other devices or versions read; a reviewer's finding contradicts an owner decision recorded above; or a dependency change is needed that `cargo deny check` would only pass with a new ignore.

## Left to the builder

The exact caps and timeouts (channel cap, write timeout, `Resize` and input weights), error code names, `fernjo-fs` function names and option shape, whether to sweep old fixed-name temp files, the task breakdown and order within each step, wording of the marker tooltip and the CHANGELOG lines, and the severity ruling on each finding (with the reviewer's own rating kept alongside).
