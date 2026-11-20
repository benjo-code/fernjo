import { mount, unmount } from "svelte";
import { afterEach, expect, test } from "vitest";
import App from "./App.svelte";
import {
  apiError,
  conn,
  credential,
  dataFolder,
  folder,
  mockApi,
  resolved,
  vaultStatus,
} from "./lib/testing";

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

test("the_sidebar_footer_buttons_show_their_labels_next_to_the_icons", async () => {
  const { api } = mockApi();
  const app = mount(App, { target: document.body, props: { api } });
  await settle();
  const buttons = [...document.querySelectorAll<HTMLButtonElement>(".sidebar .foot button")];
  expect(buttons.map((b) => b.textContent?.trim())).toEqual(["Settings", "Import / Export"]);
  // an icon stays in each button, and the accessible name equals the visible text (label in name)
  expect(buttons.map((b) => b.querySelector("svg") !== null)).toEqual([true, true]);
  expect(buttons.map((b) => b.getAttribute("aria-label"))).toEqual(["Settings", "Import / Export"]);
  expect(buttons[1]!.getAttribute("title")).toBe("Import, export & backup");
  // the Import / Export button opens the dialog under its new name
  buttons[1]!.click();
  await settle();
  expect(document.querySelector("[role=dialog] h2")?.textContent).toBe("Import, export & backup");
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
  document.querySelector<HTMLButtonElement>("button[aria-label='Import / Export']")!.click();
  await settle();
  await click("mRemoteNG file…");
  await click("Import");
  expect(calls.importApply).toHaveLength(1);
  expect(calls.listCredentials.length).toBe(listed + 1);
  unmount(app);
});

test("the shell grid uses the stored sidebar width", async () => {
  localStorage.setItem("fernjo.layout.v1", '{"sidebarWidth":420,"propertiesHeight":null}');
  try {
    const { api } = mockApi([]);
    const app = mount(App, { target: document.body, props: { api } });
    await new Promise((r) => setTimeout(r, 0));
    const shell = document.querySelector<HTMLElement>(".shell")!;
    expect(shell.style.gridTemplateColumns).toBe("420px 1fr");
    unmount(app);
  } finally {
    localStorage.clear();
  }
});

test("without a stored width the shell grid keeps the token default", async () => {
  localStorage.clear();
  const { api } = mockApi([]);
  const app = mount(App, { target: document.body, props: { api } });
  await new Promise((r) => setTimeout(r, 0));
  expect(document.querySelector<HTMLElement>(".shell")!.style.gridTemplateColumns).toBe("");
  unmount(app);
});

// the Credential row of the properties panel: its buttons reach the vault from a connection
async function selectConnection(impl: Parameters<typeof mockApi>[1], id = "web-01") {
  const mock = mockApi([conn(id)], {
    getNode: async (nodeId: string) => resolved({ id: nodeId, name: nodeId }),
    ...impl,
  });
  const app = mount(App, { target: document.body, props: { api: mock.api } });
  await settle();
  document.querySelector<HTMLElement>(`[data-node-id='${id}']`)!.click();
  await settle();
  return { app, ...mock };
}
const rowButton = (text: string) =>
  [...document.querySelectorAll<HTMLButtonElement>("[data-field='credential'] button")].find(
    (b) => b.textContent?.trim() === text,
  );

test("the_credential_row_create_vault_button_opens_and_focuses_the_create_form", async () => {
  const { app } = await selectConnection({
    vaultStatus: async () => vaultStatus({ state: "none" }),
  });
  expect(document.querySelector("#vault-create-password")).toBeNull();
  rowButton("Create vault")!.click();
  await settle();
  expect(document.activeElement).toBe(document.querySelector("#vault-create-password"));
  unmount(app);
});

test("the_credential_row_unlock_button_opens_and_focuses_the_unlock_form", async () => {
  const { app } = await selectConnection({
    vaultStatus: async () => vaultStatus({ state: "locked" }),
  });
  rowButton("Unlock")!.click();
  await settle();
  expect(document.activeElement).toBe(document.querySelector("#vault-unlock-password"));
  unmount(app);
});

test("the_credential_row_manage_button_opens_the_vault_manager_without_a_new_entry", async () => {
  const { app } = await selectConnection({
    vaultStatus: async () => vaultStatus({ state: "unlocked" }),
    listCredentials: async () => [credential({ id: "c1", name: "Prod root" })],
  });
  rowButton("Manage…")!.click();
  await settle();
  expect(document.querySelector("main")?.textContent).toContain("Credentials");
  expect(document.querySelector("#cred-name")).toBeNull();
  unmount(app);
});

test("new_credential_in_the_dropdown_opens_the_manager_on_a_new_entry_each_time", async () => {
  const { app, calls } = await selectConnection({
    vaultStatus: async () => vaultStatus({ state: "unlocked" }),
    listCredentials: async () => [credential({ id: "c1", name: "Prod root", username: "root" })],
  });
  const choose = async () => {
    const select = document.querySelector<HTMLSelectElement>("[data-field='credential'] select")!;
    select.value = select.options[select.options.length - 1]!.value;
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
  };
  await choose();
  const name = () => document.querySelector<HTMLInputElement>("#cred-name");
  expect(name()).not.toBeNull();
  expect(name()!.value).toBe("");
  expect((document.querySelector("#cred-kind") as HTMLSelectElement).disabled).toBe(false);
  expect(calls.updateNode).toHaveLength(0);

  // already open on another entry: the request switches to the new entry again
  document.querySelector<HTMLElement>("[data-credential='c1']")!.click();
  await settle();
  expect(name()!.value).toBe("Prod root");
  await choose();
  expect(name()!.value).toBe("");
  expect(calls.updateNode).toHaveLength(0);

  // closed and reopened through the vault chip: no new entry is pre-selected
  [...document.querySelectorAll("main button")]
    .find((b) => b.textContent === "Close")!
    .dispatchEvent(new MouseEvent("click", { bubbles: true }));
  await settle();
  document.querySelector<HTMLButtonElement>("button[aria-label^='Vault']")!.click();
  await settle();
  expect(document.querySelector("main")?.textContent).toContain("Credentials");
  expect(name()).toBeNull();
  unmount(app);
});
