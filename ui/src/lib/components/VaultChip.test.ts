import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, describe, expect, test, vi } from "vitest";
import { apiError, mockApi, vaultStatus } from "../testing";
import type { VaultStatus } from "../types";
import { VaultStore } from "../vault.svelte";
import VaultChip from "./VaultChip.svelte";

let mounted: ReturnType<typeof mount> | null = null;

afterEach(() => {
  if (mounted) unmount(mounted);
  mounted = null;
  document.body.innerHTML = "";
});

async function setup(status: VaultStatus) {
  const { api } = mockApi([], { vaultStatus: async () => status });
  const vault = new VaultStore(api);
  await vault.init();
  const onOpenManager = vi.fn();
  const onFocusUnlock = vi.fn();
  mounted = mount(VaultChip, {
    target: document.body,
    props: { vault, onOpenManager, onFocusUnlock },
  });
  await tick();
  flushSync();
  const chip = document.querySelector<HTMLButtonElement>("button")!;
  return { chip, onOpenManager, onFocusUnlock };
}

describe("VaultChip", () => {
  test("texts_for_none_locked_unlocked_and_unusable", async () => {
    const texts: Record<VaultStatus["state"], string> = {
      none: "No vault",
      locked: "Locked",
      unlocked: "Unlocked",
      unusable: "Vault unusable",
    };
    for (const state of Object.keys(texts) as VaultStatus["state"][]) {
      const { chip } = await setup(vaultStatus({ state }));
      expect(chip.textContent?.trim()).toBe(texts[state]);
      expect(chip.getAttribute("aria-label")!.toLowerCase()).toContain(texts[state].toLowerCase());
      unmount(mounted!);
      mounted = null;
      document.body.innerHTML = "";
    }
  });

  test("click_while_unlocked_opens_the_manager", async () => {
    const { chip, onOpenManager, onFocusUnlock } = await setup(vaultStatus({ state: "unlocked" }));
    chip.click();
    expect(onOpenManager).toHaveBeenCalledTimes(1);
    expect(onFocusUnlock).not.toHaveBeenCalled();
  });

  test("click_while_locked_focuses_the_unlock_form", async () => {
    for (const state of ["locked", "none", "unusable"] as const) {
      const { chip, onOpenManager, onFocusUnlock } = await setup(vaultStatus({ state }));
      chip.click();
      expect(onFocusUnlock).toHaveBeenCalledTimes(1);
      expect(onOpenManager).not.toHaveBeenCalled();
      unmount(mounted!);
      mounted = null;
      document.body.innerHTML = "";
    }
  });

  test("renders_nothing_until_the_status_is_known", async () => {
    const { api } = mockApi();
    const vault = new VaultStore(api);
    mounted = mount(VaultChip, {
      target: document.body,
      props: { vault, onOpenManager: () => {}, onFocusUnlock: () => {} },
    });
    await tick();
    expect(document.querySelector("button")).toBeNull();
  });

  test("a_failed_init_shows_a_neutral_unavailable_chip_without_actions", async () => {
    const { api } = mockApi([], {
      vaultStatus: async () => {
        throw apiError("internal", "boom");
      },
    });
    const vault = new VaultStore(api);
    await vault.init();
    mounted = mount(VaultChip, {
      target: document.body,
      props: { vault, onOpenManager: () => {}, onFocusUnlock: () => {} },
    });
    await tick();
    expect(document.body.textContent).toContain("Vault unavailable");
    expect(document.querySelector("button")).toBeNull();
  });
});
