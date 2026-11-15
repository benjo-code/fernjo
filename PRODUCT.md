# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

(Desktop app: a Tauri 2 webview on Windows (WebView2), macOS (WKWebView) and Linux (WebKitGTK 4.1, Wayland on GNOME/KDE). No mobile, no browser deployment.)

## Users

One primary user: the owner, a developer and admin who manages homelab and work servers. They use fernjo every day on Windows, macOS and Linux, mostly for SSH, sometimes RDP. Their tree holds about 50–150 connections in nested folders, and 3–6 sessions are usually open at once, often for hours. A wider audience is possible later but does not drive decisions now.

## Product Purpose

fernjo is a cross-platform SSH and RDP connection manager in the spirit of mRemoteNG. It has a connection tree in a sidebar, sessions as tabs in one window, settings inherited from folders, an encrypted credential vault and mRemoteNG import. It exists because mRemoteNG is Windows-only and the Linux/macOS alternatives organise connections poorly (Remmina opens a window per connection).

Success: the owner replaces mRemoteNG with fernjo for daily work on every machine, starting by importing the existing mRemoteNG connection file.

## Positioning

The mRemoteNG way of working (one window, folder tree with inheritance, tabs) on all three desktop platforms, with a cleaner, modern UI, a Rust core and secrets that never leave Rust. It is a lightweight alternative to mRemoteNG, MobaXterm and Remote Desktop Manager, not a team product.

## Operating Context

- Long working sessions: the window stays open all day next to other tools, and terminal tabs stay connected for hours.
- Screens range from laptop panels at 125–200 % scaling to large external monitors.
- Daily loop: filter or browse the tree → open a connection (double-click/Enter) → work in the terminal → switch between a few tabs → reconnect dropped sessions.
- The vault is unlocked by master password or the OS keychain; the tree stays browsable while the vault is locked.
- Data lives in a folder the user may sync with Syncthing/Nextcloud/iCloud.

## Capabilities and Constraints

- Planned: connection tree (folders, connections, unlimited nesting, manual order, drag and drop, filter on name/host/description) and a properties panel showing local vs inherited values (M2). Credential vault with master password and keychain auto-unlock (M3). SSH terminal tabs with xterm.js (M4). mRemoteNG import and backup export (M5). Installers and a settings dialog with theme light/dark/system (M6). RDP tabs (M7).
- Out of scope for now: built-in sync, SFTP, port forwarding, jump hosts, local shell tabs, split panes, other protocols, team features, code signing.
- Technical: Tauri 2 + Svelte 5 + TypeScript; all logic in Rust, TypeScript for presentation only. Strict CSP, so no remote fonts or assets at runtime. Must start fast (cold start < 2 s) and stay light (idle < ~150 MB).
- Secrets (passwords, keys, passphrases) never reach the webview, except through an explicit "reveal" action in the vault UI.
- UI language: English. Theme: light, dark, follow system.
- Terminology: connection, folder, session, tab, vault, credential, inherited.

## Brand Commitments

- Name: **fernjo**, always lowercase. "fern" comes from the German *Fernverbindung* (remote connection), "jo" from the owner's developer name "benjo". The name is **not** a plant reference. *(Inference: fern-plant imagery would misread the name.)*
- No logo, colours or other existing assets to preserve; the visual identity is designed in milestone 1.

## Evidence on Hand

- No screenshots, users, testimonials or benchmarks exist. Mockups use synthetic data only: example hostnames and RFC 5737 IP ranges, never the owner's real environment.
- The owner's real `confCons.xml` arrives as a sanitised fixture in M5. It is not for design use.

## Product Principles

1. **One window, everything in reach.** Connections never scatter into separate windows; the tree and the tabs are the whole workspace.
2. **The terminal wins.** Session content gets the space and the keys; chrome stays out of the way of hours of terminal work.
3. **Structure you can trust.** The tree, inheritance and session states are always legible: where a value comes from, which session is live, which one dropped.
4. **Secrets stay in Rust.** The UI shows names and IDs, never secret values, except on an explicit reveal.
5. **Light and fast.** Nothing in the UI may cost start time or idle memory without a reason.

## Accessibility & Inclusion

- Eye comfort over long sessions: low glare, calm surfaces, text that stays readable for hours, in light and dark.
- Must hold up from laptop screens at 125–200 % scaling to large monitors; the minimum window is 900×600.
- WCAG AA contrast for text and essential UI in both themes, including the terminal palette.
