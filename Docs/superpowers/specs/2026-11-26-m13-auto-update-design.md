# M13: Automatic updates (design)

Status: draft for owner approval, 2026-11-26. Subproject 1 (fernjo app), milestone 13; M14 is the security gate for the release that ships it (v0.4.0).

## 1. Intent

fernjo is installed by hand today. The owner uses it daily on three machines and wants new versions to arrive without visiting GitHub: the app notices a new release, says so in the UI, and one click downloads, verifies and installs it and starts the new version. The first manual install of v0.4.0 is the last manual update.

Why it is safe to build now: the release pipeline already produces installers on GitHub-hosted runners, releases are immutable and attested, and the repository is public, so the release feed needs no server of our own.

## 2. Goals, non-goals, anti-goals

**Goals**
- A strictly newer stable version is found automatically (shortly after start, then every 24 h) and on demand.
- One click: download, signature check, install, restart, through an app-themed window that shows the stages.
- The update path adds as little trust as possible: one pinned HTTPS feed, one pinned public key, signature checked before anything is installed, the private key never on a developer machine's disk after set-up and never visible to build code.
- Where a self-update is impossible (`.deb`, a read-only install location) the window says so and offers the release page instead.
- The webview capability list shrinks from `core:default` to what the app needs (moved here from the backlog because the update commands touch the same file).

