import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, expect, test } from "vitest";
import { startedTab, vaultStatus } from "../testing";
import type { SessionEvent } from "../types";
import { VaultStore } from "../vault.svelte";
import SessionPrompt from "./SessionPrompt.svelte";

let mounted: ReturnType<typeof mount> | null = null;

afterEach(() => {
  if (mounted) unmount(mounted);
  mounted = null;
  document.body.innerHTML = "";
});

const title = () => document.querySelector("h2")?.textContent?.trim() ?? null;
const dialog = () => document.querySelector('[role="dialog"], [role="alertdialog"]');

test("session_prompt_picks_the_variant", async () => {
  const t = await startedTab({ vaultStatus: async () => vaultStatus({ state: "locked" }) });
  const vault = new VaultStore(t.api);
  await vault.init();
  mounted = mount(SessionPrompt, {
    target: document.body,
    props: { tab: t.tab, sessions: t.sessions, vault },
  });
  flushSync();
  // nothing to ask: nothing rendered, so the terminal below stays reachable
  expect(dialog()).toBeNull();

  const show = async (e: SessionEvent) => {
    t.emit(e);
    flushSync();
    await tick();
  };

  await show({
    kind: "prompt",
    id: 1,
    prompt: {
      kind: "hostKey",
      host: "h",
      port: 22,
      keyType: "ssh-ed25519",
      fingerprint: "SHA256:x",
    },
  });
  expect(title()).toBe("Trust this host key?");

  await show({
    kind: "prompt",
    id: 2,
    prompt: {
      kind: "hostKeyChanged",
      host: "h",
      port: 22,
      keyType: "ssh-ed25519",
      old: ["a"],
      new: "b",
    },
  });
  expect(title()).toContain("host key has changed");
  expect(document.querySelector('[role="alertdialog"]')).not.toBeNull();

  await show({ kind: "prompt", id: 3, prompt: { kind: "password", user: "u", host: "h" } });
  expect(title()).toBe("Password required");

  await show({ kind: "prompt", id: 4, prompt: { kind: "keyPassphrase" } });
  expect(title()).toBe("Key passphrase required");

  await show({ kind: "prompt", id: 5, prompt: { kind: "username", host: "h" } });
  expect(title()).toBe("User name required");

  await show({
    kind: "prompt",
    id: 6,
    prompt: {
      kind: "keyboardInteractive",
      name: "",
      instructions: "",
      prompts: [{ text: "Code:", echo: true }],
    },
  });
  expect(title()).toBe("Authentication required");

  t.tab.prompt = null;
  await show({ kind: "state", state: "needsVault", detail: null });
  expect(title()).toBe("n1 needs the vault");

  await show({ kind: "state", state: "connecting", detail: null });
  expect(dialog()).toBeNull();
});

test("a_new_prompt_id_remounts_so_typed_text_does_not_carry_over", async () => {
  const t = await startedTab();
  mounted = mount(SessionPrompt, {
    target: document.body,
    props: { tab: t.tab, sessions: t.sessions, vault: new VaultStore(t.api) },
  });
  const ki = (id: number): SessionEvent => ({
    kind: "prompt",
    id,
    prompt: {
      kind: "keyboardInteractive",
      name: "",
      instructions: "",
      prompts: [{ text: "Code:", echo: true }],
    },
  });
  t.emit(ki(1));
  flushSync();
  await tick();
  const input = document.querySelector<HTMLInputElement>("input")!;
  input.value = "left over";
  input.dispatchEvent(new Event("input", { bubbles: true }));
  t.emit(ki(2));
  flushSync();
  await tick();
  expect(document.querySelector<HTMLInputElement>("input")!.value).toBe("");
});
