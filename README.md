# fernjo

A lightweight, cross-platform remote connection manager for **SSH and RDP** on Windows, macOS and Linux, built on Rust.

All your connections live in one folder tree in the sidebar, sessions open as tabs in a single window, and passwords and keys are kept in an encrypted vault. Existing connections can be imported from mRemoteNG, including saved passwords.

**Status:** in development – see [Docs/ROADMAP.md](Docs/ROADMAP.md). Nothing is installable yet; the first release (v0.1.0) brings SSH, v0.2.0 adds RDP.

## Features (planned)

- Connection tree with nested folders; folders pass settings such as username, credential or port down to their connections
- SSH sessions in tabs with a full terminal (xterm.js), agent/key/password/keyboard-interactive login and host key verification
- RDP sessions in tabs (IronRDP)
- Encrypted credential vault (Argon2id + XChaCha20-Poly1305), optional unlock via the OS keychain
- Import from mRemoteNG (`confCons.xml` 2.6, including saved passwords), export of the connections as JSON and an encrypted full backup with restore (formats in [Docs/backup-format.md](Docs/backup-format.md))
- Data folder you can sync yourself (Syncthing, Nextcloud, iCloud) – the vault stays encrypted

Design and scope: [Docs/superpowers/specs/2026-11-15-fernjo-design.md](Docs/superpowers/specs/2026-11-15-fernjo-design.md)

## SSH

Double-click a connection (or press Enter on it) to open it as a tab; the context menu has "Open another session" for a second tab. Sign-in follows OpenSSH's order: SSH agent, then the key or password of the credential the connection references in the vault, then keyboard-interactive, then a prompt in the tab. Everything the server asks (host key, password, passphrase, one-time code) is asked inside that tab, never in a popup.

- **Host keys:** fernjo keeps its own `known_hosts` file in the data folder (OpenSSH format, hashed entries are read). A new host asks for confirmation once ("Trust and connect" remembers it, "Trust once" does not); a key that differs from the stored one is blocked with a warning and only replaced after an explicit "Replace key and connect".
- **Agent:** `SSH_AUTH_SOCK` on Linux and macOS, the OpenSSH named pipe (`\\.\pipe\openssh-ssh-agent`) on Windows. Pageant is not supported yet.
- **Terminal:** xterm.js with search and clickable links; font family, size and scrollback are set in Settings (gear in the sidebar). A dropped connection keeps the screen, shows a struck lamp and a Reconnect button.
- Secrets never leave the Rust side except for the explicit "reveal" in the vault.

### Keyboard shortcuts

Plain Ctrl+letter and Alt+letter always go to the shell. App shortcuts are Ctrl+Shift+letter on Windows and Linux and Cmd+letter on macOS (there, Ctrl+Shift+letter stays with the terminal).

| Action | Windows / Linux | macOS |
|---|---|---|
| Next tab | Ctrl+Tab | Ctrl+Tab |
| Previous tab | Ctrl+Shift+Tab | Ctrl+Shift+Tab |
| Close tab | Ctrl+Shift+W | Cmd+W |
| Focus the connection filter | Ctrl+Shift+K | Cmd+K |
| Search in the terminal | Ctrl+Shift+F | Cmd+F |
| Copy selection (terminal) | Ctrl+Shift+C | Cmd+C |
| Paste (terminal) | Ctrl+Shift+V | Cmd+V |

Copy and paste only apply while the terminal has focus; elsewhere the page keeps its normal behaviour.

## Setup

Prerequisites: Rust (stable, version pinned in `rust-toolchain.toml`), Node.js LTS with pnpm, the [Tauri 2 system prerequisites](https://tauri.app/start/prerequisites/) for your OS, `cargo-deny` (`cargo install cargo-deny --locked`), and the .NET 10 SDK for the roadmap tool.

```bash
pnpm install
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
