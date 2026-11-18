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
- **Dense, compact tree:** row height 22–24 px, small icons, little padding; at least 23 rows visible at 1280×800 with the properties panel open (adjusted from 22–28 px / 25 rows during M1: with a five-field properties panel, 25 rows only fit at 22 px); indentation legible over at least 4 levels.
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

## Chosen direction
**Ulm, revised**, approved by the owner on 2026-11-15 ("ulm revised is approved") after one feedback round. Artifact: https://claude.ai/artifact/9BhGCKAfNbHvbsUUQQwhas (private); source `Docs/design/m01-concepts.html`.

## Direction contract
- **THESIS:** fernjo as a well-made Braun instrument: quiet housing, precise type, and only the state that matters lights up. Refuses the grey IDE clone and the neon terminal costume.
- **OWN-WORLD:** warm grey housing (sidebar, tab strip), off-white working surfaces, charcoal ink; in dark the ET66 body: near-black warm greys with bone text. Yellow (#f0b400) is the key colour, only for the action that matters and for Reconnect on dropped tabs. Green/amber/red appear only in lamps and marks. Hanken Grotesk, Red Hat Mono; small radii (4/6 px), 1 px rules.
- **STORY:** the owner browses the tree by structure, sees at a glance which sessions run, opens one, works for hours; nothing competes with the terminal; a dropped session and a locked vault ask for exactly one action.
- **FIRST VIEWPORT:** sidebar 300 px: filter + vault state, tree (24 px rows, optional grey host column, lamp column at the far right), collapsible properties panel below; tab strip over the terminal on the right. Signature move: the lamp column (no lamp = no session, pulsing ring = connecting, filled = live, struck = dropped) plus the yellow key.
- **FORM:** Braun / HfG Ulm functionalism, position 1 on the ordered list (impeccable's pick), seed: seed:1c7386560d20e2dc
- **FINISH:** unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance

## Owner adjustments (round 1, binding)
Lamps only for running sessions; selection = darker row + left bar + semibold, focus = 1 px outline; non-modal vault (sidebar note, password asked in the tab of the connection that needs credentials); host column optional; properties panel collapsible; labels normal case 12 px grey; yellow Reconnect on every dropped tab; terminal minimum contrast 4.5:1 on by default (xterm.js `minimumContrastRatio`), palette unchanged.

## Unresolved decisions
- The default terminal font is Red Hat Mono (from the direction); user-configurable in M6.
