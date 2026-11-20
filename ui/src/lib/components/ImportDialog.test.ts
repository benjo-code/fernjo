import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, describe, expect, test, vi } from "vitest";
import { ImportStore } from "../import.svelte";
import { SessionStore } from "../sessions.svelte";
import { TreeStore } from "../store.svelte";
import {
  apiError,
  conn,
  credential,
  deferred,
  mockApi,
  preview,
  report,
  vaultStatus,
} from "../testing";
import type { Api, ApplyResult, ImportPreview, PickedFile, TreeNode } from "../types";
import { VaultStore } from "../vault.svelte";
import ImportDialog from "./ImportDialog.svelte";
import TreeView from "./TreeView.svelte";

let mounted: ReturnType<typeof mount> | null = null;

afterEach(() => {
  if (mounted) unmount(mounted);
  mounted = null;
  document.body.innerHTML = "";
});

const settle = async () => {
  await new Promise((r) => setTimeout(r, 0));
  await tick();
  flushSync();
};

async function setup(impl: Partial<Api> = {}, children: TreeNode[] = []) {
  const m = mockApi(children, impl);
  const tree = new TreeStore(m.api);
  await tree.init();
  const vault = new VaultStore(m.api);
  await vault.init();
  const store = new ImportStore(m.api, (s) => void tree.applySnapshot(s));
  const onclose = vi.fn();
  mounted = mount(ImportDialog, { target: document.body, props: { store, vault, tree, onclose } });
  await settle();
  return { ...m, store, tree, vault, onclose };
}

const button = (label: string) =>
  [...document.querySelectorAll("button")].find((b) => b.textContent?.trim() === label)!;
const alert = () => document.querySelector("[role=alert]");
const input = (id: string) => document.getElementById(id) as HTMLInputElement;
const type = (el: HTMLInputElement, value: string) => {
  el.value = value;
  el.dispatchEvent(new Event("input", { bubbles: true }));
};
const click = async (label: string) => {
  button(label).click();
  await settle();
};
const escape = () =>
  document.activeElement!.dispatchEvent(
    new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
  );

const withCounts = (credentials = 0) =>
  report({
    counts: {
      folders: 2,
      ssh: 3,
      rdp: 1,
      credentials,
      sharedCredentials: 0,
      skipped: 1,
      partial: 1,
      problems: 0,
    },
    skipped: [{ path: "Lab/old", text: "unsupported protocol VNC" }],
    partial: [{ path: "Lab/web", text: "gateway not mapped" }],
  });

const applied = (): ApplyResult => ({
  snapshot: { children: [conn("imported")], notices: [] },
  report: withCounts(),
  restored: null,
});

