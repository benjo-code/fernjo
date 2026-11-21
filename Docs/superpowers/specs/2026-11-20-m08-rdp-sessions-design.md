# M8 RDP sessions: design decisions

Status: draft for owner approval, 2026-11-20. Scope, tasks and acceptance criteria are those of Milestone 8 in `Docs/roadmap.json`; the rendering path and the spike bar are those of the main spec, section 5.6. This note records only the decisions those leave open. The plan follows from all three.

## Intent

v0.2.0 makes RDP a first-class tab next to SSH: a connection with protocol `rdp` opens a desktop in the tab area, with NLA login from the vault, keyboard and mouse, text clipboard both ways, fit-to-tab resize and fullscreen, smooth enough for daily admin work. SSH stays the main use; RDP must not make it worse.

## Goals, non-goals, anti-goals

- Goals: the five tasks and three acceptance criteria of M8 as written in the roadmap.
- Non-goals: file or drive redirection, audio, printers, smart cards, multi-monitor, RD Gateway, saved `.rdp` files, VNC or other protocols (backlog "More protocols"), restore tabs after restart.
- Anti-goal: shipping v0.2.0 with slow RDP. If the bar is missed, the milestone records a scope change instead of lowering the bar (main spec 5.6).

## Constraints

- `fernjo-rdp` is created in this milestone, never depends on Tauri, and is testable without the UI (CLAUDE.md, architecture).
- Secrets stay in Rust: the RDP password and domain are resolved into a connect plan in Rust and never reach the webview or the logs; the no-secret table in `vault_commands.rs` covers every new command (three-places rule).
- Clipboard text crosses the webview by design, so it is treated as user data, not as a secret; it is never logged.
- Licences: IronRDP and its dependencies must pass `cargo deny check`; a new licence goes to the owner, as with `arboard`.
- UI follows `DESIGN.md` and the tokens, with the WebKitGTK 4.1 CSS limits from CLAUDE.md. No new visual direction.
- This machine has no working WebView2 runtime, no RDP host and only Windows, so every measurement and every real-host check is the owner's, on named machines. Code, unit tests and a headless integration test are mine.

## Approaches

- **A. One plan with a gate after the spike (picked, owner said "your recommendation").** Everything that does not depend on the render path is planned in full; the frame transport and canvas tasks are marked gated, each with what happens if the spike fails.
- B. Plan only the spike and the render-independent groundwork, plan the rest after measuring. Least rework, but a second planning round for work that is mostly unaffected by the result.
- C. Skip the spike. Breaks the spec and the roadmap.

## Decisions

### 1. Spike (my call)

- Two throwaway parts: (a) a headless cargo example in `crates/fernjo-rdp/examples/` that connects with IronRDP to a real host, drives a scripted scroll and window-drag load for a fixed time and reports decoded frames/s, bytes per frame and decode time per frame; (b) a bare page in the real app that receives synthetic 1920x1080 RGBA region updates over a Tauri channel and paints them to a canvas, reporting painted frames/s and the channel's per-message cost. Together they separate "IronRDP is too slow" from "the webview path is too slow".
- The owner runs (b) on Windows, macOS and Linux and (a) against one Windows host from at least one machine, and judges keystroke latency by typing into a real session in the prototype. Numbers go into `Docs/DECISIONS.md`. The spike code is deleted before the milestone's PR; only the result stays.
- Gate: at least 30 frames/s while scrolling and dragging on every OS, latency acceptable to the owner. Optimise inside the canvas path first (frame coalescing, dirty-region batching, binary channel payloads, `putImageData` or WebGL upload) before declaring a miss. A miss ends the build tasks marked gated and starts the scope-change task.

### 2. Settings and inheritance (my call)

- Four new optional fields on `Settings` in `fernjo-core`, flat like `keepalive_secs`: `domain`, `rdpResolution` (fit to tab, or fixed width and height), `rdpColorDepth` and `rdpClipboard`. Inherited node to folder to built-in default like every other setting; effective values and their source come from `resolve`, never from the UI.
- Domain precedence: the credential's domain wins over the node's domain setting when the credential has one (the vault entry already carries an optional domain).
- `connections.json` moves to `schemaVersion` 3 with a no-op migration, exactly as schema 2 did for `credential` (`one_to_two` in `persist.rs`): v0.1.1 would silently drop the new fields on its next save, so it must refuse a version 3 file as a newer schema instead. *(Resolved while planning; the first draft left this open.)*
- The mRemoteNG mapper already maps RDP connections; mapping its RDP flags (domain, resolution, depth) is in scope only for fields that have an equivalent here.

### 3. Session plumbing (my call)

- RDP sessions reuse the manager, tab model, prompts and close-all of M4 and M7 instead of a parallel stack. `ConnectPlan` becomes a protocol-tagged value; `connect_plan::resolve` builds the RDP variant and `rdp_is_refused` goes away.
- The terminal byte stream and the RDP frame stream are two payload kinds on the session channel. The wire contract at the top of `sessions.rs` is extended, not forked, and the decision goes into `Docs/DECISIONS.md` ("M8 session IPC").
- Certificate handling: the server certificate is shown on first use and pinned per host in the device-local folder, like SSH host keys but separate files; a changed certificate is a blocking prompt. Whether this reuses the host-key prompt component or gets its own is the builder's call.
- NLA failure, wrong password, host unreachable and server disconnect map to the existing end-of-session reasons (`disconnected` with a reason, Reconnect on the tab).

### 4. Tab behaviour (my call)

- Fit to tab: the canvas size follows the tab area; a resize is debounced and sent as a dynamic resize when the server supports it (display control), else the session keeps its negotiated size and scales to fit.
- Fullscreen hides the sidebar and tab bar of the window (an in-app view, not an OS fullscreen window) with a fixed escape key combination shown briefly; while a remote desktop has focus, `routeKey` forwards everything except that combination and the existing tab shortcuts that do not collide with Windows keys. The exact set is a plan detail.
- Clipboard: text only, both directions, gated by the setting; the webview side uses the paste and copy events and the async clipboard API where the permission works, and the plan records the WebView2 permission-prompt problem from "M4 clipboard path" as a known risk.

### 5. Plan shape (my call)

One plan file. Phases: (1) groundwork that does not depend on the spike: `fernjo-rdp` crate with an IronRDP connect and NLA test against a throwaway RDP container or the owner's host, RDP settings in core with inheritance and persistence tests; (2) the spike and an owner gate (measure, decide, record); (3) gated build: frame transport, canvas, input, resize, fullscreen, clipboard, session plumbing, tab UI, settings UI; (4) docs, roadmap, release evidence for v0.2.0. Phase 2 stops for the owner's numbers; the plan says so explicitly. If the gate fails, phase 3 is replaced by the scope-change task and a new milestone.

## Left to the builder

- IronRDP crate set and version (pinned; checked against `cargo deny` and the MSRV of the pinned toolchain).
- Frame message layout, dirty-region batching and the size at which a full frame is sent instead of regions.
- Whether the CI integration test uses an RDP server container; if none is reliable, the headless test is an owner check marked open with the command that runs it.
- Keyboard mapping details (scancodes versus key codes, layout handling) as long as the common Windows shortcuts work in the session.
