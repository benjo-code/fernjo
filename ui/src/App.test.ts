import { mount, unmount } from "svelte";
import { afterEach, expect, test } from "vitest";
import App from "./App.svelte";
import { conn, folder, mockApi } from "./lib/testing";

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
