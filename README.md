# fernjo

A lightweight, cross-platform remote connection manager for **SSH and RDP** on Windows, macOS and Linux, built on Rust.

All your connections live in one folder tree in the sidebar, sessions open as tabs in a single window, and passwords and keys are kept in an encrypted vault. Existing connections can be imported from mRemoteNG, including saved passwords.

**Status:** in development – see [Docs/ROADMAP.md](Docs/ROADMAP.md). The latest release, v0.2.0, is published: SSH and RDP sessions, the vault, import and backup are implemented, and the installers for Windows, macOS and Linux are on the [releases page](https://github.com/benjo-code/fernjo/releases).

## Features

Working now:

- Connection tree with nested folders; folders pass settings such as username, credential or port down to their connections
- SSH sessions in tabs with a full terminal (xterm.js), agent/key/password/keyboard-interactive login and host key verification
- RDP sessions in tabs (IronRDP): TLS and network level authentication, certificate pinning, a canvas that follows the tab size (or a fixed resolution), keyboard and mouse, text clipboard both ways and an in-app full screen. Tried by the owner from Windows, macOS and Linux against Windows 11 hosts with development builds (`pnpm tauri dev`), not against Windows Server, and not yet in an installed release build; the clipboard depends on what each web view allows. Not supported: drive, printer and audio redirection, gateways and Kerberos. Known limit: a host whose certificate the TLS library (rustls) cannot use fails with "the server certificate is not supported" before any prompt: an RSA key under 2048 or over 8192 bits, a certificate that is not version 3, one with an unknown extension marked critical, or one that is not valid DER. A key type other than RSA, ECDSA P-256/P-384 or Ed25519 cannot connect either; usually the server then finds no common signature scheme and the error is the general "the secure connection to the server failed". Self-signed certificates, certificates signed with SHA-1 and certificates for another name or past their expiry date are fine (fernjo pins the certificate instead).
- Encrypted credential vault (Argon2id + XChaCha20-Poly1305), optional unlock via the OS keychain
- Import from mRemoteNG (`confCons.xml` 2.6, including saved passwords), export of the connections as JSON and an encrypted full backup with restore (formats in [Docs/backup-format.md](Docs/backup-format.md))
- Data folder you can sync yourself (Syncthing, Nextcloud, iCloud) – the vault stays encrypted
- Quick connect: type `user@host[:port]` (or `rdp://user@host`) under the filter and open a session without saving a connection
- Tabs of the last run come back after a restart as disconnected tabs (Reconnect or "Reconnect all"; switch off in Settings)
- Optional auto-lock of the vault after an idle time (Settings)
- Light and dark theme, following the system or set by hand

Design and scope: [Docs/superpowers/specs/2026-11-15-fernjo-design.md](Docs/superpowers/specs/2026-11-15-fernjo-design.md)

## Install

Each GitHub release holds five installers and a checksum file:

| Platform | File |
|---|---|
| Windows (x64) | `fernjo_<version>_x64-setup.exe` |
| macOS, Apple Silicon | `fernjo_<version>_aarch64.dmg` |
| macOS, Intel | `fernjo_<version>_x64.dmg` |
| Linux (x64) | `fernjo_<version>_amd64.deb` and `fernjo_<version>_amd64.AppImage` |
| All | `SHA256SUMS` |

The installers are not code-signed (and the macOS images are not notarised), so Windows and macOS warn on the first start; the steps below get past that. Check a download against `SHA256SUMS` first:

```bash
sha256sum -c --ignore-missing SHA256SUMS   # Linux (GNU coreutils)
shasum -a 256 <file>                       # macOS: compare the printed hash with the line for <file> in SHA256SUMS
```

```powershell
(Get-FileHash .\fernjo_<version>_x64-setup.exe -Algorithm SHA256).Hash   # compare with the line in SHA256SUMS
```

`SHA256SUMS` sits in the same release as the installers, so it only shows that the download is intact. To check where an installer came from, verify its build provenance with the [GitHub CLI](https://cli.github.com/):

```bash
gh attestation verify <installer file> --repo benjo-code/fernjo
```

A passing check proves that the file was built by this repository's release workflow from the tagged commit. Releases before v0.3.0 have no attestation.

**Windows.** Run the setup. SmartScreen shows "Windows protected your PC": choose "More info", then "Run anyway". The install is per user (under `%LOCALAPPDATA%\fernjo`) and needs no administrator rights. Uninstalling keeps your data in `%APPDATA%\fernjo`.

**macOS** (the flow below, from the quarantine flag through Open Anyway to the running app, was reported to work by the project owner on an Intel Mac (macOS 15) on 2026-11-18, so with the Intel image; no per-step details. The Apple Silicon image has not been started on a real Apple Silicon Mac). Open the `.dmg` and drag fernjo to Applications. The first start is blocked because the app is unsigned and not notarised: open System Settings, Privacy & Security, scroll to the message about fernjo and choose "Open Anyway". Alternatively remove the quarantine flag once: `xattr -dr com.apple.quarantine /Applications/fernjo.app`. Use the Apple Silicon image on M-series Macs and the Intel image otherwise; macOS 11 or newer.

**Linux** (Wayland on recent GNOME and KDE is the target setup; Ubuntu 22.04 or newer). Debian and Ubuntu: `sudo apt install ./fernjo_<version>_amd64.deb`. AppImage: `chmod +x fernjo_<version>_amd64.AppImage` and run it; it needs FUSE 2, which is the package `libfuse2` on Ubuntu 22.04 and `libfuse2t64` on 24.04. The CI release build installs the `.deb` and extracts the AppImage (`--appimage-extract`); the AppImage was reported by the project owner to start and work on a CachyOS (Arch) laptop under Wayland on 2026-11-18, without extra environment variables (details not itemised; the desktop environment and the FUSE package were not stated). The `.deb` on a real Debian or Ubuntu desktop, other distributions, and the FUSE note (from documented behaviour) are not checked. The title bar icon needs a desktop entry: the `.deb` installs one; an AppImage has none until something integrates it, so fernjo offers to add one at its start (to `~/.local/share`, only if you agree; Gear Lever or AppImageLauncher do the same).

## Settings

The gear in the sidebar opens Settings: the colour theme (System, Light or Dark; System follows the operating system and changes with it), the terminal (font family, size, scrollback and whether a tab closes when the shell exits) and the data folder (see [Docs/DECISIONS.md](Docs/DECISIONS.md) for how the folder is chosen). The choices are stored in `settings.json` in the config folder. By default that is also the data folder; once you choose a separate data folder, `settings.json` stays in the config folder, so syncing the data folder does not carry the settings to another machine.

## SSH

Double-click a connection (or press Enter on it) to open it as a tab; the context menu has "Open another session" for a second tab. Sign-in follows OpenSSH's order: SSH agent, then the key or password of the credential the connection references in the vault, then keyboard-interactive, then a prompt in the tab. Everything the server asks (host key, password, passphrase, one-time code) is asked inside that tab, never in a popup.

- **Host keys:** fernjo keeps its own `known_hosts` file in the data folder (OpenSSH format, hashed entries are read). A new host asks for confirmation once ("Trust and connect" remembers it, "Trust once" does not); a key that differs from the stored one is blocked with a warning and only replaced after an explicit "Replace key and connect". Because a synced data folder can be written by others, a host that is already in `known_hosts` but that this device has not confirmed itself also asks once per device ("Confirm and connect"); these confirmations are kept outside the synced folder.
- **Agent:** `SSH_AUTH_SOCK` on Linux and macOS, the OpenSSH named pipe (`\\.\pipe\openssh-ssh-agent`) on Windows. Pageant is supported on Windows too and is tried after the OpenSSH pipe (when the pipe is unavailable or holds no keys); if both hold keys, only the OpenSSH agent is used.
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
| Quick connect | Ctrl+Shift+N | Cmd+N |
| Search in the terminal | Ctrl+Shift+F | Cmd+F |
| Copy selection (terminal) | Ctrl+Shift+C | Cmd+C |
| Paste (terminal) | Ctrl+Shift+V | Cmd+V |
| Remote desktop full screen (in and out) | Ctrl+Shift+F11 | Ctrl+Shift+F11 |
| Select several connections (tree) | Ctrl+click | Cmd+click |
| Select a range (tree) | Shift+click | Shift+click |

Copy and paste only apply while the terminal has focus; elsewhere the page keeps its normal behaviour. In a remote desktop (RDP tab) only full screen and Ctrl+Tab belong to fernjo; every other key goes to the remote machine.

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

Please report vulnerabilities privately, see [SECURITY.md](SECURITY.md).

**Privacy.** fernjo's own code makes no network connections except the SSH and RDP sessions you open. It has no update check, no telemetry and no analytics. Your connections, settings and encrypted vault stay in your data folder on your computer. The window itself is drawn by your operating system's web view (WebView2 on Windows, WebKit on macOS and Linux), a separate component that is outside fernjo's code. On Windows we measured what that component does while fernjo sits idle (WebView2 runtime 157.0.4341.60 on Windows 11, every process that fernjo starts watched with `Get-NetTCPConnection`): in three runs of 100 to 180 s, about 62 s after start the WebView2 network process opened one HTTPS connection to an address that belongs to `edge.microsoft.com`, and a network log showed it was the runtime's component updater (a POST to `edge.microsoft.com/componentupdater/api/v1/update`). fernjo switches that updater off for its own window (`--disable-component-update`); with the switch no connection appeared in four runs of 100 to 180 s, the last one with the setting built into the release binary. This covers one machine, one runtime version and a few idle minutes: the Windows web view may still contact Microsoft for other reasons (for example crash reports), and keeping the installed WebView2 runtime itself up to date stays with Windows. The web views on macOS and Linux may make background connections of their own; they were not checked. Notes on the synced data folder are in [SECURITY.md](SECURITY.md).

## License

Licensed under either of

- Apache License, Version 2.0 (`LICENSE-APACHE`)
- MIT license (`LICENSE-MIT`)

at your option. Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
