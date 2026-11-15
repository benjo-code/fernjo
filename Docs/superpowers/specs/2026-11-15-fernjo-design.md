# fernjo – Design

**Status:** approved by the owner on 2026-11-15 · **Date:** 2026-11-15 · **Roadmap:** [../../ROADMAP.md](../../ROADMAP.md)

## 1. Intent

fernjo is a cross-platform (Windows, macOS, Linux) remote connection manager for **SSH and RDP**, built on Rust. It is a lightweight alternative to mRemoteNG, MobaXterm and Remote Desktop Manager.

**Why:** The owner uses mRemoteNG and likes its structure, but it is Windows-only. Linux and macOS alternatives (e.g. Remmina) organise connections poorly: Remmina opens a new window per connection. fernjo brings the mRemoteNG way of working to all three platforms with a cleaner, modern UI.

**Primary user:** the owner, personally, on all three systems. SSH is used more than RDP. A wider audience is possible later but does not drive decisions now.

**Success:** the owner replaces mRemoteNG with fernjo for daily work on every machine, starting by importing the existing mRemoteNG connection file.

## 2. Goals, non-goals, anti-goals

**Goals**
- One main window: connection tree in a sidebar, sessions as tabs in the main area.
- Folder/tree organisation with settings inherited from folders (mRemoteNG-style).
- SSH sessions with a high-quality terminal; RDP sessions in tabs.
- Encrypted credential vault, unlocked by a master password, optionally remembered via the OS keychain.
- Faithful import from mRemoteNG (`confCons.xml`, including saved passwords); export for backup and migration.
- Native installers for Windows, macOS and Linux.

**Non-goals (for now, may move to the backlog)**
- Built-in device sync (users sync the data folder themselves, see §5.4).
- Protocols other than SSH and RDP (VNC, Telnet, serial, HTTP).
- SFTP file browser, port forwarding UI, jump hosts, local shell tabs, split panes.
- Team/multi-user features, a server component, a plugin system, mobile apps.
- Code signing, notarisation and auto-update.

**Anti-goals (failure even if it "works")**
- A secret ever lands in plaintext on disk, in logs, in the webview's storage, or in the connections file.
- Connections open in separate windows by default (the Remmina problem).
- Heavy, slow app: it must start fast and stay light at idle.
- Lock-in: the data format is documented and exportable.

## 3. User experience

```
┌──────────────────────┬──────────────────────────────────────────┐
│ 🔍 filter…           │ [web-01 ×] [db-02 ×] [DC01 (RDP) ×]      │
│ ▾ Homelab            ├──────────────────────────────────────────┤
│   ▾ Proxmox          │                                          │
│      ● pve-01  SSH   │   user@web-01:~$ _                       │
│      ○ pve-02  SSH   │                                          │
│   ▸ Docker hosts     │        (SSH terminal or RDP desktop      │
│ ▾ Work               │         renders here)                    │
│   ▸ Windows servers  │                                          │
│      ● DC01    RDP   │                                          │
├──────────────────────┤                                          │
│ Properties: web-01   │                                          │
│ host · port · user…  │                                          │
└──────────────────────┴──────────────────────────────────────────┘
```

- **Sidebar:** filter box (matches name, host, description), tree of folders and connections (unlimited nesting, manual order, drag and drop to move/reorder), indicator for open connections, context menu (new folder/connection, duplicate, rename, delete, connect).
- **Properties panel** below the tree for the selected node; every inheritable field shows whether it is set locally or inherited from which folder, and can be reset to "inherit".
- **Tabs:** double-click (or Enter) opens a connection as a tab. Opening an already-open connection focuses its tab; "Open another session" in the context menu opens a second one. Tabs show protocol and state (connecting / connected / disconnected) and offer reconnect when disconnected. Tabs can be reordered. Detaching a tab into its own window is backlog.
- **Keyboard:** app shortcuts (next/previous tab, close tab, focus filter) must not steal keys the terminal needs; terminal input wins inside a focused terminal except for a small, documented set of app shortcuts.
- **Theme:** light, dark, follow system. UI language: English.

