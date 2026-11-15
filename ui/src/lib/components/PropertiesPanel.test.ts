import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { TreeStore } from "../store.svelte";
import { VaultStore } from "../vault.svelte";
import { apiError, conn, credential, folder, mockApi, resolved, vaultStatus } from "../testing";
import type { Api, ResolvedNode, TreeNode } from "../types";
import PropertiesPanel from "./PropertiesPanel.svelte";

let mounted: ReturnType<typeof mount> | null = null;

beforeEach(() => {
  localStorage.clear();
});

afterEach(() => {
  if (mounted) unmount(mounted);
  mounted = null;
  document.body.innerHTML = "";
});

const tree = (): TreeNode[] => [folder("Homelab", [conn("web-01"), conn("DC01", "rdp")])];

const inheritedUser = {
  local: null,
  effective: "ops",
  source: { kind: "folder" as const, id: "Homelab", name: "Homelab" },
};
const localUser = { local: "root", effective: "root", source: { kind: "local" as const } };

async function settle() {
  await tick();
  await new Promise((r) => setTimeout(r, 0));
  flushSync();
}

async function setup(
  details: Record<string, ResolvedNode>,
  select: string | null = "web-01",
  impl: Partial<Api> = {},
  vaultImpl: Partial<Api> = {},
) {
  const mock = mockApi(tree(), {
    getNode: async (id: string) => details[id],
    ...impl,
  });
  const store = new TreeStore(mock.api);
  await store.init();
  const vault = new VaultStore(mockApi([], vaultImpl).api);
  await vault.init();
  mounted = mount(PropertiesPanel, { target: document.body, props: { store, vault } });
  if (select) store.select(select);
  await settle();
  return { store, vault, ...mock };
}

const field = (name: string) => document.querySelector<HTMLElement>(`[data-field="${name}"]`);
const input = (name: string) => field(name)!.querySelector<HTMLInputElement>("input, select")!;
const selectEl = (name: string) => field(name)!.querySelector<HTMLSelectElement>("select")!;
const hasField = (name: string) => field(name) !== null;

async function commit(name: string, text: string, how: "Enter" | "blur" = "Enter") {
  const el = input(name);
  if (el instanceof HTMLSelectElement) {
    el.value = text;
    el.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
    return;
  }
  el.value = text;
  el.dispatchEvent(new Event("input", { bubbles: true }));
  if (how === "Enter") {
    el.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  } else {
    el.dispatchEvent(new FocusEvent("blur"));
  }
  await settle();
}

