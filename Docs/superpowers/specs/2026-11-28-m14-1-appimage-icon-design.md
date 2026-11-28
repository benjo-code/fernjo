# M14.1: Linux AppImage icon in the dock and start menu

**Date:** 2026-11-28 · **Status:** awaiting the owner's approval · **Size:** small change

## Intent

After "Add" in the application menu dialog (v0.4.0, KDE Plasma) the start menu entry and the dock showed a blank icon, which looks like a fault. Users should know that the icon can take until the next log-in to appear.

## Diagnosis (owner's laptop, 2026-11-28)

- Plasma 6.8.3, Wayland, `XDG_DATA_HOME` unset.
- The written `fernjo.desktop` is what `desktop_entry` produces; the icon is at `~/.local/share/icons/hicolor/128x128/apps/fernjo.png`.
- After logging out and in again, the start menu and the dock show the icon, with the files unchanged.
- `hicolor/128x128/apps` existed before the icon was written, so a newly created folder is not the cause. The listing of `hicolor` shows no `icon-theme.cache`.
- Conclusion: the entry and the icon are correct; the running Plasma session did not pick the new icon up. Why it did not is not established.
- Comparison with an AppImage integrated by another tool (Gear Lever): not done; the log-in test made it unnecessary.

## Approaches

| Approach | Trade-off | Verdict |
|---|---|---|
| A. Absolute path in `Icon=` | Small and independent of the desktop; would probably show at once, untested | Not chosen |
| B. Send KDE an "icons changed" message after writing | Runs an external program from the AppImage, KDE only, effect untested | Not chosen |
| C. Say that the icon may appear only after the next log-in | Almost no risk; the blank icon stays until then | **Chosen by the owner, 2026-11-28** |

## Design

- **Dialog** (`src-tauri/src/app_menu.rs`): a new paragraph after the one that asks to add fernjo: "The icon may appear in the start menu and dock only after you log out and in again."
- **Same dialog, fix on the way:** the current message has runs of ten spaces in the middle of sentences (lost line continuations in the string). They go.
- **Testability:** the message is built by a pure function, tested on every OS (the note is present, no double spaces, folder and AppImage path are named).
- **README** (Linux install paragraph): after "Add" the start menu and dock may show a blank icon until the next log-in (seen on KDE Plasma 6.8.3, Wayland).
- **Records:** `CHANGELOG.md` `[Unreleased]`, `Docs/DECISIONS.md` (cause, choice, rejected approaches), `CLAUDE.md` (replaces "the cause is not known").
- **Roadmap:** scope change. The code fix and the update of older installations are dropped; goal, acceptance criteria and the v0.5.0 release text follow.

## Non-goals and anti-goals

- No change to what is written, where, or when: the entry text stays byte-identical, so nobody who chose "Add" with v0.4.0 or earlier is asked again (a test pins the text). Their icon appears with the next log-in.
- No external programs, no cache refresh, no extra icon sizes.
- No claim that the cause is a cache.

## Acceptance

- Build, test and lint are green; `dotnet run tools/roadmap.cs -- check` passes.
- Tests pin the dialog text and the unchanged entry text; the dialog test fails without the change.
- Owner, KDE Plasma: an AppImage built from the milestone branch, started with `XDG_CONFIG_HOME` and `XDG_DATA_HOME` pointing at scratch folders, shows the dialog with the note and clean text; answered "Not now", so no real files change. That the icon appears after a log-in is already shown by the diagnosis above.
- GNOME: recorded as not checked.
