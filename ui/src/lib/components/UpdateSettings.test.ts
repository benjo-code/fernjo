import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, describe, expect, test, vi } from "vitest";
import { mockApi, updateStatus } from "../testing";
import type { UpdateStatus } from "../types";
import { UpdateStore } from "../update.svelte";
import UpdateSettings from "./UpdateSettings.svelte";

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

async function setup(status: UpdateStatus = updateStatus(), impl = {}) {
  const m = mockApi([], { updateStatus: async () => status, ...impl });
  const update = new UpdateStore(m.api);
  await update.init();
  const onshow = vi.fn();
  mounted = mount(UpdateSettings, { target: document.body, props: { update, onshow } });
  await settle();
  return { ...m, update, onshow };
}

const button = (label: string) =>
  [...document.querySelectorAll("button")].find((b) => b.textContent?.trim() === label);

describe("UpdateSettings", () => {
  test("shows_installed_version_and_checkbox", async () => {
    const { calls } = await setup();
    expect(document.body.textContent).toContain("Installed version 0.4.0");
    const box = document.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
    expect(box.checked).toBe(true);
    box.click();
    await settle();
    expect(calls.setCheckForUpdates).toEqual([[false]]);
  });

  test("check_now_shows_up_to_date_result", async () => {
    const { calls } = await setup();
    button("Check now")!.click();
    await settle();
    expect(calls.updateCheck).toEqual([[true]]);
    expect(document.body.textContent).toContain("You have the latest version");
  });

  test("check_now_shows_the_failure_text", async () => {
    await setup(updateStatus(), {
      updateCheck: async () => updateStatus({ state: "failed", error: "feed_unreachable" }),
    });
    button("Check now")!.click();
    await settle();
    expect(document.body.textContent).toContain("Could not reach the update server");
  });

  test("an_available_version_can_be_shown", async () => {
    const { onshow } = await setup(
      updateStatus({ state: "available", version: "0.5.0", install: { kind: "selfUpdate" } }),
    );
    expect(document.body.textContent).toContain("Version 0.5.0 is available");
    button("Show update")!.click();
    expect(onshow).toHaveBeenCalledTimes(1);
  });

  test("a_skipped_offer_is_worded_as_skipped", async () => {
    const { onshow } = await setup(
      updateStatus({
        state: "available",
        version: "0.5.0",
        install: { kind: "selfUpdate" },
        skipped: true,
      }),
    );
    expect(document.body.textContent).toContain("You skipped version 0.5.0");
    expect(document.body.textContent).not.toContain("is available");
    button("Show update")!.click();
    expect(onshow).toHaveBeenCalledTimes(1);
  });

  test("the_result_line_says_what_is_running", async () => {
    const base = { version: "0.5.0", install: { kind: "selfUpdate" } as const };
    for (const [state, text] of [
      ["downloading", "Downloading version 0.5.0"],
      ["ready", "Version 0.5.0 is ready to install"],
      ["installing", "Installing version 0.5.0"],
    ] as const) {
      await setup(updateStatus({ ...base, state }));
      expect(document.body.textContent, state).toContain(text);
      unmount(mounted!);
      mounted = null;
      document.body.innerHTML = "";
    }
  });

  test("it_says_when_automatic_checks_never_run", async () => {
    await setup(updateStatus({ unsavedReason: "env" }));
    expect(document.body.textContent).toContain(
      "Automatic checks do not run while FERNJO_DATA_DIR",
    );
    unmount(mounted!);
    mounted = null;
    document.body.innerHTML = "";
    await setup(updateStatus({ unsavedReason: "settingsUnusable" }));
    expect(document.body.textContent).toContain("Automatic checks do not run until settings.json");
    unmount(mounted!);
    mounted = null;
    document.body.innerHTML = "";
    // no settings folder: the checks still run, they are only not remembered
    await setup(updateStatus({ unsavedReason: "noConfigDir" }));
    expect(document.body.textContent).not.toContain("Automatic checks do not run");
    unmount(mounted!);
    mounted = null;
    document.body.innerHTML = "";
    await setup(updateStatus());
    expect(document.body.textContent).not.toContain("Automatic checks do not run");
  });

  test("unsaved_note_names_the_real_cause", async () => {
    await setup(updateStatus({ unsavedReason: "env" }));
    expect(document.body.textContent).toContain("FERNJO_DATA_DIR");
    unmount(mounted!);
    mounted = null;
    document.body.innerHTML = "";
    await setup(updateStatus({ unsavedReason: "settingsUnusable" }));
    expect(document.body.textContent).toContain("settings.json cannot be read");
    expect(document.body.textContent).not.toContain("FERNJO_DATA_DIR");
  });

  test("a_failed_save_puts_the_tick_back", async () => {
    await setup(updateStatus(), {
      setCheckForUpdates: async () => {
        throw { code: "settings_unusable", message: "no" };
      },
    });
    const box = document.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
    box.click();
    await settle();
    expect(box.checked).toBe(true);
    expect(document.querySelector('[role="alert"]')).not.toBeNull();
  });
});
