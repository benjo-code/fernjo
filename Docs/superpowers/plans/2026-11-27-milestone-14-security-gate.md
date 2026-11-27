# Milestone 14: Security check and release readiness for v0.4.0 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Check everything since v0.3.1 independently, fix what is found, and write a go or no-go verdict for v0.4.0 without performing the release.

**Architecture:** A Windows test job, the threat model and the attack tests come first, so the reviewers read their result (Tasks 1 to 3). Then four independent reviews, the scans and the pipeline rehearsal (Tasks 4 to 6), triage, fixes and one owner batch (Tasks 7 to 9), and the gate (Task 10).

**Tech Stack:** Rust 1.100 workspace, Tauri 2, `tauri-plugin-updater` =2.14.1, Svelte 5 + TypeScript, vitest, bash and Node for `tools/`, GitHub Actions.

**Spec:** [Docs/superpowers/specs/2026-11-27-m14-security-gate-v0.4.0-design.md](../specs/2026-11-27-m14-security-gate-v0.4.0-design.md). Executors read both. Branch: `feature/m14-security-gate-v0.4.0` (already created from `dev`).

## Global Constraints

- No release: no version bump, no release branch, no tag, no PR to `main`.
- The real signing key and its password never reach this machine, a file in the repository, a log, a reply or a shell history.
- Secrets stay in Rust. This milestone plans no new `#[tauri::command]`; a fix that adds one follows the three-places rule.
- Never weaken a check, the workflow policy or the environment protection, skip a test, add a `deny.toml` ignore to get green, or unset `CI` to make tests skip.
- Workflows: GitHub-hosted runners only, actions pinned to commit SHAs, `contents: read`, plain block YAML (`tools/check-workflows.sh`).
- Native cargo builds are blocked on this machine (do not change that Windows setting, do not run `cargo clean`). Below, **`CARGO <args>`** means:
  `MSYS_NO_PATHCONV=1 docker run --rm -v "d:/VS-Code/fernjo:/src" -v fernjo-target:/target -v fernjo-cargo:/opt/cargo/registry -v fernjo-rustup:/opt/rustup fernjo-dev bash -c "cd /src && export CARGO_TARGET_DIR=/target CARGO_HOME=/opt/cargo && cargo <args>"`
- Evidence says where each command ran (container, native, CI) and never claims more than was run.
- Working files (reports, signatures, rehearsal) live in `.superpowers/sdd/2026-11-27-milestone-14-security-gate/` (gitignored), called **`WORK/`** below.
- A finding that also affects an already released version is raised to the owner before its description is pushed (the repository is public).
- Only the owner merges Dependabot pull requests; CodeQL alerts are dismissed only on the owner's word.
- Conventional Commits without attribution lines; stage files by name; check `git show --stat HEAD` after each commit.
- Stop rules (spec): a High that cannot be fixed; a failed key check; a fix that would change the pinned key or endpoint, a file format or a wire contract, or need the workflow policy or environment protection loosened; a finding that contradicts an owner decision in the spec; a dependency change that needs a new ignore.

## Review Focus

