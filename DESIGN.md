---
name: fernjo
description: "Ulm, revised": a quiet Braun-style instrument for SSH and RDP sessions, where only running state lights up.
colors:
  bg: "#f7f6f3"
  surface: "#fbfaf7"
  surface-raised: "#fdfcfa"
  sidebar: "#e6e4de"
  border: "#cfccc3"
  border-strong: "#77736b"
  border-input: "#7f7b73"
  tab-text: "#55534d"
  text: "#1e1e1c"
  text-muted: "#5a5852"
  accent: "#f0b400"
  accent-text: "#1e1e1c"
  selection: "#d1cdc3"
  selection-bar: "#1e1e1c"
  hover: "#eeece7"
  focus: "#1e1e1c"
  success: "#2f7d22"
  warning: "#9a5d00"
  danger: "#b3261e"
  lamp-live: "#2f7d22"
  lamp-connecting: "#9a5d00"
  lamp-dropped: "#b3261e"
  ssh: "#5a5852"
  rdp: "#5a5852"
  overlay: "#2826226b"
  shadow: "#1e1c1847"
  term-bg: "#fbfaf7"
  term-fg: "#232320"
  term-cursor: "#1e1e1c"
  term-selection: "#eadfb8"
  bg-dark: "#1c1c1a"
  surface-dark: "#222220"
  surface-raised-dark: "#2a2a27"
  sidebar-dark: "#262624"
  border-dark: "#363531"
  border-strong-dark: "#77746d"
  border-input-dark: "#75736c"
  tab-text-dark: "#a3a097"
  text-dark: "#eceae4"
  text-muted-dark: "#a3a097"
  accent-dark: "#f0b400"
  accent-text-dark: "#1c1c1a"
  selection-dark: "#4a4843"
  selection-bar-dark: "#eceae4"
  hover-dark: "#2e2d2a"
  focus-dark: "#eceae4"
  success-dark: "#6cc04a"
  warning-dark: "#e0a030"
  danger-dark: "#ff7a70"
  lamp-live-dark: "#6cc04a"
  lamp-connecting-dark: "#e0a030"
  lamp-dropped-dark: "#ff7a70"
  ssh-dark: "#a3a097"
  rdp-dark: "#a3a097"
  overlay-dark: "#00000085"
  shadow-dark: "#00000080"
  term-bg-dark: "#1c1c1a"
  term-fg-dark: "#e2e0d9"
  term-cursor-dark: "#f0b400"
  term-selection-dark: "#4a4535"
typography:
  title:
    fontFamily: "Hanken Grotesk Variable, Hanken Grotesk, system-ui, sans-serif"
    fontSize: "17px"
    fontWeight: 600
    lineHeight: 1.4
    letterSpacing: "0em"
  body:
    fontFamily: "Hanken Grotesk Variable, Hanken Grotesk, system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.4
    fontFeature: "tnum"
  body-strong:
    fontFamily: "Hanken Grotesk Variable, Hanken Grotesk, system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 600
    lineHeight: 1.4
    fontFeature: "tnum"
  label:
    fontFamily: "Hanken Grotesk Variable, Hanken Grotesk, system-ui, sans-serif"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 1.3
    letterSpacing: "0em"
  caption:
    fontFamily: "Hanken Grotesk Variable, Hanken Grotesk, system-ui, sans-serif"
    fontSize: "11px"
    fontWeight: 400
    lineHeight: 1.3
  mono:
    fontFamily: "Red Hat Mono Variable, ui-monospace, Cascadia Mono, Consolas, monospace"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: "19px"
rounded:
  sm: "4px"
  md: "6px"
spacing:
  "1": "2px"
  "2": "4px"
  "3": "8px"
  "4": "12px"
  "5": "16px"
  "6": "24px"
