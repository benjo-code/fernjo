# Changelog

All notable changes to this project are documented here.

Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), versioning according to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Every release entry names the milestones it contains.

## [Unreleased]

## [0.1.1] - 2026-11-20

Polish and hardening: milestone M7 (no silent loss of synced data, quit confirmation, tabs that close on a deliberate shell exit, resizable panels, multi-select in the tree, a UI whose vault, import and settings can be found without a hint, and the Low findings of the M6 reviews settled).

### Added

- Data safety (M7): fernjo no longer overwrites a `connections.json` that a sync tool or another device changed while the app was running. The save is refused, a bar above the tree says so and offers **Reload**, which loads the other version (your edit that was refused is not applied; make it again after the reload). If the changed file cannot be read, the tree you see stays and the error is shown. It is a check before the write, not a lock, so a sync tool can still win a race of a few milliseconds.
- Quit confirmation (M7): closing the window while at least one session is connected asks "Close fernjo?" with "Close" and "Keep open"; with no connected session (or only connecting or disconnected tabs) the window closes at once. On macOS, Cmd+Q still quits without asking.
- Close on exit (M7): a tab closes by itself when the remote shell ends on purpose (`exit`, `logout` or Ctrl+D, when the shell reports exit status 0 after at least five seconds of connection). A non-zero exit status, a lost connection, a failed login or a session that ends right after it starts keeps the tab with Reconnect. Settings has a checkbox "Close the tab when the shell exits" (on by default).
- Resizable panels (M7): drag the right edge of the sidebar or the top edge of the properties panel, or focus the separator and use the arrow keys (Shift for bigger steps, Home and End for the limits); double-click resets. The sizes are remembered on this device, and the tree always keeps its rows.
- Multi-select in the connection tree (M7): Ctrl or Cmd+click toggles a row, Shift+click or Shift+arrow selects a range, and dragging a selected row moves the whole selection in one step. Deleting several asks first and says how many connections and folders go; the context menu on a selection offers Delete only, and the properties panel shows how many items are selected.
- Discoverability (M7, from a design review of the whole app, `Docs/design/m07-discoverability-review.md`):
  - The vault is named where you look for it: the sidebar chip reads "Vault: unlocked", "Vault: locked" or "Vault: none", the manager is titled "Vault", and the no-vault note is a slim one-line row with a "Create vault" button; a tab that waits for the vault now says "Unlock vault" (it said "Needs vault").
  - A connection's Credential row has "Create vault", "Unlock" or "Manage…" next to the value and a last entry "New credential…" in its dropdown; the row sits right under Username so it is visible without scrolling.
  - The sidebar footer buttons carry text ("Settings", "Import / Export") and the dialog is called "Import, export & backup".
  - An empty tree shows a numbered start list (add a connection, import from mRemoteNG, optionally create a vault); with connections the main area says "Double-click a connection (or press Enter) to open it."
  - Settings says what is applied live and what "Save terminal settings" stores, and lists the keyboard and mouse shortcuts for your platform; the context menu shows the key hints (Enter, F2, Del).
  - A dropped tab keeps its title and shows the reason ("The connection was lost.", "The remote shell ended (exit status 3).") in a bar above the terminal with Reconnect.
  - The host key prompt explains what each choice does.
