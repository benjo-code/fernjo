# Milestone 14: security check and release readiness for v0.4.0 — design

Date: 2026-11-27 · Status: awaiting the owner's approval · Roadmap: milestone 14 in `Docs/ROADMAP.md` · Branch: `feature/m14-security-gate-v0.4.0`

## Intent

v0.4.0 is the first version with automatic updates. It is installed by hand once, and from then on it takes whatever its updater accepts. A flaw in that updater, or a signing key that does not match, can only be repaired by another manual reinstall on every device. So v0.4.0 is released only after an independent security check of what changed since v0.3.1 (77 files, about 7,500 added lines, mostly the M13 updater) and a written go or no-go verdict. M14 prepares the release and does not perform it.

## Goals

- Every area has a written, independent review, and every finding sits in `Docs/release-checklist.md` with severity, disposition and evidence.
- No High finding is open. Every Medium is fixed, or its remaining risk is recorded in `Docs/DECISIONS.md` with the owner's acceptance. Every Low is fixed or accepted by the owner with the reason recorded.
- The update path is attacked on purpose and the result recorded.
- The tests, the updater's among them, run on Windows.
- The chance that the first signed release run fails is reduced as far as possible without giving the signing key a second way to be used.
- A go or no-go verdict for v0.4.0 is written, naming every known gap and every owner check that is still open or deferred.

## Non-goals

- No release: no version bump, no release branch, no tag, no PR to `main`.
- No new features. UI changes only where a finding needs one.
- No update through the real app against a test feed (owner decision 2026-11-27: the updater is checked with the real feed at the release after v0.4.0; the test feed stays optional).
- No signing rehearsal on GitHub (it would need the tag-only rule of the `release-signing` environment loosened).
- No code signing or notarisation.

## Anti-goals

- A finding "fixed" without a test or recorded check that fails without the fix.
- A High or Medium fix that nobody but its author has read.
- Reviewers that saw each other's findings or were handed the expected conclusions.
- The real signing key or its password reaching this machine, a file in the repository, a log, a reply or a shell history.
- Anything shown as verified that was not run: the install handover, the real GitHub feed and the real app UI are not covered by headless tests.
- Weakening a check, the workflow policy or the environment protection, skipping a test or adding a `deny.toml` ignore just to get green.

## Constraints

- Everything in `CLAUDE.md`, in particular: secrets stay in Rust; the three-places rule for commands; GitHub-hosted runners only; Dependabot pull requests are never merged by Claude; CodeQL alerts are dismissed only on the owner's word; releases are immutable and `v*` tags cannot move.
- The development machine blocks freshly built native binaries: Rust build, test and clippy run in the Linux image `fernjo-dev` (Docker). Until M14 adds the Windows test job (section 3), CI runs tests and clippy on Linux only; Windows and macOS code is only compiled, by the release workflow's builds. macOS stays that way. The evidence says what ran where. The machine has no usable WebView2 runtime, so real-app checks are the owner's.
- Review reports are working files under `.superpowers/` (gitignored); the findings go into `Docs/release-checklist.md`.

## Approaches

**Order of the work.**

1. *Threat model, then independent reviews and checks, then triage and fixes, then the gate (chosen).* Few items are already known, so nothing needs fixing before the reviewers read the code.
2. *Fix the known residual limits of M13 first, then review* (the order M11 used). Those limits are few and several are decisions for the owner, so they can go through triage like findings.
3. *One reviewer for the whole diff.* Cheaper, but the roadmap asks for an independent review per area.

**Reducing the risk of the first signed run.** Signing, signature verification and writing `latest.json` run only on a tag push and have never run on GitHub.

- A. *Owner check of the key copy plus a local rehearsal of the pipeline's commands with the test key (chosen; the owner left the pick to the builder).* Shows that the owner's key and password match the public key in the app, and that the commands work on real build artifacts. Does not show the GitHub secrets or the job wiring.
- B. *A plus a signing rehearsal on GitHub.* Tests the secrets too, but needs the environment's tag-only rule loosened, which is the main protection of the key.
- C. *Accept the risk.* A workflow bug then costs the v0.4.0 version number.

**Windows tests.** No test of the M13 updater has run on Windows: CI tests on Linux only, and this machine blocked the binaries during M13.