### Visual design direction (dedicated milestone, owner's request)
The look is decided before any real UI is built. In its own milestone, at least **three distinct design concepts** are created with the `impeccable` skill. Each concept is a static mockup of the main window (tree with folders, tabs, an SSH terminal, properties panel) plus the vault unlock dialog, in light and dark. All concepts are published together as **one Claude artifact with a switcher** so the owner can flip between them. The owner picks one, possibly combining parts of several. The result is recorded as `DESIGN.md` (principles, typography, colour, spacing, components) plus design tokens as CSS variables in `ui/`, and every later UI milestone builds on it.

## 4. Architecture

**Approach chosen:** Tauri 2 shell with a Rust core and a web UI (Svelte 5 + TypeScript), terminal via xterm.js. See §8 for the alternatives considered.

```
Cargo workspace
├── crates/fernjo-core     data model, tree, inheritance, persistence (no UI, no I/O to network)
├── crates/fernjo-vault    encrypted credential vault, OS keychain integration
├── crates/fernjo-ssh      SSH sessions (russh + tokio)
├── crates/fernjo-rdp      RDP sessions (IronRDP)            ← added in the RDP milestone
├── crates/fernjo-import   mRemoteNG import, fernjo export
└── src-tauri              Tauri app: commands, events, session manager, wiring
ui/                        Svelte 5 + TypeScript frontend (Vite), xterm.js
```

Each crate is created in the milestone that needs it, not up front.