components:
  button-primary:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.accent-text}"
    typography: "{typography.body}"
    rounded: "{rounded.sm}"
    padding: "0 14px"
    height: "32px"
  button-secondary:
    backgroundColor: "transparent"
    textColor: "{colors.text}"
    typography: "{typography.body}"
    rounded: "{rounded.sm}"
    padding: "0 14px"
    height: "32px"
  button-reconnect:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.accent-text}"
    typography: "{typography.caption}"
    rounded: "{rounded.sm}"
    padding: "2px 7px"
  input:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text}"
    typography: "{typography.body}"
    rounded: "{rounded.sm}"
    padding: "0 4px 0 10px"
    height: "32px"
  filter-box:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text-muted}"
    typography: "{typography.body}"
    rounded: "{rounded.sm}"
    padding: "0 8px"
    height: "28px"
  tree-row:
    backgroundColor: "{colors.sidebar}"
    textColor: "{colors.text}"
    typography: "{typography.body}"
    height: "24px"
  tree-row-hover:
    backgroundColor: "{colors.hover}"
  tree-row-selected:
    backgroundColor: "{colors.selection}"
    textColor: "{colors.text}"
    typography: "{typography.body-strong}"
  tab:
    backgroundColor: "{colors.sidebar}"
    textColor: "{colors.tab-text}"
    typography: "{typography.body}"
    padding: "0 8px 0 12px"
    height: "36px"
  tab-active:
    backgroundColor: "{colors.term-bg}"
    textColor: "{colors.text}"
  vault-note:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text-muted}"
    typography: "{typography.label}"
    rounded: "{rounded.sm}"
    padding: "7px 8px 7px 10px"
  checkbox:
    backgroundColor: "{colors.surface}"
    size: "16px"
---

# Design System: fernjo

## Overview

**Creative North Star: "The Quiet Instrument"**

fernjo is a well-made Braun instrument in the HfG Ulm tradition: a warm grey housing (sidebar and tab strip), off-white working surfaces and charcoal ink in light; in dark, the ET66 body, with near-black warm greys and bone text. The housing stays silent. Only the state that matters lights up: a lamp beside a running session, and one yellow key for the one action the current context asks for.

The terminal wins. The owner works in sessions for hours, so nothing in the chrome competes with the session area: no decorative shadows, no gradients, no chrome colour beyond greys. The tree is dense and structural (24 px rows, folders by customer, site and role) because the day starts by browsing it, not by searching. Light and dark are both first-class and tuned for low glare over long sessions.

The world rejects the neon terminal costume (black with glowing green or cyan), the grey 2010 enterprise admin tool, and playful, rounded consumer styling. An IDE-like familiarity is allowed.

The tokens live in `ui/src/styles/tokens.css` (the shipped source of truth; `ui/src/styles/tokens.test.ts` guards them), base styles in `ui/src/styles/base.css`. The approved components and states are rendered in `Docs/design/m01-concepts.html`, concept "Ulm, revised" (classes `c-ulm c-ulm2`); the other concepts in that file are the declined comparison set and are not part of this system. Where the artifact and `tokens.css` differ, `tokens.css` wins.

