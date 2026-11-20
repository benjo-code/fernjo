# M7 discoverability review

Review of the whole app (Windows, release build of `feature/m07-polish-hardening`, window 1280x800, scratch data folder, light and dark). Screenshots are in `m07-review/`; the figures named below are the light variants, each has a `-dark` twin. Constraints: `DESIGN.md` and the tokens stay as they are, no new visual direction, every proposal reuses existing components.

## Summary for the owner

1. The vault has no name where you look for it. The chip says only "Unlocked" with a padlock, the manager it opens is titled "Credentials", and the Credential property offers only a dropdown with "None" (no way to the manager). Proposals D1, D2 and D3 fix this together.
2. The two footer buttons (Settings gear, Import arrow) are icon-only and 28 px small. The second one is called "Import & backup" but also holds Export and Restore (D4).
3. A first run shows three different empty hints at once (vault note, tree offer, "Open a connection from the tree") and no order of steps (D6).
4. The properties panel opens with only two rows visible at 1280x800, so the Credential row is below the fold (D5).
5. A dropped tab hides its reason, and no shortcut or multi-select hint exists anywhere in the UI (D9, D10, D11).

How this was made: states captured over the WebView DevTools port only (no OS input) against a throwaway data folder and the throwaway SSH container; the critique ran the `impeccable` skill's critique command in a single context (no sub-agents available to this task, so the isolated A/B split was not used: degraded run). Its detector found nothing relevant (a font in a test file, and the 3 px selected-row bar that `DESIGN.md` defines as `selection-bar`); nothing from it is turned into a finding. Not captured: macOS and Linux (not available here), native `title` tooltips (not rendered in screenshots; judged from source), keyboard focus rings, a vault in the `unusable`, `rolled_back` or auto-unlock-issue states, a changed host key prompt, filter with no matches.

## Findings

**D1. The vault chip does not say "vault".** `m07-review/09-vault-unlocked-credential-row-light.png`, `13-vault-locked-light.png`, `01-first-run-light.png`. The chip reads "Unlocked", "Locked" or "No vault" next to a padlock; a reader sees an unlocked something. There is no `title`, only an `aria-label`. Change: label "Vault: unlocked" / "Vault: locked" / "Vault: none" (keep the padlock), add `title="Credential vault"` (unlocked: "Credential vault, open the credentials"). At 300 px width this fits if the filter box gives up a few pixels (the chip is `flex: none`; check at the minimum sidebar width, otherwise use "Vault" plus the state as the second word only in the locked and none states). Size S. Files: `VaultChip.svelte`, `VaultChip.test.ts`.

**D2. The manager is titled "Credentials" and nothing connects it to the chip.** `10-vault-manager-empty-light.png`, `12-vault-manager-with-credential-light.png`. After clicking "Unlocked" the main area shows "Credentials", "0 stored", "+ New". Change: title "Vault", with "Credentials" as the list heading ("1 credential" instead of "1 stored"); empty state "No credentials yet. Create one with + New and pick it on a connection's Credential property." Keep the buttons Lock, Change password, Close. Size S. Files: `VaultManager.svelte`, `VaultManager.test.ts`.

**D3. No path from a connection's Credential property to the vault.** `03-properties-credential-row-no-vault-light.png`, `09-vault-unlocked-credential-row-light.png`. Without a vault the row is the plain text "No vault", locked it is "Vault locked", unlocked it is a dropdown with "None" only, even when no credential exists. All three are dead ends. Change: next to the value add a small text button (existing `btn small`): "Create vault" (none, focuses the create form), "Unlock" (locked, focuses the unlock form), "Manage…" (unlocked, opens the manager); in the dropdown add a last entry "New credential…" that opens the manager on a new entry. Size M. Files: `PropertiesPanel.svelte`, `PropertiesPanel.test.ts`, `App.svelte` and `Sidebar.svelte` (pass the open-vault and focus-form callbacks down).

**D4. Footer buttons are icon-only and the import button is misnamed.** `02-tree-properties-no-vault-light.png` (bottom left), `05-import-and-backup-light.png`. A gear and a download arrow, 28 px, no text; the arrow also suggests "export". The dialog it opens has Import, Export and Restore. Change: show text next to the icons ("Settings", "Import / Export") in the footer, which has the width; rename the dialog title and `aria-label`/`title` to "Import, export & backup"; rename the empty-tree button consistently ("Import from mRemoteNG…" stays, it is specific). Size S. Files: `Sidebar.svelte`, `ImportDialog.svelte`, `ImportDialog.test.ts`, `Sidebar` related tests.

**D5. The properties panel starts too small for a connection.** `02-tree-properties-no-vault-light.png`, `03-properties-credential-row-no-vault-light.png`. With the tree at its minimum height the panel shows the header and two rows (Name, Description) and a thin scrollbar; Host, Username and Credential are below. Change: raise the default panel height so Name, Description, Protocol and Host show (about 6 rows), or order the rows by use: Name, Host, Port, Username, Credential, then Description, Keepalive, Protocol. The tree keeps its 23 rows promise only at larger windows, so check 1280x800 stays usable. Size M. Files: `PropertiesPanel.svelte`, `layout.svelte.ts`, `PropertiesPanel.test.ts`, `layout` test.

**D6. First run has no order of steps.** `01-first-run-light.png`. Three separate texts compete: the vault note ("No vault yet", Create vault), the tree offer ("No connections yet", New folder, New connection, Import from mRemoteNG…) and the main area ("Open a connection from the tree", which cannot be done yet). The chip repeats the note ("No vault" next to "No vault yet"). Change: while the tree is empty replace the main-area line with a short start list using existing `btn`s: "1. Add a connection (New connection / Import from mRemoteNG…)", "2. Optional: create a vault to store logins". Keep the note and tree offer; hide the tree offer's own text duplication by leaving only the buttons. Size M. Files: `App.svelte`, `TreeView.svelte`, tests.

