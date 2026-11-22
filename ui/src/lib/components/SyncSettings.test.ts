import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, describe, expect, test, vi } from "vitest";
import { SyncStore } from "../sync.svelte";
import { apiError, deferred, mockApi, syncStatus } from "../testing";
import type { Api, SyncStatus } from "../types";
import SyncSettings from "./SyncSettings.svelte";

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

async function setup(status: SyncStatus, impl: Partial<Api> = {}) {
  const m = mockApi([], { syncStatus: async () => status, ...impl });
  const sync = new SyncStore(m.api);
  await sync.init();
  const onexport = vi.fn();
  mounted = mount(SyncSettings, { target: document.body, props: { sync, onexport } });
  await settle();
  return { ...m, sync, onexport };
}

const button = (label: string) =>
  [...document.querySelectorAll("button")].find((b) => b.textContent?.trim() === label);
const click = async (label: string) => {
  const b = button(label);
  expect(b, `button "${label}"`).toBeDefined();
  b!.click();
  await settle();
};
const fill = async (selector: string, value: string) => {
  const el = document.querySelector<HTMLInputElement | HTMLTextAreaElement>(selector)!;
  el.value = value;
  el.dispatchEvent(new Event("input", { bubbles: true }));
  await settle();
};

/** Text on the page including what read-only fields hold (textContent leaves values out). */
const shown = () =>
  (
    document.body.textContent +
    [...document.querySelectorAll("textarea")].map((a) => a.value).join(" ")
  ).replace(/\s+/g, " ");

const paired = (partial: Partial<SyncStatus> = {}) =>
  syncStatus({
    state: "ok",
    deviceName: "Gaming PC",
    hubUrl: "https://hub.example.lan",
    ...partial,
  });

