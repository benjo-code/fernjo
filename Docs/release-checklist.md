# Release checklist

<!-- TASK 10 PLACEHOLDER: the reusable release checklist (steps for every release) and the v0.1.0 readiness verdict are written in Task 10. Nothing is written here yet. -->

## v0.1.0 security review

Review date: 2026-11-18 (Windows 11, release binary `target/release/fernjo.exe` (built after the last commit that touched `src-tauri`, `crates` or `ui`, so it matches HEAD `1bc2762` in source); CI dry-run installers of run `44205574554`, PR #10). Every check below records its command, its result and the date. Real-app checks ran with a scratch `FERNJO_DATA_DIR`; the owner's real data folder was not read, printed or modified, and "Unlock automatically on this device" was never ticked.

### Findings

| # | area | finding | severity | decision | fixed in |
|---|------|---------|----------|----------|----------|
| 1 | Data at rest | On Unix `connections.json`, `settings.json`, `vault.fernjo`, `known_hosts` and their `.bak` copies were created through `File::create`/`create_dir_all`, i.e. with the default umask (usually 0644 / 0755): other local users could read host names, user names and the encrypted vault. Found by reading `write_atomic` (core), the vault writer and the `known_hosts` writer; not run on a Unix machine (none available). | Medium | Fixed here: mode 0600 on creation (and on the `.bak`), 0700 for a folder fernjo creates, Windows ACLs unchanged; unix-only test per crate. The core and vault tests were type-checked for `x86_64-unknown-linux-gnu` but not run (no Linux host); the `fernjo-ssh` one could not even be cross-checked (needs a C cross compiler). **Run on Linux/macOS:** `cargo test -p fernjo-core -p fernjo-vault -p fernjo-ssh --lib` (CI's `rust` job does this once pushed). | 64aa818 |
| 2 | SSH crypto | russh's default host key list ends with plain `ssh-rsa` (RSA over SHA-1), which OpenSSH stopped offering in 8.8 (confirmed against the local OpenSSH 10.3 defaults, `ssh -G localhost`). It was offered on first contact with a host. A failing unit test showed the list before the fix. | Medium | Fixed here: removed in `client_config`; `client_config_offers_no_weak_algorithms` pins the whole offered policy. Servers that only have a SHA-1 RSA host key now fail to connect, like with a current `ssh`. The live-server tests were not rerun (docker daemon not running): **run** `eval "$(tools/ssh-test-server.sh start)"; cargo test -p fernjo-ssh; tools/ssh-test-server.sh stop` (CI runs them). | 6648cff |
| 3 | Supply chain | `deny.toml` still carried the time-boxed ignore `RUSTSEC-2023-0071` (RSA Marvin attack) although `cargo update -p russh` resolves to russh 0.65.2 and `rsa` 0.10.0, which is not affected. | Low | Fixed here: lockfile updated (russh 0.65.2, rsa 0.10.0, ssh-key 0.7.0-rc.12, pkcs1 0.8.0), ignore removed; `cargo deny check`, `cargo test --workspace --locked` and clippy green. | 5791506 |
| 4 | Webview | The main webview can be navigated to a remote origin from the page (`location.href = 'http://127.0.0.1:8099/'` loaded a page from a test server). The remote page gets `window.__TAURI_INTERNALS__`, but every command (`list_tree`, `vault_status`, `get_data_folder`) was refused with "not allowed by ACL", because `capabilities/default.json` has no `remote` section. No IPC reaches remote content, so this is not the Medium of the brief. | Low | Backlog: an `on_navigation` guard that only allows the app origin (`tauri.localhost` / `localhost:1420` in dev) as defence in depth. Not fixed here. | - |
| 5 | Network | The WebView2 runtime (`msedgewebview2.exe`, `--type=utility`, i.e. its network service), not fernjo code, opens one HTTPS connection to `150.171.27.11:443` (a Microsoft address, no reverse DNS) about 60 s after start; seen in two of two runs. `--disable-background-networking` did not stop it. Purpose not identified. fernjo.exe itself made none. | Low | Accepted and stated in the privacy statement (see below); backlog: find out what the connection is and whether a WebView2 policy or a Tauri `additionalBrowserArgs` flag turns it off. | - |
| 6 | Release binary | The CI-built binaries embed the builder's cargo-registry paths in panic locations (`C:\Users\runneradmin\.cargo\registry\...` 580 times in the Windows exe, `/home/runner/.cargo/...` in the Linux binary, `/Users/runner/.cargo/...` in both macOS binaries). Neither the owner's account name nor `VS-Code` occurs in any installer. The local binary does contain the owner's account name in the same way, but local builds are not distributed. | Low | Accepted; backlog: `RUSTFLAGS=--remap-path-prefix` in the release workflow if the paths should disappear. | - |
| 7 | Supply chain | Open Dependabot alerts (2): `glib` 0.18.5 (GHSA-wrw7-89jp-8q8g, = `RUSTSEC-2024-0429`, Linux only, via Tauri's GTK3 stack, fix is glib >= 0.20 which Tauri 2 does not allow) and `minimatch` 10.2.6 (GHSA-g3hv-75wg-m249, npm, development dependency of typescript-eslint, `pnpm audit --prod` is clean). | Low | glib: known gap, recorded since M0 in `Docs/DECISIONS.md`; revisit when Tauri leaves GTK3. minimatch: dev-only, take the update when typescript-eslint moves (backlog). | - |
| 8 | Repository | The required status checks of `main` and `dev` (`roadmap`, `rust`, `ui`, `tauri-build`, `cargo-deny`) do not include `workflows` (added in M6), nor the release dry-run builds. | Low | Owner action after the merge: add `workflows` to the required checks of both branches (a GitHub setting, not changed by this review). | - |

Rows from the independent reviewers (Step 9) go here, numbered from 9, each with its reason when rejected:

| # | area | finding | severity | decision | fixed in |
|---|------|---------|----------|----------|----------|

### Checks (command, result, date 2026-11-18)

**Step 1 - Secrets**

- `cargo test -p fernjo-app --lib -- no_response_contains_a_secret command_table_covers_every_tauri_command`: 2 passed. No finding.
- `rg -n "println!|eprintln!|dbg!|tracing::|log::" crates src-tauri/src` (Grep tool, same pattern): hits only in `crates/fernjo-core/examples/generate_tree.rs`, `crates/fernjo-vault/src/vault.rs` (an `#[ignore]` timing test that prints durations), `crates/fernjo-ssh/tests/**` and `src-tauri/src/sessions/tests.rs` (skip messages), plus `tauri_plugin_dialog::` which only matches because it ends in `log::`. Each hit read. No finding.
- `rg -n "localStorage|sessionStorage" ui/src -g '!*.test.ts'`: `store.svelte.ts` (the set of expanded folder ids under `STORAGE_KEY`) and `theme.svelte.ts` (the theme name under `THEME_KEY`). View state only. No finding.
- `rg "@html" ui/src`: empty. No finding.
- Real app (release binary, scratch `FERNJO_DATA_DIR`, DevTools port): vault created, a password entry and a private-key entry (with passphrase) created with made-up sentinel secrets (`SENTINEL-VAULTPW-...`, `SENTINEL-SSHPW-...`, `SENTINEL-KEYBODY-...`, `SENTINEL-PASSPHRASE-...`), a connection bound to each, `open_session` run twice (both ended with "Could not connect to the host.", see OPEN), `reveal_credential` called, the vault locked, the page reloaded and the vault password typed into the real unlock dialog with `Input.insertText` and Enter. Window closed, then a byte search (ASCII and UTF-16LE) of the prefix `SENTINEL-` and each sentinel's suffix: scratch data folder (4 files, 1662 bytes) 0 matches; `%LOCALAPPDATA%\io.github.benjo-code.fernjo` (785 files, 117,020,309 bytes) 0 matches. Positive control: the connection name `sentinel-conn` is found in `connections.json` and `connections.json.bak` and not in the WebView2 folder. The username of a credential does not appear in `connections.json` either. No finding.

**Step 2 - Webview and IPC surface**

- `src-tauri/capabilities/default.json`: `"permissions": ["core:default"]`, window `main` only. No finding.
- From the page (`Runtime.evaluate`, release binary): `plugin:dialog|open`, `plugin:opener|open_url`, `plugin:opener|reveal_item_in_dir`, `plugin:fs|read_file` and `plugin:shell|open` all refused ("not allowed by ACL"). `open_external_url` refused `file:`, `javascript:`, `data:`, `\\server\share\x`, `//server/share`, `http://` and `http:` with `invalid_url`. No finding.
- `check_external_url` tests: `open_external_url_accepts_only_http_and_https` covered `file:`, `javascript:`, `data:`, host-less `http:` and now also UNC paths (added here, commit 216679b).
- Served CSP of the release binary (`Network.responseReceived` on a reload): `img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self' 'sha256-x3TDFUPE0pD8bvCROrBBzM23ZqLo7rRFMpX7Kj/yTYU='; connect-src ipc: http://ipc.localhost; default-src 'self'`. The four directives of `tauri.conf.json` are unchanged; Tauri adds the `script-src` hash for its own inline script. A `fetch(location.href)` from the page failed with "Failed to fetch", so the CSP is enforced. No finding.
- Navigation guard: see finding 4 (possible, but no IPC for the remote page).

**Step 3 - Release-binary hygiene**

- Byte search of `target/release/fernjo.exe` (ASCII and UTF-16LE): `FERNJO_DIALOG_FILE` 0, `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` 0, `remote-debugging-port` 0 (`FERNJO_DATA_DIR` is present, by design). No finding.
- DevTools in the release binary: `tauri` has no `devtools` feature in `src-tauri/Cargo.toml`; a release binary started without the environment variable has nothing on port 9222 (`Get-NetTCPConnection -LocalPort 9222 -State Listen` empty). They are reachable with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=...`, as used for these checks. Accepted: whoever can set the environment of the process already runs code as the user. Recorded, no finding.
- Installers of the dry run (`gh run download 44205574554`, extracted with 7-Zip into a scratch folder, never run): in `fernjo.exe` (NSIS payload), `usr/bin/fernjo` of the deb and the AppImage and `Contents/MacOS/fernjo` of both dmgs, the strings `benja`, `TiefSee` and `VS-Code` do not occur (the one `benja` in the AppImage is "benjamin@webkit.org" in a bundled WebKit copyright file). The runner's home appears as described in finding 6. The THIRD-PARTY-NOTICES resource is in all five installers. No finding beyond 6.

**Step 4 - Network behaviour**

- `cargo tree -e normal --workspace --target all`: `reqwest` 0.13.5 (with `hyper` 1.12.0 and `hyper-util` 0.1.21) is pulled in only by the `tauri` crate (`cargo tree -i reqwest`); no fernjo crate depends on `reqwest`, `ureq`, `hyper` or an updater (`grep` over `Cargo.toml` files and sources: none). The only URL literal in fernjo's own sources (Rust, TypeScript, Svelte) is `http://x/` in a doc comment.
- Release binary idle, started without the debugging variable, `Get-NetTCPConnection` over `fernjo.exe` and all descendants every 2 s for 150 s (and 20 s samples to 60 s in a first run): no connection other than loopback until about 60 s, then the single WebView2 connection of finding 5 in both runs. Therefore the literal "loopback only for 60 s" condition does not hold for the WebView2 runtime; it holds for fernjo.exe.
- No update check (no updater plugin, `tauri_conf.rs` guards `bundle` against one), no telemetry in fernjo's code.
- Proposed privacy statement (all three conditions do not hold literally, so the wording is limited to what was verified; for the controller or Task 10 to put into the README):

  > fernjo makes no network connections of its own except the SSH connections you open. It has no update check, no telemetry and no analytics. Your connections, settings and encrypted vault stay in your data folder on your computer. fernjo displays its window with the web view of your operating system (WebView2 on Windows, WebKit on macOS and Linux); that component is separate from fernjo. In our check on Windows, the WebView2 runtime opened one HTTPS connection to a Microsoft address about a minute after start, which fernjo does not start and cannot switch off. macOS and Linux were not checked.

**Step 5 - Data at rest**

- Read `write_atomic` (`crates/fernjo-core/src/persist.rs`, used for `connections.json` and `settings.json`), `write_atomic` in `crates/fernjo-vault/src/vault.rs` and in `crates/fernjo-ssh/src/known_hosts.rs`: all used `File::create` and `create_dir_all`, no mode set, `.bak` through `fs::copy` (copies the source mode). Gap confirmed by reading: finding 1, fixed. `write_exact` (exports to a user-chosen path) and the data-folder move (`copy_atomic`, `fs::copy` keeps the source mode) are left alone on purpose. Windows ACL inheritance unchanged.

**Step 6 - SSH crypto policy**

- Offered algorithms read from `russh` 0.65.1/0.65.2 `Preferred::DEFAULT` (`negotiation.rs`) and compared with `ssh -G localhost` (OpenSSH 10.3p1): full list and comparison in `Docs/DECISIONS.md`, "M6 security review: SSH algorithm policy". No SHA-1 key exchange or MAC, no CBC, no 3DES, no group1; plain `ssh-rsa` host key was offered: finding 2, fixed.

**Step 7 - Supply chain**

- `cargo deny check`: `advisories ok, bans ok, licenses ok, sources ok` before and after the update; `cargo deny check advisories` lists no unused ignore.
- `pnpm audit --prod`: "No known vulnerabilities found".
- Ignore review against `Cargo.lock`: `RUSTSEC-2023-0071` removed (finding 3); `RUSTSEC-2024-0370` (proc-macro-error), `-2026-0370/0371/0372` (zvariant, zbus_names, zvariant_derive) and `-2026-0543` (chrono, via russh-util) still match crates in the lockfile and keep their reasons.
- `pnpm install --frozen-lockfile`: "Lockfile is up to date". `cargo build --workspace --locked`, `cargo test --workspace --locked` (all green, live-SSH tests skipped without the container) and `cargo clippy --workspace --all-targets --locked -- -D warnings`: green after the update.
- `gh api repos/benjo-code/fernjo/dependabot/alerts --jq '[.[]|select(.state=="open")]|length'`: 2 (finding 7).
- `gh api "repos/benjo-code/fernjo/code-scanning/alerts?per_page=100"`: 100 alerts, all `dismissed` (all `rust/hard-coded-cryptographic-value` in test and sentinel code); 0 open on `dev`, `main`, the feature branch and the PR merge ref. Nothing for the owner to dismiss.

**Step 8 - Pipeline and repository**

- `bash tools/check-workflows.sh`: exit 0.
- `grep -rn "secrets\." .github/workflows`: no match (only `github.token` is used, in the publish job).
- Re-read with the `gh api` commands of M0's evidence (GET only): branch protection of `main` and `dev` returns `[["roadmap","rust","ui","tauri-build","cargo-deny"],false,false,true]` for both (as in M0, but see finding 8); repository `public`, `secret_scanning` enabled, `secret_scanning_push_protection` enabled, `dependabot_security_updates` enabled; private vulnerability reporting `true`; fork-PR approval `all_external_contributors`; default workflow permissions `read`, cannot approve PR reviews; no self-hosted runners registered (`total_count` 0). Same as M0.
- `release.yml` read again: the tag and `inputs.tag` reach shells only through `env:` (`TAG`), matrix values are static, `ref:` and `concurrency.group` use the event values outside any shell, `pull_request` runs with `contents: read` and no secrets, `contents: write` only in `publish`. No finding.
- History scan: not run with gitleaks (see OPEN). A substitute, weaker check: `git grep` over all 31 revisions (`git rev-list --all`) for private-key headers and the token shapes `ghp_`, `github_pat_`, `AKIA`, `xox*-`, `AIza` found nothing.

### Open items

- gitleaks over the whole history: not run. Tool not installed, the Docker daemon was not running here (`docker version` could not reach the engine), and installing the CLI with `go install` was refused by the permission system. Verify with `docker run --rm -v "<repo>:/repo:ro" ghcr.io/gitleaks/gitleaks:v8.30.1 detect --source /repo --redact --no-banner` (expected `no leaks found`; tag as used in M0, confirm it exists first) or the local CLI `gitleaks detect --redact`.
- The sentinel check of Step 1 did not reach a real SSH authentication (no reachable SSH server: the Docker daemon was not running). The connect plan resolved the secrets and the connection failed at TCP connect. Verify with `eval "$(tools/ssh-test-server.sh start)"`, a connection to `FERNJO_TEST_SSH_ADDR` with a sentinel password credential (wrong password, auth fails after the secret was used) and the same byte search; stop with `tools/ssh-test-server.sh stop`.
- Unix file-mode tests (finding 1): run on Linux/macOS, command in the table.
- SSH integration tests against the container after the russh update and the host key change (finding 2/3): command in the table.
- Linux and macOS network behaviour (privacy statement): not checked.
