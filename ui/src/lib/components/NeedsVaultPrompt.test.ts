import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, expect, test } from "vitest";
import { apiError, startedTab, vaultStatus } from "../testing";
import type { Api } from "../types";
import { VaultStore } from "../vault.svelte";
import NeedsVaultPrompt from "./NeedsVaultPrompt.svelte";

let mounted: ReturnType<typeof mount> | null = null;

afterEach(() => {
  if (mounted) unmount(mounted);
  mounted = null;
  document.body.innerHTML = "";
});

async function setup(impl: Partial<Api> = {}) {
  const t = await startedTab({
    vaultStatus: async () => vaultStatus({ state: "locked" }),
    ...impl,
  });
  t.emit({ kind: "state", state: "needsVault", detail: null });
  const vault = new VaultStore(t.api);
  await vault.init();
  mounted = mount(NeedsVaultPrompt, {
    target: document.body,
    props: { tab: t.tab, sessions: t.sessions, vault },
  });
  flushSync();
  await tick();
  return { ...t, vault };
}

const field = () => document.querySelector<HTMLInputElement>('input:not([type="checkbox"])')!;
const button = (text: string) =>
  [...document.querySelectorAll<HTMLButtonElement>("button")].find(
    (b) => b.textContent?.trim() === text,
  )!;
const settle = async () => {
  await tick();
  await new Promise((r) => setTimeout(r, 0));
  flushSync();
};

async function type(el: HTMLInputElement, text: string) {
  el.value = text;
  el.dispatchEvent(new Event("input", { bubbles: true }));
  await tick();
}

test("needs_vault_unlock_calls_vault_then_resume", async () => {
  const order: string[] = [];
  const t = await setup({
    vaultUnlock: async () => {
      order.push("unlock");
      return vaultStatus({ state: "unlocked" });
    },
    resumeSession: async () => {
      order.push("resume");
    },
  });
  expect(document.body.textContent).toContain("n1 needs the vault");
  expect(document.body.textContent!.replace(/\s+/g, " ")).toContain("other tabs keep running");
  expect(document.activeElement).toBe(field());
  expect(document.body.textContent).toContain("Unlock automatically on this device");
  expect(button("Unlock and connect").classList.contains("primary")).toBe(true);
  expect(document.querySelectorAll(".primary")).toHaveLength(1);

  await type(field(), "master pw");
  const typed = field();
  document.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click();
  button("Unlock and connect").click();
  // the typed text is gone from the field before the unlock call settles
  flushSync();
  expect(typed.value).toBe("");
  await settle();
  expect(order).toEqual(["unlock", "resume"]);
  expect(t.calls.vaultUnlock).toEqual([["master pw", true]]);
  expect(t.calls.resumeSession).toEqual([["sid", false]]);
});

test("needs_vault_wrong_password_shows_error_and_keeps_prompt", async () => {
  const t = await setup({
    vaultUnlock: async () => {
      throw apiError("wrong_password", "wrong");
    },
  });
  await type(field(), "guess");
  button("Unlock and connect").click();
  await settle();
  const alert = document.querySelector('[role="alert"]')!;
  expect(alert.textContent).toContain("Wrong master password. Check Caps Lock and try again.");
  expect(field().getAttribute("aria-invalid")).toBe("true");
  expect(t.calls.resumeSession).toHaveLength(0);
  // the prompt stays, the field is empty again and has the focus
  expect(button("Unlock and connect")).toBeDefined();
  expect(field().value).toBe("");
  expect(document.activeElement).toBe(field());
});

test("needs_vault_cancel_resumes_with_skip", async () => {
  const t = await setup();
  button("Cancel").click();
  await settle();
  expect(t.calls.resumeSession).toEqual([["sid", true]]);
  expect(t.calls.vaultUnlock).toHaveLength(0);
});

test("needs_vault_escape_cancels_like_the_button", async () => {
  const t = await setup();
  field().dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  await settle();
  expect(t.calls.resumeSession).toEqual([["sid", true]]);
});
