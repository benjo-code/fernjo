import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, describe, expect, test, vi } from "vitest";
import { mockApi, updateStatus } from "../testing";
import type { UpdateStatus } from "../types";
import { UpdateStore } from "../update.svelte";
import UpdateDialog from "./UpdateDialog.svelte";

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

const AVAILABLE = updateStatus({
  state: "available",
  version: "0.5.0",
  notes: "Fixes and more",
  install: { kind: "selfUpdate" },
});

async function setup(status: UpdateStatus, impl = {}) {
  const m = mockApi([], { updateStatus: async () => status, ...impl });
  const update = new UpdateStore(m.api);
  await update.init();
  const onclose = vi.fn();
  mounted = mount(UpdateDialog, { target: document.body, props: { update, onclose } });
  await settle();
  return { ...m, update, onclose };
}

const button = (label: string) =>
  [...document.querySelectorAll("button")].find((b) => b.textContent?.trim() === label);
const labels = () => [...document.querySelectorAll("button")].map((b) => b.textContent?.trim());

describe("UpdateDialog", () => {
  test("available_shows_both_versions_notes_and_three_buttons", async () => {
    await setup(AVAILABLE);
    const text = document.body.textContent ?? "";
    expect(text).toContain("0.5.0");
    expect(text).toContain("0.4.0");
    expect(text).toContain("Fixes and more");
    expect(labels()).toEqual(["Update and restart", "Skip this version", "Later"]);
    expect(document.querySelector('[role="dialog"]')).not.toBeNull();
  });

  test("notes_with_markup_render_as_text", async () => {
    const markup = '<img src=x onerror="alert(1)"> <b>bold</b>';
    await setup({ ...AVAILABLE, notes: markup });
    expect(document.body.textContent).toContain(markup);
    expect(document.querySelector("img")).toBeNull();
    expect(document.querySelector("b")).toBeNull();
  });

  test("downloading_shows_progress_and_stage", async () => {
    await setup(
      updateStatus({
        state: "downloading",
        version: "0.5.0",
        install: { kind: "selfUpdate" },
        received: 12 * 1048576,
        total: 48 * 1048576,
      }),
    );
    const bar = document.querySelector("progress")!;
    expect(bar.value).toBe(12 * 1048576);
    expect(bar.max).toBe(48 * 1048576);
    expect(document.body.textContent).toContain("Downloading… 12 of 48 MB");
    // the app stays usable while it downloads: the window can be closed
    expect(button("Later")).toBeDefined();
  });

  test("download_without_a_length_is_indeterminate", async () => {
    await setup(
      updateStatus({ state: "downloading", version: "0.5.0", received: 1048576, total: null }),
    );
    const bar = document.querySelector("progress")!;
    expect(bar.hasAttribute("value")).toBe(false);
  });

  test("installing_has_no_close_button", async () => {
    const { onclose } = await setup(
      updateStatus({ state: "installing", version: "0.5.0", install: { kind: "selfUpdate" } }),
    );
    expect(document.querySelectorAll("button")).toHaveLength(0);
    expect(document.body.textContent).toContain("Installing… the app restarts when done");
    document
      .querySelector('[role="dialog"]')!
      .dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await settle();
    expect(onclose).not.toHaveBeenCalled();
  });

  test("ready_offers_restart_to_update", async () => {
    const { calls } = await setup(
      updateStatus({ state: "ready", version: "0.5.0", install: { kind: "selfUpdate" } }),
    );
    expect(labels()).toEqual(["Restart to update", "Later"]);
    button("Restart to update")!.click();
    await settle();
    expect(calls.updateInstall).toHaveLength(1);
  });

  test("failed_offers_retry_and_release_page", async () => {
    const { calls } = await setup(
      updateStatus({
        state: "failed",
        error: "download_refused",
        version: "0.5.0",
        install: { kind: "releasePage", reason: "not_writable" },
      }),
    );
    expect(document.body.textContent).toContain("download was refused");
    expect(labels()).toEqual(["Retry", "Open release page", "Close"]);
    button("Open release page")!.click();
    await settle();
    expect(calls.updateOpenReleasePage).toHaveLength(1);
  });

  test("a_failed_check_retries_the_check_manually_and_has_no_release_page", async () => {
    const { calls } = await setup(updateStatus({ state: "failed", error: "feed_unreachable" }));
    expect(document.body.textContent).toContain("Could not reach the update server");
    // no version is known, so there is no release page to open
    expect(labels()).toEqual(["Retry", "Close"]);
    button("Retry")!.click();
    await settle();
    expect(calls.updateCheck).toEqual([[true]]);
  });

  test("a_failed_install_retries_the_install", async () => {
    const { calls } = await setup(
      updateStatus({
        state: "failed",
        error: "signature_invalid",
        version: "0.5.0",
        install: { kind: "selfUpdate" },
      }),
    );
    button("Retry")!.click();
    await settle();
    expect(calls.updateInstall).toHaveLength(1);
    expect(calls.updateCheck).toHaveLength(0);
  });

  test("release_page_only_has_no_install_button", async () => {
    const { calls } = await setup({
      ...AVAILABLE,
      install: { kind: "releasePage", reason: "package_install" },
    });
    expect(document.body.textContent).toContain("installed from a package");
    expect(labels()).toEqual(["Open release page", "Skip this version", "Later"]);
    button("Open release page")!.click();
    await settle();
    expect(calls.updateOpenReleasePage).toHaveLength(1);
    expect(calls.updateInstall).toHaveLength(0);
  });

  test("skip_and_later_buttons_call_the_right_actions", async () => {
    const first = await setup(AVAILABLE);
    button("Later")!.click();
    await settle();
    expect(first.onclose).toHaveBeenCalledTimes(1);
    expect(first.calls.updateSkip).toHaveLength(0);
    button("Skip this version")!.click();
    await settle();
    expect(first.calls.updateSkip).toHaveLength(1);
    expect(first.onclose).toHaveBeenCalledTimes(2);
  });

  test("a_failed_skip_keeps_the_window_and_shows_the_error", async () => {
    const { onclose } = await setup(AVAILABLE, {
      updateSkip: async () => {
        throw { code: "settings_unusable", message: "settings.json cannot be read" };
      },
    });
    button("Skip this version")!.click();
    await settle();
    expect(onclose).not.toHaveBeenCalled();
    expect(document.querySelector('[role="alert"]')?.textContent).toContain("cannot be read");
  });

  test("focus_moves_into_the_dialog_and_back", async () => {
    const opener = document.createElement("button");
    document.body.append(opener);
    opener.focus();
    const m = mockApi([], { updateStatus: async () => AVAILABLE });
    const update = new UpdateStore(m.api);
    await update.init();
    mounted = mount(UpdateDialog, { target: document.body, props: { update, onclose: () => {} } });
    await settle();
    expect(document.querySelector('[role="dialog"]')!.contains(document.activeElement)).toBe(true);
    unmount(mounted);
    mounted = null;
    await settle();
    expect(document.activeElement).toBe(opener);
  });
});