**Non-goals**
- OS code signing or notarisation (the installers stay unsigned; the README's first-start steps stay).
- Delta updates, update channels (beta/nightly), downgrade, rollback after a bad update.
- Downloading in the background before the user clicks.
- A second trusted key, key rotation inside the updater, or a runtime override of feed or key.
- Updating the hub (it has its own image path, M12).
- Telemetry of any kind; no "what's new" screen after the restart.

**Anti-goals (a failure even if it works)**
- An update that installs without a valid signature from the pinned key.
- The update check talking to anything but the pinned feed, or sending anything beyond what one HTTPS GET carries.
- The update key reachable by dependency build scripts, by a fork pull request or by a branch run.
- An update that ends connected sessions without the same "Install update?" confirmation the other restarts use, or that touches `connections.json`, the vault or `settings.json`.
- Release notes rendered as HTML.

## 3. Constraints that shape the design

- All logic in Rust (`src-tauri/src/updater.rs`); the UI is presentation (`DESIGN.md`, tokens only).
- Never call `AppHandle::restart` directly; every restart goes through the close confirmation (`ask_to_close`, `CloseThen`, `sync_runner::finish_after_push`).
- New commands count in the three-places rule and the no-secret guard; the update commands carry no secret, but the guard's table must still list them.
- `tauri_plugin_single_instance::init` stays first; the nav guard stays second; new plugin goes after them.
- Workflows: GitHub-hosted runners only, actions pinned to SHAs, `contents: read` by default, `tools/check-workflows.sh` stays green (it is line-based: write plain block YAML), secrets reach scripts only through `env:`.
- A published release takes no new files, so everything (installers, update files, `latest.json`) is attached to the draft first.
- `FERNJO_DATA_DIR` means "a scratch run": it must not start talking to the real feed on its own.
- No new crate: the plugin lives in `src-tauri`; the pure decisions (what to offer, what to show, how a platform installs) are plain functions so they run in tests on every OS.

## 4. Approaches considered

**Updater.**
- A. **`tauri-plugin-updater` (chosen).** Official, maintained with Tauri, minisign verification, per-platform handover (NSIS, `.app` replace, AppImage replace). Cost: a dependency whose behaviour (what it verifies where, when it exits the process) has to be proven by tests and a short spike, and the plugin must be driven from Rust only.
- B. Own updater (download, verify with `minisign-verify`, replace files). Rejected: the per-platform handover is exactly the part that is easy to get wrong, and it would be our own code on the most security-sensitive path.
- C. Notice only ("new version, here is the link"). Rejected: does not meet the goal; it stays as the fallback for installs that cannot self-update.

**Signing the update files** (chosen by the owner after the options were laid out).
- 1. **Separate `sign` job (chosen).** Build jobs run without any key. A job with `contents: read` downloads the build artifacts, runs `tools/verify-installers.sh` on them, signs exactly the expected update files with `tauri signer sign` and uploads the `.sig` files. The key exists only there. It is skipped on pull requests and forks.
- 2. A late step in the build job: key and build output on one runner. Rejected.
- 3. Key as environment variable on `tauri build`: every dependency build script and proc-macro could read it. Rejected.

What option 1 does and does not give: the key cannot be read by build code, and only an expected, verified file set is signed. It does not make a compromised build job harmless (it could hand over a malicious binary that then gets signed); the attestation and the independent review in M14 cover that side.

## 5. Design

### 5.1 Feed and request
- One endpoint, pinned in `tauri.conf.json` (`plugins.updater.endpoints`) with the public key (`pubkey`): `https://github.com/benjo-code/fernjo/releases/latest/download/latest.json`. GitHub's "latest" skips drafts and pre-releases, which fits "stable only".
- The URL has no template variables (no version, no OS, no id): every device requests the same URL. GitHub sees an IP address and a User-Agent, nothing else. The shipped config never sets `dangerousInsecureTransportProtocol`.
- The download links in `latest.json` point at the release's own assets. GitHub answers both URLs with a redirect to its asset CDN, so the redirect to another host is normal; the signature, not the host, is what protects the install. The "exactly one GET" check therefore counts one request to the pinned URL (plus its redirect hops) and nothing else.
- Timeouts as named constants: check 15 s, download stall 60 s without progress.

### 5.2 `latest.json` and update files
- Entries: `windows-x86_64` (the NSIS setup `.exe`), `darwin-aarch64` and `darwin-x86_64` (`.app.tar.gz`), `linux-x86_64` (the `.AppImage`). Each has `url` and `signature` (the `.sig` text). Top level: `version` (equals the tag), `notes` (the matching `CHANGELOG.md` section as plain text), `pub_date`.
- The Windows and Linux update files are the installers we already ship. macOS needs a tarball of the `.app`: the macOS jobs add the `app` bundle target and a step packs `fernjo.app` into `fernjo_<version>_<arch>.app.tar.gz` (named per architecture, so the two never collide).
- `bundle.createUpdaterArtifacts` stays **off**: with it on, `tauri build` demands the private key, which would put the key back into the build job. The artifacts are made from the normal bundles instead (see the spike in 6).
- `tools/write-latest-json.mjs` (Node, because it is JSON; tested in `tools/test-release-scripts.sh`) writes `latest.json` from the verified files, their `.sig` files and the changelog section, and its `--check` mode verifies that `latest.json` names exactly those files at this tag's release URLs with the signatures of the `.sig` files; `tools/verify-installers.sh` learns the new files and a `--release` form (it stays the one place that lists expected names).
- `latest.json` is written and attached by the `publish` job, which already holds `contents: write`; it is covered by the release's build provenance like the installers. The per-platform signatures inside it are what the app verifies.

### 5.3 Pipeline
- `build` jobs: unchanged permissions, no secret; on macOS also `app` bundle and the tarball; upload the update files with the installers.
- New `sign` job: `needs: build`, `contents: read`, only for a `v*` tag ref (not a pull request, not a branch dispatch), uses a GitHub Environment (`release-signing`, deployment rule: tags `v*`) that holds `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`; the secrets are `env:` of the one signing step only; the job uploads `.sig` files as an artifact.
- `sign` runs `tools/verify-installers.sh --artifacts` on the build artifacts first and signs only the four update files (the Windows setup, the two `.app.tar.gz`, the AppImage); the `.deb` and the `.dmg` are installers only and get no `.sig`. The `.sig` files are therefore expected for exactly those four, and the verifier rejects a missing, extra or misnamed one on the tag path (the pull request path has none). *(Builder-check answer.)*
- `publish`: needs `sign`; downloads the signatures, runs the verifier, **checks every `.sig` against the public key in `tauri.conf.json`** (a published release is immutable, so a signature made with the wrong key must fail here, not after publication; done in a separate `verify-signatures` job with no secret and no write token, so no dependency build script runs next to either: a small cargo example in `fernjo-core` over `minisign-verify`), writes `latest.json`, attaches all to the draft, then publishes as today.
- `environment:` on the `sign` job is a new key for `tools/check-workflows.sh`. If the line-based checker refuses it, it is extended narrowly (only `environment: release-signing` is accepted, with a test for that and for every other value being refused) and the reason goes into `Docs/DECISIONS.md`; the policy is not otherwise weakened.
- The pull request dry run builds as before, skips `sign`, and runs the script tests (so `latest.json` shape is checked without the key).
- The Environment, the two secrets and the key itself are owner actions in GitHub settings (section 8); nothing in the repo can create them.

### 5.4 Key lifecycle (recorded in `Docs/DECISIONS.md`)
- **Order**: the owner generates the pair first, as the opening step of the plan (a few minutes; the plan gives the exact command), so the real public key is what `tauri.conf.json` carries from the first commit that has an updater section. The pinning test asserts a well-formed minisign public key that is not the test key. If the owner cannot do it before the code is ready, the fallback is a clearly named placeholder plus a guard in `tools/check-release-version.sh` that fails a tag build while the placeholder is still there, so a release can never ship an app that can never update. *(Builder-check answer, my call; the owner can pick the fallback.)*
- The owner generates the pair once (`pnpm tauri signer generate`, with a password), commits only the public key, puts the private key and password into the Environment secrets, and keeps an offline backup of both (password manager). The key file is never written inside the repo.
- The app trusts exactly one public key. **Lost key** (no one can sign): installed apps can never update again; users reinstall by hand and the new build carries the new public key. **Leaked key**: delete the secrets at once; while the attacker also needs write access to our releases (the second factor), publish a normal update that carries a new public key, signed with the old key, as the last use of the old key; if write access leaked as well, assume a manual reinstall.
- A committed test key pair for the Rust and script tests is allowed only if it is plainly labelled test-only and cannot verify anything signed by the real key; the real key is never used in a test.

### 5.5 Rust side (`src-tauri/src/updater.rs`)
- State kept in `AppState` (one place, one lock, no network I/O under it): `Idle`, `Checking`, `Available { version, notes, install: SelfUpdate | ReleasePageOnly(reason) }`, `Downloading { received, total }`, `Ready` (downloaded and verified bytes held in memory; no temp file, so no path to get wrong), `Installing`, `Failed { code }`. Every change emits `update-status` (payload is the whole status, like `sync-status`).
- **What is offered** (`is_offered(current, candidate, skipped)`, a pure function): only when `candidate` is a strictly greater stable semver (no pre-release or build tag) and not equal to the skipped version. The plugin's own comparison is replaced by this one; both must agree before an update shows.
- **Timing**: first check 60 s after start, then every 24 h while running (a small scheduler over injected time, like `sync_runner`'s `Scheduler`); only when `checkForUpdates` is on and the run is not a scratch run. A failed automatic check changes nothing and shows nothing; the next one is in 24 h. A manual check is always allowed (one run slot, so checks never overlap) and reports "up to date", the new version, or the failure.
- **Download bound**: the update file is held in memory, so it has a cap like every other server-fed buffer in this repo: `MAX_UPDATE_BYTES` = 256 MiB (the shipped installers are far smaller). The download is refused when the declared length is above the cap and is cut off (the download future is dropped) when the received bytes pass it; an oversize case is in the fake-feed tests. *(Builder-check answer, my call.)*
- **Install flow** (confirm after download, so a slow download never ends anything): "Update and restart" (`update_install`) downloads while the app stays fully usable (progress events; the plugin verifies the signature before it returns the bytes) and the state becomes `Ready`. Then, at that moment, `connected_count` is read: with nothing connected the install continues at once (the click was the confirmation). With connected sessions the existing dialog asks, with its own wording ("Install update?", "Installing the update ends them.", buttons "Install" and "Keep open"; a new `CloseThen::Install`, texts from `sessions::close_prompt`). On "Keep open" the state stays `Ready` with the bytes held and the dialog offers "Restart to update"; `update_install` called again in `Ready` skips the download and asks again, re-reading `connected_count`. On "Install": `close_all`, the last sync push (`finish_after_push`), then the plugin's `install(bytes)`. A new check clears `Ready` only when it finds another version. On Windows the plugin hands over to the NSIS setup and exits the app; the installer starts the new version. On macOS and Linux the files are replaced and Rust requests a restart through `request_restart`. Download and install are the plugin's two separate calls, never `download_and_install` (that one would skip the confirmation).
- **Where self-update works** (`install_kind(platform, env, paths)`, a pure function with injected inputs): Windows: yes (per-user installer, no administrator rights). macOS: yes when the `.app` bundle's parent folder is writable. Linux: yes when `APPIMAGE` is set and the file's folder is writable; no `APPIMAGE` means `.deb` (or a build from source), so the release page. Otherwise `ReleasePageOnly` with a plain reason. The release page URL is built in Rust from the validated version (`vMAJOR.MINOR.PATCH`) and opened through the existing `open_external_url` path; the UI never supplies a URL.
- **Release notes**: cut to a fixed cap (16 KiB, on a character boundary), control characters other than tab and newline removed, in Rust; the UI shows them as text only (`white-space: pre-wrap`, Svelte text binding, never `{@html}`; a test greps the UI for `{@html`).
- **Settings** (`settings.json`, `crates/fernjo-core/src/settings.rs`): `checkForUpdates` (default on) and `skippedUpdateVersion` (string, set by "Skip this version", ignored by automatic checks, shown again by a manual check; a newer version than the skipped one is offered normally). With `FERNJO_DATA_DIR` set: no automatic checks, settings memory-only, "Check now" still works because it is an explicit click. With an unusable `settings.json`: automatic checks off, a specific note (like `unsavedNote()`), skip cannot be saved. The installed version comes from `CARGO_PKG_VERSION`.
- **Commands** (`update_status`, `update_check`, `update_install`, `update_skip`, `set_check_for_updates`, `update_open_release_page`; `update-status` event): the roadmap named three; `update_status` (the UI's initial read), `set_check_for_updates` (the settings checkbox) and `update_open_release_page` (no argument, so the UI never supplies a URL) are added by this design. All six count in the three-places rule and have `table()` entries; the file joins the count guard's `sources`. The webview never gets the plugin's own JS commands: the capability grants no `updater:*` permission.
- **Tests**: a local fake feed (a small HTTP server in the test, a test key pair, the plugin driven through its builder; the endpoint override and insecure transport sit behind a cargo feature `update-test-hooks` that is off by default and never enabled by the release workflow) shows: newer offered; same or older, and a pre-release, not offered; a missing or wrong signature refused; a tampered download refused; an unreachable feed changes nothing and shows no start-up error; the logging server sees one GET and nothing else. The pure functions get table tests on every OS. Platform handover cannot run in CI: those are owner checks.

### 5.6 UI
- Sidebar footer: an "Update available" button next to the sync line (shown only in `Available`/`Failed`), opening `UpdateDialog.svelte`.
- Dialog states: *available* (new and installed version, release notes, "Update and restart", "Later", "Skip this version"); *working* (progress bar and stage: downloading, verifying, installing, restarting; no close while installing; on Windows "installing" is the last stage the app can show, because the installer takes over); *failed* (the reason, "Retry", "Open release page"); *release-page only* (the reason and "Open release page", no install button).
- Settings gets an "Updates" section: installed version, the checkbox, "Check now", the last result line, the specific notes above. The dialog follows `DESIGN.md` and the existing dialog pattern (`ConfirmDialog` / `SettingsDialog`), no new colour token.
- `UpdateStore` (`ui/src/lib/update.svelte.ts`): view state only, listens to `update-status`, ignores a new action while busy.

### 5.7 Capability narrowing
- `src-tauri/capabilities/default.json` drops `core:default` for `core:event:allow-listen`, `core:event:allow-unlisten` and whatever else a grep of `@tauri-apps/api` use plus a real-build check shows is needed (checked with the DevTools-port recipe in a `pnpm tauri build --no-bundle` binary: tree loads, a session opens, events arrive, the update window opens). The list is pinned in `src-tauri/tests/tauri_conf.rs`, which also replaces `no_updater_and_no_signing_material` with a test that pins exactly the updater configuration (one endpoint, the public key, no insecure flag, no `createUpdaterArtifacts`) and no `updater:*` permission.

### 5.8 Docs
`README.md` privacy paragraph and `SECURITY.md` state the check (one HTTPS GET to the release feed, no account, no identifiers beyond the request, off with one setting, and that the download comes from the same release after a click); `Docs/release-checklist.md` drops "No update check"; `Docs/DECISIONS.md` records the choices above (feed, no template variables, signing job, key lifecycle, no `createUpdaterArtifacts`, no runtime override); `CLAUDE.md` gets the lasting facts; `CHANGELOG.md` `[Unreleased]` one short user-readable entry.

### 5.9 What M13 proves and what waits
The acceptance criteria are the ones in `Docs/ROADMAP.md` under Milestone 13 (the plan copies them as its checklist). Inside M13 close: build/test/lint, the fake-feed tests, the `latest.json` shape tests and the pull request dry run, the guards and the one-GET check, the capability check in a real build, and the owner checks against a **test feed** (an older locally built version updating to a newer one). Not provable in M13: an update from a real released version, because v0.4.0 is the first build that carries the updater; the first real end-to-end update (v0.4.0 to a v0.5.0 candidate) is already a task of the M17 gate in the roadmap, and the v0.4.0 release criteria only require that the feed and its signatures are valid. Owner-check criteria stay unticked until run, with the command or steps named.

## 6. Spike before the plan commits to it (no product code)

Four plugin and tooling facts the design relies on; each is checked first and recorded, and a wrong one comes back to the owner before work continues:
1. `pnpm tauri build` works with the public key configured and `createUpdaterArtifacts` off, and a tarball of `fernjo.app` made by our own `tar` step has the layout the plugin's macOS install accepts (Windows and Linux use the installers unchanged). The macOS part can only be fully proven on a Mac (owner check); the layout is compared with what Tauri's own tarball contains.
2. `Update::download` verifies the signature before it returns bytes (also proven by the tampered-download test).
3. On Windows `install` hands over to the installer and exits the process; on Linux and macOS it returns, so Rust restarts.
4. The current stable `tauri-plugin-updater` 2.x line to pin exactly (the registry's newest entry is a 3.0 alpha, which is not used), its licences and new transitive crates against `cargo deny`.

## 7. Parts left to the builder
Exact module split inside `updater.rs`, constants (delays, caps, timeouts as named), the UI layout details within `DESIGN.md`, the precise capability list, the fake feed's shape, how `tools/write-latest-json.sh` gets the changelog section, wording of the notes and reasons.

## 8. Owner actions and open checks
Actions only the owner can do (the plan lists them as blocked steps, never done silently): generate the key pair; create the GitHub Environment `release-signing` with the tag rule and the two secrets; back up the key.
Owner checks (cannot run in CI): update through the window on Windows, macOS and Linux AppImage against a test feed (a test build made with a merged config file that sets the test endpoint, the test public key and the insecure flag; the file lives under `tools/` and is never used by the release workflow); data files untouched; no administrator prompt on Windows; no Gatekeeper or SmartScreen warning for the updated app (expected, not yet seen); `.deb` and a read-only AppImage location show the release page offer.

## 9. Changes against the roadmap text (to go into "Scope changes")
- Three extra commands (`update_status`, `set_check_for_updates`, `update_open_release_page`).
- The "exactly one GET" criterion counts the redirect hops of that one request.
- The signing step is a separate `sign` job, not a step of the build job.
- `createUpdaterArtifacts` is deliberately not used.

## 10. Notes from implementation
- `update_open_release_page` takes no argument and, when no version is on offer, opens the generic releases page (`https://github.com/benjo-code/fernjo/releases/latest`, built in Rust) instead of failing, so every failed state can offer "Open release page".
- The signing flow is split further than section 9 says: `sign` (the only job with the secrets), `verify-signatures` (no secret) and `publish`; `latest.json` is written by `tools/write-latest-json.mjs`. The Environment `release-signing` needs its required reviewer and tag policy before the first tag, because GitHub creates a missing environment without protection. Details: `Docs/DECISIONS.md` "M13: update feed, signing job, key lifecycle and scope changes".