describe("PropertiesPanel", () => {
  test("header_shows_name_and_kind", async () => {
    await setup({ "web-01": resolved({ id: "web-01", name: "web-01" }) });
    expect(document.querySelector(".header .title")!.textContent).toBe("web-01");
    expect(document.querySelector(".header .kind")!.textContent).toBe("SSH connection");
    unmount(mounted!);
    document.body.innerHTML = "";
    await setup({ DC01: resolved({ id: "DC01", name: "DC01", protocol: "rdp" }) }, "DC01");
    expect(document.querySelector(".header .kind")!.textContent).toBe("RDP connection");
    unmount(mounted!);
    document.body.innerHTML = "";
    await setup(
      { Homelab: resolved({ id: "Homelab", name: "Homelab", kind: "folder" }) },
      "Homelab",
    );
    expect(document.querySelector(".header .kind")!.textContent).toBe("Folder");
  });

  test("shows_inherited_value_with_from_folder_mark_and_no_reset", async () => {
    await setup({ "web-01": resolved({ id: "web-01", name: "web-01", username: inheritedUser }) });
    expect(input("username").value).toBe("ops");
    expect(field("username")!.textContent).toContain("from Homelab");
    expect(field("username")!.querySelector("button")).toBeNull();
  });

  test("editing_commits_on_blur_and_enter_and_reverts_on_escape", async () => {
    const { calls } = await setup({
      "web-01": resolved({ id: "web-01", name: "web-01", username: inheritedUser }),
    });
    await commit("username", "root");
    expect(calls.updateNode).toEqual([["web-01", { username: "root" }]]);
    await commit("username", "admin", "blur");
    expect(calls.updateNode[1]).toEqual(["web-01", { username: "admin" }]);

    const el = input("username");
    el.value = "zzz";
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await settle();
    expect(el.value).toBe("ops");
    expect(calls.updateNode).toHaveLength(2);
  });

  test("leaving_a_field_unchanged_makes_no_call", async () => {
    const { calls } = await setup({
      "web-01": resolved({ id: "web-01", name: "web-01", username: inheritedUser }),
    });
    await commit("username", "ops", "blur"); // tabbing through must not pin anything
    await commit("name", "web-01");
    await commit("host", "h.lab");
    expect(calls.updateNode).toHaveLength(0);
  });

  test("enter_on_an_unchanged_inherited_value_pins_it_locally", async () => {
    const { calls } = await setup({
      "web-01": resolved({ id: "web-01", name: "web-01", username: inheritedUser }),
    });
    await commit("username", "ops"); // same text as the inherited value, but Enter
    expect(calls.updateNode).toEqual([["web-01", { username: "ops" }]]);
    await commit("port", "22");
    expect(calls.updateNode[1]).toEqual(["web-01", { port: 22 }]);
  });

  test("enter_on_an_unchanged_local_value_makes_no_call", async () => {
    const { calls } = await setup({
      "web-01": resolved({ id: "web-01", name: "web-01", username: localUser }),
    });
    await commit("username", "root");
    expect(calls.updateNode).toHaveLength(0);
  });

  test("local_value_shows_inherit_button_which_sends_null", async () => {
    const { calls } = await setup({
      "web-01": resolved({ id: "web-01", name: "web-01", username: localUser }),
    });
    const reset = field("username")!.querySelector("button")!;
    expect(reset.textContent).toContain("Inherit");
    reset.click();
    await settle();
    expect(calls.updateNode).toEqual([["web-01", { username: null }]]);
  });

  test("clearing_a_local_field_resets_to_inherit", async () => {
    const { calls } = await setup({
      "web-01": resolved({ id: "web-01", name: "web-01", username: localUser }),
    });
    await commit("username", "  ");
    expect(calls.updateNode).toEqual([["web-01", { username: null }]]);
  });

  test("clearing_an_inherited_field_does_nothing", async () => {
    const { calls } = await setup({
      "web-01": resolved({ id: "web-01", name: "web-01", username: inheritedUser }),
    });
    await commit("username", "");
    expect(calls.updateNode).toHaveLength(0);
    expect(input("username").value).toBe("ops");
  });

  test("port_out_of_range_shows_rust_error_and_reverts", async () => {
    const { store } = await setup(
      { "web-01": resolved({ id: "web-01", name: "web-01" }) },
      "web-01",
      {
        updateNode: async () => {
          throw apiError("invalid_port", "The port must be between 1 and 65535");
        },
      },
    );
    await commit("port", "70000");
    expect(field("port")!.textContent).toContain("The port must be between 1 and 65535");
    expect(input("port").value).toBe("22");
    expect(store.error).toBeNull(); // inline, not in the global bar
  });

  test("an_inline_error_clears_on_the_next_edit", async () => {
    await setup({ "web-01": resolved({ id: "web-01", name: "web-01" }) }, "web-01", {
      updateNode: async () => {
        throw apiError("invalid_port", "bad port");
      },
    });
    await commit("port", "70000");
    expect(field("port")!.textContent).toContain("bad port");
    input("port").dispatchEvent(new Event("input", { bubbles: true }));
    await settle();
    expect(field("port")!.textContent).not.toContain("bad port");
  });

  test("non_numeric_port_is_rejected_without_calling_the_api", async () => {
    const { calls } = await setup({ "web-01": resolved({ id: "web-01", name: "web-01" }) });
    for (const bad of ["abc", "22.5", "-1", "1e3"]) {
      await commit("port", bad);
      expect(field("port")!.textContent).toContain("Port must be a whole number");
      expect(input("port").value).toBe("22");
    }
    expect(calls.updateNode).toHaveLength(0);
  });

  test("port_and_keepalive_numbers_are_sent_as_numbers", async () => {
    const { calls } = await setup({ "web-01": resolved({ id: "web-01", name: "web-01" }) });
    await commit("port", " 2222 ");
    await commit("keepalive", "0");
    expect(calls.updateNode).toEqual([
      ["web-01", { port: 2222 }],
      ["web-01", { keepaliveSecs: 0 }],
    ]);
  });

  test("folder_has_no_host_or_protocol_rows_and_rdp_connection_has_no_keepalive_row", async () => {
    await setup(
      {
        Homelab: resolved({
          id: "Homelab",
          name: "Homelab",
          kind: "folder",
          protocol: null,
          host: null,
        }),
      },
      "Homelab",
    );
    expect(hasField("host")).toBe(false);
    expect(hasField("protocol")).toBe(false);
    expect(hasField("port")).toBe(true);
    expect(hasField("username")).toBe(true);
    expect(hasField("keepalive")).toBe(true);
    unmount(mounted!);
    document.body.innerHTML = "";

    await setup({ DC01: resolved({ id: "DC01", name: "DC01", protocol: "rdp" }) }, "DC01");
    expect(hasField("host")).toBe(true);
    expect(hasField("protocol")).toBe(true);
    expect(hasField("keepalive")).toBe(false);
  });

  test("default_sourced_values_show_a_muted_default_mark", async () => {
    await setup({ "web-01": resolved({ id: "web-01", name: "web-01" }) });
    expect(field("port")!.textContent).toContain("default");
    expect(input("port").value).toBe("22");
    expect(field("keepalive")!.textContent).toContain("default");
    // username has no value at all, so nothing to call a default
    expect(input("username").value).toBe("");
    expect(field("username")!.textContent).not.toContain("default");
  });

  test("folder_port_placeholder_says_default_per_protocol", async () => {
    await setup(
      {
        Homelab: resolved({
          id: "Homelab",
          name: "Homelab",
          kind: "folder",
          protocol: null,
          host: null,
          port: { local: null, effective: null, source: { kind: "default" } },
        }),
      },
      "Homelab",
    );
    expect(input("port").value).toBe("");
    expect(input("port").placeholder).toBe("22 (SSH), 3389 (RDP)");
  });

  test("name_and_description_edit_and_empty_name_is_rejected_inline", async () => {
    const { calls } = await setup(
      { "web-01": resolved({ id: "web-01", name: "web-01", description: "main" }) },
      "web-01",
      {
        updateNode: async (_id, patch) => {
          if (patch.name === "") throw apiError("empty_name", "The name must not be empty");
          return { id: "web-01", snapshot: { children: tree(), notices: [] } };
        },
      },
    );
    expect(input("name").value).toBe("web-01");
    expect(input("description").value).toBe("main");
    await commit("name", "web-02");
    await commit("description", "backup host");
    expect(calls.updateNode).toEqual([
      ["web-01", { name: "web-02" }],
      ["web-01", { description: "backup host" }],
    ]);
    await commit("name", "");
    expect(field("name")!.textContent).toContain("The name must not be empty");
    expect(input("name").value).toBe("web-01");
  });

  test("protocol_and_host_edit_send_patches", async () => {
    const { calls } = await setup({
      "web-01": resolved({ id: "web-01", name: "web-01", host: "old.lab" }),
    });
    await commit("host", "new.lab");
    await commit("protocol", "rdp");
    expect(calls.updateNode).toEqual([
      ["web-01", { host: "new.lab" }],
      ["web-01", { protocol: "rdp" }],
    ]);
  });

  test("panel_shows_empty_state_when_nothing_is_selected_or_the_selection_vanished", async () => {
    const { store, setTree } = await setup(
      { "web-01": resolved({ id: "web-01", name: "web-01" }) },
      null,
    );
    expect(document.body.textContent).toContain("Select a connection or folder");
    expect(document.querySelector("[data-field]")).toBeNull();
    store.select("web-01");
    await settle();
    expect(hasField("name")).toBe(true);
    setTree([folder("Homelab", [conn("DC01", "rdp")])]);
    await store.remove("web-01");
    await settle();
    expect(document.body.textContent).toContain("Select a connection or folder");
    expect(hasField("name")).toBe(false);
  });

  test("header_collapse_hides_the_body", async () => {
    await setup({ "web-01": resolved({ id: "web-01", name: "web-01" }) });
    const header = document.querySelector<HTMLButtonElement>(".header")!;
    expect(header.getAttribute("aria-expanded")).toBe("true");
    expect(document.querySelector(".body")).not.toBeNull();
    header.click();
    flushSync();
    expect(header.getAttribute("aria-expanded")).toBe("false");
    expect(document.querySelector(".body")).toBeNull();
    expect(document.querySelector(".header .title")!.textContent).toBe("web-01");
    header.click();
    flushSync();
    expect(document.querySelector(".body")).not.toBeNull();
  });

  test("markup_in_values_is_shown_literally", async () => {
    await setup({ "web-01": resolved({ id: "web-01", name: "<b>x</b>" }) });
    expect(document.querySelector(".header .title")!.textContent).toBe("<b>x</b>");
    expect(document.querySelector(".header b")).toBeNull();
    expect(input("name").value).toBe("<b>x</b>");
  });
});

