# Milestone 1 – UI design direction Implementation Plan

**Goal:** The visual direction of fernjo is chosen by the owner from at least three concepts and recorded, so every UI milestone builds on it.

**Approach:** The `impeccable` skill drives the design work. Its `init` step records product truth in `PRODUCT.md`. Its new-work flow (Operate mode, code-led) derives the directions and runs `concept-seed`. At least three directions are built as static HTML mockups in **one** Claude artifact that switches between concept, theme and view; this artifact replaces impeccable's decision page, because the roadmap asks for it. The owner picks and the chosen direction is iterated in the same artifact. The result goes into `ui/` as a token file plus base styles applied to the empty shell. A new app icon follows the direction. The impeccable documenter writes `DESIGN.md` from that finished result.

**Tech stack:** Svelte 5 · plain CSS custom properties (no CSS framework, no preprocessor) · Vitest (`?raw` CSS import) · impeccable 4.8 (`init`, new-work, `concept-seed`, `impeccable-finish-reviewer`, `impeccable-documenter`) · Claude Artifact (load the `artifact-design` skill, then `quickstart` with intent `other`) · `pnpm tauri icon`

**Spec:** [2026-11-15-fernjo-design.md](../specs/2026-11-15-fernjo-design.md) (§3 user experience incl. "Visual design direction", §5.2 unlock, §5.3 terminal, §6 performance targets, §10 icon set left to the builder). Roadmap: [M1 in ROADMAP.md](../../ROADMAP.md).

## Global Constraints

- **Scope:** design only. No Tauri command, no Svelte components for the tree, tabs, terminal, properties panel or vault (those come in M2 to M4), no xterm.js, no settings UI (the theme switch lives in M6). The shell only takes on the tokens: background, text, type and the `fernjo` heading.
- **Native window frame:** concepts keep the OS title bar (Tauri default decorations). A custom title bar would be an architecture decision outside M1.
- **Shipped CSS must run in every target webview.** That means WebView2 on Windows, WKWebView on macOS, and WebKitGTK 4.1 on Ubuntu 22.04; the Vite build target is `safari13`. So `ui/` CSS uses colours as `#rrggbb` (or `#rrggbbaa`). It uses no `oklch()`, `color-mix()`, `light-dark()` or CSS nesting, and no `@layer`, `:has()` or container queries. The artifact mockups may use anything, but whatever moves into `ui/` follows this rule.
- **No network at runtime:** fonts are bundled (an `@fontsource*` npm package or `woff2` files with their licence file), licensed OFL-1.1 or permissive. The CSP in `src-tauri/tauri.conf.json` and `src-tauri/tests/tauri_conf.rs` stay unchanged. Font budget: at most ~300 KB `woff2` in total (spec §6 start-time target).
- **Synthetic data only** in mockups, screenshots and docs. Hosts are names like `pve-01.lab.example`, IPs come from RFC 5737 (`192.0.2.0/24`, `198.51.100.0/24`, `203.0.113.0/24`), users are names like `admin`/`deploy`, and fingerprints are made up. Nothing comes from the owner's real environment.
- **Token names are an interface** that M2 to M6 consume. They are fixed in Task 6 (Interfaces) and may only grow, not be renamed, without a DECISIONS entry.
- **Impeccable rules apply:** run `impeccable context` once per session (`~/.claude/skills/impeccable/scripts/impeccable.cmd` on Windows if `sh` is unavailable). Run `concept-seed` before any concept code. The direction contract never goes into shipped source or the artifact. Read `reference/craft-floor.md` immediately before writing UI code.
- **Owner checkpoints** (structured question tool; stop and wait): the init interview (Task 2), the new-work questions (Task 2), the concept choice and each iteration round (Task 5), icon approval (Task 7), and the visual check of the shell (Task 6). Pushing, opening the PR and merging into `dev` are covered by CLAUDE.md ("Claude merges these PRs itself once … CI checks are green").
- **Git:** all work on `feature/m01-design-direction`, Conventional Commits, files staged by name, `git show --stat HEAD` after every commit. Every commit that finishes a roadmap task ticks it in `Docs/roadmap.json` and regenerates `Docs/ROADMAP.md`. Acceptance criteria are ticked only with evidence (Task 10).
- **"Done"** means `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm lint`, `pnpm test`, `pnpm ui:build`, `cargo deny check` and `dotnet run tools/roadmap.cs -- check` are all green.

