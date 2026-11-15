---
version: 1
slug: "ui-src-app-svelte"
primary_target: "ui/src/App.svelte"
related_targets: ["Docs/design/m01-concepts.html"]
---

# Surface brief: fernjo main window

## Scope and mode
The main application window of fernjo plus the vault unlock dialog. Mode: **Operate**. Build path: code-led (no image generation; Operate surfaces are code-led). Milestone 1 delivers static concepts (one artifact, switchable) and the chosen direction as tokens in `ui/`; the real components follow in M2–M4.

## Audience, job, task
The owner, all day, on Windows/macOS/Linux, 125–200 % laptop scaling up to large monitors. Typical setup: 50–150 connections, 3–6 open tabs. The morning starts by **browsing the tree** by structure (customer/site/role), not by searching. Then the owner opens sessions, works in terminals for hours, switches tabs and reconnects dropped sessions.

## Layout (fixed by product, carried over from mRemoteNG on the owner's request)
- One window with the native OS title bar (no custom title bar).
- **Sidebar left:** filter box, then the connection tree, then the **properties panel stacked under the tree**, always visible.
- **Right:** a **classic tab strip** (plain tabs, close button per tab) over the session area.
- Design size 1280×800; minimum 900×600.

## Density
- **Dense, compact tree:** row height 22–28 px, small icons, little padding; at least 25 rows visible at 1280×800 with the properties panel open; indentation legible over at least 4 levels.
- **Protocol icon per row** (SSH vs RDP readable at a glance, not by colour alone).

## Themes
Light and dark, both first-class; follow system by default (switch arrives in M6). Eye comfort over long sessions: low glare, calm surfaces.

## State inventory every concept must show
- **Tree:** at least 30 nodes over 3 levels; expanded and collapsed folders; SSH and RDP rows; open-session indicator; one selected row; one keyboard-focused row.
- **Tabs (4):** connected (active), connecting, disconnected with a reconnect affordance, one RDP tab.
- **Terminal:** a realistic SSH session (prompt, coloured `ls` output, an ANSI 16-colour sample line, cursor).
- **Properties panel:** the selected connection, with one locally set value plus a reset-to-inherit control, several values marked "inherited from <folder>", and the credential reference by **name only**.
- **Vault lock state:** visible somewhere in the chrome.
- **Unlock dialog** over the dimmed main window: master password (masked) with a show toggle, "Unlock automatically on this device" checkbox, primary "Unlock"; plus an error state "Wrong master password".

## Anti-references (owner)
Not a hacker/neon look (black with glowing green/cyan). Not grey 2010 enterprise admin tool. Not playful or rounded consumer-app styling. (An IDE-like familiarity was not rejected.)

## Constraints
- Synthetic data only: example hostnames (`*.lab.example`, `*.corp.example`), RFC 5737 IPs (192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24), generic users.
- Whatever moves into `ui/` must render in WebView2, WKWebView and WebKitGTK 4.1 (Ubuntu 22.04), Vite target `safari13`: hex colours; no `oklch()`, `color-mix()`, `light-dark()`, CSS nesting, `@layer`, `:has()`, container queries.
- Fonts are bundled (OFL-1.1 or permissive), ≤ ~300 KB woff2 in total; no network at runtime.
- The name means *Fernverbindung* (remote connection), not the plant.

## Unresolved decisions
- The visual world (chosen in Task 5 of the M1 plan).
- The default terminal font (named in DESIGN.md; user-configurable in M6).