describe("PropertiesPanel credential row", () => {
  const ID = "3f2b8c1e-aaaa-4bbb-8ccc-0123456789ab";
  const OTHER = "9d1e7a00-bbbb-4ccc-8ddd-ba9876543210";
  const unlocked: Partial<Api> = {
    vaultStatus: async () => vaultStatus({ state: "unlocked" }),
    listCredentials: async () => [
      credential({ id: ID, name: "Prod root" }),
      credential({ id: OTHER, name: "Lab admin" }),
    ],
  };
  const localCred = { local: ID, effective: ID, source: { kind: "local" as const } };
  const folderCred = {
    local: null,
    effective: ID,
    source: { kind: "folder" as const, id: "Homelab", name: "Homelab" },
  };
  const node = (credentialField = resolved({ id: "web-01", name: "web-01" }).credential) => ({
    "web-01": resolved({ id: "web-01", name: "web-01", credential: credentialField }),
  });

  test("credential_row_lists_entry_names_when_unlocked", async () => {
    await setup(node(), "web-01", {}, unlocked);
    const select = selectEl("credential");
    expect([...select.options].map((o) => o.textContent)).toEqual([
      "None",
      "Prod root",
      "Lab admin",
    ]);
    expect(select.value).toBe("");
  });

  test("credential_row_is_shown_for_folders_too", async () => {
    await setup(
      {
        Homelab: resolved({
          id: "Homelab",
          name: "Homelab",
          kind: "folder",
          protocol: null,
          host: null,
        }),
      },
      "Homelab",
      {},
      unlocked,
    );
    expect(hasField("credential")).toBe(true);
  });

  test("choosing_an_entry_sends_a_credential_patch", async () => {
    const { calls } = await setup(node(), "web-01", {}, unlocked);
    await commit("credential", OTHER);
    expect(calls.updateNode).toEqual([["web-01", { credential: OTHER }]]);
  });

  test("choosing_none_on_a_local_credential_resets_it_and_on_an_unset_one_does_nothing", async () => {
    const { calls } = await setup(node(localCred), "web-01", {}, unlocked);
    await commit("credential", "");
    expect(calls.updateNode).toEqual([["web-01", { credential: null }]]);
    unmount(mounted!);
    document.body.innerHTML = "";
    const again = await setup(node(), "web-01", {}, unlocked);
    await commit("credential", "");
    expect(again.calls.updateNode).toHaveLength(0);
  });

  test("inherited_credential_shows_from_folder_and_inherit_resets_it", async () => {
    await setup(node(folderCred), "web-01", {}, unlocked);
    expect(field("credential")!.textContent).toContain("from Homelab");
    expect(field("credential")!.querySelector("button")).toBeNull();
    expect(selectEl("credential").selectedOptions[0].textContent).toBe("Prod root");
    // an inherited choice can be overridden; the local one then offers Inherit
    unmount(mounted!);
    document.body.innerHTML = "";
    const { calls } = await setup(node(localCred), "web-01", {}, unlocked);
    const reset = field("credential")!.querySelector("button")!;
    expect(reset.textContent).toContain("Inherit");
    reset.click();
    await settle();
    expect(calls.updateNode).toEqual([["web-01", { credential: null }]]);
  });

  test("locked_vault_shows_vault_locked_and_offers_no_choice", async () => {
    const { calls } = await setup(
      node(localCred),
      "web-01",
      {},
      {
        vaultStatus: async () => vaultStatus({ state: "locked" }),
      },
    );
    expect(field("credential")!.textContent).toContain("Vault locked");
    expect(field("credential")!.querySelector("select, input, button")).toBeNull();
    expect(calls.updateNode).toHaveLength(0);
  });

  test("no_vault_reads_no_vault_and_is_read_only", async () => {
    await setup(node(), "web-01", {}, { vaultStatus: async () => vaultStatus({ state: "none" }) });
    expect(field("credential")!.textContent).toContain("No vault");
    expect(field("credential")!.querySelector("select, input, button")).toBeNull();
  });

  test("unknown_credential_id_shows_missing_credential", async () => {
    const { calls } = await setup(
      node({ local: "ffff", effective: "ffff", source: { kind: "local" } }),
      "web-01",
      {},
      unlocked,
    );
    expect(field("credential")!.textContent).toContain("Missing credential");
    expect(field("credential")!.querySelector("select")).toBeNull();
    // a dangling local reference can still be cleared
    field("credential")!.querySelector("button")!.click();
    await settle();
    expect(calls.updateNode).toEqual([["web-01", { credential: null }]]);
  });

  test("no_secret_or_id_text_is_shown_for_the_credential_value", async () => {
    await setup(
      node(localCred),
      "web-01",
      {},
      {
        ...unlocked,
        listCredentials: async () => [
          credential({
            id: ID,
            name: "Prod root",
            username: "SECRET-SENTINEL-USER",
            notes: "SECRET-SENTINEL",
          }),
        ],
      },
    );
    // option `value` attributes carry the id (that is how the select addresses an entry); only
    // visible text and accessible names must not
    const row = field("credential")!;
    expect(row.textContent).toContain("Prod root");
    expect(row.textContent).not.toContain(ID);
    expect(row.textContent).not.toContain("SECRET-SENTINEL");
    for (const el of row.querySelectorAll("[aria-label], [title]")) {
      expect(el.getAttribute("aria-label") ?? "").not.toContain(ID);
      expect(el.getAttribute("title") ?? "").not.toContain(ID);
    }
    expect(selectEl("credential").selectedOptions[0].textContent).toBe("Prod root");
  });
});