- A. *A permanent Windows test job in CI (chosen; the owner left the pick to the builder).* Covers this and later releases at no cost on a public repository. A first run may show failures that M14 then has to fix.
- B. *The owner runs the tests once on their Windows PC.* No CI change, but needs a build setup there and covers only this release.
- C. *Change nothing* and name it as a gap in the verdict.

## Design

### 1. Threat model

A new `Docs/DECISIONS.md` entry "M14 threat model" that adds to the M11 one what v0.4.0 brings: the update feed, the signing key and the pinned public key, the downloaded bytes, the install handover, the two new settings and the narrowed webview capability. Per asset it states what touches secrets, the data folder, the network and the webview, and what a hostile or compromised feed, network path, redirect target, pull request or fork, build dependency, tag pusher, repository administrator, local process of the same user and webview script can do. It names the accepted trust assumptions and is the reviewers' brief.

### 2. Independent reviews

Four fresh read-only subagents on the strongest available model, in parallel, each with only its own brief: the threat model entry, its file list, the severity scale of the "v0.1.0 security review", and the list of known items (stated with their disposition, without the reasoning). Probes only in a scratch folder.

1. **Updater:** `src-tauri/src/updater.rs` and `updater/`, the close and restart paths it touches (`lib.rs`, `sessions.rs`, `sync_runner.rs`, `state.rs`) and the source of the pinned `tauri-plugin-updater`: hostile or compromised feed, downgrade, redirect, tampered or truncated download, release notes as text, the handover to the installer and its file paths.
2. **Release pipeline and key handling:** `release.yml`, `ci.yml`, `tools/` (including `update-test.conf.json`), `verify_update_signature`, `.gitleaks.toml`, the GitHub environment and tag rules (read only): secrets only in the signing step, fork pull requests, artifact contents, `latest.json`. Also the hub deploy files changed since v0.3.1 (`compose.podman.yaml`, `fernjo-hub-compose.service`, `Docs/hub.md`), which fit no other area.
3. **Webview surface:** the capability file, `tauri.conf.json` and its tests, the full command list and the no-secret guard, the update UI (`ui/src/lib/update*`, the update components, the sidebar marker).
4. **Settings and privacy:** `settings.rs`, `app_settings.rs`, the scheduler's off switches, and whether `README.md`, `SECURITY.md` and the privacy statement are true for what the app sends over the network.

### 3. Scans and checks

- `/security-review` over the whole range `v0.3.1..HEAD` (on the milestone branch the default "pending changes" range would be empty).
- `cargo deny check`, `pnpm audit --prod`, open Dependabot and CodeQL alerts (`gh api`), `bash tools/check-workflows.sh`, gitleaks over the history.
- Secrets stay in Rust: the no-secret guard covers the six update commands; nothing secret reaches `connections.json`, `settings.json`, logs, sync data, update data or webview storage; the logs and artifacts of the dry run hold no key material.

**Windows test job.** `ci.yml` gets a job on a GitHub-hosted Windows runner that runs the workspace tests, under the existing workflow rules (pinned actions, `contents: read`, `check-workflows.sh` green). It is added first, so the reviews and fixes start from its result. Tests that need the Linux OpenSSH container are left out explicitly in that job and keep running in the Linux job; unsetting `CI` to make them skip is not allowed. A failure that shows a defect is entered and rated like any finding; a failure of the test setup is fixed in M14 without weakening the test. The job does not exercise the install handover.

### 4. The recorded attack on the update path

Headless: the fake feed with the real updater plugin in a mock app, plus the UI tests. A table in `Docs/release-checklist.md` maps each scenario to its test and result:

| Scenario | Today |
|---|---|
| Feed with a lower version | covered |
| Signature made with another key | covered |
| Modified installer behind a valid `latest.json` | covered |
| Redirect to another host | missing: the redirect test stays on one server; add a second server, genuine bytes accepted, modified bytes refused |
| Release notes with markup | covered (Rust and UI) |
| Truncated download | to be checked, added if missing |
| Older signed file announced as newer; signature without a version | first covered, second to be checked |

"Installs nothing" means: a refused download yields no bytes and leaves the updater with nothing to install, and a test says so. A test added here is shown to fail when its protection is removed. Not covered, and said so: the platform install handover, GitHub's real redirects and the real window.

### 5. First-run risk

