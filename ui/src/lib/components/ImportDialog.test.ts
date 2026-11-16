import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, describe, expect, test, vi } from "vitest";
import { ImportStore } from "../import.svelte";
import { SessionStore } from "../sessions.svelte";
import { TreeStore } from "../store.svelte";
import { apiError, conn, deferred, mockApi, preview, report, vaultStatus } from "../testing";
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
  test("dialog_lists_the_six_actions", async () => {
    await setup();
    const dialog = document.querySelector("[role=dialog]")!;
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    expect(document.querySelector("h2")?.textContent).toBe("Import & backup");
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
      "This replaces 3 folders and connections and 0 credentials with the backup from 2026-11-01 (5 and 2). Open sessions are closed.",
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

  test("escape_without_a_pending_import_just_closes", async () => {
    const { calls, onclose } = await setup();
    escape();
    expect(calls.importCancel).toHaveLength(0);
    expect(onclose).toHaveBeenCalledTimes(1);
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
      props: { store: tree, sessions: new SessionStore(m.api, (id) => id), onimport },
    });
    flushSync();
    expect(button("New folder")).toBeTruthy();
    button("Import from mRemoteNG…").click();
    expect(onimport).toHaveBeenCalledTimes(1);
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
