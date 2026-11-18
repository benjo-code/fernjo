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

/** The note is compact until the button is clicked; the form then replaces that button. */
async function expand(label = "Unlock") {
  button(label)!.click();
  await settle();
}

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
    await expand();
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
    // focus is back in the field, not stranded on the body
    expect(document.activeElement).toBe(field);
  });

  test("remember_checkbox_value_is_passed_to_vault_unlock", async () => {
    const { calls } = await setup(vaultStatus({ state: "locked" }), {
      vaultUnlock: async () => vaultStatus({ state: "locked" }),
    });
    await expand();
    const remember = document.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
    expect(document.body.textContent).toContain("Unlock automatically on this device");
    expect(remember.checked).toBe(false);
    await type(fields()[0], "pw one");
    button("Unlock")!.click();
    await settle();
    // (the mock stays locked, so the form collapsed after the "success"; open it again)
    await expand();
    const again = document.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
    expect(again.checked).toBe(false);
    again.click();
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
    await expand();
    await type(fields()[0], "secret one");
    button("Unlock")!.click();
    await settle();
    // collapsed after the success (the mock stays locked); the reopened field is empty
    expect(document.querySelector("input")).toBeNull();
    await expand();
    expect(fields()[0].value).toBe("");
  });

  test("password_field_is_masked_with_a_pressed_toggle", async () => {
    await setup(vaultStatus({ state: "locked" }));
    await expand();
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

  test("unusable_vault_shows_the_reason_path_and_how_to_restore_the_backup", async () => {
    await setup(
      vaultStatus({
        state: "unusable",
        reason: "the vault file cannot be read: file is too short",
        path: "D:\\data\\vault.fernjo",
      }),
    );
    const text = document.body.textContent!.replace(/\s+/g, " ");
    // the reason is shown as Rust wrote it, once, without a repeated prefix
    expect(text).toContain("the vault file cannot be read: file is too short");
    expect(text).not.toContain("cannot be used: the vault file");
    expect(text.match(/cannot be read/g)).toHaveLength(1);
    expect(text).toContain("D:\\data\\vault.fernjo");
    expect(text).toContain(
      "To restore an earlier version, close fernjo and copy vault.fernjo.bak over vault.fernjo.",
    );
    expect(text).toContain("D:\\data\\vault.fernjo.bak");
    expect(button("Create vault")).toBeUndefined();
    expect(button("Unlock")).toBeUndefined();
  });

  test("locked_and_no_vault_notes_are_compact_until_asked", async () => {
    await setup(vaultStatus({ state: "locked" }));
    expect(document.body.textContent).toContain("Vault locked");
    expect(document.body.textContent).toContain("Connections with stored credentials ask for it.");
    expect(button("Unlock")).toBeDefined();
    expect(document.querySelector("input")).toBeNull();
    expect(document.querySelector("form")).toBeNull();
    unmount(mounted!);
    mounted = null;
    document.body.innerHTML = "";

    await setup(vaultStatus({ state: "none" }));
    expect(button("Create vault")).toBeDefined();
    expect(document.querySelector("input")).toBeNull();
  });

  test("the_unlock_button_expands_the_form_with_the_field_focused_and_cancel_collapses_it", async () => {
    await setup(vaultStatus({ state: "locked" }));
    await expand();
    expect(fields()).toHaveLength(1);
    expect(document.activeElement).toBe(fields()[0]);
    expect(document.querySelector('input[type="checkbox"]')).not.toBeNull();
    // the compact button is replaced by the form's own submit button
    expect(buttons().filter((b) => b.textContent?.trim() === "Unlock")).toHaveLength(1);
    button("Cancel")!.click();
    await settle();
    expect(document.querySelector("input")).toBeNull();
    expect(button("Unlock")).toBeDefined();
  });

  test("focus_form_expands_and_focuses_the_password_field", async () => {
    const mock = mockApi([], { vaultStatus: async () => vaultStatus({ state: "locked" }) });
    const vault = new VaultStore(mock.api);
    await vault.init();
    const instance = mount(VaultNote, { target: document.body, props: { vault } });
    mounted = instance;
    await settle();
    expect(document.querySelector("input")).toBeNull();
    await (instance as unknown as { focusForm: () => Promise<void> }).focusForm();
    await settle();
    expect(document.activeElement).toBe(fields()[0]);
  });

  test("a_create_error_returns_focus_to_the_password_field", async () => {
    await setup(vaultStatus({ state: "none" }), {
      vaultCreate: async () => {
        throw apiError("weak_password", "The master password is too short.");
      },
    });
    await expand("Create vault");
    await type(fields()[0], "short");
    await type(fields()[1], "short");
    button("Create vault")!.click();
    await settle();
    expect(document.activeElement).toBe(fields()[0]);
  });

  test("an_unlocked_vault_shows_no_note", async () => {
    await setup(vaultStatus({ state: "unlocked" }));
    expect(document.querySelector('[role="region"]')).toBeNull();
  });

  test("locked_with_auto_unlock_starts_checked_and_shows_the_hint", async () => {
    await setup(vaultStatus({ state: "locked", autoUnlock: true }));
    await expand();
    expect(document.querySelector<HTMLInputElement>('input[type="checkbox"]')!.checked).toBe(true);
    expect(document.body.textContent).toContain("Lock then lasts until the next start.");
  });

  test("a_status_arriving_after_mount_updates_the_untouched_checkbox", async () => {
    const { vault } = await setup(vaultStatus({ state: "locked" }), {
      vaultUnlock: async () => vaultStatus({ state: "locked", autoUnlock: true }),
    });
    await expand();
    const box = document.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
    expect(box.checked).toBe(false);
    expect(document.body.textContent).not.toContain("Lock then lasts");
    await vault.unlock("x", false);
    await settle();
    expect(box.checked).toBe(true);
    expect(document.body.textContent).toContain("Lock then lasts until the next start.");
  });

  test("a_user_who_unchecked_the_box_keeps_the_choice_when_the_status_refreshes", async () => {
    const { vault } = await setup(vaultStatus({ state: "locked", autoUnlock: true }), {
      vaultUnlock: async () => vaultStatus({ state: "locked", autoUnlock: true }),
      vaultSetAutoUnlock: async () => vaultStatus({ state: "locked", autoUnlock: false }),
    });
    await expand();
    const box = document.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
    box.click();
    await settle();
    expect(box.checked).toBe(false);
    await vault.unlock("x", false);
    await settle();
    expect(box.checked).toBe(false);
  });

  test("a_failed_auto_unlock_shows_the_note_line_in_the_locked_note", async () => {
    await setup(
      vaultStatus({ state: "locked", autoUnlock: true, autoUnlockIssue: "key_not_accepted" }),
    );
    expect(document.body.textContent).toContain(
      "Automatic unlock did not work this time (the vault file may be incomplete or damaged). Your stored key was kept. Enter the master password.",
    );
    expect(document.querySelector('[role="status"]')).not.toBeNull();
  });

  test("the_unavailable_issue_shows_the_keep_key_line", async () => {
    await setup(vaultStatus({ state: "locked", autoUnlockIssue: "unavailable" }));
    expect(document.body.textContent).toContain(
      "Automatic unlock did not work this time (the vault file may be incomplete or damaged). Your stored key was kept. Enter the master password.",
    );
  });

  test("an_unreadable_keychain_shows_the_neutral_line_and_never_claims_a_stored_key", async () => {
    await setup(vaultStatus({ state: "locked", autoUnlockIssue: "keychain_unavailable" }));
    const text = document.body.textContent ?? "";
    expect(text).toContain(
      "The OS keychain could not be read, so automatic unlock was skipped. Enter the master password.",
    );
    expect(text).not.toContain("stored key");
    expect(text).not.toContain("damaged");
  });

  test("an_unknown_issue_code_falls_back_to_the_neutral_line", async () => {
    await setup(
      vaultStatus({
        state: "locked",
        autoUnlockIssue: "from_the_future" as unknown as "unavailable",
      }),
    );
    const text = document.body.textContent ?? "";
    expect(text).toContain("automatic unlock was skipped");
    expect(text).not.toContain("stored key");
  });

  test("a_locked_note_without_an_issue_has_no_auto_unlock_line", async () => {
    await setup(vaultStatus({ state: "locked", autoUnlock: true }));
    expect(document.body.textContent).not.toContain("Automatic unlock did not work");
  });

  test("the_note_line_goes_away_when_the_vault_unlocks", async () => {
    const { vault } = await setup(
      vaultStatus({ state: "locked", autoUnlock: true, autoUnlockIssue: "key_not_accepted" }),
      { vaultUnlock: async () => vaultStatus({ state: "unlocked", autoUnlock: true }) },
    );
    await vault.unlock("x", true);
    await settle();
    expect(document.body.textContent).not.toContain("Automatic unlock did not work");
  });

  test("a_rolled_back_vault_shows_its_own_note_line", async () => {
    await setup(vaultStatus({ state: "locked", autoUnlock: true, autoUnlockIssue: "rolled_back" }));
    expect(document.body.textContent).toContain(
      "Automatic unlock did not open this vault: the file is older than the last version this device opened. Enter the master password to review.",
    );
    expect(document.body.textContent).not.toContain("damaged");
  });

  test("a_rolled_back_unlock_shows_the_error_and_offers_the_older_vault_with_the_password_kept", async () => {
    const message = "The vault file is older than the version this device last opened.";
    const { calls } = await setup(vaultStatus({ state: "locked" }), {
      vaultUnlock: async () => {
        throw apiError("rolled_back", message);
      },
      vaultUnlockOlder: async () => vaultStatus({ state: "unlocked" }),
    });
    await expand();
    expect(button("Open the older vault")).toBeUndefined();
    await type(fields()[0], "master password");
    button("Unlock")!.click();
    await settle();
    expect(document.body.textContent).toContain(message);
    expect(fields()[0].value).toBe("master password");
    const older = button("Open the older vault");
    expect(older).toBeDefined();

    older!.click();
    await settle();
    expect(calls.vaultUnlockOlder).toEqual([["master password", false]]);
    // success closes the form and clears the field
    expect(document.querySelector('[role="region"]')).toBeNull();
  });

  test("the_older_vault_offer_passes_the_remember_choice_and_goes_away_on_edit_or_cancel", async () => {
    const { calls } = await setup(vaultStatus({ state: "locked" }), {
      vaultUnlock: async () => {
        throw apiError("rolled_back", "older");
      },
      vaultUnlockOlder: async () => {
        throw apiError("wrong_password", "wrong");
      },
    });
    await expand();
    await type(fields()[0], "master password");
    const box = document.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
    box.click();
    await settle();
    button("Unlock")!.click();
    await settle();
    button("Open the older vault")!.click();
    await settle();
    expect(calls.vaultUnlockOlder).toEqual([["master password", true]]);
    // a failure other than rolled_back hides the offer again and shows the usual text
    expect(button("Open the older vault")).toBeUndefined();
    expect(document.body.textContent).toContain("Wrong master password");

    // a new attempt that is rolled back offers it again; typing removes it
    button("Unlock")!.click();
    await settle();
    expect(button("Open the older vault")).toBeDefined();
    await type(fields()[0], "master password 2");
    expect(button("Open the older vault")).toBeUndefined();

    button("Unlock")!.click();
    await settle();
    expect(button("Open the older vault")).toBeDefined();
    button("Cancel")!.click();
    await settle();
    await expand();
    expect(button("Open the older vault")).toBeUndefined();
  });
});