- Security hardening (M7, from the Low findings of the M6 reviews, triaged in `Docs/DECISIONS.md`):
  - Vault: the Argon2 cost read from a vault file or backup header is bounded to at most four times the default per setting (memory, passes and lanes: about 16 times the default's work, roughly 40% below the old 1 GiB x 20 passes) and anything above is refused as unreadable, so a hostile synced file cannot make every unlock attempt take many seconds; `vault.fernjo` and `known_hosts` are written through a random temporary file name created with `create_new` (no fixed name that a planted symlink could redirect) and, on Unix, the directory is synced afterwards; changing the master password raises a weaker stored cost to the current default; the Windows keychain item is stored as `Local` (not roamed between machines) when it is newly written; an existing item switches the next time the key is stored (untick and tick "Unlock automatically on this device" in the vault manager).
  - Import: long and control characters in names, hosts and descriptions of JSON and backup imports are cleaned like mRemoteNG imports; text from a backup header is cut before it reaches the window; an export may not target the configuration folder, the device-local folder or a pending data folder; a reload of the page during a slow import no longer freezes the window; a notice appears when `settings.json` points at a data folder that contains the device-local folder.

### Changed

- The SSH agent's sign step has its own limit of 120 seconds (M7), so an agent that waits for a touch or a confirmation per signature (`ssh-add -c`) no longer runs into the 15 second connect timeout.
- The Settings dialog names the real reason when settings are not saved (M7): `FERNJO_DATA_DIR` is set, `settings.json` is unreadable or from a newer version, or the device has no settings folder. Before, it always blamed `FERNJO_DATA_DIR`.
- The Windows web view no longer runs its component updater (M7): about a minute after start it contacted `edge.microsoft.com` once while fernjo sat idle; `--disable-component-update` stops that (measured, see `Docs/DECISIONS.md`)
- Release binaries no longer embed the build machine's cargo registry paths (M7): the release workflow remaps the paths and fails when the built binary still contains one. The release workflow checks this on every release build; the dry run of pull request #16 passed that check on Windows, macOS (both architectures) and Linux.

### Fixed

- An oversize `connections.json` or `settings.json` (over 64 MiB) is refused as unreadable instead of being read whole (M7).
- A `known_hosts` line that lists several hosts (`a,b ssh-ed25519 ...`) no longer keeps the old key trusted for a host whose key you replace, and "trust always" for a host that presented a different key of the same type replaces the stored one (M7).
- A hostile keyboard-interactive prompt from a server is shortened (at most 16 prompts, long texts cut) before it is shown (M7).
- Linux (M7): the default window icon is now also applied explicitly after the window is created, as an attempt to remove the placeholder icon in the title bar under Wayland. Unconfirmed until it is looked at on a Linux desktop.

## [0.1.0] - 2026-11-18

First release, the "Daily SSH driver": milestones M0 to M6 (project foundation, design, connection tree, credential vault, SSH sessions in tabs, mRemoteNG import with export and backup, packaging and security fixes).

### Added

- Project foundation (M0): Rust workspace, Tauri 2 shell with an empty window, Svelte 5 frontend, CI on GitHub-hosted runners (build, tests, lint, `cargo deny`, CodeQL), dual MIT/Apache-2.0 licence.
- Visual design "Ulm, revised": light and dark theme that follows the system setting, bundled fonts (Hanken Grotesk, Red Hat Mono), design tokens for all upcoming UI (M1).
- New app icon (M1).
- Connection tree (M2): folders and SSH/RDP connections in a sidebar tree with filter, expand/collapse, keyboard navigation, rename, duplicate, delete with confirmation, context menu and drag and drop; the tree is saved to `connections.json` in the app's data folder (atomic write with a `.bak` backup).
- Inherited settings (M2): username, port and keepalive can be set on a folder and are inherited by everything below it; the properties panel shows "from <folder>" and an "Inherit" reset.
- Single instance (M2): starting fernjo a second time brings the running window to the front.
- Credential vault (M3): passwords and private keys (with optional passphrase) are stored in an encrypted vault file (`vault.fernjo`: Argon2id, XChaCha20-Poly1305, wrapped random data key; format in `Docs/vault-format.md`). Create it with a master password, lock and unlock it, change the master password, and manage entries in the vault manager (list, create, edit, explicit reveal that hides again after 10 seconds, delete with a usage warning). Optionally unlock automatically on this device through the OS keychain (Windows Credential Manager, macOS Keychain, Secret Service).
- Credential reference (M3): a connection or folder can reference a vault entry in the properties panel; the reference is inherited like username and port ("from <folder>"). `connections.json` stores only the entry's id (schema version 2; builds from before M3 refuse the file instead of dropping the reference).
- Data folder (M3): the data folder can be changed from the gear button in the sidebar; the data is copied to the chosen folder and the change applies after a restart. `FERNJO_DATA_DIR` still overrides it.
- Sync-conflict warning (M3): copies such as `connections.sync-conflict-…json` left by Syncthing, Nextcloud or Dropbox in the data folder are reported in a notice.
- Vault safety (M3): the vault refuses to overwrite a `vault.fernjo` that a sync tool or another device changed after it was unlocked ("Lock and unlock the vault to load the changes"); replacing a private key keeps its passphrase unless you tick "Remove stored passphrase"; unticking "Unlock automatically on this device" while unlocking turns it off; the lock note is compact until you ask for the password form; an unusable vault names `vault.fernjo.bak` for a manual restore. Changing the master password does not change older copies of the vault file (backups, sync history): they keep opening with the previous password.
- SSH sessions in tabs (M4): double-click a connection to open it as a terminal tab (xterm.js, WebGL renderer for the visible tab, search, clickable links, copy and paste). Tabs show connecting, connected and disconnected states with a lamp, can be reordered, closed (with a confirmation while connected), opened a second time as another session and reconnected after a dropped connection; the screen is kept.
- SSH sign-in (M4): SSH agent, then key (Ed25519, RSA, encrypted keys with passphrase), password, keyboard-interactive, then an inline prompt in the tab. A locked vault parks the tab until you unlock it.
- Host key verification (M4): fernjo's own OpenSSH-format `known_hosts` in the data folder; a first-contact fingerprint prompt (trust always or once) and a blocking warning when a stored key changes.
- Terminal settings (M4): font family, font size and scrollback in the Settings dialog (the gear in the sidebar), applied live and stored in `settings.json`.
- App shortcuts (M4): tab switching, close tab, filter, terminal search, copy and paste that leave the terminal's own keys alone; listed in the README.
- Import and backup (M5): import connections and passwords from mRemoteNG (confCons.xml, version 2.6); export connections as JSON; encrypted full backup and restore (formats in `Docs/backup-format.md`).
- Double-clicking a folder in the tree expands or collapses it (M4).
- Theme setting (M6): Settings has an Appearance section with System, Light and Dark; the choice is stored in `settings.json` and applied from a copy kept in the web view as soon as the page script runs, so a stored theme normally does not flash the other one at start (without that copy, on a first start, it follows the system theme until the settings have loaded).
- Installers (M6): the release workflow builds a Windows setup (`fernjo_<version>_x64-setup.exe`), macOS images for Apple Silicon and Intel (`_aarch64.dmg`, `_x64.dmg`) and a Linux `.deb` and AppImage (`_amd64`) on GitHub-hosted runners and attaches them to a published release. Each installer is smoke-tested on its runner first. The installers are not code-signed; README, section "Install", explains the Windows and macOS warnings.
- `SHA256SUMS` (M6): every release carries a checksum file for all installers.
- Third-party notices (M6): the licences of the bundled Rust crates, npm packages and the two fonts ship inside every installer (`THIRD-PARTY-NOTICES.md`).
- Per-user Windows install (M6): the setup installs under `%LOCALAPPDATA%\fernjo` without administrator rights; uninstalling keeps `%APPDATA%\fernjo`.
- Security hardening (M6, from the pre-release security review in `Docs/release-checklist.md`):
  - On Unix the data files (`connections.json`, `settings.json`, `vault.fernjo`, `known_hosts` and their `.bak` copies) are created with mode 0600 and a data folder fernjo creates with 0700; before, they followed the default umask. Windows ACLs are unchanged. The new tests ran green in CI on Linux (run 44218999770); macOS was not run.
  - Host names that cannot be written safely to `known_hosts` (separators, whitespace, control characters) are refused before any traffic with the error `invalid_host`.
  - An oversize `vault.fernjo` (over 64 MiB) is refused as unusable instead of being read whole, and is never overwritten.
  - Importing a hostile mRemoteNG file is bounded: too many attributes or namespace declarations are refused (`too_complex`), long names are shortened, over-long host names are dropped and over-long password attributes are not decrypted.
  - The folder depth limit is now enforced on every create, move and duplicate (`too_deep`), not only on import and restore, so a tree can no longer grow past what `connections.json` can be loaded from.
  - The main window refuses to navigate away from the app's own origin, so a remote page can no longer be loaded into it and never reaches the app's commands. On Windows the web view (WebView2) still sends one request for a cancelled top-level navigation and discards the answer; that is not prevented.
  - Hosts are confirmed per device: a host key that is in the synced `known_hosts` but that this device has not confirmed asks once ("Confirm and connect", "Connect once" or "Cancel") before any credential is sent. After this update every existing host asks once per device. Confirmations are kept in a device-local folder outside the synced data folder (`<OS local data dir>/fernjo-device`, `%LOCALAPPDATA%\fernjo-device` on Windows), which is created on the first save; choosing a data folder that contains it or lies inside it is refused.
  - Rollback protection for the vault: each vault file carries a generation counter and each device remembers the highest one it opened. An older `vault.fernjo` is refused after the master password proved genuine ("older than the version this device last opened"); automatic unlock never accepts it, and "Open the older vault" opens it anyway and re-stamps it. Not covered: the first open on a new device, two histories with the same generation, and devices that also run an older build (they may see repeated prompts). Existing vaults open unchanged; the first save adds the counter.
  - Automatic unlock never deletes the stored key from the OS keychain any more, and the lock note says when it did not work (damaged or incomplete vault file, or an older file). Unticking "Unlock automatically on this device" still removes the key.

### Changed

- SSH: the plain `ssh-rsa` host key algorithm (RSA over SHA-1) is no longer offered, like OpenSSH since 8.8; a server that only has a SHA-1 RSA host key can no longer be connected to (M6).
- Dependencies: russh updated to 0.65.2 (with `rsa` 0.10), which removes the `RUSTSEC-2023-0071` advisory ignore (M6).
- The gear dialog is now called "Settings" and also holds the terminal section (M4).
- Release builds strip symbols and link with thin LTO (M6).

### Fixed

- Settings dialog (M6): a late-arriving load no longer overwrites what you already typed; the "Saved" note disappears when you edit a terminal field again; a failed load of the terminal settings shows an error and keeps Save disabled; Enter in a terminal field no longer saves while an input method composition is active.
- Windows installer (M6): `fernjo_<version>_x64-setup.exe` now carries the fernjo app icon instead of the generic NSIS icon.