**Key Characteristics:**
- Quiet housing: warm greys, 1 px rules, small radii (4 px / 6 px).
- The terminal wins: the session surface is the largest, calmest area.
- Only running state lights up: no session, no lamp.
- One key colour: yellow (#f0b400) for the one action that matters.
- State is told by shape first; colour only supports it.
- Dense: at least 23 tree rows visible at 1280×800 with the properties panel open.

## Colors

A warm, near-neutral grey family carries the whole interface; one yellow key and three state colours are the only chroma, each confined to a defined job.

Every token below exists in both themes. Light values live in `:root`; dark values in `:root[data-theme="dark"]` and in the `prefers-color-scheme: dark` block (identical).

### Primary
- **Key Yellow** (`--color-accent`, #f0b400 in both themes), with **Key Ink** (`--color-accent-text`, #1e1e1c light / #1c1c1a dark) on it: the fill of the single action that matters in the current context ("Unlock and connect" in an unlock prompt) and of Reconnect on every dropped tab. In dark, also the terminal cursor (`--term-cursor`).

### Neutral (housing, surfaces, ink, rules)

| Role | Token | Light | Dark |
|---|---|---|---|
| Window background | `--color-bg` | #f7f6f3 | #1c1c1a |
| Working surface (inputs, vault note, light terminal) | `--color-surface` | #fbfaf7 | #222220 |
| Raised surface (dialogs, popovers) | `--color-surface-raised` | #fdfcfa | #2a2a27 |
| Housing (sidebar, tab strip) | `--color-sidebar` | #e6e4de | #262624 |
| Rule (1 px dividers, tab separators) | `--color-border` | #cfccc3 | #363531 |
| Strong rule (secondary button outline, scrollbar thumb) | `--color-border-strong` | #77736b | #77746d |
| Input border (fields, filter, checkbox) | `--color-border-input` | #7f7b73 | #75736c |
| Inactive tab text | `--color-tab-text` | #55534d | #a3a097 |
| Ink | `--color-text` | #1e1e1c | #eceae4 |
| Muted ink (labels, host column, chevrons, inherited marks) | `--color-text-muted` | #5a5852 | #a3a097 |
| Selected row | `--color-selection` | #d1cdc3 | #4a4843 |
| Selection bar | `--color-selection-bar` | #1e1e1c | #eceae4 |
| Row hover | `--color-hover` | #eeece7 | #2e2d2a |
| Keyboard focus | `--color-focus` | #1e1e1c | #eceae4 |
| Protocol icon SSH | `--color-ssh` | #5a5852 | #a3a097 |
| Protocol icon RDP | `--color-rdp` | #5a5852 | #a3a097 |
| Dimming overlay | `--color-overlay` | #2826226b | #00000085 |
| Shadow colour (only inside `--shadow-raised`) | `--color-shadow` | #1e1c1847 | #00000080 |

### State (lamps and marks only)

| Role | Token | Light | Dark |
|---|---|---|---|
| Success | `--color-success` | #2f7d22 | #6cc04a |
| Warning | `--color-warning` | #9a5d00 | #e0a030 |
| Danger (also error text and error field border) | `--color-danger` | #b3261e | #ff7a70 |
| Lamp live | `--color-lamp-live` | #2f7d22 | #6cc04a |
| Lamp connecting | `--color-lamp-connecting` | #9a5d00 | #e0a030 |
| Lamp dropped | `--color-lamp-dropped` | #b3261e | #ff7a70 |

The light lamps are darker than the concept's first values (#2f8a22, #a86400) so they hold 3:1 on the selected row; `tokens.test.ts` checks every lamp against `--color-sidebar` and `--color-selection` at 3:1.

### Named Rules

**The One Key Rule.** `--color-accent` fills only the one action that matters in the current context (for example "Unlock and connect") and Reconnect on every dropped tab (owner decision: every dropped tab, not only the active one). Never for chrome, active tabs, selection, links, focus or decoration. If two yellow actions compete in one context, one of them is wrong.

**The Lamps-Only Rule.** Green, amber and red appear only inside lamps and marks (tree lamp column, tab state mark) and in error text for a failed action. Every state colour is paired with a shape (filled dot, ring, struck ring, alert icon) so the state reads without colour.

**The Shape-Not-Colour Rule.** SSH and RDP icons share one grey (`--color-ssh` and `--color-rdp` hold the same value) and are told apart by shape: SSH is a terminal window with a prompt chevron, RDP is a monitor on a stand. The two tokens exist so a future change stays possible without renaming, not as an invitation to colour-code protocols.

## Typography

**UI Font:** Hanken Grotesk Variable (`--font-ui`, with Hanken Grotesk, system-ui, sans-serif)
**Mono Font:** Red Hat Mono Variable (`--font-mono`, with ui-monospace, Cascadia Mono, Consolas, monospace); the terminal default, user-configurable in M6.

**Character:** a precise, slightly warm grotesk for the instrument's labelling, paired with a calm mono for the session. Both are bundled from npm (`@fontsource-variable/hanken-grotesk` and `@fontsource-variable/red-hat-mono`, pinned 5.3.0, OFL-1.1, normal style only, about 99 KB woff2 in total), imported in `ui/src/main.ts`; no network fonts at runtime.

Body text uses tabular figures (`font-variant-numeric: tabular-nums`) so addresses, ports and times align.

### Hierarchy
- **Title** (`--weight-strong` 600, `--text-lg` 17px, line-height 1.4): headings of unlock prompts and dialogs, the shell `h1`.
- **Body** (`--weight-regular` 400, `--text-md` 13px, line-height 1.4): tree rows, tab titles, property values, prompt copy.
- **Body strong** (600, 13px): folder names, the selected row, the properties panel header name, the vault note title.
- **Medium** (`--weight-medium` 500, 13px): button labels and field labels.
- **Label** (400, `--text-sm` 12px, muted): property keys, "from <folder>" marks, host column, vault chip, vault note body, the properties panel kind ("SSH connection"). Normal case, no letter-spacing.
- **Caption** (400, `--text-xs` 11px): tab status text ("Connecting…", "Needs vault") and the Reconnect button.
- **Mono** (400, 13px, 19px line-height): the terminal.

### Named Rules

**The Normal-Case Label Rule.** Labels are normal case, 12 px, muted. No uppercase, no tracking (the original Ulm's uppercase labels were rejected by the owner).

## Layout

The window keeps the native OS title bar. A fixed-width sidebar (`--sidebar-width` 300px) sits left: filter box and vault state, then the connection tree, then the collapsible properties panel stacked under the tree. Right: a classic tab strip (36 px high) over the session area. Design size 1280×800, minimum 900×600, at 125–200 % scaling.

Density is a requirement, not a preference: tree rows are `--row-height-tree` (24px), and at 1280×800 with the properties panel open at least 23 tree rows stay visible. Indentation is 14 px per level (artifact value, no token yet) and stays legible over at least four levels.

Spacing follows `--space-1` to `--space-6` (2, 4, 8, 12, 16, 24 px). Sidebar head and tree use 8 px gutters; the properties panel uses 8 px / 10 px padding with a 84 px key column; prompt stacks use 14 px gaps.

## Elevation & Depth

The system is flat. Depth comes from tonal layering: the housing (`--color-sidebar`) frames the lighter working surfaces (`--color-bg`, `--color-surface`, the terminal), separated by 1 px rules (`--color-border`).

### Shadow Vocabulary
- **Raised** (`--shadow-raised`: `0 2px 6px var(--color-shadow), 0 24px 48px -12px var(--color-shadow)`): dialogs and popovers only, on `--color-surface-raised`. A modal dialog dims the window behind it with `--color-overlay`.

### Named Rules

**The No-Decoration Rule.** No shadow on rows, tabs, buttons, inputs, the vault note or panels. If it is not floating above the window, it has no shadow.

## Shapes

Small, exact radii and hairline rules. `--radius-sm` (4px) for buttons, inputs, the filter box, the vault chip and note, the tab close button and the Reconnect button; `--radius-md` (6px) for dialogs and popovers; the checkbox box uses 3 px. Rules are 1 px everywhere. Lamps are circles (`--lamp-size` 9px, 1.5 px ring). Icons are 16 px line drawings (1.5 px stroke, round caps and joins, `currentColor`), 12 px inside small controls; state marks in tabs are 10 px.

## Components

### Tree row
Quiet, dense and structural.
- **Folder:** chevron (14 px, muted; rotated when collapsed) plus name in 600.
- **Connection:** protocol icon (16 px, muted, shape-coded) plus name in 400, indented past the chevron column; optional host column and the lamp column on the right.
- **Hover:** background `--color-hover`.
- **Selection:** Selection is the higher-contrast row plus a 3 px ink bar (`--color-selection-bar`) on the left plus semibold text; in light the row is darker (#d1cdc3), in dark it is lighter (#4a4843) than the sidebar. The owner approved this in the revised artifact in both themes. Text on the selected row is `--color-text`.
- **Keyboard focus:** a 1 px inset outline (`outline: 1px solid var(--color-focus)`, `outline-offset: -1px`, set globally in `base.css` for `:focus-visible`), deliberately weaker than selection so the two never read as the same thing.

### Lamp column
The signature of the world. Lamps sit in the last column of the row (right of the host column when it is shown).
- **No session:** no lamp.
- **Connecting:** a ring in `--color-lamp-connecting`, pulsing in opacity over `--duration-pulse` (1600ms) with `--easing-pulse`; the pulse is off under `prefers-reduced-motion`.
- **Live:** a filled dot in `--color-lamp-live`.
- **Dropped:** a ring struck through diagonally in `--color-lamp-dropped`.
The tab strip repeats the same shapes as 10 px marks.

### Host column
Optional (a setting). The address in `--color-text-muted`, 12 px, right-aligned before the lamp; when hidden, the lamp moves to the right edge.

### Filter box
28 px high, `--color-surface` field with a 1 px `--color-border-input` border, `--radius-sm`, search icon and placeholder "Filter connections" in muted ink.

### Vault state and lock note
- **Chip** beside the filter (28 px, 1 px `--color-border`, 12 px, lock or unlock icon): "Unlocked" or "Locked".
- **Lock note:** while locked, a note sits at the top of the sidebar under the filter: `--color-surface` background, 1 px `--color-border`, `--radius-sm`, lock icon, title "Vault locked" (13 px, 600), body "Connections with stored credentials ask for it." (12 px, muted), and an outlined "Unlock" button (26 px high). The note is **compact** (title and button on one line, body under it) so that it leaves the 23 tree rows and the properties panel their room; the master password form (field, "Unlock automatically on this device", hint, and for "No vault yet" the repeat field and the no-recovery sentence) expands inline only after the button or the chip is clicked, with the password field focused, and "Cancel" collapses it again. While the form is open the tree may shrink to make room. An unusable vault shows Rust's reason as written, the file path and how to restore `vault.fernjo.bak`.
- **Never modal:** the tree stays usable while locked. The master password is asked inside the tab of the connection that needs credentials.

### Properties panel
Stacked under the tree, separated by a 1 px rule.
- **Header** (26 px, clickable to collapse): chevron, connection name in 600, kind in 12 px muted at the right. Collapsed, only the header remains.
- **Rows** (min 24 px): 84 px key column (12 px muted label), value, and a right column.
- **Inherited values** carry a muted 12 px "from <folder>" mark.
- **Local values** carry an outlined "Inherit" reset (reset icon, 12 px, 1 px rule, `--radius-sm`) that returns the value to the inherited one.
- **Credential** is shown by name only (for example "deploy key"); secrets never reach the webview.

### Tabs
Classic tab strip on the housing (`--color-sidebar`), 36 px, tabs 150–260 px wide separated by 1 px `--color-border`, each with state mark, protocol icon, title, optional 11 px status and a close button.
- **Inactive:** `--color-tab-text`.
- **Active:** background `--term-bg` so it joins the session surface, text `--color-text`, 2 px ink line at the bottom.
- **Connected:** filled mark.
- **Connecting:** pulsing ring mark plus "Connecting…".
- **Disconnected:** struck ring mark plus the yellow Reconnect button (key fill, 11 px, `--radius-sm`, reconnect icon), on every dropped tab.
- **RDP:** identical structure with the monitor icon.

### Inline unlock prompt (inside a tab)
Shown in the session area of the tab that needs the vault (tab status "Needs vault"), on `--color-bg`, a 420 px column 88 px from the top with 14 px gaps: lock icon plus title "<name> needs the vault" (17 px, 600), a muted explanation that names the credential and says other tabs keep running, the master password field (masked, show toggle), the "Unlock automatically on this device" checkbox, and the actions: yellow "Unlock and connect" and an outlined "Cancel".
- **Error state:** the field border and a 1 px ring turn `--color-danger`, and an alert icon plus "Wrong master password. Check Caps Lock and try again." appear in `--color-danger`.

### Buttons
- **Shape:** 32 px high, 0 14px padding, `--radius-sm`, label in 500, optional 16 px icon with 6 px gap.
- **Primary (yellow key):** `--color-accent` fill, `--color-accent-text` ink. One per context.
- **Secondary:** transparent, 1 px `--color-border-strong` outline, `--color-text` ink.
- **Focus:** the global 1 px `--color-focus` outline.

### Confirm dialog
For destructive actions that cannot be undone (deleting a node: "Delete “Homelab” and 4 items inside?").
- **Surface:** modal, 360 px wide, on `--color-surface-raised` with `--shadow-raised` and `--radius-md`, the window behind it dimmed with `--color-overlay`. Title in 600 (13 px), an optional muted 12 px line ("This cannot be undone.").
- **Actions:** right-aligned secondary buttons. Cancel has the initial focus (Enter does not confirm); the confirm button is secondary style with `--color-danger` text. No yellow: a destructive action is never "the one action that matters".
- **Keyboard:** Escape cancels, Tab cycles between the two buttons only.

### Context menu
Popover on `--color-surface-raised` with `--shadow-raised` and `--radius-md`, 28 px items with `--radius-sm` hover on `--color-hover`. Opens at the pointer (or under the focused row with Shift+F10 / the menu key), stays inside the window, closes on Escape, outside click and after an action; arrows move, Enter activates.

### Inputs and checkbox
- **Field:** 32 px, `--color-surface`, 1 px `--color-border-input`, `--radius-sm`, 10 px left padding; trailing icon buttons in muted ink.
- **Error:** border and 1 px ring in `--color-danger`.
- **Checkbox:** 16 px box, 1 px `--color-border-input`, 3 px radius, `--color-surface` fill, label at 8 px.

### Terminal
The session surface. Colours come from the `--term-*` tokens (mapped onto the xterm.js theme in M4); the font is `--font-mono` 13px with 19px line height.

| Token | Light | Dark |
|---|---|---|
| `--term-bg` | #fbfaf7 | #1c1c1a |
| `--term-fg` | #232320 | #e2e0d9 |
| `--term-cursor` | #1e1e1c | #f0b400 |
| `--term-selection` | #eadfb8 | #4a4535 |
| `--term-ansi-0` black | #232320 | #3a3935 |
| `--term-ansi-1` red | #b3261e | #f0625a |
| `--term-ansi-2` green | #3d7a1f | #8ccf5e |
| `--term-ansi-3` yellow | #8a6000 | #f0c445 |
| `--term-ansi-4` blue | #2d5c9a | #7aa7e0 |
| `--term-ansi-5` magenta | #8a3f8f | #d08ad4 |
| `--term-ansi-6` cyan | #1d7270 | #63c4bd |
| `--term-ansi-7` white | #cfccc4 | #cfccc4 |
| `--term-ansi-8` bright black | #6b6962 | #6b6962 |
| `--term-ansi-9` bright red | #c9372c | #ff8a80 |
| `--term-ansi-10` bright green | #47862a | #a8e07f |
| `--term-ansi-11` bright yellow | #9c6c00 | #ffd86b |
| `--term-ansi-12` bright blue | #3a6cb0 | #9cc1f0 |
| `--term-ansi-13` bright magenta | #9f4ca4 | #e5a8e8 |
| `--term-ansi-14` bright cyan | #22827d | #8fded7 |
| `--term-ansi-15` bright white | #ffffff | #f7f6f2 |

**The Minimum-Contrast Rule.** `--terminal-min-contrast` (4.5) is applied through xterm.js `minimumContrastRatio`, on by default: faint foregrounds are lifted when drawn, the palette itself stays unchanged. ANSI 1–6 and 9–14 hold at least 3:1 on `--term-bg` in both themes (tested in `tokens.test.ts`); 0, 7, 8 and 15 are background-adjacent by design and rely on the minimum contrast.

### Theming mechanism
Light is the default in `:root`. Dark is defined twice with identical contents: `:root[data-theme="dark"]` and `@media (prefers-color-scheme: dark) { :root:not([data-theme="light"]) }`. With no attribute the window follows the system; the theme setting in M6 only sets `data-theme` on `<html>`. Under `prefers-reduced-motion: reduce`, `--duration-fast`, `--duration-normal` and `--duration-pulse` drop to 0ms and `base.css` stops animations and transitions.

### App icon
The charcoal housing (#1e1e1c rounded square with a faint #3a3935 rim), a bone ring (#eceae4, the local end) joined by a short diagonal stroke to the lit yellow key (#f0b400, the remote end). Sources: `src-tauri/icons/icon-source.svg` (all desktop icons) and `src-tauri/icons/icon-source-macos.svg` (same artwork scaled to Apple's 824 px grid, for `icon.icns` only). Regenerate with `pnpm tauri icon src-tauri/icons/icon-source.svg` (drop the generated `android/` and `ios/` folders), then `pnpm tauri icon src-tauri/icons/icon-source-macos.svg -o <temp dir>` and copy only `icon.icns` (see `Docs/DECISIONS.md`, entry on the app icon).

## Do's and Don'ts

### Do:
- **Do** use only tokens from `ui/src/styles/tokens.css` in shipped CSS; no literal colours, sizes or radii in components.
- **Do** write colours as hex (`#rrggbb` or `#rrggbbaa`).
- **Do** add every new colour token to the light block and to both dark blocks with identical dark values; `tokens.test.ts` enforces parity.
- **Do** treat token names as an interface: add names, never rename or remove one without a `Docs/DECISIONS.md` entry.
- **Do** pair every state colour with a shape (dot, ring, struck ring, alert icon).
- **Do** give each context at most one yellow key, and give every dropped tab its yellow Reconnect.
- **Do** keep keyboard focus as the 1 px inset outline, weaker than selection.
- **Do** keep at least 23 tree rows visible at 1280×800 with the properties panel open.
- **Do** ask for the master password inside the tab that needs it.

### Don't:
- **Don't** use `oklch()`, `color-mix()`, `light-dark()`, CSS nesting, `@layer`, `:has()` or container queries: WebKitGTK 4.1 and the `safari13` build target cannot render them.
- **Don't** use `data:` assets (fonts, images); the CSP blocks them, and Vite inlining stays off.
- **Don't** load fonts or other assets from the network at runtime.
- **Don't** use yellow for chrome, active tabs, selection, focus or decoration.
- **Don't** use green, amber or red outside lamps, marks and error text, and don't colour-code SSH and RDP.
- **Don't** show a lamp for a connection without a session.
- **Don't** make the vault modal or block the tree while it is locked.
- **Don't** put uppercase or tracked labels in the chrome.
- **Don't** add shadows other than `--shadow-raised` on dialogs and popovers.
- **Don't** send or render secrets in the UI; credentials appear by name only.
