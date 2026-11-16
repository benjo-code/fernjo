# Changelog

All notable changes to this project are documented here.

Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), versioning according to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Every release entry names the milestones it contains.

## [Unreleased]

### Added

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
- Double-clicking a folder in the tree expands or collapses it (M4).

### Changed

- The gear dialog is now called "Settings" and also holds the terminal section (M4).

### Fixed

### Removed
