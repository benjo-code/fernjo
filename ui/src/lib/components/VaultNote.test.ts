import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, describe, expect, test } from "vitest";
import { apiError, mockApi, vaultStatus } from "../testing";
import type { Api, VaultStatus } from "../types";
import { VaultStore } from "../vault.svelte";
import VaultNote from "./VaultNote.svelte";

let mounted: ReturnType<typeof mount> | null = null;

afterEach(() => {
  if (mounted) unmount(mounted);
  mounted = null;
  document.body.innerHTML = "";
});

async function settle() {
  await tick();
  await new Promise((r) => setTimeout(r, 0));
  flushSync();
}

async function setup(status: VaultStatus, impl: Partial<Api> = {}) {
  const mock = mockApi([], { vaultStatus: async () => status, ...impl });
  const vault = new VaultStore(mock.api);
  await vault.init();
  mounted = mount(VaultNote, { target: document.body, props: { vault } });
  await settle();
  return { vault, ...mock };
}

const buttons = () => [...document.querySelectorAll<HTMLButtonElement>("button")];
const button = (text: string) => buttons().find((b) => b.textContent?.trim() === text);
const fields = () =>
  [...document.querySelectorAll<HTMLInputElement>("input")].filter((i) => i.type !== "checkbox");

async function type(el: HTMLInputElement, text: string) {
  el.value = text;
  el.dispatchEvent(new Event("input", { bubbles: true }));
  await settle();
}

describe("VaultNote", () => {
  test("no_vault_offers_create_and_create_requires_matching_passwords", async () => {
    const { calls } = await setup(vaultStatus({ state: "none" }), {
      vaultCreate: async () => vaultStatus({ state: "unlocked" }),
    });
    const region = document.querySelector('[role="region"]')!;
    expect(region.textContent).toContain("No vault yet");
    button("Create vault")!.click();
    await settle();

    // the sentence about missing recovery is part of the form
    expect(document.body.textContent).toContain(
      "There is no way to recover a forgotten master password.",
    );
    expect(fields()).toHaveLength(2);

    await type(fields()[0], "a long master password");
    await type(fields()[1], "something else entirely");
    button("Create vault")!.click();
    await settle();
    expect(calls.vaultCreate).toHaveLength(0);
    const alert = document.querySelector('[role="alert"]')!;
    expect(alert.textContent).toContain("The passwords do not match.");
    expect(fields()[1].getAttribute("aria-invalid")).toBe("true");

    await type(fields()[1], "a long master password");
    document.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click();
    await settle();
    button("Create vault")!.click();
    await settle();
    expect(calls.vaultCreate).toEqual([["a long master password", true]]);
  });

  test("create_error_from_rust_is_shown_inline", async () => {
    await setup(vaultStatus({ state: "none" }), {
      vaultCreate: async () => {
        throw apiError("weak_password", "The master password is too short.");
      },
    });
    button("Create vault")!.click();
    await settle();
    await type(fields()[0], "short");
    await type(fields()[1], "short");
    button("Create vault")!.click();
    await settle();
    expect(document.querySelector('[role="alert"]')!.textContent).toContain(
      "The master password is too short.",
    );
  });

  test("wrong_password_shows_the_danger_message_tied_to_the_field", async () => {
    await setup(vaultStatus({ state: "locked" }), {
      vaultUnlock: async () => {
        throw apiError("wrong_password", "wrong");
      },
    });
    const field = fields()[0];
    await type(field, "guess");
    button("Unlock")!.click();
    await settle();
    const alert = document.querySelector('[role="alert"]')!;
    expect(alert.textContent).toContain("Wrong master password. Check Caps Lock and try again.");
    expect(field.getAttribute("aria-invalid")).toBe("true");
    expect(field.getAttribute("aria-describedby")).toBe(alert.id);
    // the text stays so the user can fix it
    expect(field.value).toBe("guess");
  });

  test("remember_checkbox_value_is_passed_to_vault_unlock", async () => {
    const { calls } = await setup(vaultStatus({ state: "locked" }), {
      vaultUnlock: async () => vaultStatus({ state: "locked" }),
    });
    const remember = document.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
    expect(document.body.textContent).toContain("Unlock automatically on this device");
    expect(remember.checked).toBe(false);
    await type(fields()[0], "pw one");
    button("Unlock")!.click();
    await settle();
    remember.click();
    await settle();
    expect(document.body.textContent).toContain("Lock then lasts until the next start.");
    await type(fields()[0], "pw two");
    button("Unlock")!.click();
    await settle();
    expect(calls.vaultUnlock).toEqual([
      ["pw one", false],
      ["pw two", true],
    ]);
  });

  test("a_successful_unlock_clears_the_password_field", async () => {
    // the note stays mounted here (the status is still locked in the mock), so the field shows it
    await setup(vaultStatus({ state: "locked" }), {
      vaultUnlock: async () => vaultStatus({ state: "locked" }),
    });
    await type(fields()[0], "secret one");
    button("Unlock")!.click();
    await settle();
    expect(fields()[0].value).toBe("");
  });

  test("password_field_is_masked_with_a_pressed_toggle", async () => {
    await setup(vaultStatus({ state: "locked" }));
    const field = fields()[0];
    expect(field.type).toBe("password");
    expect(field.getAttribute("autocomplete")).toBe("off");
    const toggle = document.querySelector<HTMLButtonElement>("button[aria-pressed]")!;
    expect(toggle.getAttribute("aria-label")).toBe("Show master password");
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
    toggle.click();
    await settle();
    expect(field.type).toBe("text");
    expect(toggle.getAttribute("aria-pressed")).toBe("true");
  });

  test("unusable_vault_shows_the_reason_and_path_and_no_create_button", async () => {
    await setup(
      vaultStatus({
        state: "unusable",
        reason: "unsupported version 9",
        path: "D:\\data\\vault.fernjo",
      }),
    );
    const text = document.body.textContent!;
    expect(text).toContain("unsupported version 9");
    expect(text).toContain("D:\\data\\vault.fernjo");
    expect(button("Create vault")).toBeUndefined();
    expect(button("Unlock")).toBeUndefined();
  });

  test("an_unlocked_vault_shows_no_note", async () => {
    await setup(vaultStatus({ state: "unlocked" }));
    expect(document.querySelector('[role="region"]')).toBeNull();
  });
});