describe("SyncSettings", () => {
  test("not_set_up_offers_setup_and_join", async () => {
    await setup(syncStatus());
    expect(button("Set up this device")).toBeDefined();
    expect(button("Join with a pairing code")).toBeDefined();
    expect(button("Sync now")).toBeUndefined();
  });

  test("setup_sends_url_secret_ca_and_device_name_and_clears_the_secret_field", async () => {
    const gate = deferred<SyncStatus>();
    const { calls, sync } = await setup(syncStatus(), {
      syncSetup: () => gate.promise,
      syncPairingCode: async () => "CODE-123",
    });
    await click("Set up this device");
    await fill("#sync-url", "https://hub.example.lan");
    await fill("#sync-secret", "hub-secret-value");
    await fill("#sync-ca", "-----BEGIN CERTIFICATE-----");
    await fill("#sync-device", "Gaming PC");
    await click("Set up");
    expect(calls.syncSetup).toEqual([
      ["https://hub.example.lan", "hub-secret-value", "-----BEGIN CERTIFICATE-----", "Gaming PC"],
    ]);
    // cleared while the call is still in flight
    expect(document.querySelector<HTMLInputElement>("#sync-secret")!.value).toBe("");

    gate.resolve(paired());
    await settle();
    // shown once after the setup, with the warning
    expect(shown()).toContain("CODE-123");
    expect(shown()).toContain(
      "Keep this code private: it contains the keys of your sync group. You need it to add your other devices.",
    );
    expect(document.body.textContent).not.toContain("hub-secret-value");
    expect(sync.pairingCode).toBe("CODE-123");
  });

  test("an_empty_ca_field_is_sent_as_null_and_an_error_is_shown_calmly", async () => {
    const { calls } = await setup(syncStatus(), {
      syncSetup: async () => {
        throw apiError("hub_unauthorized");
      },
    });
    await click("Set up this device");
    await fill("#sync-url", "https://hub.example.lan");
    await fill("#sync-secret", "s");
    await fill("#sync-device", "Gaming PC");
    await click("Set up");
    expect(calls.syncSetup[0]![2]).toBeNull();
    expect(document.body.textContent).toContain("The hub did not accept the secret.");
  });

  test("show_pairing_code_reveals_once_and_hides_on_close", async () => {
    const { calls, sync } = await setup(paired(), { syncPairingCode: async () => "CODE-XYZ" });
    expect(shown()).not.toContain("CODE-XYZ");
    expect(calls.syncPairingCode).toHaveLength(0);
    await click("Show pairing code");
    expect(shown()).toContain("CODE-XYZ");
    await click("Hide pairing code");
    expect(shown()).not.toContain("CODE-XYZ");
    expect(sync.pairingCode).toBeNull();

    await click("Show pairing code");
    expect(sync.pairingCode).toBe("CODE-XYZ");
    // closing the dialog unmounts the section and takes the code with it
    unmount(mounted!);
    mounted = null;
    expect(sync.pairingCode).toBeNull();
  });

  test("join_asks_for_confirmation_with_the_counts_before_replacing", async () => {
    const { calls, onexport } = await setup(syncStatus(), {
      syncJoin: async (_code, _name, confirm) =>
        confirm
          ? paired()
          : syncStatus({
              state: "needs_confirmation",
              confirmation: { connections: 12, folders: 3, vaultEntries: 5, hasVault: true },
            }),
    });
    await click("Join with a pairing code");
    await fill("#sync-code", "PAIR-ME");
    await fill("#sync-device", "Laptop");
    await click("Join");
    expect(calls.syncJoin).toEqual([["PAIR-ME", "Laptop", false]]);
    const text = document.body.textContent!;
    expect(text).toContain(
      "Joining replaces the connections on this device (12 connections, 3 folders) and, if the sync group has a vault, this device's vault (5 entries).",
    );
    expect(text).toContain("Open sessions will be closed.");
    expect(text).toContain("master password of your sync group");

    await click("Export a backup first…");
    expect(onexport).toHaveBeenCalledTimes(1);
    expect(calls.syncJoin).toHaveLength(1);

    await click("Replace and join");
    expect(calls.syncJoin).toEqual([
      ["PAIR-ME", "Laptop", false],
      ["PAIR-ME", "Laptop", true],
    ]);
    expect(document.querySelector<HTMLTextAreaElement>("#sync-code")).toBeNull();
  });

  test("join_confirmation_words_a_locked_vault_and_no_vault", async () => {
    await setup(syncStatus(), {
      syncJoin: async () =>
        syncStatus({
          state: "needs_confirmation",
          confirmation: { connections: 1, folders: 0, vaultEntries: null, hasVault: true },
        }),
    });
    await click("Join with a pairing code");
    await fill("#sync-code", "C");
    await fill("#sync-device", "D");
    await click("Join");
    expect(document.body.textContent).toContain("this device's vault.");
    expect(document.body.textContent).not.toContain("entries)");
  });

  test("cancelling_the_join_confirmation_does_not_join", async () => {
    const { calls } = await setup(syncStatus(), {
      syncJoin: async () =>
        syncStatus({
          state: "needs_confirmation",
          confirmation: { connections: 0, folders: 0, vaultEntries: 0, hasVault: false },
        }),
    });
    await click("Join with a pairing code");
    await fill("#sync-code", "C");
    await fill("#sync-device", "D");
    await click("Join");
    expect(document.body.textContent).not.toContain("this device's vault");
    await click("Cancel");
    expect(calls.syncJoin).toHaveLength(1);
    expect(button("Replace and join")).toBeUndefined();
  });

  test("reseed_is_only_offered_when_rolled_back", async () => {
    await setup(paired());
    expect(button("Re-seed hub…")).toBeUndefined();
    unmount(mounted!);
    mounted = null;
    document.body.innerHTML = "";

    const { calls } = await setup(paired({ state: "rolled_back" }));
    await click("Re-seed hub…");
    expect(document.querySelector('[role="alertdialog"]')).not.toBeNull();
    expect(calls.syncReseed).toHaveLength(0);
    document.querySelector<HTMLButtonElement>('[role="alertdialog"] .danger')!.click();
    await settle();
    expect(calls.syncReseed).toHaveLength(1);
  });

  test("leave_asks_first", async () => {
    const { calls } = await setup(paired());
    await click("Leave sync…");
    const dialog = document.querySelector('[role="alertdialog"]');
    expect(dialog).not.toBeNull();
    expect(calls.syncLeave).toHaveLength(0);
    const cancel = [...dialog!.querySelectorAll("button")].find((b) => b.textContent === "Cancel")!;
    cancel.click();
    await settle();
    expect(calls.syncLeave).toHaveLength(0);
    expect(document.querySelector('[role="alertdialog"]')).toBeNull();

    await click("Leave sync…");
    document.querySelector<HTMLButtonElement>('[role="alertdialog"] .danger')!.click();
    await settle();
    expect(calls.syncLeave).toHaveLength(1);
  });

  test("edit_hub_secret_sends_it_and_clears_the_field", async () => {
    const gate = deferred<SyncStatus>();
    const { calls } = await setup(paired(), { syncEditSecret: () => gate.promise });
    await click("Edit hub secret");
    await fill("#sync-new-secret", "new-secret");
    await click("Save secret");
    expect(calls.syncEditSecret).toEqual([["new-secret"]]);
    expect(document.querySelector<HTMLInputElement>("#sync-new-secret")!.value).toBe("");
    gate.resolve(paired());
    await settle();
    expect(document.querySelector("#sync-new-secret")).toBeNull();
  });

  test("sync_now_calls_rust", async () => {
    const { calls } = await setup(paired());
    await click("Sync now");
    expect(calls.syncNow).toHaveLength(1);
  });
});