## Review Focus

These are failure modes the spec implies but no acceptance criterion exercises, most likely first. Each is pinned in the named task.

1. **Three re-skins of one layout.** The concepts differ only in colour, so the choice is meaningless. Expected: each concept differs from every other on at least three axes (colour strategy, type, density/row height, chrome structure, how the properties panel and tabs sit). Each concept card names its axes in the artifact. → Task 3, Task 4.
2. **Light mode as an afterthought** (or dark as an inverted light). Expected: both themes are designed per concept, and the final tokens pass the contrast test in both. → Task 4 (contrast script per concept), Task 6 (`tokens.test.ts`).
3. **CSS that WebKitGTK on Ubuntu 22.04 or the `safari13` target cannot render** (`oklch`, `color-mix`, `light-dark()`, nesting). The app looks broken on Linux only. Expected: a test rejects these in `ui/` CSS. → Task 6.
4. **Fonts loaded from the network** (Google Fonts copied from the artifact). The CSP blocks them silently, or a privacy leak appears if someone loosens the CSP. Expected: fonts are bundled, the CSP is unchanged, and the webview console shows no CSP error. → Task 6.
5. **Unreadable terminal colours** (ANSI yellow on a light background, ANSI blue on a dark one). Expected: ANSI 1–6 and 9–14 reach at least 3:1 against the terminal background and the terminal text at least 4.5:1, in both themes. → Task 6.
6. **A tree too loose for hundreds of connections.** Expected: tree rows are 22–28 px high, and the mockup shows at least 25 tree rows in a 1280×800 window with the properties panel visible. → Task 3 (brief), Task 4.
7. **Real data in the public repo** (hostnames or IPs from the owner's environment pasted into mockups). → Global Constraints; checked in Task 4 Step 5.
8. **The direction contract leaks into shipped files** (impeccable rule). → Task 8 (reviewer check).

---

### Task 1: Roadmap status and impeccable housekeeping

**Files:**
- Modify: `Docs/roadmap.json`, `Docs/ROADMAP.md` (generated), `.gitignore`

- [ ] **Step 1:** Set M1 `status` to `in-progress`, then generate and check.
- [ ] **Step 2: Ignore impeccable's working files, keep its records.** Add to `.gitignore`: `.impeccable/*`, `!.impeccable/config.json` and `!.impeccable/design.json`, with the comment "impeccable: mocks, reviews and live sessions are local; config and design.json are committed". Check with `git check-ignore -q .impeccable/mocks/x` (ignored) and `git check-ignore -q .impeccable/design.json` (fails, so not ignored).
- [ ] **Step 3: Commit** `chore: start milestone 1, ignore impeccable working files`.

### Task 2: Product context and design brief

**Files:**
- Create: `PRODUCT.md` (repo root, written by impeccable `init`), the impeccable surface brief for the main window (at the path `impeccable context` resolves)

- [ ] **Step 1:** Run `impeccable context` (once for the session). PRODUCT.md is missing, so run `init`. It interviews the owner (at most three questions per round) and writes `PRODUCT.md`. Platform `web` (Tauri webview, desktop only). Facts come from the spec (§1 intent, §2 anti-goals, §3 UX). Nothing is invented; inferred facts are labelled.
- [ ] **Step 2: New-work questions (Operate mode).** Ask the owner about task, information, states, frequency, what must stay untouched and what would feel wrong, in two or three questions.
- [ ] **Step 3: Write the design brief into the surface brief** (roadmap task 1). It must contain:
  - **Layout:** one window, sidebar left (filter, tree, properties panel below the tree), tab bar and session area right; native title bar; design size 1280×800, minimum 900×600 (`src-tauri/tauri.conf.json`).
  - **Density:** hundreds of connections; tree row height 22–28 px; at least 25 tree rows visible at 1280×800 with the properties panel open; indentation readable over at least 4 levels.
  - **Themes:** light and dark, both first-class; "follow system" is the default (switch in M6).
  - **Screens and states every concept shows:**
    - **Main window:** filter box (empty); tree with at least 30 nodes over 3 levels (expanded and collapsed folders, SSH and RDP badges, open-session indicator, a selected row, a keyboard-focused row).
    - **Tabs:** four tabs, connected (active), connecting, disconnected with a reconnect affordance, and one RDP tab.
    - **Terminal:** a realistic SSH session (prompt, coloured `ls` output, an ANSI 16-colour sample line, cursor).
    - **Properties panel:** the selected connection, with one local value plus reset, values "inherited from <folder>", and a credential reference by name only.
    - **Vault lock state:** visible somewhere.
    - **Unlock dialog** over the dimmed main window: master password (masked) with a show toggle, "Unlock automatically on this device", primary "Unlock", and an error state "Wrong master password".
  - **Constraints:** the synthetic-data rule and the shipped-CSS rule from Global Constraints.
- [ ] **Step 4: Tick** roadmap task "Design brief from spec §3…"; generate; commit `docs: product context and design brief for milestone 1` (`PRODUCT.md`, surface brief, roadmap files).

### Task 3: Directions via impeccable new-work

**Files:** none in the repo (directions live in the session until the artifact is built; the chosen one is recorded in Task 5).

- [ ] **Step 1:** Follow `reference/new-work.md` §3 "Create or replace the visual world": the mechanism sentence, the audience scene, and seven candidates from the audience's world. Per impeccable's Operate rule, these do not come from terminals or control panels; candidates span at least three material families.
- [ ] **Step 2:** Run `impeccable concept-seed --scope direction --mode operate` and follow its output (verdicts, raises, pick card). Build path: code-led (Operate).
- [ ] **Step 3: Fix the concept set.** The concepts are the full cards of the hand: the assigned direction, the pick card if there is one, and every winning or competitive challenger. **At least three.** If the honest hand has fewer, re-roll (plain register) as impeccable prescribes, and never promote a declined challenger. Write down, for each concept, its name, thesis, palette, type, tree row height and the three or more axes on which it differs from the others (Review Focus 1).
- [ ] **Step 4: Disclose the substitution** in one line to the owner: the directions are presented through the roadmap's artifact with a switcher instead of impeccable's decision page. Then continue.

### Task 4: Concept artifact with switcher

**Files:**
- Create: `Docs/design/m01-concepts.html` (the artifact source, committed as the durable record; `Docs/` is outside ESLint and Prettier)

- [ ] **Step 1:** Load the `artifact-design` skill, then call `Artifact` `quickstart` with intent `other`. Read `reference/craft-floor.md` (impeccable) before writing the page.
- [ ] **Step 2: Build one static HTML page:**
  - **Switcher:** concept (≥3), theme (light/dark, independent of the viewer's OS theme), and view (main window / unlock dialog / unlock dialog with error).
  - **Each concept** renders the full brief from Task 2 Step 3 at a fixed 1280×800 frame, scaled to fit, plus a caption with thesis, axes, type and tree row height.
  - **No JavaScript beyond the switcher**, and no runtime capabilities. Fonts may come from Google Fonts in the artifact only.
- [ ] **Step 3: Contrast check per concept and theme.** Write a scratchpad script (outside the repo) that pulls each concept's palette and checks WCAG contrast: text on background and surface ≥ 4.5, muted text ≥ 4.5, button label on accent ≥ 4.5, focus indicator ≥ 3, terminal text ≥ 4.5, ANSI 1–6/9–14 ≥ 3. Fix failures before publishing; nobody gets to pick an inaccessible concept.
- [ ] **Step 4: Publish** with `icon: "layout"` and a one-sentence description. Open it in the browser (Claude in Chrome: new tab) and check every combination (concepts × 2 themes × 3 views) for layout breaks and the brief's state inventory. Fix, then republish to the same URL.
- [ ] **Step 5: Data check.** Run `grep -nE '([0-9]{1,3}\.){3}[0-9]{1,3}' Docs/design/m01-concepts.html` and review every hit: only RFC 5737 ranges are allowed. Check the hostnames by eye.
- [ ] **Step 6: Tick** roadmap tasks "Create at least three distinct design concepts…" and "Publish all concepts together…" (the artifact URL goes into the commit message); generate; commit `docs(design): milestone 1 design concepts`.

### Task 5: Owner's choice and iteration

**Files:**
- Modify: `Docs/design/m01-concepts.html`, the surface brief (direction contract)

- [ ] **Step 1: Ask the owner** with the structured question tool. Each option is one concept, named, with a one-line thesis. The artifact link goes into the question text, and "Other" stays open for combinations ("type from A, colours from B").
- [ ] **Step 2: Iterate on the chosen direction** in the same artifact: add the combined or refined concept as a new switcher entry, keep the originals for comparison, and republish to the same URL. Repeat until the owner says explicitly that the direction is approved. Record the approval wording and date for Task 9.
- [ ] **Step 3: Direction contract.** Write the six blocks plus the FINISH line into the surface brief, per impeccable `new-work.md` §5. It never goes into `ui/` or the artifact.
- [ ] **Step 4: Tick** roadmap task "Owner picks a concept…"; generate; commit `docs(design): chosen direction and contract`.

### Task 6: Design tokens in `ui/`, applied to the shell (TDD)

**Files:**
- Create: `ui/src/styles/tokens.css`, `ui/src/styles/base.css`, `ui/src/styles/tokens.test.ts`, `ui/src/vite-env.d.ts` (only if `svelte-check` needs `/// <reference types="vite/client" />` for `?raw`)
- Modify: `ui/src/main.ts`, `ui/src/App.svelte`, `ui/index.html`, `package.json` and `pnpm-lock.yaml` (font package, if used)

**Interfaces** (names fixed here; values come from the chosen direction):
- **Theme mechanism:** light in `:root`; dark in `:root[data-theme="dark"]` **and** in `@media (prefers-color-scheme: dark) { :root:not([data-theme="light"]) { … } }`, with identical contents. M6 only sets `data-theme` on `<html>`. `ui/index.html` gets `<meta name="color-scheme" content="light dark">`.
- **Colour tokens** (themed, `#rrggbb`/`#rrggbbaa`):
  - Backgrounds and surfaces: `--color-bg`, `--color-surface`, `--color-surface-raised`, `--color-sidebar`
  - Lines and text: `--color-border`, `--color-text`, `--color-text-muted`
  - Accent: `--color-accent`, `--color-accent-text` (label on accent)
  - Tree: `--color-selection`, `--color-hover`, `--color-focus`
  - Status: `--color-danger`, `--color-warning`, `--color-success`
  - Protocol badges: `--color-ssh`, `--color-rdp`
  - Overlay: `--color-overlay` (dialog backdrop)
- **Terminal tokens** (themed): `--term-bg`, `--term-fg`, `--term-cursor`, `--term-selection`, `--term-ansi-0` … `--term-ansi-15` (M4 maps them onto the xterm.js theme).
- **Untheme'd tokens:**
  - Type: `--font-ui`, `--font-mono`, `--text-xs`/`-sm`/`-md`/`-lg`, `--weight-regular`/`-strong`
  - Spacing and shape: `--space-1` … `--space-6`, `--radius-sm`/`-md`, `--row-height-tree`, `--shadow-raised`
  - Motion: `--duration-fast`/`-normal` (with a `prefers-reduced-motion` override to `0ms`)
  - The chosen direction may add tokens; it may not drop these.

- [ ] **Step 1: Write the failing test** `ui/src/styles/tokens.test.ts`. It imports `./tokens.css?raw` and `./base.css?raw`, parses the three blocks with a small regex parser, and implements WCAG 2.x relative luminance in the test file. Tests:
  - `every interface token is defined`: all names above exist (the light block holds the themed ones too).
  - `dark blocks are identical`: the `[data-theme="dark"]` and the media-query block define the same names with the same values.
  - `themes define the same colour tokens`: the set of `--color-*`/`--term-*` names in light equals the set in dark.
  - `colours are hex`: every `--color-*`/`--term-*` value matches `^#([0-9a-f]{6}|[0-9a-f]{8})$`.
  - `no unsupported css`: neither file contains `oklch(`, `color-mix(`, `light-dark(`, `@layer`, `:has(`, `@container`, a nested `&`, or `@import url(http`.
  - `contrast in both themes`:
    - ≥ 4.5: text on bg, surface and sidebar; muted text on bg and sidebar; accent-text on accent; danger on bg; term-fg on term-bg.
    - ≥ 3: focus on bg and sidebar; ANSI 1–6 and 9–14 on term-bg.
    - Translucent tokens are measured composited over `--color-bg`.
- [ ] **Step 2: Run** `pnpm test` → FAIL (files missing).
- [ ] **Step 3: Implement** `tokens.css` with the values of the approved direction. `base.css` holds:
  - box-sizing reset
  - `html`/`body` with `--color-bg`, `--color-text`, `--font-ui`, `--text-md`
  - `:focus-visible` outline with `--color-focus`
  - `::selection`
  - reduced-motion handling
  - the `h1` style for the shell

  Import both from `ui/src/main.ts` (tokens first). Bundle fonts via an `@fontsource*` package (exact version, added with `pnpm add`) or `woff2` files plus licence under `ui/src/assets/fonts/`. `App.svelte` stays a single `<h1>fernjo</h1>`; styling comes from `base.css`, so `App.test.ts` stays valid unchanged.
- [ ] **Step 4: Run** `pnpm test` → PASS; `pnpm lint` → green.
- [ ] **Step 5: Build check.** `pnpm ui:build`. Then `grep -l -- "--color-bg" ui/dist/assets/*.css` finds the token file, and `ls ui/dist/assets` shows the bundled fonts. Total `woff2` size is within the budget (state the number).
- [ ] **Step 6: Manual check** `pnpm tauri dev` on Windows, with the OS in light mode, then dark mode:
  - background, text and font follow the direction;
  - devtools (`Ctrl+Shift+I`) console shows no CSP or font-loading error;
  - the owner confirms the look.

  Record the result; macOS/Linux stay open (`pnpm tauri dev`).
- [ ] **Step 7: Tick** roadmap task "Design tokens as CSS variables…"; generate; commit `feat(ui): design tokens and base styles for the shell`.

### Task 7: App icon (scope change from planning)

**Files:**
- Create: `src-tauri/icons/icon-source.svg`
- Modify: `src-tauri/icons/*` (regenerated)

- [ ] **Step 1: Design the icon** from the approved direction as an SVG on a 1024×1024 canvas. It is a plain signifier in the direction's palette, must stay legible at 32×32, and must work on light and dark taskbars/docks. Show it to the owner in the artifact (new switcher view "Icon" with 16, 32, 128 and 512 px previews on light and dark backgrounds) and get approval.
- [ ] **Step 2: Generate** `pnpm tauri icon src-tauri/icons/icon-source.svg`. If the CLI rejects SVG, render a 1024 PNG in the scratchpad first and pass that, with the source SVG still committed. The set must contain the same files as before (`git status src-tauri/icons` shows modifications only, no deletions).
- [ ] **Step 3: Check** `cargo build --workspace` and `pnpm tauri dev`: the window or taskbar shows the new icon on Windows.
- [ ] **Step 4: Tick** the roadmap task "App icon…"; generate; commit `feat: app icon from the design direction`.

### Task 8: Finish review and `DESIGN.md`

**Files:**
- Create: `DESIGN.md` (repo root), `.impeccable/design.json`

- [ ] **Step 1: Finish review.** Spawn `impeccable-finish-reviewer` with the direction contract, the chosen concept in `Docs/design/m01-concepts.html`, and `ui/src/styles/`. Fix the material findings. Also check that no contract text appears in `ui/`, `Docs/design/` or the artifact (Review Focus 8).
- [ ] **Step 2: Documentation.** Spawn `impeccable-documenter` with the project root, the artifact path (the chosen concept plus `ui/src/styles/tokens.css`), the direction contract, `PRODUCT.md`, `reference/document.md`, and the write boundary `DESIGN.md` and `.impeccable/design.json` only. `DESIGN.md` must cover:
  - principles, typography (faces, sizes, bundled fonts and licence), colour (roles, both themes), spacing and density (tree row height), components (tree row, tab with states, properties field with the inherited marker, buttons, inputs, dialog, badges);
  - the **terminal colour scheme** (16 ANSI colours per theme, with the `--term-*` mapping);
  - the shipped-CSS rule (hex colours, no modern colour functions) and the token interface as the source of truth (`ui/src/styles/tokens.css`).
- [ ] **Step 3: Verify** that `DESIGN.md` and `.impeccable/design.json` exist and that every token named in `DESIGN.md` exists in `tokens.css` (one-off scratchpad check script).
- [ ] **Step 4: Tick** roadmap task "Record the direction in `DESIGN.md`…"; generate; commit `docs: DESIGN.md for the fernjo direction`.

### Task 9: Decisions, project memory, changelog

**Files:**
- Modify: `Docs/DECISIONS.md`, `CLAUDE.md`, `CHANGELOG.md`

- [ ] **Step 1: DECISIONS.md** (new entries on top, template format):
  - **"Visual direction: <name>"** names the chosen concept, the owner's approval (date, wording from Task 5), why it won, and the rejected concepts with one line each. The artifact link is noted as private to the owner, and `Docs/design/m01-concepts.html` is the durable record.
  - **"UI tokens and theming"** covers the token file as the interface, the `data-theme` mechanism, the hex-only and no-modern-CSS rule with its reason (WebKitGTK 4.1 / `safari13`), and the bundled fonts with name, version and licence.
  - **"App icon"** covers the placeholder being replaced and the source SVG.

  Older entries stay unchanged (the M0 icon follow-up is answered by the new entry).
- [ ] **Step 2: CLAUDE.md** "Project-specific": one bullet saying that UI work uses only tokens from `ui/src/styles/tokens.css` and follows `DESIGN.md`; that shipped CSS stays hex-only without `oklch`/`color-mix`/`light-dark()`/nesting (guarded by `ui/src/styles/tokens.test.ts`); and that new tokens are added to the light and both dark blocks.
- [ ] **Step 3: CHANGELOG.md** `[Unreleased]` → `Added`: "Visual design: light and dark theme following the system setting, bundled fonts, new app icon."
- [ ] **Step 4: Tick** roadmap task "Decision entry in `Docs/DECISIONS.md`…"; generate; commit `docs: decisions for the design direction`.

### Task 10: Verification, acceptance, PR

- [ ] **Step 1: Full local gate.** Run `pnpm install --frozen-lockfile`, `pnpm ui:build`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm lint`, `pnpm test`, `cargo deny check` and `dotnet run tools/roadmap.cs -- check`. All must be green; keep the output.
- [ ] **Step 2: Tick acceptance criteria with `evidence`:**
  1. **Artifact:** the URL, the list of concepts, and the combinations checked in Task 4 Step 4 (concepts × light/dark × 3 views).
  2. **Owner approval:** a pointer to the DECISIONS entry with the date and wording.
  3. **DESIGN.md and tokens:** `DESIGN.md` exists; `ui/src/main.ts` imports `styles/tokens.css`; `grep` on `ui/dist/assets/*.css` finds it; the commands and results from Step 1. macOS/Linux visual checks are explicitly open.
- [ ] **Step 3:** Set M1 `status` `done` with `completedOn`, then generate and check. Commit `docs: milestone 1 done`.
- [ ] **Step 4: Push, PR, merge.** Push and run `gh pr create --base dev` with title `feat: milestone 1 – UI design direction`. The body lists the gate results, the artifact link and what is open. Watch `gh pr checks --watch` until `roadmap`, `rust`, `ui`, `tauri-build` and `cargo-deny` are green; fix any failure at its cause. Then squash merge.
- [ ] **Step 5: Report** what is verified and what is open: the macOS/Linux visual check of the shell (`pnpm tauri dev`) and the icon on macOS and Linux.