**Rules**
- **Secrets stay in Rust.** The frontend receives connection metadata and credential IDs/names only, never passwords, private keys or passphrases. Authentication happens in Rust. The only exception is an explicit "reveal" action in the vault UI.
- The protocol and vault crates do not depend on Tauri; they are usable and testable without the UI.
- Frontend ↔ Rust: Tauri commands for requests, Tauri channels for streaming session data (terminal bytes, RDP frame updates).
- Async runtime: tokio (Tauri's runtime).
- Rust edition 2024, stable toolchain pinned in `rust-toolchain.toml`. Node LTS with pnpm for the frontend.

## 5. Components

### 5.1 Connection model (`fernjo-core`)
- Tree of nodes, each with a UUID, name, description and manual sort order.
  - **Folder:** children plus a partial set of default settings.
  - **Connection:** protocol (`ssh` | `rdp`), host, plus a partial set of settings.
- **Inheritance:** every inheritable setting is optional on a node; the effective value is the first value found walking from the connection up through its folders, then the built-in default. Inheritable: username, credential reference, port, SSH settings (keepalive interval), RDP settings (domain, resolution/scaling, colour depth, clipboard on/off). Host and protocol are never inherited.
- **Persistence:** `connections.json` with a `schemaVersion`, human-readable, no secrets. Writes are atomic (write temp file, then rename) and the previous version is kept as `connections.json.bak`. Migration code handles older `schemaVersion`s.

### 5.2 Credential vault (`fernjo-vault`)
- **Entries:** id, name, username, optional domain, secret (password, or private key in OpenSSH/PEM format with optional passphrase), notes. Connections and folders reference entries by id; several connections can share one entry.
- **File:** `vault.fernjo`, whole-file authenticated encryption with a **wrapped data key**: a random 256-bit data key encrypts the entries; the header holds magic, format version, KDF parameters, salt and the data key wrapped by the key derived from the master password. KDF Argon2id, cipher XChaCha20-Poly1305 (for both wrapping and content), both from audited RustCrypto crates. No home-made crypto. Atomic writes as for connections. *(Builder call, from the review: keeps keychain entries on other devices valid when the master password changes on one synced device.)*
- **Unlock:** master password at app start (created on first run). Option "Unlock automatically on this device" stores the **data key** in the OS keychain (`keyring` crate: Windows Credential Manager, macOS Keychain, Secret Service on Linux). Manual "Lock" action. Secrets in memory are wrapped in `secrecy`/`zeroize` types.
- **Changing the master password** re-wraps the data key; the data key itself is not rotated, so the entries are unchanged (the file is re-saved, which re-seals the content under the same data key).
- Without an unlocked vault, the tree is still browsable; connecting with a stored credential prompts for unlock.

### 5.3 SSH (`fernjo-ssh`)
- russh client over tokio with an interactive PTY channel (`xterm-256color`), window resize, keepalive.
- **Auth:** password, public key (from vault or a key file path), SSH agent (OpenSSH agent: Unix socket on macOS/Linux, named pipe on Windows), keyboard-interactive (prompts shown in the UI, enough for 2FA codes). Order follows OpenSSH: agent → key from the credential → password from the credential → keyboard-interactive → ask for the password. Unavailable methods are skipped (no agent running, vault locked and the user cancels the unlock prompt). No per-node order list in v0.1. *(Builder call.)*
- **Host keys:** fernjo keeps its own `known_hosts` (OpenSSH format) in the data folder. First connection: dialog with the SHA-256 fingerprint (trust once / always / cancel). Changed key: blocking warning; connecting requires explicitly replacing the stored key.
- **Terminal (UI):** xterm.js with fit, WebGL renderer, search and web-links addons; configurable font family/size and scrollback; copy/paste via shortcuts and context menu.

### 5.4 Data folder and sync
- Default data folder: the OS config directory (`directories` crate), e.g. `%APPDATA%\fernjo`, `~/Library/Application Support/fernjo`, `~/.config/fernjo`.
- The user can choose another folder in settings. Because the vault is encrypted and the connections file holds no secrets, putting the folder in Syncthing/Nextcloud/iCloud gives end-to-end-encrypted sync without sync code in fernjo. Concurrent edits on two devices are not merged (last writer wins, `.bak` kept). Built-in sync is backlog.
- **Single instance:** a second launch focuses the running window (Tauri single-instance plugin). On start, fernjo looks for sync-conflict copies next to its files (e.g. Syncthing's `*.sync-conflict-*`) and shows a warning naming them; it does not merge them. *(Builder call.)*
- App settings (theme, terminal font, data folder location) live in `settings.json` in the OS config directory, never in the synced folder's vault.

### 5.5 Import and export (`fernjo-import`)
- **mRemoteNG import:** `confCons.xml` (current format: AES-GCM with PBKDF2-derived key; default key `mR3m`, or a user-supplied custom key; full-file encryption supported). Older CBC-based files only if cheap; otherwise reported as unsupported.
  - Containers → folders, `SSH2` → SSH, `RDP` → RDP; other protocols are skipped and listed.
  - mRemoteNG inheritance flags map to fernjo inheritance where an equivalent exists.
  - Passwords → vault entries; identical (username, domain, password) combinations become one shared entry.
  - Imported into a new top-level folder "Imported from mRemoteNG (YYYY-MM-DD)". A report lists imported, skipped and partially mapped items.
- **Export:** (a) connections only as fernjo JSON (no secrets); (b) full backup: connections + vault in one file encrypted with a separate export password. Restore/import of both formats.
- Export to mRemoteNG format is backlog.

### 5.6 RDP (`fernjo-rdp`)
- IronRDP client: TLS + NLA (CredSSP), credentials from vault, resolution "fit to tab" with dynamic resize where the server supports it (else fixed), keyboard and mouse input, text clipboard both ways, fullscreen toggle.
- **Rendering path:** IronRDP decodes into an RGBA framebuffer in Rust; changed regions are streamed over a Tauri channel to the UI and drawn on a canvas. **The milestone starts with a throwaway spike** measuring this path at 1920×1080 on all three OSes. Bar: at least 30 frames/s while scrolling a window or dragging, and keystroke-to-screen latency that is not noticeably worse than the OS's own RDP client on a LAN; final judgement is the owner's manual check. If the bar is missed, first optimise within the canvas path (WebGL upload, frame coalescing). Only if that fails is the native render surface (wgpu/softbuffer over the tab area) planned, as a **separate milestone** before v0.2.0, with the decision recorded in `DECISIONS.md`. Slow RDP is not acceptable for v0.2.0. *(Builder call.)*

### 5.7 Packaging
- Tauri bundler: Windows NSIS installer, macOS `.dmg` (Apple Silicon and Intel), Linux AppImage and `.deb`. Builds unsigned; first-launch workarounds documented in the README.
- Release builds run in GitHub Actions on GitHub-hosted runners for all three OSes when a release is created (see §7).

## 6. Quality

- **Rust:** `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace`.
- **UI:** ESLint, Prettier, `svelte-check`, Vitest for tree/inheritance display logic.
- **Tests that matter most:** inheritance resolution; vault round trip, wrong password, tampered file; mRemoteNG import against fixture files with known passwords (default and custom key); SSH integration test against an OpenSSH server in a container (Linux CI only; skipped locally when Docker is absent, never silently in CI).
- **Performance targets (manual checks):** cold start to usable window under 2 s on a typical dev machine; idle memory with no sessions under ~150 MB; tree with 500 connections filters without visible lag.

## 7. Constraints

- Rust is the foundation: all logic (model, vault, protocols, import) in Rust; TypeScript only for presentation.
- The roadmap tool needs the .NET 10 SDK (template requirement).
- **CI runs on GitHub-hosted runners** (owner's decision): `ubuntu-latest` for PR checks (Rust build/test/lint, UI lint/test, SSH integration test), plus `macos-latest` and `windows-latest` for the release builds. The org's self-hosted runner has no root and cannot install WebKitGTK; it is not used.
- **The repo goes public early** (in M0) so hosted-runner minutes are free (owner's decision). Consequences, all handled in M0:
  - Before flipping visibility, the history is scanned for secrets.
  - The `roadmap` job moves from `[self-hosted, dotnet-linux]` to `ubuntu-latest`. Self-hosted runners must never run workflows of a public repo, because fork PRs could execute code on them.
  - Repo security is switched on and watched: secret scanning with push protection, Dependabot alerts and security updates, CodeQL for Rust and JavaScript/TypeScript on pushes and PRs, private vulnerability reporting with a `SECURITY.md`, and Actions approval required for PRs from outside contributors. Workflows keep `permissions: contents: read` by default, and actions stay pinned to commit SHAs.
  - `cargo-deny` (advisories, licences, sources) runs in CI.
- **Licence:** `MIT OR Apache-2.0` (dual, the Rust ecosystem convention; owner asked for open source and left the choice to the builder). It is compatible with all planned dependencies, and Apache-2.0 adds a patent grant. *(Builder call; GPL-3.0 would be the alternative if forks must stay open.)*
- Licences of dependencies (russh Apache-2.0, IronRDP MIT/Apache-2.0, xterm.js MIT, Tauri MIT/Apache-2.0) are all permissive; `cargo-deny` enforces an allow-list.
- **Linux target:** Wayland sessions on recent GNOME/KDE distros are the tested setup (owner's daily environment); X11 is expected to work but is not regularly tested. Minimum for packages: Ubuntu 22.04 (WebKitGTK 4.1).
- **mRemoteNG:** the owner uses a recent version (1.76+, AES-GCM format) and will provide a sanitised `confCons.xml` as a fixture when the import milestone (M5) starts. Support for the older CBC format is therefore optional (backlog if not cheap).

## 8. Approaches considered

| Approach | Trade-off | Verdict |
|---|---|---|
| **A. Tauri 2 + Svelte/TS + xterm.js** | Best terminal for free (xterm.js powers VS Code), fast UI iteration; RDP frames must travel to a canvas, WebKitGTK on Linux is the weakest webview; UI code is TypeScript | **Chosen**: SSH is the main use and the motivation is structure/design |
| B. Native Rust UI (egui/Iced) + alacritty_terminal | All Rust, best RDP rendering; terminal widget immature, months of work on selection/scrollback/fonts; modern look is harder | Rejected |
| C. Slint | Nice native look, native RDP rendering; no terminal widget at all | Rejected |
| Electron | Heavy, non-Rust core | Rejected early |
| Qt bindings | Awkward Rust bindings, licensing friction | Rejected early |
| RDP via FreeRDP (C) or OS client | Mature, but C dependency / no client on macOS/Linux | Rejected in favour of IronRDP; FreeRDP stays a fallback if IronRDP fails the spike for protocol reasons |

## 9. Build order (→ roadmap milestones)

0. Repo scaffold & CI (workspace, Tauri shell with empty window, CI on GitHub-hosted runners, Dependabot, repo public with security features on)
1. UI design direction (≥3 concepts via `impeccable`, artifact with switcher, owner picks, `DESIGN.md` + tokens)
2. Connection tree & data model
3. Credential vault
4. SSH sessions in tabs
5. mRemoteNG import & export
6. Packaging & app settings → **release v0.1.0** (daily SSH driver)
7. RDP sessions → **release v0.2.0**

## 10. Left to the builder

Exact crate versions (recorded in `DECISIONS.md` when chosen), Argon2id parameters (target ~0.5 s unlock on a typical machine), Svelte component structure and state management, exact shortcut set, icon set, the exact `connections.json` field names.
