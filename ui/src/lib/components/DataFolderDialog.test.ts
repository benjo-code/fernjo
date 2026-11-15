import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, describe, expect, test, vi } from "vitest";
import { apiError, dataFolder, deferred, mockApi } from "../testing";
import type { Api, DataFolderInfo } from "../types";
import DataFolderDialog from "./DataFolderDialog.svelte";

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

async function setup(impl: Partial<Api> = {}) {
  const m = mockApi([], impl);
  const onclose = vi.fn();
  const onchange = vi.fn();
  mounted = mount(DataFolderDialog, {
    target: document.body,
    props: { api: m.api, onclose, onchange },
  });
  await settle();
  return { ...m, onclose, onchange };
}

const button = (label: string) =>
  [...document.querySelectorAll("button")].find((b) => b.textContent?.trim() === label)!;
const primaries = () => document.querySelectorAll("button.primary");
const alert = () => document.querySelector("[role=alert]");

describe("DataFolderDialog", () => {
  test("shows_current_path_and_source", async () => {
    const { calls } = await setup({
      getDataFolder: async () => dataFolder({ path: "C:\\data\\fernjo", source: "settings" }),
    });
    expect(calls.getDataFolder).toHaveLength(1);
    const dialog = document.querySelector("[role=dialog]")!;
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    expect(dialog.textContent).toContain("Data folder");
    expect(dialog.textContent).toContain("C:\\data\\fernjo");
    expect(dialog.textContent).toContain("Chosen in settings");
  });

  test("default_source_is_named", async () => {
    await setup();
    expect(document.body.textContent).toContain("Default location");
  });

  test("choose_calls_the_api_and_offers_restart", async () => {
    const chosen = dataFolder({ path: "/new", source: "settings", restartRequired: true });
    const { calls, onchange } = await setup({ chooseDataFolder: async () => chosen });
    button("Choose folder…").click();
    await settle();
    expect(calls.chooseDataFolder).toHaveLength(1);
    expect(document.body.textContent).toContain("/new");
    expect(document.body.textContent).toContain("Restart to use the new folder");
    expect(button("Restart now")).toBeTruthy();
    expect(onchange).toHaveBeenCalledWith(chosen);
  });

  test("cancelled_picker_changes_nothing", async () => {
    const { onchange } = await setup({
      getDataFolder: async () => dataFolder({ path: "/keep" }),
      chooseDataFolder: async () => null,
    });
    button("Choose folder…").click();
    await settle();
    expect(document.body.textContent).toContain("/keep");
    expect(document.body.textContent).not.toContain("Restart to use the new folder");
    expect(alert()).toBeNull();
    expect(onchange).not.toHaveBeenCalled();
  });

  test("env_controlled_folder_disables_choosing", async () => {
    await setup({
      getDataFolder: async () => dataFolder({ source: "env", changeable: false }),
    });
    expect(document.body.textContent).toContain("Set by FERNJO_DATA_DIR");
    expect((button("Choose folder…") as HTMLButtonElement).disabled).toBe(true);
  });

  test("a_non_env_unchangeable_folder_says_so_and_disables_choosing", async () => {
    await setup({
      getDataFolder: async () => dataFolder({ source: "default", changeable: false }),
    });
    expect(document.body.textContent).toContain("Cannot be changed here.");
    expect(document.body.textContent).not.toContain("Set by FERNJO_DATA_DIR");
    expect((button("Choose folder…") as HTMLButtonElement).disabled).toBe(true);
  });

  test("folder_errors_show_inline", async () => {
    await setup({
      chooseDataFolder: async () => {
        throw apiError("folder_nested", "That folder is inside the current one.");
      },
    });
    button("Choose folder…").click();
    await settle();
    expect(alert()?.textContent).toContain("That folder is inside the current one.");
    expect(document.querySelector("[role=dialog]")).not.toBeNull();
  });

  test("a_failing_get_data_folder_shows_the_error_inline", async () => {
    await setup({
      getDataFolder: async () => {
        throw apiError("no_config_dir", "No config directory.");
      },
    });
    expect(alert()?.textContent).toContain("No config directory.");
  });

  test("loading_state_is_neutral_until_the_info_arrives", async () => {
    const d = deferred<DataFolderInfo>();
    const m = mockApi([], { getDataFolder: () => d.promise });
    mounted = mount(DataFolderDialog, {
      target: document.body,
      props: { api: m.api, onclose: () => {}, onchange: () => {} },
    });
    await tick();
    expect(document.body.textContent).toContain("Loading…");
    d.resolve(dataFolder({ path: "/late" }));
    await settle();
    expect(document.body.textContent).toContain("/late");
    expect(document.body.textContent).not.toContain("Loading…");
  });

  test("existing_files_after_a_choice_show_a_warning", async () => {
    await setup({
      chooseDataFolder: async () =>
        dataFolder({
          path: "/new",
          restartRequired: true,
          existingFiles: ["connections.json", "vault.json"],
        }),
    });
    button("Choose folder…").click();
    await settle();
    const text = document.body.textContent!;
    expect(text).toContain(
      "This folder already contains connections.json, vault.json. fernjo will use them as they are; nothing was copied.",
    );
    expect(alert()).toBeNull();
  });

  test("no_warning_when_the_chosen_folder_had_no_files", async () => {
    await setup({
      chooseDataFolder: async () => dataFolder({ path: "/new", restartRequired: true }),
    });
    button("Choose folder…").click();
    await settle();
    expect(document.body.textContent).not.toContain("already contains");
    expect(document.body.textContent).not.toContain("copied");
  });

  test("exactly_one_primary_button_before_and_after_choosing", async () => {
    await setup({
      chooseDataFolder: async () => dataFolder({ path: "/new", restartRequired: true }),
    });
    expect(primaries()).toHaveLength(1);
    expect(primaries()[0].textContent?.trim()).toBe("Choose folder…");
    button("Choose folder…").click();
    await settle();
    expect(primaries()).toHaveLength(1);
    expect(primaries()[0].textContent?.trim()).toBe("Restart now");
    expect(button("Choose folder…").classList.contains("primary")).toBe(false);
  });

  test("restart_now_calls_restart_app", async () => {
    const { calls } = await setup({
      chooseDataFolder: async () => dataFolder({ path: "/new", restartRequired: true }),
    });
    button("Choose folder…").click();
    await settle();
    button("Restart now").click();
    await settle();
    expect(calls.restartApp).toHaveLength(1);
    expect((button("Restart now") as HTMLButtonElement).disabled).toBe(true);
  });

  test("escape_closes_the_dialog", async () => {
    const { onclose } = await setup();
    document
      .querySelector("[role=dialog]")!
      .dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    expect(onclose).toHaveBeenCalledTimes(1);
  });

  test("focus_moves_into_the_dialog_stays_inside_and_returns_to_the_opener", async () => {
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
