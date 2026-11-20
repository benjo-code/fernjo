import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, describe, expect, test, vi } from "vitest";
import { TreeStore } from "../store.svelte";
import { apiError, conn, credential, deferred, mockApi, snapshot, vaultStatus } from "../testing";
import type { Api, CredentialInfo, RevealedSecret } from "../types";
import { VaultStore } from "../vault.svelte";
import VaultManager from "./VaultManager.svelte";

const SENTINEL = "SENTINEL-secret-9f3a7c";

let mounted: ReturnType<typeof mount> | null = null;

afterEach(() => {
  if (mounted) unmount(mounted);
  mounted = null;
  document.body.innerHTML = "";
  vi.useRealTimers();
});

async function settle() {
  await tick();
  await new Promise((r) => setTimeout(r, 0));
  flushSync();
}

/** For tests under fake timers: flushes microtasks without waiting for a real timer. */
async function settleFake() {
  await vi.advanceTimersByTimeAsync(0);
  flushSync();
}

const entries: CredentialInfo[] = [
  credential({ id: "c1", name: "web", username: "root", notes: "n" }),
  credential({ id: "c2", name: "deploy key", kind: "privateKey", username: "deploy" }),
];

async function setup(
  impl: Partial<Api> = {},
  options: { fake?: boolean; list?: CredentialInfo[]; revealMs?: number } = {},
) {
  const mock = mockApi([], {
    vaultStatus: async () => vaultStatus({ state: "unlocked" }),
    listCredentials: async () => options.list ?? entries,
    ...impl,
  });
  const vault = new VaultStore(mock.api);
  const tree = new TreeStore(mock.api);
  await vault.init();
  const onclose = vi.fn();
  mounted = mount(VaultManager, {
    target: document.body,
    props: { vault, tree, onclose, revealMs: options.revealMs },
  });
  if (options.fake) await settleFake();
  else await settle();
  return { vault, tree, onclose, ...mock };
}

const buttons = () => [...document.querySelectorAll<HTMLButtonElement>("button")];
const button = (text: string) => buttons().find((b) => b.textContent?.trim() === text);
const input = (id: string) => document.getElementById(id) as HTMLInputElement;
const rows = () => [...document.querySelectorAll<HTMLElement>("[data-credential]")];
const pick = (index: number) => rows()[index].click();
const inputValues = () =>
  [...document.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>("input,textarea")].map(
    (i) => i.value,
  );

async function type(el: HTMLInputElement | HTMLTextAreaElement, text: string) {
  el.value = text;
  el.dispatchEvent(new Event("input", { bubbles: true }));
  await settle();
}

