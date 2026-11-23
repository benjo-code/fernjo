import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { OpenTabsStore } from "../open-tabs.svelte";
import { SessionStore } from "../sessions.svelte";
import { TerminalSettingsStore } from "../terminal-settings.svelte";
import { SyncStore } from "../sync.svelte";
import { ThemeStore } from "../theme.svelte";
import {
  apiError,
  autoLockInfo,
  dataFolder,
  deferred,
  mockApi,
  openTabsInfo,
  terminalSettings,
  themeInfo,
} from "../testing";
import type { Api, DataFolderInfo } from "../types";
import SettingsDialog from "./SettingsDialog.svelte";
import dialogSource from "./SettingsDialog.svelte?raw";

let mounted: ReturnType<typeof mount> | null = null;

beforeEach(() => {
  delete document.documentElement.dataset.theme;
  localStorage.clear();
});

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

const newOpenTabs = (api: Api) =>
  new OpenTabsStore(
    api,
    new SessionStore(
      api,
      (id) => id,
      () => true,
    ),
  );

async function setup(impl: Partial<Api> = {}, mac?: boolean) {
  const m = mockApi([], impl);
  const openTabs = newOpenTabs(m.api);
  await openTabs.load();
  const onclose = vi.fn();
  const onchange = vi.fn();
  mounted = mount(SettingsDialog, {
    target: document.body,
    props: {
      api: m.api,
      terminal: new TerminalSettingsStore(m.api),
      theme: new ThemeStore(m.api),
      sync: new SyncStore(m.api),
      openTabs,
      onclose,
      onchange,
      ...(mac === undefined ? {} : { mac }),
    },
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
        theme: new ThemeStore(m.api),
        sync: new SyncStore(m.api),
        openTabs: newOpenTabs(m.api),
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
    button("Save terminal settings").click();
    await settle();
    expect(calls.setTerminalSettings[0]).toEqual([
      { fontFamily: null, fontSize: 18, scrollback: 2500, closeOnExit: true },
    ]);
    expect(alert()).toBeNull();

    type(field("Font size"), "5");
    button("Save terminal settings").click();
    await settle();
    expect(alert()?.textContent).toBe("Font size must be between 8 and 32.");

    type(field("Font size"), "18");
    type(field("Scrollback"), "200000");
    button("Save terminal settings").click();
    await settle();
    expect(alert()?.textContent).toBe("Scrollback must be between 1000 and 100000 lines.");

    // a field that is not a number is sent as 0 and rejected by Rust, never judged here
    type(field("Font size"), "");
    type(field("Scrollback"), "2000");
    button("Save terminal settings").click();
    await settle();
    expect(calls.setTerminalSettings.at(-1)).toEqual([
      { fontFamily: null, fontSize: 0, scrollback: 2000, closeOnExit: true },
    ]);
  });

  test("the close-on-exit checkbox saves through setTerminalSettings", async () => {
    const { calls } = await setup({
      getTerminalSettings: async () => terminalSettings({ closeOnExit: true }),
      setTerminalSettings: async (s) => terminalSettings(s),
    });
    const box = field("Close the tab when the shell exits");
    expect(box.checked).toBe(true);
    box.click();
    await settle();
    expect(box.checked).toBe(false);
    expect(calls.setTerminalSettings.at(-1)).toEqual([
      { fontFamily: null, fontSize: 14, scrollback: 10000, closeOnExit: false },
    ]);
  });

  test("saving another terminal field sends the stored closeOnExit unchanged", async () => {
    const { calls } = await setup({
      getTerminalSettings: async () => terminalSettings({ closeOnExit: false }),
      setTerminalSettings: async (s) => terminalSettings(s),
    });
    type(field("Font size"), "18");
    button("Save terminal settings").click();
    await settle();
    expect(calls.setTerminalSettings.at(-1)).toEqual([
      { fontFamily: null, fontSize: 18, scrollback: 10000, closeOnExit: false },
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
      button("Save terminal settings").click();
      await settle();
      expect(calls.setTerminalSettings.at(-1)).toEqual([
        { fontFamily: null, fontSize: wireSize, scrollback: wireLines, closeOnExit: true },
      ]);
    }
    // never the raw serde text
    expect(alert()?.textContent).toBe("Could not save the terminal settings");
  });

  test("non_persistent_note_is_shown_when_persistent_is_false", async () => {
    const note = "Applies to this run only; settings are not saved while FERNJO_DATA_DIR is set";
    await setup({
      getTerminalSettings: async () =>
        terminalSettings({ persistent: false, unsavedReason: "env" }),
    });
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

describe("appearance", () => {
  const radios = () => [...document.querySelectorAll<HTMLInputElement>("input[type=radio]")];
  const radio = (label: string) => radios().find((r) => r.value === label)!;

  test("appearance_shows_system_light_dark_as_a_radio_group_with_the_current_choice", async () => {
    await setup({ getTheme: async () => themeInfo({ theme: "light" }) });
    const group = document.querySelector("[role=radiogroup]")!;
    expect(group.getAttribute("aria-label")).toBe("Theme");
    expect(radios().map((r) => r.value)).toEqual(["system", "light", "dark"]);
    expect(radio("light").checked).toBe(true);
    expect(radio("dark").checked).toBe(false);
    expect(document.body.textContent).toContain("Appearance");
  });

  test("choosing_a_theme_applies_it_at_once", async () => {
    const { calls } = await setup();
    radio("dark").click();
    await settle();
    expect(calls.setTheme).toEqual([["dark"]]);
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(radio("dark").checked).toBe(true);
  });

  test("a_rejected_theme_shows_an_error_and_keeps_the_old_choice", async () => {
    await setup({
      getTheme: async () => themeInfo({ theme: "light" }),
      setTheme: async () => {
        throw apiError("settings_write_failed", "rust text");
      },
    });
    radio("dark").click();
    await settle();
    expect(alert()?.textContent).toBe("Could not save the theme");
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(radio("light").checked).toBe(true);
  });

  test("a_memory_only_theme_says_it_is_not_remembered", async () => {
    const note = "Applies to this run only; settings are not saved while FERNJO_DATA_DIR is set";
    await setup({ getTheme: async () => themeInfo({ persistent: false, unsavedReason: "env" }) });
    // the terminal section is persistent here, so the note comes from the appearance section
    expect(document.body.textContent).toContain(note);
    unmount(mounted!);
    mounted = null;
    document.body.innerHTML = "";
    await setup();
    expect(document.body.textContent).not.toContain(note);
  });
});

describe("terminal section follow-ups", () => {
  const field = (label: string) =>
    document.querySelector<HTMLInputElement>(`input[aria-label='${label}']`)!;
  const type = (el: HTMLInputElement, value: string) => {
    el.value = value;
    el.dispatchEvent(new Event("input", { bubbles: true }));
  };

  test("saved_note_disappears_when_a_terminal_field_is_edited", async () => {
    await setup();
    button("Save terminal settings").click();
    await settle();
    expect(document.querySelector("[role=status]")?.textContent).toBe("Saved");
    type(field("Font size"), "16");
    await settle();
    expect(document.querySelector("[role=status]")).toBeNull();
  });

  test("a_late_load_does_not_overwrite_what_the_user_already_typed", async () => {
    const d = deferred<ReturnType<typeof terminalSettings>>();
    await setup({ getTerminalSettings: () => d.promise });
    type(field("Font size"), "21");
    d.resolve(terminalSettings({ fontSize: 15, scrollback: 4000 }));
    await settle();
    expect(field("Font size").value).toBe("21");
    // untouched fields still take the loaded value
    expect(field("Scrollback").value).toBe("4000");
  });

  test("a_failed_load_shows_an_error_and_keeps_save_disabled", async () => {
    const { calls } = await setup({
      getTerminalSettings: async () => {
        throw apiError("internal", "boom");
      },
    });
    expect(alert()?.textContent).toBe("Could not load the terminal settings");
    expect(button("Save terminal settings").getAttribute("aria-disabled")).toBe("true");
    button("Save terminal settings").click();
    await settle();
    expect(calls.setTerminalSettings).toHaveLength(0);
  });

  test("enter_during_ime_composition_does_not_submit", async () => {
    const { calls } = await setup();
    const input = field("Font family");
    input.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", isComposing: true, bubbles: true }),
    );
    await settle();
    expect(calls.setTerminalSettings).toHaveLength(0);
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await settle();
    expect(calls.setTerminalSettings).toHaveLength(1);
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

  test("an_unusable_settings_json_shows_its_own_note_not_the_FERNJO_DATA_DIR_one", async () => {
    const own = "Not saved: settings.json cannot be read";
    const env = "FERNJO_DATA_DIR is set";
    await setup({
      getTerminalSettings: async () =>
        terminalSettings({ persistent: false, unsavedReason: "settingsUnusable" }),
    });
    expect(document.body.textContent).toContain(own);
    expect(document.body.textContent).not.toContain(env);
    unmount(mounted!);
    mounted = null;
    document.body.innerHTML = "";
    await setup({
      getTheme: async () => themeInfo({ persistent: false, unsavedReason: "settingsUnusable" }),
    });
    expect(document.body.textContent).toContain(own);
    expect(document.body.textContent).not.toContain(env);
  });
});

describe("live versus saved controls", () => {
  const field = (label: string) =>
    document.querySelector<HTMLInputElement>(`input[aria-label='${label}']`)!;

  test("the_terminal_save_button_says_what_it_saves", async () => {
    await setup();
    expect(button("Save")).toBeUndefined();
    expect(button("Save terminal settings")).toBeTruthy();
  });

  test("one_hint_line_says_appearance_and_the_checkbox_apply_at_once", async () => {
    await setup();
    const hint = [...document.querySelectorAll("p")].filter(
      (p) => p.textContent === "Appearance and the checkbox apply at once.",
    );
    expect(hint).toHaveLength(1);
    // it sits in the terminal group, between the checkbox and the save button
    const box = field("Close the tab when the shell exits");
    const save = button("Save terminal settings");
    expect(box.compareDocumentPosition(hint[0]) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(hint[0].compareDocumentPosition(save) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });
});

describe("keyboard and mouse section", () => {
  const section = () => {
    const heading = [...document.querySelectorAll("h3")].find(
      (h) => h.textContent === "Keyboard and mouse",
    );
    return heading?.nextElementSibling ?? null;
  };
  const rows = () =>
    [...(section()?.querySelectorAll("li") ?? [])].map((li) =>
      li.textContent!.replace(/\s+/g, " ").trim(),
    );

  test("the_section_comes_last_and_is_read_only", async () => {
    await setup();
    const headings = [...document.querySelectorAll("h3")].map((h) => h.textContent);
    expect(headings.at(-1)).toBe("Keyboard and mouse");
    expect(rows().length).toBeGreaterThan(5);
    expect(section()?.querySelector("input, button")).toBeNull();
  });

  test("windows_and_linux_show_ctrl_shift_chords", async () => {
    await setup({}, false);
    const text = rows();
    expect(text).toContain("Filter connections Ctrl+Shift+K");
    expect(text).toContain("Next tab Ctrl+Tab");
    expect(text).toContain("Previous tab Ctrl+Shift+Tab");
    expect(text).toContain("Close tab Ctrl+Shift+W");
    expect(text).toContain("Search in the terminal Ctrl+Shift+F");
    expect(text).toContain("Copy in the terminal Ctrl+Shift+C");
    expect(text).toContain("Paste in the terminal Ctrl+Shift+V");
    expect(text).toContain("Open a connection Double-click or Enter");
    expect(text).toContain("Select several Ctrl+click");
    expect(text).toContain("Select a range Shift+click");
    expect(text).toContain("Rename F2");
    expect(text).toContain("Delete selected Delete");
    expect(text.join(" ")).not.toContain("Cmd");
  });

  test("macos_shows_cmd_chords", async () => {
    await setup({}, true);
    const text = rows();
    expect(text).toContain("Filter connections Cmd+K");
    expect(text).toContain("Close tab Cmd+W");
    expect(text).toContain("Search in the terminal Cmd+F");
    expect(text).toContain("Copy in the terminal Cmd+C");
    expect(text).toContain("Select several Cmd+click");
    expect(text).toContain("Next tab Ctrl+Tab");
    expect(text.join(" ")).not.toContain("Ctrl+Shift+K");
  });
});

describe("vault auto-lock", () => {
  const field = () =>
    document.querySelector<HTMLInputElement>("input[aria-label^='Lock the vault after']")!;
  const enter = async (value: string) => {
    field().value = value;
    field().dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
  };

  test("the_auto_lock_field_saves_minutes_and_off", async () => {
    const { calls } = await setup({
      getVaultAutoLock: async () => autoLockInfo({ minutes: 15 }),
    });
    expect(field().value).toBe("15");
    expect(document.body.textContent).toContain("minutes of inactivity");
    await enter("30");
    await enter("");
    expect(calls.setVaultAutoLock).toEqual([[30], [null]]);
    expect(field().value).toBe("");
  });

  test("an_out_of_range_value_shows_rusts_refusal_and_keeps_the_stored_one", async () => {
    await setup({
      getVaultAutoLock: async () => autoLockInfo({ minutes: 15 }),
      setVaultAutoLock: async () => {
        throw apiError("invalid_auto_lock", "rust text");
      },
    });
    await enter("5000");
    expect(alert()?.textContent).toContain("between 1 and 1440");
    expect(field().value).toBe("15");
  });

  test("a_memory_only_auto_lock_says_it_is_not_remembered", async () => {
    await setup({
      getVaultAutoLock: async () => autoLockInfo({ persistent: false, unsavedReason: "env" }),
    });
    expect(document.body.textContent).toContain("FERNJO_DATA_DIR");
  });

  test("the_reopen_tabs_checkbox_saves_and_shows_the_unsaved_note", async () => {
    const { calls } = await setup({
      getOpenTabs: async () => openTabsInfo({ persistent: false, unsavedReason: "env" }),
      setReopenTabs: async (enabled) =>
        openTabsInfo({ reopen: enabled, persistent: false, unsavedReason: "env" }),
    });
    const box = document.querySelector<HTMLInputElement>(
      'input[aria-label="Reopen tabs on start"]',
    )!;
    expect(box.checked).toBe(true);
    expect(document.body.textContent).toContain("FERNJO_DATA_DIR");
    box.click();
    await settle();
    expect(calls.setReopenTabs).toEqual([[false]]);
    expect(box.checked).toBe(false);
  });

  test("a_refused_reopen_tabs_change_puts_the_tick_back", async () => {
    await setup({
      setReopenTabs: async () => {
        throw apiError("settings_unusable", "no");
      },
    });
    const box = document.querySelector<HTMLInputElement>(
      'input[aria-label="Reopen tabs on start"]',
    )!;
    box.click();
    await settle();
    expect(box.checked).toBe(true);
    expect(alert()?.textContent).toContain("Could not save the setting");
  });
});