**Owner check of the key copy.** The owner signs a copy of the committed fixture `payload.bin` with their own key copy and `--app-version 0.0.0` (so the app would never accept that signature), following written steps that keep the key and password off disk and out of the shell history, and sends only the `.sig` file. It is verified with `verify_update_signature` against `src-tauri/tauri.conf.json`. The signature is not committed; the result is recorded. A failure is cheap now and expensive later: before v0.4.0 no installed copy trusts the key, so a new pair can replace it at no cost to users. The verdict is "go" only when this check passed or the owner waived it in writing. *(Builder's call after the check:)* after it passed, the steps offer the owner, as an optional action of their own, to set both `release-signing` secrets again from the verified copy. The GitHub secrets stay unverified either way and the verdict says so.

**Rehearsal of the pipeline.** With the artifacts of a dry run of this branch, the commands of the `sign`, `verify-signatures` and `publish` jobs up to `SHA256SUMS` are run locally with the test key (and a configuration that holds the test public key). Copies renamed to 0.4.0 and a scratch copy of the changelog with `[Unreleased]` turned into `[0.4.0]` show that the real release notes fit `latest.json`. Not rehearsable: the draft check, the attestation, the upload and the publish step. The environment protection, secret names and tag ruleset are read back with `gh api` (read only).

### 6. Triage and fixes

- A section "v0.4.0 security review" in `Docs/release-checklist.md`, in the style of the earlier ones: reviewer severity and ruling both kept where they differ, numbering continued after row 80.
- Entered as known findings: the "Known residual limits" of the M13 section (recorded, but never put to the owner for acceptance) and the macOS Cmd+Q path from the backlog. *(Builder's call after the check:)* the owner is asked for the Cmd+Q check at the start, with the installed v0.3.x app (the close confirmation has been in it since v0.3.0), so the result can arrive before triage. It is rated and fixed or accepted like any finding.
- *(Builder's call after the check:)* a finding that only a real app on Windows, macOS or a Linux desktop can prove counts as fixed when its headless test or recorded check fails without the fix and passes with it. The proof on the real platform is then an owner check, run or deferred by the owner and named in the verdict.
- High and Medium are fixed, test first; each gets a second read by a fresh reviewer who sees the finding and the fix diff. A Low in what v0.4.0 will do on an installed device (checking, verifying, installing) is preferably fixed, because it cannot be repaired remotely if the updater is the broken part.
- The work runs to the end of the reviews and the fixes that need no owner, then stops once with one batch: acceptances, the Cmd+Q result, CodeQL dismissals, any stop-rule item. A High that cannot be fixed is raised at once.

### 7. Release-readiness gate

A "v0.4.0 readiness" section in `Docs/release-checklist.md` following the reusable checklist: gate commands with results on the final head (and where each ran), CI of the milestone pull request green, a release workflow dry run on that head (M14 may change no packaging input, so it is started with `gh workflow run release.yml --ref <branch>`, which only builds), `CHANGELOG.md` `[Unreleased]` short and readable, README, `SECURITY.md` and privacy statement true, known gaps, one go or no-go line per area and overall. The version bump and `check-release-version.sh v0.4.0` belong to the release branch; the gate names the files to change. The release pull request's own dry run is recorded when the release is made.

### 8. Owner checks

Listed with commands or steps; the owner runs or consciously defers each:

- the key copy check (section 5, new);
- macOS Cmd+Q with a connected session;
- M13: the first signed run (can only happen at the release), the capability check on macOS and Linux, and the update, release page offer and window look already deferred to the release after v0.4.0;
- the older checks still open from the v0.3.0 and v0.3.1 gaps, by reference.

M14 is done when the verdict is written and every check is either run or deferred by the owner.

### Roadmap and docs

A scope-change entry dated 2026-11-27: the key copy check, the pipeline rehearsal and the Windows test job are added to M14; the dry run is taken on the milestone head and the version check stays with the release branch. `Docs/DECISIONS.md` gets the threat model and the owner's decisions, `CLAUDE.md` only lasting facts, `CHANGELOG.md` only if a fix changes what a user sees.

## Stop rules

Stop and ask the owner when: a High cannot be fixed within the milestone; the key copy check fails; a fix would change the pinned public key or endpoint, a file format or a wire contract; a fix needs the workflow policy or the environment protection loosened; a finding contradicts an owner decision above; a dependency change passes `cargo deny check` only with a new ignore.

## Left to the builder

Task breakdown and order, reviewer prompts and file lists in detail, test names, the exact commands of the owner check and the rehearsal, severity rulings (with the reviewer's rating kept), and the wording of the verdict.
