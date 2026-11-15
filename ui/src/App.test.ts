import { mount, unmount } from "svelte";
import { afterEach, expect, test } from "vitest";
import App from "./App.svelte";
import { apiError, conn, folder, mockApi, vaultStatus } from "./lib/testing";

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

test("a_restart_required_error_from_a_vault_call_shows_in_the_notice_bar", async () => {
  const { api } = mockApi([], {
    vaultStatus: async () => vaultStatus({ state: "locked" }),
    vaultUnlock: async () => {
      throw apiError("restart_required", "Restart fernjo to use the new data folder.");
    },
  });
  const app = mount(App, { target: document.body, props: { api } });
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
