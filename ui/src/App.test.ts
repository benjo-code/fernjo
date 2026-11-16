import { mount, unmount } from "svelte";
import { afterEach, expect, test } from "vitest";
import App from "./App.svelte";
import { apiError, conn, dataFolder, folder, mockApi, vaultStatus } from "./lib/testing";

afterEach(() => {
  document.body.innerHTML = "";
});

test("mounts_with_a_mock_api_and_renders_the_shell", async () => {
  const { api, calls } = mockApi([folder("Homelab", [conn("pve-01")])]);
  const app = mount(App, { target: document.body, props: { api } });
  await new Promise((r) => setTimeout(r, 0));
  expect(calls.listTree).toHaveLength(1);
  expect(document.querySelector('aside[aria-label="Connections"]')).not.toBeNull();
  expect(document.querySelector("main")?.textContent).toContain("Open a connection from the tree");
  unmount(app);
});

const settle = () => new Promise((r) => setTimeout(r, 0));

test("the_vault_chip_opens_the_manager_view_only_while_unlocked_and_close_returns", async () => {
  const { api } = mockApi([], { vaultStatus: async () => vaultStatus({ state: "unlocked" }) });
  const app = mount(App, { target: document.body, props: { api } });
  await settle();
  const chip = document.querySelector<HTMLButtonElement>("button[aria-label^='Vault']")!;
  chip.click();
  await settle();
  expect(document.querySelector("main")?.textContent).toContain("Credentials");
  [...document.querySelectorAll("main button")]
    .find((b) => b.textContent === "Close")
    ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  await settle();
  expect(document.querySelector("main")?.textContent).toContain("Open a connection from the tree");
  unmount(app);
});

test("clicking_the_locked_chip_focuses_the_password_field_in_the_lock_note", async () => {
  const { api } = mockApi([], { vaultStatus: async () => vaultStatus({ state: "locked" }) });
  const app = mount(App, { target: document.body, props: { api } });
  await settle();
  document.querySelector<HTMLButtonElement>("button[aria-label^='Vault']")!.click();
  await settle();
  expect(document.activeElement).toBe(document.querySelector("#vault-unlock-password"));
  unmount(app);
});

test("the_sidebar_lets_the_tree_shrink_only_while_the_vault_form_is_open", async () => {
  const { api } = mockApi([], { vaultStatus: async () => vaultStatus({ state: "locked" }) });
  const app = mount(App, { target: document.body, props: { api } });
  await settle();
  const aside = document.querySelector("aside")!;
  expect(aside.classList.contains("note-open")).toBe(false);
  [...document.querySelectorAll("button")].find((b) => b.textContent?.trim() === "Unlock")!.click();
  await settle();
  expect(aside.classList.contains("note-open")).toBe(true);
  [...document.querySelectorAll("button")].find((b) => b.textContent?.trim() === "Cancel")!.click();
  await settle();
  expect(aside.classList.contains("note-open")).toBe(false);
  unmount(app);
});

test("a_restart_required_error_from_a_vault_call_shows_in_the_notice_bar", async () => {
  const { api } = mockApi([], {
    vaultStatus: async () => vaultStatus({ state: "locked" }),
    vaultUnlock: async () => {
      throw apiError("restart_required", "Restart fernjo to use the new data folder.");
    },
  });
  const app = mount(App, { target: document.body, props: { api } });
  await settle();
  // the note is compact until asked: expand the form first
  [...document.querySelectorAll("button")].find((b) => b.textContent?.trim() === "Unlock")!.click();
  await settle();
  const field = document.querySelector<HTMLInputElement>("#vault-unlock-password")!;
  field.value = "pw";
  field.dispatchEvent(new Event("input", { bubbles: true }));
  await settle();
  [...document.querySelectorAll("button")].find((b) => b.textContent?.trim() === "Unlock")!.click();
  await settle();
  const bar = document.querySelector(".bar.error");
  expect(bar?.textContent).toContain("Restart fernjo to use the new data folder.");
  unmount(app);
});

test("the_gear_button_opens_the_data_folder_dialog_and_escape_closes_it", async () => {
  const { api, calls } = mockApi();
  const app = mount(App, { target: document.body, props: { api } });
  await settle();
  const gear = document.querySelector<HTMLButtonElement>("button[aria-label='Settings']")!;
  gear.click();
  await settle();
  expect(document.querySelector("[role=dialog]")?.textContent).toContain("Data folder");
  expect(calls.getDataFolder.length).toBeGreaterThan(0);
  document
    .querySelector("[role=dialog]")!
    .dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  await settle();
  expect(document.querySelector("[role=dialog]")).toBeNull();
  unmount(app);
});

test("a_pending_restart_shows_a_persistent_bar_at_start_without_using_the_error_slot", async () => {
  const { api, calls } = mockApi([], {
    getDataFolder: async () => dataFolder({ restartRequired: true }),
  });
  const app = mount(App, { target: document.body, props: { api } });
  await settle();
  const bar = document.querySelector(".bar.restart")!;
  expect(bar.textContent).toContain("Restart fernjo to use the new data folder.");
  expect(bar.textContent).not.toContain("Dismiss");
  expect(document.querySelector(".bar.error")).toBeNull();
  [...bar.querySelectorAll("button")].find((b) => b.textContent?.trim() === "Restart now")!.click();
  await settle();
  expect(calls.restartApp).toHaveLength(1);
  unmount(app);
});

test("choosing_a_folder_in_the_dialog_raises_the_persistent_restart_bar", async () => {
  const { api } = mockApi([], {
    chooseDataFolder: async () => dataFolder({ path: "/new", restartRequired: true }),
  });
  const app = mount(App, { target: document.body, props: { api } });
  await settle();
  expect(document.querySelector(".bar.restart")).toBeNull();
  document.querySelector<HTMLButtonElement>("button[aria-label='Settings']")!.click();
  await settle();
  [...document.querySelectorAll("[role=dialog] button")]
    .find((b) => b.textContent?.trim() === "Choose folder…")!
    .dispatchEvent(new MouseEvent("click", { bubbles: true }));
  await settle();
  expect(document.querySelector(".bar.restart")).not.toBeNull();
  unmount(app);
});

test("an_applied_import_reloads_the_vault_credential_list", async () => {
  const { api, calls } = mockApi([], {
    vaultStatus: async () => vaultStatus({ state: "unlocked" }),
  });
  const app = mount(App, { target: document.body, props: { api } });
  await settle();
  const listed = calls.listCredentials.length;
  const click = async (label: string) => {
    [...document.querySelectorAll("button")].find((b) => b.textContent?.trim() === label)!.click();
    await settle();
  };
  document.querySelector<HTMLButtonElement>("button[aria-label='Import & backup']")!.click();
  await settle();
  await click("mRemoteNG file…");
  await click("Import");
  expect(calls.importApply).toHaveLength(1);
  expect(calls.listCredentials.length).toBe(listed + 1);
  unmount(app);
});
