# fernjo

A lightweight, cross-platform remote connection manager for **SSH and RDP** on Windows, macOS and Linux, built on Rust.

All your connections live in one folder tree in the sidebar, sessions open as tabs in a single window, and passwords and keys are kept in an encrypted vault. Existing connections can be imported from mRemoteNG, including saved passwords.

**Status:** in development – see [Docs/ROADMAP.md](Docs/ROADMAP.md). Nothing is installable yet; the first release (v0.1.0) brings SSH, v0.2.0 adds RDP.

## Features (planned)

- Connection tree with nested folders; folders pass settings such as username, credential or port down to their connections
- SSH sessions in tabs with a full terminal (xterm.js), agent/key/password/keyboard-interactive login and host key verification
- RDP sessions in tabs (IronRDP)
- Encrypted credential vault (Argon2id + XChaCha20-Poly1305), optional unlock via the OS keychain
- Import from mRemoteNG, export and encrypted backup
- Data folder you can sync yourself (Syncthing, Nextcloud, iCloud) – the vault stays encrypted

Design and scope: [Docs/superpowers/specs/2026-11-15-fernjo-design.md](Docs/superpowers/specs/2026-11-15-fernjo-design.md)

## Setup

Prerequisites: Rust (stable, version pinned in `rust-toolchain.toml`), Node.js LTS with pnpm, the [Tauri 2 system prerequisites](https://tauri.app/start/prerequisites/) for your OS, and the .NET 10 SDK for the roadmap tool.

```bash
pnpm install
pnpm ui:build                                          # first, on a fresh checkout: the Tauri crate embeds ui/dist
pnpm tauri dev                                         # run the app
cargo test --workspace                                 # Rust tests
pnpm test                                              # UI tests
pnpm lint                                              # ESLint, Prettier, svelte-check
cargo fmt --all --check                                # Rust formatting
cargo clippy --workspace --all-targets -- -D warnings  # Rust lints
cargo deny check                                       # licences, advisories, sources
```

## Security

Please report vulnerabilities privately, see `SECURITY.md` (added in milestone 0).

## License

Licensed under either of

- Apache License, Version 2.0 (`LICENSE-APACHE`)
- MIT license (`LICENSE-MIT`)

at your option. Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
