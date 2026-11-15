# fernjo – Decisions

Log of architecture and technology decisions. Newest entries on top.

**When an entry is due:** choice of framework, library or version, architecture and interface decisions, deliberately rejected approaches, workarounds with consequences – anything you would otherwise have to discuss again in six months.

**Rules:** Do not rewrite entries after the fact. If a decision is revised, add a new entry and append "Superseded by: <title>" to the old one.

<!-- Template:

## YYYY-MM-DD – <Title>
**Context:** <Which problem, which constraints?>
**Decision:** <What was decided?>
**Rationale:** <Why?>
**Rejected alternatives:** <What else was considered and why not>
-->

## 2026-11-15 – cargo-deny: licence allow-list and advisory ignores
**Context:** M0 adds a `cargo-deny` CI job (advisories, bans, licences, sources) that must be green on the workspace including Tauri's dependency tree (cargo-deny 0.20.2, run with `--all-features`; the graph covers all platforms, so Linux-only crates are checked too).
**Decision:** `deny.toml` allows only `MIT`, `Apache-2.0`, `Apache-2.0 WITH LLVM-exception`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Zlib`, `Unicode-3.0`, `CC0-1.0`, `MPL-2.0`; no licence was added beyond this starting list (the first run reported no licence violations). Sources are crates.io only (unknown registry and git denied); duplicate versions only warn. One advisory is ignored: `RUSTSEC-2024-0370` (`proc-macro-error` unmaintained, pulled in by `glib-macros` through Tauri's Linux GTK3 stack; no safe upgrade exists). Remove the ignore when Tauri moves off GTK3 (or `glib-macros` drops `proc-macro-error`); `cargo deny check` will then report the entry as unused, as a *warning* (not an error), so look for it in the output after Tauri upgrades. Wildcard dependencies are not allowed (default `warn` is not loosened) and the licence confidence threshold stays at its default.
**Rationale:** The allow-list is limited to permissive licences and MPL-2.0 (weak copyleft), all compatible with distribution under `MIT OR Apache-2.0`. The ignore is a single, id-specific entry with a reason: the crate is a build-time proc macro we cannot replace, and it is an unmaintained notice, not a vulnerability.
**Rejected alternatives:** Disabling the unmaintained check (`unmaintained = "none"`) or blanket-ignoring advisories (would hide future findings); allowing licences speculatively (every addition should be driven by a real finding); skipping crates in the licence check.

## 2026-11-15 – Licence MIT OR Apache-2.0
**Context:** The owner wants fernjo open source and left the choice of licence open.
**Decision:** Dual licence `MIT OR Apache-2.0`.
**Rationale:** Rust ecosystem convention, compatible with all planned dependencies (russh, IronRDP, Tauri, xterm.js), and Apache-2.0 adds a patent grant.
**Rejected alternatives:** GPL-3.0 (would keep forks open, but more friction and not needed for a personal tool); proprietary (the owner wants open source).

## 2026-11-15 – Public repo, CI only on GitHub-hosted runners
**Context:** Tauri on Linux needs WebKitGTK system packages; the org's self-hosted runner (`dotnet-linux`) has no root. Release builds need macOS and Windows machines. Hosted-runner minutes cost money on private repos.
**Decision:** The repo is made public in M0, and all CI, including the `roadmap` job, runs on GitHub-hosted runners. Repo security is switched on: secret scanning with push protection, Dependabot alerts and updates, CodeQL, private vulnerability reporting, `cargo-deny`, and approval for workflows from outside contributors.
**Rationale:** Free hosted minutes for public repos, all three OSes available. Self-hosted runners must never run workflows of a public repo, because fork PRs could execute code on them.
**Rejected alternatives:** Self-hosted runner with WebKitGTK added (no root, and unsafe once public); mixed self-hosted/hosted (more complexity, the same public-repo risk).

## 2026-11-15 – Vault: wrapped data key, Argon2id, XChaCha20-Poly1305
**Context:** Credentials must be stored encrypted, the vault file may be synced between devices by the user, and "unlock automatically on this device" uses the OS keychain.
**Decision:** A random 256-bit data key encrypts the whole vault file; the master password derives (Argon2id) a key that wraps the data key. The OS keychain stores the data key. RustCrypto crates only.
**Rationale:** Changing the master password only re-wraps the data key, so keychain entries on other synced devices stay valid; no home-made crypto.
**Rejected alternatives:** Storing the password-derived key in the keychain (invalidated on every password change); OS keychain as the only store (does not sync across OSes, no portable backup).

## 2026-11-15 – SSH via russh, RDP via IronRDP
**Context:** The core must be Rust; SSH is used more than RDP.
**Decision:** russh (tokio) for SSH; IronRDP for RDP, rendered through a framebuffer streamed to a canvas. The RDP milestone starts with a measured spike; a native render surface is the fallback (its own milestone if needed).
**Rationale:** Both are pure Rust and actively maintained; IronRDP supports TLS, NLA/CredSSP and clipboard.
**Rejected alternatives:** FreeRDP via FFI (C dependency, kept as fallback if IronRDP fails for protocol reasons); launching the OS RDP client (none on macOS/Linux, breaks the one-window idea); libssh2 bindings (C dependency).

## 2026-11-15 – UI: Tauri 2 + Svelte 5 + xterm.js
**Context:** fernjo needs a high-quality terminal (main use is SSH), a modern UI (the reason for the project) and RDP rendering, on three OSes.
**Decision:** Tauri 2 shell, Svelte 5 + TypeScript frontend, xterm.js for the terminal. All logic stays in Rust crates that do not depend on Tauri.
**Rationale:** xterm.js is a mature terminal (used by VS Code); web UI makes design iteration cheap. RDP via canvas is the accepted risk, checked by a spike.
**Rejected alternatives:** egui/Iced + alacritty_terminal (all Rust, best RDP, but immature terminal widget and harder to make look modern); Slint (no terminal widget); Electron (heavy, non-Rust core); Qt bindings (awkward in Rust).
