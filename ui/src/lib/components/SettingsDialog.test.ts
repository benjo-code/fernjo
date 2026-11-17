import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, describe, expect, test, vi } from "vitest";
import { TerminalSettingsStore } from "../terminal-settings.svelte";
import { apiError, dataFolder, deferred, mockApi, terminalSettings } from "../testing";
import type { Api, DataFolderInfo } from "../types";
import SettingsDialog from "./SettingsDialog.svelte";
import dialogSource from "./SettingsDialog.svelte?raw";

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
  mounted = mount(SettingsDialog, {
    target: document.body,
    props: { api: m.api, terminal: new TerminalSettingsStore(m.api), onclose, onchange },
  });
  await settle();
  return { ...m, onclose, onchange };
}

const button = (label: string) =>
  [...document.querySelectorAll("button")].find((b) => b.textContent?.trim() === label)!;
const primaries = () => document.querySelectorAll("button.primary");
const alert = () => document.querySelector("[role=alert]");

describe("SettingsDialog", () => {
  test("shows_current_path_and_source", async () => {
    const { calls } = await setup({
      getDataFolder: async () => dataFolder({ path: "C:\\data\\fernjo", source: "settings" }),
    });
    expect(calls.getDataFolder).toHaveLength(1);
    const dialog = document.querySelector("[role=dialog]")!;
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    expect(dialog.textContent).toContain("Data folder");
    expect(dialog.getAttribute("aria-labelledby")).toBeTruthy();
    expect(document.querySelector("h2")?.textContent).toBe("Settings");
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
    mounted = mount(SettingsDialog, {
      target: document.body,
      props: {
        api: m.api,
        terminal: new TerminalSettingsStore(m.api),
        onclose: () => {},
        onchange: () => {},
      },
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
    // aria-disabled, not disabled: a focused button that becomes disabled drops focus to <body>
    expect(button("Restart now").getAttribute("aria-disabled")).toBe("true");
    expect((button("Restart now") as HTMLButtonElement).disabled).toBe(false);
  });

  test("while_choosing_focus_stays_in_the_dialog_and_escape_still_closes", async () => {
    const pending = deferred<DataFolderInfo | null>();
    const { calls, onclose } = await setup({ chooseDataFolder: () => pending.promise });
    const choose = button("Choose folder…") as HTMLButtonElement;
    choose.focus();
    choose.click();
    await settle();
    const dialog = document.querySelector("[role=dialog]")!;
    expect(choose.getAttribute("aria-disabled")).toBe("true");
    expect(choose.disabled).toBe(false);
    expect(dialog.contains(document.activeElement)).toBe(true);
    // a second click while busy does nothing
    choose.click();
    await settle();
    expect(calls.chooseDataFolder).toHaveLength(1);
    // Escape from the focused element still reaches the dialog and closes it
    document.activeElement!.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
    );
    expect(onclose).toHaveBeenCalledTimes(1);
    pending.resolve(null);
    await settle();
    expect(choose.getAttribute("aria-disabled")).not.toBe("true");
  });

  const field = (label: string) =>
    document.querySelector<HTMLInputElement>(`input[aria-label='${label}']`)!;
  const type = (el: HTMLInputElement, value: string) => {
    el.value = value;
    el.dispatchEvent(new Event("input", { bubbles: true }));
  };

  test("terminal_section_saves_the_values_and_shows_inline_errors", async () => {
    const { calls } = await setup({
      getTerminalSettings: async () =>
        terminalSettings({ fontFamily: "Fira Code", fontSize: 15, scrollback: 4000 }),
      setTerminalSettings: async (s) => {
        if (s.fontSize < 8) throw apiError("invalid_font_size", "rust text");
        if (s.scrollback > 100000) throw apiError("invalid_scrollback", "rust text");
        return terminalSettings(s);
      },
    });
    expect(calls.getTerminalSettings).toHaveLength(1);
    expect(field("Font family").value).toBe("Fira Code");
    expect(field("Font size").value).toBe("15");
    expect(field("Scrollback").value).toBe("4000");

    type(field("Font family"), "  ");
    type(field("Font size"), "18");
    type(field("Scrollback"), "2500");
    button("Save").click();
    await settle();
    expect(calls.setTerminalSettings[0]).toEqual([
      { fontFamily: null, fontSize: 18, scrollback: 2500 },
    ]);
    expect(alert()).toBeNull();

    type(field("Font size"), "5");
    button("Save").click();
    await settle();
    expect(alert()?.textContent).toBe("Font size must be between 8 and 32.");

    type(field("Font size"), "18");
    type(field("Scrollback"), "200000");
    button("Save").click();
    await settle();
    expect(alert()?.textContent).toBe("Scrollback must be between 1000 and 100000 lines.");

    // a field that is not a number is sent as 0 and rejected by Rust, never judged here
    type(field("Font size"), "");
    type(field("Scrollback"), "2000");
    button("Save").click();
    await settle();
    expect(calls.setTerminalSettings.at(-1)).toEqual([
      { fontFamily: null, fontSize: 0, scrollback: 2000 },
    ]);
  });

  test("values_that_do_not_fit_the_wire_type_are_sent_as_zero_and_unknown_errors_stay_generic", async () => {
    const { calls } = await setup({
      setTerminalSettings: async () => {
        throw apiError("internal", "invalid type: floating point `13.5`, expected u16");
      },
    });
    const cases: [string, string, number, number][] = [
      ["13.5", "5000", 0, 5000],
      ["-5", "5000", 0, 5000],
      ["", "5000", 0, 5000],
      ["70000", "5000", 0, 5000],
      ["60000", "5000", 60000, 5000],
      ["14", "5000000000", 14, 0],
    ];
    for (const [size, lines, wireSize, wireLines] of cases) {
      type(field("Font size"), size);
      type(field("Scrollback"), lines);
      button("Save").click();
      await settle();
      expect(calls.setTerminalSettings.at(-1)).toEqual([
        { fontFamily: null, fontSize: wireSize, scrollback: wireLines },
      ]);
    }
    // never the raw serde text
    expect(alert()?.textContent).toBe("Could not save the terminal settings");
  });

  test("non_persistent_note_is_shown_when_persistent_is_false", async () => {
    const note = "Applies to this run only; settings are not saved while FERNJO_DATA_DIR is set";
    await setup({ getTerminalSettings: async () => terminalSettings({ persistent: false }) });
    expect(document.body.textContent).toContain(note);
    unmount(mounted!);
    mounted = null;
    document.body.innerHTML = "";
    await setup();
    expect(document.body.textContent).not.toContain(note);
  });

  test("data_folder_section_still_works", async () => {
    const { onchange } = await setup({
      chooseDataFolder: async () => dataFolder({ path: "/new", restartRequired: true }),
    });
    expect(document.body.textContent).toContain("Terminal");
    button("Choose folder…").click();
    await settle();
    expect(document.body.textContent).toContain("Restart to use the new folder");
    expect(onchange).toHaveBeenCalled();
  });

  test("tab_stays_inside_the_dialog_across_the_fields", async () => {
    await setup();
    const dialog = document.querySelector("[role=dialog]")!;
    field("Scrollback").focus();
    dialog.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true }));
    expect(dialog.contains(document.activeElement)).toBe(true);
    expect(document.activeElement).not.toBe(field("Scrollback"));
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

describe("short windows", () => {
  // jsdom has no layout, so the rule itself is checked: the dialog must scroll inside the window
  // so that Close and Save stay reachable (verified in the real app at the 600 px minimum height).
  test("the dialog is capped to the viewport and scrolls", () => {
    const rule = /\.dialog\s*\{([^}]*)\}/.exec(dialogSource);
    expect(rule).not.toBeNull();
    expect(rule![1]).toMatch(/max-height:\s*calc\(100vh - var\(--space-6\)\)/);
    expect(rule![1]).toMatch(/overflow-y:\s*auto/);
  });
});