describe("ImportDialog", () => {
  // the plan said "six actions"; the three groups hold five (2 + 2 + 1), plus Close
  test("dialog_lists_the_five_actions_and_close", async () => {
    await setup();
    const dialog = document.querySelector("[role=dialog]")!;
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    expect(document.querySelector("h2")?.textContent).toBe("Import, export & backup");
    const labels = [...dialog.querySelectorAll("button")].map((b) => b.textContent?.trim());
    expect(labels).toEqual([
      "mRemoteNG file…",
      "fernjo connections file…",
      "Connections (JSON)",
      "Full backup…",
      "Backup file…",
      "Close",
    ]);
    const groups = [...dialog.querySelectorAll("h3")].map((h) => h.textContent);
    expect(groups).toEqual(["Import", "Export", "Restore"]);
  });

  test("mremoteng_flow_shows_preview_counts_then_result", async () => {
    const { calls, tree } = await setup({
      importInspect: async () => preview({ report: withCounts() }),
      importApply: async () => applied(),
    });
    await click("mRemoteNG file…");
    expect(calls.importPick).toEqual([["mremoteng"]]);
    const text = document.body.textContent!;
    expect(text).toContain("confCons.xml");
    expect(text).toContain("2 folders, 4 connections (3 SSH, 1 RDP)");
    // lists exist but are collapsed
    expect(document.body.textContent).not.toContain("unsupported protocol VNC");
    expect(calls.importApply).toHaveLength(0);
    await click("Import");
    expect(calls.importApply).toEqual([[{ withoutPasswords: false }]]);
    expect(document.body.textContent).toContain("2 folders, 4 connections");
    expect(tree.children.map((n) => n.name)).toEqual(["imported"]);
    // back to the groups, buttons usable again
    expect(button("Backup file…")).toBeTruthy();
  });

  test("custom_key_prompt_keeps_the_field_and_shows_the_wrong_key_message", async () => {
    const seen: (string | null)[] = [];
    await setup({
      importInspect: async (secret) => {
        seen.push(secret);
        if (secret === null) throw apiError("key_needed", "This file needs a key.");
        if (secret === "bad") throw apiError("wrong_key", "That key does not open the file.");
        return preview({ report: withCounts() });
      },
    });
    await click("mRemoteNG file…");
    expect(input("import-secret")).not.toBeNull();
    expect(alert()).toBeNull();
    type(input("import-secret"), "bad");
    await click("Continue");
    expect(seen).toEqual([null, "bad"]);
    expect(alert()?.textContent).toContain("That key does not open the file.");
    // the field stays and is empty again (the typed secret is not kept)
    expect(input("import-secret")).not.toBeNull();
    expect(input("import-secret").value).toBe("");
    type(input("import-secret"), "good");
    await click("Continue");
    expect(document.body.textContent).toContain("2 folders, 4 connections");
    expect(input("import-secret")).toBeNull();
  });

  test("a_wrong_key_puts_the_cursor_back_in_the_key_field", async () => {
    await setup({
      importInspect: async (secret) => {
        if (secret === null) throw apiError("key_needed", "This file needs a key.");
        throw apiError("wrong_key", "That key does not open the file.");
      },
    });
    await click("mRemoteNG file…");
    type(input("import-secret"), "bad");
    // the focused button is the one that was clicked, as in the real dialog
    button("Continue").focus();
    await click("Continue");
    await settle();
    expect(alert()?.textContent).toContain("That key does not open the file.");
    expect(document.activeElement).toBe(input("import-secret"));
  });

  test("skipped_and_partial_lists_are_collapsed_until_opened", async () => {
    await setup({ importInspect: async () => preview({ report: withCounts() }) });
    await click("mRemoteNG file…");
    const skipped = button("▸ Skipped (1)");
    expect(skipped.getAttribute("aria-expanded")).toBe("false");
    expect(document.body.textContent).not.toContain("gateway not mapped");
    skipped.click();
    await settle();
    expect(document.body.textContent).toContain("Lab/old");
    expect(document.body.textContent).toContain("unsupported protocol VNC");
    expect(document.body.textContent).not.toContain("gateway not mapped");
    expect(document.body.textContent).not.toContain("Problems");
    button("▸ Partly mapped (1)").click();
    await settle();
    expect(document.body.textContent).toContain("gateway not mapped");
  });

  test("restore_confirmation_names_counts_and_has_cancel_focused", async () => {
    const children: TreeNode[] = [conn("a"), conn("b"), conn("c")];
    const p: ImportPreview = preview({
      kind: "backup",
      fileName: "fernjo-backup.fernjo",
      report: null,
      backup: {
        createdAt: "2026-11-01T10:00:00Z",
        app: "fernjo 0.5.0",
        folders: 1,
        connections: 4,
        credentials: 2,
      },
    });
    const { calls } = await setup(
      {
        importInspect: async (secret) => {
          if (secret === null) throw apiError("password_needed", "Needs a password.");
          return p;
        },
        importApply: async () => ({
          ...applied(),
          report: null,
          restored: {
            createdAt: "2026-11-01T10:00:00Z",
            folders: 1,
            connections: 4,
            credentials: 2,
          },
        }),
      },
      children,
    );
    await click("Backup file…");
    type(input("import-secret"), "correct horse");
    await click("Continue");
    expect(calls.importApply).toHaveLength(0);
    await click("Restore…");
    const confirm = document.querySelector("[role=alertdialog]")!;
    expect(confirm.textContent).toContain(
      "This replaces your current 3 folders and connections with the backup from 2026-11-01, which has 5 folders and connections and 2 credentials. Open sessions are closed.",
    );
    const buttons = [...confirm.querySelectorAll("button")];
    expect(document.activeElement).toBe(buttons[0]);
    expect(buttons[0].textContent?.trim()).toBe("Cancel");
    expect(buttons[1].classList.contains("primary")).toBe(false);
    // cancel keeps the preview
    buttons[0].click();
    await settle();
    expect(document.querySelector("[role=alertdialog]")).toBeNull();
    expect(calls.importApply).toHaveLength(0);
    await click("Restore…");
    [...document.querySelectorAll("[role=alertdialog] button")][1].dispatchEvent(
      new MouseEvent("click", { bubbles: true }),
    );
    await settle();
    expect(calls.importApply).toEqual([[{ withoutPasswords: false }]]);
    expect(document.body.textContent).toContain("Restored");
  });

  /** `vault` is the state Rust reports with the preview (what the dialog decides on). */
  const backupPreview = (
    needsVault = false,
    vault: "none" | "locked" | "unlocked" = "none",
  ): ImportPreview =>
    preview({
      kind: "backup",
      fileName: "b.fernjo",
      report: null,
      needsVault,
      vault: vaultStatus({ state: vault }),
      backup: {
        createdAt: "2026-11-01T10:00:00Z",
        app: "fernjo 0.5.0",
        folders: 1,
        connections: 1,
        credentials: 1,
      },
    });
  const restored = (): ApplyResult => ({
    ...applied(),
    report: null,
    restored: { createdAt: "2026-11-01T10:00:00Z", folders: 1, connections: 1, credentials: 1 },
  });

  test("restore_confirmation_counts_current_credentials_only_when_the_vault_is_unlocked", async () => {
    await setup({
      vaultStatus: async () => vaultStatus({ state: "unlocked" }),
      listCredentials: async () => [credential({ id: "c1" })],
      importPick: async () => ({ fileName: "b.fernjo" }),
      importInspect: async () => backupPreview(true, "unlocked"),
    });
    await click("Backup file…");
    await click("Restore…");
    expect(document.querySelector("[role=alertdialog]")!.textContent).toContain(
      "This replaces your current 0 folders and connections and 1 credential with the backup from 2026-11-01, which has 2 folders and connections and 1 credential.",
    );
  });

  test("restore_confirmation_does_not_claim_zero_credentials_for_a_locked_vault", async () => {
    await setup({
      vaultStatus: async () => vaultStatus({ state: "locked" }),
      importInspect: async () => backupPreview(false, "locked"),
    });
    await click("Backup file…");
    await click("Restore…");
    const text = document.querySelector("[role=alertdialog]")!.textContent!;
    expect(text).not.toContain("0 credentials");
    expect(text).toContain("The credentials in the vault are replaced too.");
  });

  test("a_stale_ui_vault_status_cannot_skip_the_vault_form_of_a_restore", async () => {
    // the UI store believes the vault is unlocked; Rust said "locked" when it inspected
    const { vault } = await setup({
      vaultStatus: async () => vaultStatus({ state: "unlocked" }),
      importPick: async () => ({ fileName: "b.fernjo" }),
      importInspect: async () => ({
        ...backupPreview(true),
        vault: vaultStatus({ state: "locked" }),
      }),
    });
    expect(vault.status?.state).toBe("unlocked");
    await click("Backup file…");
    expect(button("Restore…")).toBeUndefined();
    expect(input("import-vault-password")).not.toBeNull();
    expect(document.body.textContent).toContain("unlock it first");
  });

  test("restore_preview_unlocks_the_vault_before_the_confirmation", async () => {
    const { calls } = await setup({
      vaultStatus: async () => vaultStatus({ state: "locked" }),
      vaultUnlock: async () => vaultStatus({ state: "unlocked" }),
      importInspect: async () => backupPreview(true, "locked"),
      importApply: async () => restored(),
    });
    await click("Backup file…");
    expect(button("Restore…")).toBeUndefined();
    expect(button("Import without passwords")).toBeUndefined();
    type(input("import-vault-password"), "master");
    await click("Unlock");
    expect(calls.vaultUnlock[0][0]).toBe("master");
    expect(calls.importApply).toHaveLength(0);
    await click("Restore…");
    expect(document.querySelector("[role=alertdialog]")!.textContent).toContain("1 credential");
  });

  test("restore_with_a_locked_vault_uses_restore_wording_unlocks_and_applies", async () => {
    let attempts = 0;
    const { calls } = await setup({
      vaultStatus: async () => vaultStatus({ state: "locked" }),
      vaultUnlock: async () => vaultStatus({ state: "unlocked" }),
      importInspect: async () => backupPreview(false, "locked"),
      importApply: async () => {
        attempts++;
        if (attempts === 1) throw apiError("vault_needed", "Unlock the vault first.");
        return restored();
      },
    });
    await click("Backup file…");
    await click("Restore…");
    await click("Restore");
    expect(calls.importApply).toHaveLength(1);
    expect(document.body.textContent).toContain("A restore replaces the vault");
    expect(document.body.textContent).not.toContain("Imported passwords");
    expect(button("Import without passwords")).toBeUndefined();
    type(input("import-vault-password"), "master");
    await click("Unlock and restore");
    expect(calls.importApply).toHaveLength(2);
    expect(document.body.textContent).toContain("Restored 2 folders and connections");
  });

  test("import_without_passwords_sends_the_flag_and_is_hidden_without_needs_vault", async () => {
    const { calls } = await setup({
      importInspect: async () => preview({ report: withCounts(2), needsVault: true }),
      importApply: async () => applied(),
    });
    await click("mRemoteNG file…");
    await click("Import without passwords");
    expect(calls.importApply).toEqual([[{ withoutPasswords: true }]]);
    unmount(mounted!);
    mounted = null;
    document.body.innerHTML = "";
    await setup({ importInspect: async () => preview({ report: withCounts(0) }) });
    await click("mRemoteNG file…");
    expect(button("Import without passwords")).toBeUndefined();
  });

  test("export_backup_needs_matching_passwords_of_8_chars", async () => {
    const { calls } = await setup({
      exportBackup: async () => ({ fileName: "b.fernjo", droppedCredentials: 0 }),
    });
    await click("Full backup…");
    const save = () => button("Save backup…");
    expect(save().getAttribute("aria-disabled")).toBe("true");
    type(input("backup-password"), "short");
    type(input("backup-repeat"), "short");
    await settle();
    expect(save().getAttribute("aria-disabled")).toBe("true");
    type(input("backup-password"), "long enough 1");
    type(input("backup-repeat"), "long enough 2");
    await settle();
    expect(save().getAttribute("aria-disabled")).toBe("true");
    save().click();
    await settle();
    expect(calls.exportBackup).toHaveLength(0);
    type(input("backup-repeat"), "long enough 1");
    await settle();
    expect(save().getAttribute("aria-disabled")).toBe("false");
    save().click();
    await settle();
    expect(calls.exportBackup).toEqual([["long enough 1"]]);
    expect(document.body.textContent).toContain("b.fernjo");
    // the password is not kept
    expect(input("backup-password")).toBeNull();
  });

  test("a_password_too_short_error_from_rust_shows_inline_and_clears_the_fields", async () => {
    await setup({
      exportBackup: async () => {
        throw apiError("password_too_short", "Use at least 8 characters.");
      },
    });
    await click("Full backup…");
    type(input("backup-password"), "12345678");
    type(input("backup-repeat"), "12345678");
    await settle();
    await click("Save backup…");
    expect(alert()?.textContent).toContain("Use at least 8 characters.");
    expect(input("backup-password").value).toBe("");
  });

  test("vault_unlock_rolled_back_points_to_the_vault_note", async () => {
    let attempts = 0;
    const { calls } = await setup({
      vaultStatus: async () => vaultStatus({ state: "locked" }),
      vaultUnlock: async () => {
        throw apiError("rolled_back", "The vault file is older than the last version.");
      },
      importInspect: async () =>
        preview({
          report: withCounts(2),
          needsVault: true,
          vault: vaultStatus({ state: "locked" }),
        }),
      importApply: async () => {
        attempts++;
        throw apiError("vault_needed", "Unlock the vault first.");
      },
    });
    await click("mRemoteNG file…");
    await click("Import");
    type(input("import-vault-password"), "master");
    await click("Unlock and import");
    expect(calls.vaultUnlock).toHaveLength(1);
    expect(document.body.textContent).toContain(
      "The vault file is older than the last version. Unlock from the vault note to review it.",
    );
    expect(attempts).toBe(1);
  });

  test("vault_needed_step_offers_unlock_and_resumes_the_apply", async () => {
    let attempts = 0;
    const { calls, vault } = await setup({
      vaultStatus: async () => vaultStatus({ state: "locked" }),
      vaultUnlock: async () => vaultStatus({ state: "unlocked" }),
      importInspect: async () =>
        preview({
          report: withCounts(2),
          needsVault: true,
          vault: vaultStatus({ state: "locked" }),
        }),
      importApply: async () => {
        attempts++;
        if (attempts === 1) throw apiError("vault_needed", "Unlock the vault first.");
        return applied();
      },
    });
    await click("mRemoteNG file…");
    await click("Import");
    expect(calls.importApply).toHaveLength(1);
    expect(button("Unlock and import")).toBeTruthy();
    type(input("import-vault-password"), "master");
    await click("Unlock and import");
    expect(calls.vaultUnlock[0][0]).toBe("master");
    expect(vault.status?.state).toBe("unlocked");
    expect(calls.importApply).toEqual([
      [{ withoutPasswords: false }],
      [{ withoutPasswords: false }],
    ]);
    expect(document.body.textContent).toContain("2 folders, 4 connections");
  });

  test("a_stale_plan_error_offers_start_over", async () => {
    const { store } = await setup({
      importInspect: async () => preview({ report: withCounts() }),
      importApply: async () => {
        throw apiError("no_pending_import", "There is no pending import.");
      },
    });
    await click("mRemoteNG file…");
    await click("Import");
    expect(alert()?.textContent).toBe("There is no pending import.");
    await click("Start over");
    expect(store.step).toBe("idle");
    expect(alert()).toBeNull();
  });

  test("escape_cancels_a_pending_import_via_import_cancel", async () => {
    const { calls, onclose, store } = await setup({
      importInspect: async () => preview({ report: withCounts() }),
    });
    await click("mRemoteNG file…");
    expect(store.step).toBe("preview");
    escape();
    await settle();
    expect(calls.importCancel).toHaveLength(1);
    expect(onclose).toHaveBeenCalledTimes(1);
    expect(store.step).toBe("idle");
  });

  test("closing_always_tells_rust_to_drop_what_is_pending", async () => {
    // harmless when nothing is pending, and the only safe thing when the store cannot know
    const { calls, onclose } = await setup();
    escape();
    expect(calls.importCancel).toHaveLength(1);
    expect(onclose).toHaveBeenCalledTimes(1);
  });

  test("closing_after_a_failed_apply_still_drops_the_plan_in_rust", async () => {
    // import_apply keeps the plan (with every decrypted password) on most errors, and the error
    // step has no retry: closing must clear it
    const { calls, onclose, store } = await setup({
      importInspect: async () => preview({ report: withCounts() }),
      importApply: async () => {
        throw apiError("save_failed", "could not write connections.json");
      },
    });
    await click("mRemoteNG file…");
    await click("Import");
    expect(store.step).toBe("error");
    expect(calls.importCancel).toHaveLength(0);
    escape();
    await settle();
    expect(calls.importCancel).toHaveLength(1);
    expect(onclose).toHaveBeenCalledTimes(1);
  });

  test("closing_while_the_file_is_still_being_picked_cancels_again_when_the_pick_arrives", async () => {
    // the native picker has no parent window: it can still be open when the dialog closes
    const pick = deferred<PickedFile | null>();
    const { calls, onclose, store } = await setup({ importPick: () => pick.promise });
    await click("mRemoteNG file…");
    escape();
    await settle();
    expect(calls.importCancel).toHaveLength(1);
    expect(onclose).toHaveBeenCalledTimes(1);
    pick.resolve({ fileName: "confCons.xml" });
    await settle();
    // Rust now remembers the picked path: the store must drop it again and not inspect it
    expect(calls.importInspect).toHaveLength(0);
    expect(calls.importCancel).toHaveLength(2);
    expect(store.step).toBe("idle");
  });

  test("actions_show_the_busy_state", async () => {
    const d = deferred<PickedFile | null>();
    const { calls } = await setup({ importPick: () => d.promise });
    await click("mRemoteNG file…");
    expect(button("Backup file…").getAttribute("aria-disabled")).toBe("true");
    button("Backup file…").click();
    await settle();
    expect(calls.importPick).toHaveLength(1);
    d.resolve(null);
    await settle();
    expect(button("Backup file…").getAttribute("aria-disabled")).toBe("false");
  });

  test("a_throwing_snapshot_callback_is_not_an_unhandled_rejection", async () => {
    const m = mockApi([], {
      importInspect: async () => preview({ report: withCounts() }),
      importApply: async () => applied(),
    });
    const tree = new TreeStore(m.api);
    const vault = new VaultStore(m.api);
    const store = new ImportStore(m.api, () => {
      throw new Error("boom");
    });
    mounted = mount(ImportDialog, {
      target: document.body,
      props: { store, vault, tree, onclose: () => {} },
    });
    await settle();
    await click("mRemoteNG file…");
    await click("Import");
    expect(store.step).toBe("done");
  });

  test("empty_tree_offer_opens_the_dialog", async () => {
    const m = mockApi([]);
    const tree = new TreeStore(m.api);
    await tree.init();
    const onimport = vi.fn();
    mounted = mount(TreeView, {
      target: document.body,
      props: {
        store: tree,
        sessions: new SessionStore(
          m.api,
          (id) => id,
          () => true,
        ),
        onimport,
      },
    });
    flushSync();
    expect(button("New folder")).toBeTruthy();
    button("Import from mRemoteNG…").click();
    expect(onimport).toHaveBeenCalledTimes(1);
  });

  // a browser focuses a clicked button; the next step then removes it and focus falls to body
  const focusClick = async (label: string) => {
    const b = button(label);
    b.focus();
    b.click();
    await settle();
  };
  const dialogEl = () => document.querySelector<HTMLElement>("[role=dialog]")!;
  const tab = () =>
    document.activeElement!.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }),
    );

  test("focus_stays_in_the_dialog_when_a_clicked_button_is_removed", async () => {
    const d = deferred<ApplyResult>();
    const { calls, onclose } = await setup({
      importInspect: async () => preview({ report: withCounts() }),
      importApply: () => d.promise,
    });
    await focusClick("mRemoteNG file…");
    expect(dialogEl().contains(document.activeElement)).toBe(true);
    tab();
    expect(dialogEl().contains(document.activeElement)).toBe(true);
    await focusClick("Import");
    // applying: focus stays, Escape is ignored (the apply cannot be cancelled)
    expect(dialogEl().contains(document.activeElement)).toBe(true);
    escape();
    await settle();
    expect(onclose).not.toHaveBeenCalled();
    expect(calls.importCancel).toHaveLength(0);
    d.resolve(applied());
    await settle();
    expect(document.body.textContent).toContain("2 folders, 4 connections");
    expect(dialogEl().contains(document.activeElement)).toBe(true);
    tab();
    expect(dialogEl().contains(document.activeElement)).toBe(true);
    escape();
    expect(onclose).toHaveBeenCalledTimes(1);
  });

  test("escape_cancels_the_pending_import_after_a_focused_click", async () => {
    const { calls, onclose } = await setup({
      importInspect: async () => preview({ report: withCounts() }),
    });
    await focusClick("mRemoteNG file…");
    escape();
    await settle();
    expect(calls.importCancel).toHaveLength(1);
    expect(onclose).toHaveBeenCalledTimes(1);
  });

  test("focus_stays_in_the_dialog_around_the_restore_confirmation", async () => {
    const { calls, onclose } = await setup({
      importInspect: async () => backupPreview(false),
      importApply: async () => restored(),
    });
    await focusClick("Backup file…");
    await focusClick("Restore…");
    expect(document.querySelector("[role=alertdialog]")).not.toBeNull();
    const cancel = document.querySelector<HTMLButtonElement>("[role=alertdialog] button")!;
    cancel.focus();
    cancel.click();
    await settle();
    expect(document.querySelector("[role=alertdialog]")).toBeNull();
    expect(dialogEl().contains(document.activeElement)).toBe(true);
    tab();
    expect(dialogEl().contains(document.activeElement)).toBe(true);
    await focusClick("Restore…");
    await focusClick("Restore");
    expect(calls.importApply).toHaveLength(1);
    expect(dialogEl().contains(document.activeElement)).toBe(true);
    escape();
    expect(onclose).toHaveBeenCalledTimes(1);
  });

  test("focus_returns_to_the_opener_on_close", async () => {
    const opener = document.createElement("button");
    document.body.append(opener);
    opener.focus();
    await setup();
    const dialog = document.querySelector("[role=dialog]")!;
    expect(dialog.contains(document.activeElement)).toBe(true);
    dialog.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true }));
    expect(dialog.contains(document.activeElement)).toBe(true);
    unmount(mounted!);
    mounted = null;
    await tick();
    expect(document.activeElement).toBe(opener);
  });
});