**D7. Vault wording is vague.** `14-vault-locked-unlock-form-light.png`, `18-connect-needs-unlocked-vault-light.png`, `08-vault-create-form-light.png`. "Connections with stored credentials ask for it." leaves "it" undefined; the prompt title "Needs vault" and its heading "test-container needs the vault" say the same twice; "Unlock automatically on this device" gives its explanation only after it is ticked. Change: note body "Connections that use a stored credential need the vault unlocked."; prompt tab caption "Unlock vault", heading "Unlock the vault to connect to test-container"; checkbox hint always visible: "Keeps the key in the OS keychain so the vault unlocks at start." (the existing "Lock then lasts until the next start." follows when ticked). Size S. Files: `VaultNote.svelte`, `NeedsVaultPrompt.svelte`, their tests.

**D8. Settings mixes live and saved controls.** `04-settings-light.png`. Appearance applies at once, the Terminal fields need the Save button below them, the "Close the tab when the shell exits" box saves by itself, and the same "Applies to this run only; settings are not saved while FERNJO_DATA_DIR is set" sentence appears twice (that case is developer-only, but the layout stays). Change: label the Terminal group's button "Save terminal settings", and add one hint line "Appearance and the checkbox apply at once." Size S. Files: `SettingsDialog.svelte`, `SettingsDialog.test.ts`.

**D9. A dropped tab hides why it dropped.** `17-disconnected-tab-light.png`. After `exit 3` the tab shows a red mark, the title, a one-letter caption ("T", the start of "The remote shell ended (exit status 3)", cut off by the Reconnect button) and Reconnect; the reason is only in the tooltip. Change: drop the caption for the disconnected state and show the reason as a one-line bar at the top of the terminal area ("The remote shell ended (exit status 3)." with the Reconnect button repeated), so the tab keeps its title. Size M. Files: `TabStrip.svelte`, `SessionArea.svelte`, `TabStrip.test.ts`, `SessionArea.test.ts`.

**D10. The context menu shows no shortcuts.** `06-context-menu-light.png`. Connect, New folder, New connection, Duplicate, Rename, Delete: Enter, F2 and Delete work but are listed nowhere. Change: right-aligned muted hint text in the items that have a key (Connect Enter, Rename F2, Delete Del); no hint for the others. Size S. Files: `ContextMenu.svelte`, `TreeView.svelte`, `ContextMenu.test.ts`.

**D11. Shortcuts and multi-select are not discoverable.** `07-multi-select-light.png`, `01-first-run-light.png`. Ctrl+Shift+K (filter), Ctrl+Tab and Ctrl+Shift+Tab (tabs), Ctrl+Shift+W (close tab), Ctrl+Shift+F (terminal search), Cmd variants on macOS, and Ctrl/Cmd or Shift+click to select several appear nowhere. Change: a read-only "Keyboard and mouse" section at the end of Settings (platform-correct via the existing `isMac()`), plus `title="Filter connections (Ctrl+Shift+K)"` on the filter box. The properties line "Select a single item to see its settings." already works as the multi-select hint. Size S to M. Files: `SettingsDialog.svelte`, `FilterBox.svelte`, `shortcuts.ts` (export the list), tests.

**D12. "Trust and connect" versus "Trust once" is not explained.** `15-host-key-prompt-light.png`. Both buttons carry the decision, the difference is the only thing a first-time user has to guess. Change: one muted line under the buttons: "Trust and connect remembers this key on this device. Trust once asks again next time." Size S. Files: `HostKeyPrompt.svelte` (and the synced prompt's equivalent text if it has the same buttons), tests.

**D13. Tree rows show no hint that double-click or Enter opens.** `02-tree-properties-no-vault-light.png`. The main area says "Open a connection from the tree" without saying how. Change: "Double-click a connection (or press Enter) to open it." Size S. Files: `App.svelte`.

## Answer table

| Item | Size | Files |
|---|---|---|
| D1 Vault chip label and tooltip | S | `VaultChip.svelte`, test |
| D2 Manager title and empty state | S | `VaultManager.svelte`, test |
| D3 Credential row: create, unlock, manage, new | M | `PropertiesPanel.svelte`, `App.svelte`, `Sidebar.svelte`, tests |
| D4 Footer labels, "Import, export & backup" | S | `Sidebar.svelte`, `ImportDialog.svelte`, tests |
| D5 Properties panel default height or row order | M | `PropertiesPanel.svelte`, `layout.svelte.ts`, tests |
| D6 First-run start list | M | `App.svelte`, `TreeView.svelte`, tests |
| D7 Vault wording (note, prompt, checkbox hint) | S | `VaultNote.svelte`, `NeedsVaultPrompt.svelte`, tests |
| D8 Settings: live versus saved controls | S | `SettingsDialog.svelte`, test |
| D9 Dropped tab shows its reason | M | `TabStrip.svelte`, `SessionArea.svelte`, tests |
| D10 Shortcut hints in the context menu | S | `ContextMenu.svelte`, `TreeView.svelte`, test |
| D11 Shortcut list in Settings, filter tooltip | S-M | `SettingsDialog.svelte`, `FilterBox.svelte`, `shortcuts.ts`, tests |
| D12 Host key buttons explained | S | `HostKeyPrompt.svelte`, test |
| D13 How to open a connection | S | `App.svelte` |