1. The new skip flag set on Linux or macOS: a missing SSH server must still fail the run (Task 1, `a_missing_ssh_server_is_skipped_in_ci_only_on_windows_with_the_flag`).
2. A download refused while sessions are connected: no "Install update?" dialog, nothing installed, sessions untouched (Task 3, `a_refused_download_leaves_nothing_to_install`).
3. A redirect to another host that serves the genuine file (GitHub's asset CDN): still accepted, so no fix pins the host and breaks every update (Task 3, `redirect_to_another_host_accepts_only_signed_bytes`).
4. The owner's key check goes wrong (wrong password, a bad paste): signing fails with a message, nothing secret is echoed or sent, and a wrong signature is rejected by the verification (Task 2 steps and controls).
5. The real changelog section as v0.4.0 release notes: present in `latest.json` and not cut at 16 KiB (Task 6).

---

### Task 1: Windows test job

**Files:**
- Create: `crates/fernjo-ssh/tests/support/gate.rs`
- Modify: `crates/fernjo-ssh/tests/support/mod.rs` (`server_from_env`), `src-tauri/src/sessions/tests.rs` (`server`), `.github/workflows/ci.yml`

**Interfaces:**
- Produces: `pub fn skip_allowed(windows: bool, ci: bool, no_server: bool) -> bool` in `gate.rs`; environment variable `FERNJO_TEST_NO_SSH_SERVER`; CI check named `rust (windows)`.

- [ ] **Step 1: Write the failing test** in `src-tauri/src/sessions/tests.rs`, next to `server()`, with the module line `#[path = "../../../crates/fernjo-ssh/tests/support/gate.rs"] mod ssh_gate;`:

```rust
#[test]
fn a_missing_ssh_server_is_skipped_in_ci_only_on_windows_with_the_flag() {
    for windows in [false, true] {
        for flag in [false, true] {
            assert!(ssh_gate::skip_allowed(windows, false, flag));
            assert_eq!(ssh_gate::skip_allowed(windows, true, flag), windows && flag);
        }
    }
}
```

- [ ] **Step 2: Run** `CARGO test -p fernjo-app --lib a_missing_ssh_server` → FAIL (`gate.rs` does not exist).
- [ ] **Step 3: Implement** `skip_allowed` (`!ci || (windows && no_server)`). Both server helpers replace their `CI` assertion with `assert!(skip_allowed(cfg!(windows), <CI set>, <FERNJO_TEST_NO_SSH_SERVER set>), <the existing message>)`; `support/mod.rs` declares `mod gate;`. `agent_server` needs no change (it returns early without a server).
- [ ] **Step 4: Run** `CARGO test -p fernjo-app --lib a_missing_ssh_server` → PASS; `CARGO clippy --workspace --all-targets -- -D warnings` → exit 0; `CARGO test -p fernjo-ssh` → exit 0 (server tests print "skipped").
- [ ] **Step 5: Add the job** to `ci.yml`: id `rust-windows`, `name: rust (windows)`, `runs-on: windows-latest`, `timeout-minutes: 45`, job `env: FERNJO_TEST_NO_SSH_SERVER: "1"`; steps: checkout (same SHA and `persist-credentials: false` as job `rust`), the "Install the pinned Rust toolchain" step, `Swatinem/rust-cache` (same SHA), `cargo test --workspace`. A comment says that the SSH container tests stay in job `rust` and that the flag is honoured on Windows only.
- [ ] **Step 6: Run** `bash tools/check-workflows.sh` → exit 0; `bash tools/test-release-scripts.sh` → all passed.
- [ ] **Step 7: Commit** `ci: run the workspace tests on Windows`.
- [ ] **Step 8: Push and open a draft PR** to `dev`: `git push -u origin feature/m14-security-gate-v0.4.0`, `gh pr create --draft --base dev --title "chore: security check and readiness for v0.4.0 (M14)"`. Watch with `gh pr checks <pr> --watch`. Expected: all jobs pass, `rust (windows)` included.
- [ ] **Step 9: Triage the first Windows run.** Check the log shows the updater tests ran (`updater_feed`, `updater::tests`). A failure that shows a defect: note it in `WORK/findings.md` for Task 7. A failure of the test setup: fix it without weakening the test, commit `test: ...`, push, watch again. Record the run URL and test counts in `WORK/evidence.md`.

### Task 2: Threat model and the owner's early checks

**Files:**
- Modify: `Docs/DECISIONS.md` (new entry on top), `Docs/release-checklist.md` (new section "v0.4.0 owner checks"), `Docs/roadmap.json` + `Docs/ROADMAP.md` (status `in-progress`)
- Create: `WORK/known-items.md`

- [ ] **Step 1: Read** "M11 threat model" and the four M13 entries in `Docs/DECISIONS.md`, the M13 section of `Docs/release-checklist.md`, and the M13 design.
- [ ] **Step 2: Write** "2026-11-27 – M14 threat model" (spec section 1): assets (update feed, signing key and pinned public key, downloaded bytes, install handover, `checkForUpdates` and `skippedUpdateVersion`, webview capability) against adversaries (hostile or compromised feed, network path, redirect target, pull request or fork, build dependency, tag pusher, repository administrator, local process of the same user, webview script). Each cell: what it touches or can do, what stops it, accepted or open. Name the trust assumptions. State facts only, no review conclusions.
- [ ] **Step 3: Write `WORK/known-items.md`:** each item with its disposition and no reasoning: the "Known residual limits" of the M13 section (uncapped feed body; download cap overshoot; `sign` trusts the build output and caches; install-returns and late-confirm races; macOS menu Quit), "administrators can bypass the `release-signing` rules", M11 row 73, unsigned installers, the `glib` alert. The M13 limits and Cmd+Q are marked "goes to M14 triage".
- [ ] **Step 4: Write the owner section** in `Docs/release-checklist.md` with two checks to run now and one optional step:
  - **Cmd+Q (Mac, installed v0.3.x):** connect an SSH session, press Cmd+Q; reopen, connect, use the menu's Quit. Report for each whether "Close fernjo?" appeared.
  - **Key copy check** (any machine with the repository, PowerShell 7; no key file is written):

```powershell
git switch dev; git pull; pnpm install --frozen-lockfile
Get-FileHash crates/fernjo-core/tests/fixtures/update-sig/payload.bin
# expected: 1953C943FBA181D485BBCCC8F3F41D0E660127B1F79548DE46530DB826E99A63
$dir = Join-Path $env:TEMP fernjo-key-check; New-Item -ItemType Directory -Force $dir
Copy-Item crates/fernjo-core/tests/fixtures/update-sig/payload.bin $dir
$env:TAURI_SIGNING_PRIVATE_KEY = Read-Host -MaskInput 'private key'
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = Read-Host -MaskInput 'password'
pnpm tauri signer sign --app-version 0.0.0 "$dir\payload.bin"
Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY, Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD
Get-Content "$dir\payload.bin.sig"
```

  Written with it: a bash variant using `read -rsp` for the two values; afterwards clear the clipboard and the clipboard history and close the terminal; send only the content of `payload.bin.sig`; if signing fails, send only the error line, never a screenshot of the session.
  - **Optional, after the check passed:** `gh secret set TAURI_SIGNING_PRIVATE_KEY --env release-signing` and `gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --env release-signing`, pasting at the prompt (never as an argument).
- [ ] **Step 5: Verify the steps with the test key** in a scratch folder (same commands with `src-tauri/tests/fixtures/update-test-key/test-only.privkey` as content and an empty password): a `.sig` appears.
- [ ] **Step 6: Roadmap** status `in-progress`; `dotnet run tools/roadmap.cs -- generate`, then `-- check` → valid.
- [ ] **Step 7: Commit** `docs(m14): threat model and owner checks for v0.4.0`, push. Ask the owner to run both checks now; do not wait for them.
- [ ] **Step 8 (whenever the `.sig` arrives):** save it as `WORK/key-check.sig` and run `CARGO run -q -p fernjo-core --example verify_update_signature -- src-tauri/tauri.conf.json crates/fernjo-core/tests/fixtures/update-sig/payload.bin WORK/key-check.sig --app-version 0.0.0` → `signature ok`. Controls, both must exit 1: `--app-version 0.0.1`; the configuration `crates/fernjo-core/tests/fixtures/update-sig/tauri.conf.json`. Record the result, not the signature. A failure is a stop rule: tell the owner at once.

### Task 3: The recorded attack on the update path

**Files:**
- Modify: `src-tauri/tests/updater_feed.rs`, `src-tauri/tests/support/fake_feed.rs`, `src-tauri/src/updater/tests.rs`, `Docs/release-checklist.md` (new section "v0.4.0 attack on the update path")

**Interfaces:**
- Consumes: the test helpers `serve`, `download_of`, `download_from_route`, `manifest`, `payload`, `payload_signature`, `setup`, `logged`.
- Produces: `Route::Truncated { declared: u64, body: Vec<u8> }` (200, `Content-Length: declared`, sends `body`, closes).

- [ ] **Step 1: Write the tests.**
  - `redirect_to_another_host_accepts_only_signed_bytes` (three `FakeFeed` servers; the other host is `http://localhost:<port>/...`): `front` `/latest.json` redirects to `mid` `/cdn/feed.json`; the manifest (version `9.9.9`) names `mid` `/dl/payload.bin`, which redirects to `files` `/cdn/payload.bin`. Run twice, through `download_of(&front, …)`: genuine payload → `Ok(payload())`; payload with byte 100 flipped → `Err(FeedError::BadSignature)`. Both times assert `files.log()` paths are `["/cdn/payload.bin"]`.
  - `truncated_download_is_refused`: with `short` = the first 4096 payload bytes, `Route::Body(short)` → `FeedError::BadSignature`; `Route::Truncated { declared: 8192, body: short }` → `FeedError::DownloadRefused("failed")`.
  - `signature_without_a_version_is_refused`: signature text from `../crates/fernjo-core/tests/fixtures/update-sig/no-app-version.sig`, `serve("9.9.9", …, payload())` → code `signature_invalid`.
  - In `updater/tests.rs`:

```rust
#[tokio::test]
async fn a_refused_download_leaves_nothing_to_install() {
    for refusal in [
        FeedError::BadSignature,
        FeedError::DownloadRefused("too_large"),
        FeedError::DownloadRefused("failed"),
        FeedError::DownloadRefused("stalled"),
    ] {
        let (backend, host, log) = setup();
        host.0.connected.store(1, Ordering::SeqCst);
        run_check(&*backend, &host, true).await;
        *backend.download.lock().unwrap() = Err(refusal);
        let status = run_install(&*backend, &host).await;
        assert_eq!(status.state, UpdateState::Failed, "{refusal:?}");
        assert!(host.0.rt.lock().unwrap().bytes.is_none());
        finish_install(&*backend, &host);
        assert_eq!(logged(&log), ["download"], "{refusal:?}");
    }
}
```

- [ ] **Step 2: Run** `CARGO test -p fernjo-app --test updater_feed` → FAIL to compile (`Route::Truncated` missing).
- [ ] **Step 3: Implement** the route in `fake_feed.rs`.
- [ ] **Step 4: Run** `CARGO test -p fernjo-app --test updater_feed` and `CARGO test -p fernjo-app --lib updater` → PASS. A test that fails here against unchanged product code is a finding: keep it, note it in `WORK/findings.md`, fix it in Task 8 (commit it with that fix).
- [ ] **Step 5: Prove each test fails without its protection** (temporary edits, none committed):
  - copy the plugin source (`/opt/cargo/registry/src/*/tauri-plugin-updater-2.14.1`) to `/target/plugin-mutant`, make its `verify_signature` succeed without checking, add `[patch.crates-io] tauri-plugin-updater = { path = "/target/plugin-mutant" }` to the root `Cargo.toml`: `redirect_to_another_host_accepts_only_signed_bytes`, `truncated_download_is_refused` and `tampered_download_is_refused` FAIL;
  - set `requireSignedVersion` to `false` in the test's `app()`: `signature_without_a_version_is_refused` FAILS;
  - make the `Err` arm of the download in `run_install` set `Ready` with bytes: `a_refused_download_leaves_nothing_to_install` FAILS.
  Then `git checkout -- Cargo.toml Cargo.lock`, undo the edits, delete the copy, re-run Step 4, and confirm `git status` shows only the intended files. The lying-length case has no removable protection of ours (the HTTP client's length check); record that.
- [ ] **Step 6: Write the attack table** (scenario, test, result, proof) with the rows of spec section 4, including the existing `same_and_older_version_are_not_offered`, `wrong_key_signature_is_refused`, `tampered_download_is_refused`, `signed_version_must_match_the_announced_one`, `notes_keep_markup_as_literal_text`, `notes_with_markup_render_as_text` and `no_html_injection_in_ui` (run `pnpm test` → PASS), and the sentence on what is not covered: the platform install handover, GitHub's real redirects, the real window.
- [ ] **Step 7: Commit** `test(updater): attack the update path (redirect, truncation, unversioned signature)`, push, `gh pr checks <pr> --watch` → green including `rust (windows)`.

### Task 4: Four independent reviews

**Files:**
- Create: `WORK/review-1-updater.md`, `review-2-pipeline.md`, `review-3-webview.md`, `review-4-privacy.md`

- [ ] **Step 1: Write one brief per reviewer**, holding only: the "M14 threat model" entry, `WORK/known-items.md` (do not re-report; do report if worse than recorded), the severity scale of "v0.1.0 security review", its file list and questions (spec section 2, copied verbatim), the range `v0.3.1..HEAD`, and the rules: read-only on the repository, probes only in the scratchpad, `CARGO` for builds, no GitHub write, write the report to its own file. Report format per finding: title, severity, file and function, concrete failure scenario, how it was verified (probe or source reading), suggested fix; then what was checked and found sound.
- [ ] **Step 2: Dispatch** the four fresh subagents in parallel on the most capable model available. No brief contains another's findings or an expected conclusion.
- [ ] **Step 3: Check each report** exists, names the files it read and rates each finding; send back a report that does not. Append all findings, with the reviewer's rating, to `WORK/findings.md`.

### Task 5: Scans, dependency checks and the secrets check

**Files:**
- Modify: `Docs/release-checklist.md` (section "v0.4.0 security review", subsection "Checks")

- [ ] **Step 1: `/security-review`** over `v0.3.1..HEAD`. The milestone branch's default range would not be that: run it where the base is `v0.3.1` (for example a scratch clone whose `origin/HEAD` points at `v0.3.1`) and record that the reviewed file list matches `git diff --name-only v0.3.1..HEAD`.
- [ ] **Step 2: Run and record each with its result:** `cargo deny check` (native, else in a container); `pnpm audit --prod`; `gh api "repos/benjo-code/fernjo/code-scanning/alerts?state=open"` and the same for `dependabot/alerts`; the PR's CodeQL result; `bash tools/check-workflows.sh`; `MSYS_NO_PATHCONV=1 docker run --rm -v "d:/VS-Code/fernjo:/repo:ro" ghcr.io/gitleaks/gitleaks:v8.30.1 detect --source /repo --config /repo/.gitleaks.toml --redact --no-banner`.
- [ ] **Step 3: Secrets check:** `CARGO test -p fernjo-app --lib command_table_covers_every_tauri_command` and `... no_response_contains_a_secret` pass and list the six update commands (`table()` or `APP_HANDLE_EXEMPT` with a reason); read `updater.rs`, `updater/` and `app_settings.rs` for anything secret reaching `settings.json`, events, logs or webview storage. A gap is a finding.
- [ ] **Step 4: Commit** `docs(m14): scans, dependency and secrets checks`.

### Task 6: Pipeline rehearsal and repository settings

**Files:**
- Modify: `Docs/release-checklist.md` (section "v0.4.0 release pipeline rehearsal")

- [ ] **Step 1: Dry run:** `gh workflow run release.yml --ref feature/m14-security-gate-v0.4.0`, `gh run watch <id>`: all builds and hub image builds pass, `sign`, `verify-signatures`, `publish` and the image push are skipped. `gh run download <id> -p 'installers-*' -D WORK/rehearsal/artifacts`.
- [ ] **Step 2: Rehearse with the real names** (Git Bash, repository root, `R=WORK/rehearsal`, `K=src-tauri/tests/fixtures/update-test-key`, version `0.3.1`), running the job commands of `release.yml` in order: `verify-installers.sh 0.3.1 --artifacts $R/artifacts $R/installers`; sign the four update files with `TAURI_SIGNING_PRIVATE_KEY="$(cat $K/test-only.privkey)" TAURI_SIGNING_PRIVATE_KEY_PASSWORD= pnpm tauri signer sign --app-version 0.3.1 "$PWD/<file>"`; move the `.sig` files to `$R/signatures`; verify each with `CARGO run -q -p fernjo-core --example verify_update_signature -- crates/fernjo-core/tests/fixtures/update-sig/tauri.conf.json <file> <sig> --app-version 0.3.1` → 4 × `signature ok`, and one run with `src-tauri/tauri.conf.json` must exit 1; copy the signatures back; `node tools/write-latest-json.mjs 0.3.1 $R/installers CHANGELOG.md`; `--check`; `verify-installers.sh 0.3.1 --release $R/installers`; `(cd $R/installers && sha256sum -- *) > $R/SHA256SUMS` → 12 lines.
- [ ] **Step 3: Rehearse as 0.4.0:** copies of the seven files renamed to `0.4.0` in `$R/installers-040`, changelog copy `sed 's/^## \[Unreleased\]/## [0.4.0] - 2026-11-27/' CHANGELOG.md > $R/CHANGELOG-040.md`; sign, verify, write, check, `--release` as in Step 2. Assert the `notes` of `latest.json` are non-empty, under 16384 bytes and end with the last line of the section (not cut). Record any step that diverges from the workflow (shell, OS) and that the draft check, attestation, upload and publish were not rehearsed.
- [ ] **Step 4: No key material** in the dry run: the first 40 characters of `$K/test-only.privkey` (the header every key file starts with) and `TAURI_SIGNING` values appear in neither `gh run view <id> --log` nor the downloaded artifacts.
- [ ] **Step 5: Read back the settings** (read only): `gh api repos/benjo-code/fernjo/environments/release-signing` and its `deployment-branch-policies`; `gh secret list --env release-signing`; `gh secret list`; `gh api repos/benjo-code/fernjo/rulesets` (and each ruleset); immutable releases as far as the API shows it. Expected: required reviewer, tag policy `v*` only, both secret names in the environment only, the `v*` ruleset active. A deviation is a finding.
- [ ] **Step 6: Commit** `docs(m14): release pipeline rehearsal with the test key`.

### Task 7: Triage

**Files:**
- Modify: `Docs/release-checklist.md` (section "v0.4.0 security review", subsection "Findings")

- [ ] **Step 1: Collect** `WORK/findings.md`, the `/security-review` result, the check gaps and the known items marked for triage. Merge duplicates, keeping every source.
- [ ] **Step 2: Confirm** each finding against the code or a probe before ruling; a refuted one is listed as not a defect with the reason.
- [ ] **Step 3: Write the table** (#, area, finding, severity: reviewer and ruling, decision, fixed in), numbering from 81, plus "What the reviewers found sound". Decisions: `fix`, `owner` (acceptance proposed, with the remaining risk) or `not a defect`. High and Medium: `fix`. A Low in checking, verifying or installing an update: `fix` unless the fix is riskier than the finding. A stop-rule item is raised now.
- [ ] **Step 4: Commit** `docs(m14): triage of the v0.4.0 security review`, push.

### Task 8: Fixes

**Files:** per finding.

Repeat for every row with decision `fix`, one commit per finding:

- [ ] **Step 1: Write the failing test** or recorded check that shows the finding; run it → FAIL for the stated reason.
- [ ] **Step 2: Fix**, with the smallest change that closes the scenario.
- [ ] **Step 3: Run** the test → PASS, then the touched crate's tests (`CARGO test -p <crate>`), or `pnpm test` and `pnpm lint`, or `bash tools/test-release-scripts.sh`.
- [ ] **Step 4: Commit** `fix(<scope>): <what> (M14 review #<row>)`; fill "fixed in" with the commit and test name; add a `Docs/DECISIONS.md` note where a decision is involved and a `CHANGELOG.md` `[Unreleased]` line only when a user can see the change.
- [ ] **Step 5 (High and Medium only): Second read** by a fresh subagent that gets the finding and the fix diff, nothing else. Its objections are fixed or ruled on, and recorded in the row.

After the last fix: push, `gh pr checks <pr> --watch` → green. A finding only a real app can prove counts as fixed once its test passes, and gets an owner check in the owner section.

### Task 9: The owner batch

**Files:**
- Modify: `Docs/release-checklist.md`, `Docs/DECISIONS.md`

- [ ] **Step 1: Send one message** holding, each with what it means and a recommendation: every `owner` row; the Cmd+Q and key check results or the request for them; CodeQL dismissals; every owner check still open (the two of Task 2, M13 checks 3 to 7 with check 4's macOS and Linux part, new ones from Task 8, and the earlier ones by reference to the v0.3.0 and v0.3.1 gaps) with "run" or "defer"; any stop-rule item.
- [ ] **Step 2: Wait.** Nothing is marked accepted, waived or deferred without the owner's answer.
- [ ] **Step 3: Record** the answers: rows updated, `Docs/DECISIONS.md` entry "M14: owner decisions on the security review" with reasons and remaining risks, owner section with results. A rejected acceptance goes back through Task 8. A Cmd+Q result that confirms the path is rated and handled like any finding.
- [ ] **Step 4: Commit** `docs(m14): owner decisions on the v0.4.0 review`.

### Task 10: Release-readiness gate

**Files:**
- Modify: `Docs/release-checklist.md` (section "v0.4.0 readiness"), `Docs/roadmap.json` + `Docs/ROADMAP.md`, `CLAUDE.md`, `CHANGELOG.md`, `README.md` and `SECURITY.md` only if they do not match the app

- [ ] **Step 1: Documentation:** `CHANGELOG.md` `[Unreleased]` names the milestones and reads short and user-friendly; README (features, install, privacy statement) and `SECURITY.md` match the app and review 4; `CLAUDE.md` records the lasting facts (the Windows job, `FERNJO_TEST_NO_SSH_SERVER`, and what the fixes changed). Name the files the release branch bumps; `bash tools/check-release-version.sh v0.4.0` is not run here. Commit.
- [ ] **Step 2: Gate commands on that head**, each recorded with where it ran and its counts: `CARGO build --workspace`, `CARGO test --workspace`, `CARGO fmt --all --check`, `CARGO clippy --workspace --all-targets -- -D warnings`, `CARGO clippy -p fernjo-app --lib --bins -- -D warnings`, `cargo deny check`, `pnpm test`, `pnpm lint`, `pnpm audit --prod`, `bash tools/check-workflows.sh`, `bash tools/test-release-scripts.sh`, `dotnet run tools/roadmap.cs -- check`.
- [ ] **Step 3: CI and dry run on that head:** push, `gh pr checks <pr> --watch` fully green (`rust`, `rust (windows)`, CodeQL included); `gh workflow run release.yml --ref feature/m14-security-gate-v0.4.0` green with "Check the binary for build paths" and the smoke tests in all four builds. If `tools/` or `release.yml` changed after Task 6, repeat its Steps 2 to 4 on these artifacts.
- [ ] **Step 4: Write "v0.4.0 readiness":** the reusable checklist items, the command table, the known gaps table (every open or deferred owner check, accepted findings, unverified claims: GitHub secrets, job wiring, draft and publish steps, install handover, real window, macOS tests, no real update before the release after v0.4.0), and one go or no-go line per area (security review, signing and pipeline, installers, docs, repository settings) plus overall. "Go" needs the key check passed or waived in writing.
- [ ] **Step 5: Roadmap:** tick each task and criterion only with its evidence; status `done` with `completedOn` once the verdict is written and every owner check is run or deferred, otherwise it stays `in-progress` and the verdict says what it waits for. Release `0.4.0` stays `open`. Generate and check.
- [ ] **Step 6: Commit** `docs(m14): v0.4.0 readiness and verdict`, push (documents only, so the evidence of Step 2 holds; say so). `gh pr ready <pr>`, wait for all checks, squash-merge to `dev`.
- [ ] **Step 7: Hand over:** the verdict, the open checks and the release procedure as the next step, which starts only on the owner's instruction.