describe("VaultManager", () => {
  test("lists_entries_and_selects_one_for_editing", async () => {
    await setup();
    expect(rows()).toHaveLength(2);
    expect(rows()[0].textContent).toContain("web");
    expect(rows()[0].textContent).toContain("Password");
    expect(rows()[0].textContent).toContain("root");
    expect(rows()[1].textContent).toContain("deploy key");
    expect(rows()[1].textContent).toContain("Private key");

    pick(0);
    await settle();
    expect(input("cred-name").value).toBe("web");
    expect(input("cred-username").value).toBe("root");
    expect((document.getElementById("cred-kind") as HTMLSelectElement).disabled).toBe(true);
    expect(input("cred-password").value).toBe("");
    expect(input("cred-password").placeholder).toBe("unchanged");
  });

  test("the_title_is_vault_and_credentials_heads_the_list_with_a_singular_plural_count", async () => {
    const count = () => document.querySelector(".list-head .muted")?.textContent?.trim();
    const reset = () => {
      unmount(mounted!);
      mounted = null;
      document.body.innerHTML = "";
    };
    await setup();
    expect(document.getElementById("vault-manager-title")?.textContent).toBe("Vault");
    expect(document.querySelector(".list h3")?.textContent?.trim()).toBe("Credentials");
    expect(count()).toBe("2 credentials");
    expect(document.body.textContent).not.toContain("stored");
    reset();

    await setup({}, { list: [entries[0]!] });
    expect(count()).toBe("1 credential");
    reset();

    await setup({}, { list: [] });
    expect(count()).toBe("0 credentials");
  });

  test("empty_state_offers_new", async () => {
    await setup({}, { list: [] });
    expect(document.body.textContent!.replace(/\s+/g, " ")).toContain(
      "No credentials yet. Create one with + New and pick it on a connection's Credential property.",
    );
    expect(buttons().filter((b) => b.textContent?.trim() === "+ New").length).toBeGreaterThan(0);
  });

  test("editing_without_typing_a_secret_sends_no_secret_field", async () => {
    const { calls } = await setup({
      updateCredential: async (_id, u) => credential({ id: "c1", name: u.name ?? "" }),
    });
    pick(0);
    await settle();
    await type(input("cred-name"), "web2");
    button("Save")!.click();
    await settle();
    expect(calls.updateCredential).toEqual([
      ["c1", { name: "web2", username: "root", domain: "", notes: "n" }],
    ]);
    const sent = calls.updateCredential[0][1] as Record<string, unknown>;
    expect("password" in sent || "privateKey" in sent || "passphrase" in sent).toBe(false);
  });

  test("editing_with_a_typed_password_sends_only_the_password_secret", async () => {
    const { calls } = await setup({
      updateCredential: async () => credential({ id: "c1", name: "web" }),
    });
    pick(0);
    await settle();
    await type(input("cred-password"), "new-pw-value");
    button("Save")!.click();
    await settle();
    expect(calls.updateCredential[0][1]).toEqual({
      name: "web",
      username: "root",
      domain: "",
      notes: "n",
      password: "new-pw-value",
    });
    // the typed secret is gone from the field afterwards
    expect(input("cred-password").value).toBe("");
  });

  describe("private key passphrase on edit", () => {
    const keyUpdate = () => ({
      updateCredential: async () =>
        credential({ id: "c2", name: "deploy key", kind: "privateKey" }),
    });
    const removeBox = () => document.getElementById("cred-clear-passphrase") as HTMLInputElement;

    test("a_new_key_without_a_passphrase_sends_no_passphrase_field", async () => {
      const { calls } = await setup(keyUpdate());
      pick(1);
      await settle();
      expect(input("cred-passphrase").placeholder).toBe("unchanged");
      expect(removeBox().checked).toBe(false);
      await type(document.getElementById("cred-private-key") as HTMLTextAreaElement, "NEWKEY");
      button("Save")!.click();
      await settle();
      expect(calls.updateCredential[0][1]).toEqual({
        name: "deploy key",
        username: "deploy",
        domain: "",
        notes: "",
        privateKey: "NEWKEY",
      });
    });

    test("ticking_remove_stored_passphrase_sends_clear_passphrase_only", async () => {
      const { calls } = await setup(keyUpdate());
      pick(1);
      await settle();
      removeBox().click();
      await settle();
      button("Save")!.click();
      await settle();
      const sent = calls.updateCredential[0][1] as Record<string, unknown>;
      expect(sent.clearPassphrase).toBe(true);
      expect("passphrase" in sent || "privateKey" in sent).toBe(false);
    });

    test("typing_a_passphrase_unticks_and_disables_the_checkbox", async () => {
      const { calls } = await setup(keyUpdate());
      pick(1);
      await settle();
      removeBox().click();
      await settle();
      expect(removeBox().checked).toBe(true);
      await type(input("cred-passphrase"), "new-pass");
      expect(removeBox().checked).toBe(false);
      expect(removeBox().disabled).toBe(true);
      button("Save")!.click();
      await settle();
      const sent = calls.updateCredential[0][1] as Record<string, unknown>;
      expect(sent.passphrase).toBe("new-pass");
      expect("clearPassphrase" in sent).toBe(false);
    });

    test("the_checkbox_is_only_offered_when_editing_a_private_key", async () => {
      await setup();
      pick(0);
      await settle();
      expect(removeBox()).toBeNull();
      button("+ New")!.click();
      await settle();
      const kind = document.getElementById("cred-kind") as HTMLSelectElement;
      kind.value = "privateKey";
      kind.dispatchEvent(new Event("change", { bubbles: true }));
      await settle();
      expect(removeBox()).toBeNull();
    });
  });

  test("a_changed_on_disk_error_is_shown_inline_and_keeps_the_vault_open", async () => {
    const message = "The vault file was changed by another device or a sync tool.";
    const { vault, onclose } = await setup({
      createCredential: async () => {
        throw apiError("changed_on_disk", message);
      },
    });
    button("+ New")!.click();
    await settle();
    await type(input("cred-name"), "late");
    await type(input("cred-password"), "pw-value-1");
    button("Save")!.click();
    await settle();
    expect(document.querySelector('[role="alert"]')?.textContent).toContain(message);
    expect(vault.status?.state).toBe("unlocked");
    expect(onclose).not.toHaveBeenCalled();
    expect(input("cred-name").value).toBe("late");
  });

  test("creating_a_private_key_entry_sends_key_and_passphrase_once", async () => {
    const { calls } = await setup({
      createCredential: async (d) => credential({ id: "c9", name: d.name, kind: d.kind }),
    });
    button("+ New")!.click();
    await settle();
    await type(input("cred-name"), "new key");
    const kind = document.getElementById("cred-kind") as HTMLSelectElement;
    kind.value = "privateKey";
    kind.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
    await type(document.getElementById("cred-private-key") as HTMLTextAreaElement, "KEYDATA");
    await type(input("cred-passphrase"), "pp");
    button("Save")!.click();
    await settle();
    expect(calls.createCredential).toEqual([
      [{ name: "new key", kind: "privateKey", notes: "", privateKey: "KEYDATA", passphrase: "pp" }],
    ]);
  });

  test("creating_a_password_entry_omits_empty_optional_fields", async () => {
    const { calls } = await setup({
      createCredential: async (d) => credential({ id: "c9", name: d.name }),
    });
    button("+ New")!.click();
    await settle();
    await type(input("cred-name"), "pw");
    await type(input("cred-password"), "secret-pw-123");
    button("Save")!.click();
    await settle();
    expect(calls.createCredential).toEqual([
      [{ name: "pw", kind: "password", notes: "", password: "secret-pw-123" }],
    ]);
  });

  test("empty_name_error_shows_on_the_name_field", async () => {
    await setup({
      createCredential: async () => {
        throw apiError("empty_name", "The name must not be empty.");
      },
    });
    button("+ New")!.click();
    await settle();
    await type(input("cred-password"), "x");
    button("Save")!.click();
    await settle();
    expect(document.getElementById("cred-name-error")?.textContent).toContain(
      "The name must not be empty.",
    );
  });

  test("reveal_shows_the_secret_then_hides_it_after_ten_seconds", async () => {
    vi.useFakeTimers();
    const { calls } = await setup(
      { revealCredential: async () => ({ password: SENTINEL }) },
      { fake: true },
    );
    pick(0);
    await settleFake();
    button("Reveal")!.click();
    await settleFake();
    expect(calls.revealCredential).toEqual([["c1"]]);
    expect(inputValues()).toContain(SENTINEL);

    await vi.advanceTimersByTimeAsync(9_000);
    flushSync();
    expect(inputValues()).toContain(SENTINEL);
    await vi.advanceTimersByTimeAsync(1_500);
    flushSync();
    expect(inputValues()).not.toContain(SENTINEL);
    expect(document.body.textContent).not.toContain(SENTINEL);
  });

  test("reveal_timer_length_is_injectable", async () => {
    vi.useFakeTimers();
    await setup(
      { revealCredential: async () => ({ password: SENTINEL }) },
      { fake: true, revealMs: 500 },
    );
    pick(0);
    await settleFake();
    button("Reveal")!.click();
    await settleFake();
    expect(inputValues()).toContain(SENTINEL);
    await vi.advanceTimersByTimeAsync(600);
    flushSync();
    expect(inputValues()).not.toContain(SENTINEL);
  });

  test("reveal_is_hidden_on_lock_and_on_close", async () => {
    const { vault, onclose } = await setup({
      revealCredential: async () => ({ password: SENTINEL }),
    });
    pick(0);
    await settle();
    button("Reveal")!.click();
    await settle();
    expect(inputValues()).toContain(SENTINEL);
    await vault.lock();
    await settle();
    expect(inputValues()).not.toContain(SENTINEL);
    expect(document.body.textContent).not.toContain(SENTINEL);

    // close: a fresh unlock, reveal again, then the Close button
    await vault.unlock("pw", false);
    await settle();
    pick(0);
    await settle();
    button("Reveal")!.click();
    await settle();
    expect(inputValues()).toContain(SENTINEL);
    button("Close")!.click();
    await settle();
    expect(onclose).toHaveBeenCalledOnce();
    expect(inputValues()).not.toContain(SENTINEL);
  });

  test("reveal_is_hidden_when_the_selection_changes", async () => {
    await setup({ revealCredential: async () => ({ password: SENTINEL }) });
    pick(0);
    await settle();
    button("Reveal")!.click();
    await settle();
    expect(inputValues()).toContain(SENTINEL);
    pick(1);
    await settle();
    expect(inputValues()).not.toContain(SENTINEL);
    expect(document.body.textContent).not.toContain(SENTINEL);
  });

  describe("late reveal responses", () => {
    async function pending(fake = true) {
      if (fake) vi.useFakeTimers();
      const reveals: ReturnType<typeof deferred<RevealedSecret>>[] = [];
      const ctx = await setup(
        {
          revealCredential: () => {
            const d = deferred<RevealedSecret>();
            reveals.push(d);
            return d.promise;
          },
          updateCredential: async () => credential({ id: "c1", name: "web" }),
        },
        { fake },
      );
      pick(0);
      await settleFake();
      return { ...ctx, reveals };
    }

    test("reveal_resolving_after_close_and_unmount_renders_nothing_and_leaves_no_timer", async () => {
      const { reveals, onclose } = await pending();
      button("Reveal")!.click();
      await settleFake();
      button("Close")!.click();
      await settleFake();
      expect(onclose).toHaveBeenCalledOnce();
      unmount(mounted!);
      mounted = null;
      reveals[0].resolve({ password: SENTINEL });
      await settleFake();
      expect(document.body.textContent).not.toContain(SENTINEL);
      expect(inputValues()).not.toContain(SENTINEL);
      expect(vi.getTimerCount()).toBe(0);
    });

    test("double_click_on_reveal_keeps_one_timer_and_the_second_reveal_lasts_full_length", async () => {
      const { reveals } = await pending();
      button("Reveal")!.click();
      await settleFake();
      await vi.advanceTimersByTimeAsync(5_000);
      button("Reveal")!.click();
      await settleFake();
      reveals[0].resolve({ password: "FIRST-" + SENTINEL });
      await settleFake();
      expect(inputValues().join("|")).not.toContain("FIRST-");
      reveals[1].resolve({ password: SENTINEL });
      await settleFake();
      expect(inputValues()).toContain(SENTINEL);
      expect(vi.getTimerCount()).toBe(1);
      await vi.advanceTimersByTimeAsync(9_000);
      flushSync();
      expect(inputValues()).toContain(SENTINEL);
      await vi.advanceTimersByTimeAsync(1_500);
      flushSync();
      expect(inputValues()).not.toContain(SENTINEL);
      expect(vi.getTimerCount()).toBe(0);
    });

    test("reveal_resolving_after_save_is_not_shown", async () => {
      const { reveals } = await pending();
      button("Reveal")!.click();
      await settleFake();
      button("Save")!.click();
      await settleFake();
      reveals[0].resolve({ password: SENTINEL });
      await settleFake();
      expect(inputValues()).not.toContain(SENTINEL);
      expect(document.body.textContent).not.toContain(SENTINEL);
      expect(vi.getTimerCount()).toBe(0);
    });

    test("reveal_resolving_after_the_vault_locked_is_not_shown", async () => {
      const { reveals, vault } = await pending();
      button("Reveal")!.click();
      await settleFake();
      // the vault queue serialises calls: the lock runs once the reveal has answered
      const locking = vault.lock();
      reveals[0].resolve({ password: SENTINEL });
      await locking;
      await settleFake();
      await vault.unlock("pw", false);
      await settleFake();
      await settleFake();
      expect(inputValues()).not.toContain(SENTINEL);
      expect(vi.getTimerCount()).toBe(0);
    });
  });

  test("delete_dialog_lists_usage_and_cancel_has_focus", async () => {
    await setup({
      credentialUsage: async () => [
        { id: "n1", name: "pve-01", kind: "connection" },
        { id: "n2", name: "Homelab", kind: "folder" },
      ],
    });
    pick(0);
    await settle();
    button("Delete")!.click();
    await settle();
    const dialog = document.querySelector('[role="alertdialog"]')!;
    expect(dialog.textContent).toContain("Used by: pve-01, Homelab");
    expect(dialog.textContent).toContain("They will be set back to inherit.");
    expect(document.activeElement?.textContent?.trim()).toBe("Cancel");
  });

  test("delete_dialog_for_an_unused_entry_has_no_used_by_line", async () => {
    await setup();
    pick(0);
    await settle();
    button("Delete")!.click();
    await settle();
    const dialog = document.querySelector('[role="alertdialog"]')!;
    expect(dialog.textContent).not.toContain("Used by");
  });

  test("delete_confirm_calls_delete_credential_and_refreshes_the_list", async () => {
    let list = entries;
    const { calls, tree } = await setup({
      listCredentials: async () => list,
      deleteCredential: async () => {
        list = [entries[1]];
        return { id: null, snapshot: snapshot([conn("after-delete")]) };
      },
    });
    pick(0);
    await settle();
    button("Delete")!.click();
    await settle();
    const before = calls.listCredentials.length;
    document.querySelector<HTMLButtonElement>('[role="alertdialog"] .danger')!.click();
    await settle();
    expect(calls.deleteCredential).toEqual([["c1"]]);
    expect(calls.listCredentials.length).toBeGreaterThan(before);
    expect(rows()).toHaveLength(1);
    expect(tree.children.map((n) => n.name)).toEqual(["after-delete"]);
    expect(document.querySelector('[role="alertdialog"]')).toBeNull();
  });

  test("no_secret_is_rendered_before_reveal", async () => {
    // the mock hands out the sentinel on reveal and nowhere else
    const { calls } = await setup({ revealCredential: async () => ({ password: SENTINEL }) });
    pick(0);
    await settle();
    expect(document.body.textContent).not.toContain(SENTINEL);
    expect(inputValues()).not.toContain(SENTINEL);
    expect(calls.revealCredential).toHaveLength(0);
  });

  test("change_password_success_closes_the_form_and_confirms", async () => {
    const { calls } = await setup();
    button("Change password")!.click();
    await settle();
    await type(input("vault-change-old"), "old-master-pw");
    await type(input("vault-change-new"), "new-master-password");
    await type(input("vault-change-repeat"), "new-master-password");
    button("Change")!.click();
    await settle();
    expect(calls.vaultChangePassword).toEqual([["old-master-pw", "new-master-password"]]);
    expect(document.getElementById("vault-change-old")).toBeNull();
    expect(document.body.textContent).toContain("Master password changed.");
  });

  test("change_password_wrong_old_password_shows_inline", async () => {
    await setup({
      vaultChangePassword: async () => {
        throw apiError("wrong_password", "Wrong master password.");
      },
    });
    button("Change password")!.click();
    await settle();
    await type(input("vault-change-old"), "nope");
    await type(input("vault-change-new"), "new-master-password");
    await type(input("vault-change-repeat"), "new-master-password");
    button("Change")!.click();
    await settle();
    expect(document.getElementById("vault-change-old-error")?.textContent).toContain(
      "Wrong master password",
    );
    expect(document.getElementById("vault-change-old")).not.toBeNull();
  });

  test("change_password_form_warns_that_older_copies_keep_the_old_password", async () => {
    await setup();
    button("Change password")!.click();
    await settle();
    expect(document.querySelector("form.change")?.textContent?.replace(/\s+/g, " ")).toContain(
      "Older copies of the vault file, such as backups and sync history, still open with your previous password.",
    );
  });

  test("change_password_mismatch_blocks_the_call", async () => {
    const { calls } = await setup();
    button("Change password")!.click();
    await settle();
    await type(input("vault-change-old"), "old-master-pw");
    await type(input("vault-change-new"), "new-master-password");
    await type(input("vault-change-repeat"), "different-one");
    button("Change")!.click();
    await settle();
    expect(calls.vaultChangePassword).toHaveLength(0);
    expect(document.getElementById("vault-change-repeat-error")?.textContent).toContain(
      "do not match",
    );
  });

  test("auto_unlock_toggle_calls_rust_and_shows_errors_inline", async () => {
    const { calls, vault } = await setup({
      vaultSetAutoUnlock: async () => {
        throw apiError("keychain_unavailable", "The key store is not available.");
      },
    });
    const box = document.getElementById("vault-auto-unlock") as HTMLInputElement;
    expect(box.checked).toBe(false);
    box.checked = true;
    box.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
    expect(calls.vaultSetAutoUnlock).toEqual([[true]]);
    expect(document.body.textContent).toContain("The key store is not available.");
    expect(box.checked).toBe(false);
    expect(vault.status?.autoUnlock).toBe(false);
  });

  test("auto_unlock_on_shows_the_hint", async () => {
    await setup({
      vaultStatus: async () => vaultStatus({ state: "unlocked", autoUnlock: true }),
    });
    expect((document.getElementById("vault-auto-unlock") as HTMLInputElement).checked).toBe(true);
    expect(document.body.textContent).toContain("Lock then lasts until the next start.");
  });
});
