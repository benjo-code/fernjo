import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, expect, test } from "vitest";
import { startedTab } from "../testing";
import type { Prompt } from "../types";
import HostKeyChangedPrompt from "./HostKeyChangedPrompt.svelte";

let mounted: ReturnType<typeof mount> | null = null;

afterEach(() => {
  if (mounted) unmount(mounted);
  mounted = null;
  document.body.innerHTML = "";
});

type Changed = Extract<Prompt, { kind: "hostKeyChanged" }>;
const prompt: Changed = {
  kind: "hostKeyChanged",
  host: "files.example.org",
  port: 22,
  keyType: "ssh-ed25519",
  old: ["SHA256:OLDONE", "SHA256:OLDTWO"],
  new: "SHA256:NEWKEY",
};

async function setup() {
  const t = await startedTab();
  t.emit({ kind: "prompt", id: 3, prompt });
  mounted = mount(HostKeyChangedPrompt, {
    target: document.body,
    props: { tab: t.tab, sessions: t.sessions, prompt },
  });
  flushSync();
  await tick();
  return t;
}

function remount() {
  unmount(mounted!);
  mounted = null;
  document.body.innerHTML = "";
}

const button = (text: string) =>
  [...document.querySelectorAll<HTMLButtonElement>("button")].find(
    (b) => b.textContent?.trim() === text,
  )!;

const keydown = (key: string, shiftKey = false) =>
  document.activeElement!.dispatchEvent(
    new KeyboardEvent("keydown", { key, shiftKey, bubbles: true, cancelable: true }),
  );

test("host_key_changed_focuses_cancel_and_replace_is_not_the_accent_button", async () => {
  const t = await setup();
  expect(document.activeElement).toBe(button("Cancel"));
  const replace = button("Replace key and connect");
  expect(replace.classList.contains("primary")).toBe(false);
  expect(replace.classList.contains("danger")).toBe(true);
  // there is no yellow key in this context at all
  expect(document.querySelectorAll(".primary")).toHaveLength(0);

  const text = document.body.textContent!.replace(/\s+/g, " ");
  expect(text).toContain("SHA256:OLDONE");
  expect(text).toContain("SHA256:OLDTWO");
  expect(text).toContain("SHA256:NEWKEY");
  expect(text).toContain("differs from the one stored");

  button("Cancel").click();
  expect(t.calls.answerPrompt[0]![2]).toEqual({ kind: "hostKeyChanged", decision: "reject" });
});

test("host_key_changed_replace_sends_replace_and_escape_rejects", async () => {
  let t = await setup();
  button("Replace key and connect").click();
  expect(t.calls.answerPrompt[0]![2]).toEqual({ kind: "hostKeyChanged", decision: "replace" });
  remount();

  t = await setup();
  keydown("Escape");
  expect(t.calls.answerPrompt[0]![2]).toEqual({ kind: "hostKeyChanged", decision: "reject" });
});

test("host_key_changed_tab_cycles_between_the_two_buttons_only", async () => {
  await setup();
  const cancel = button("Cancel");
  const replace = button("Replace key and connect");
  expect(document.activeElement).toBe(cancel);
  keydown("Tab");
  expect(document.activeElement).toBe(replace);
  keydown("Tab");
  expect(document.activeElement).toBe(cancel);
  keydown("Tab", true);
  expect(document.activeElement).toBe(replace);
  keydown("Tab", true);
  expect(document.activeElement).toBe(cancel);
  expect(document.querySelector('[role="alertdialog"]')).not.toBeNull();
});

test("host_key_changed_lists_a_repeated_stored_fingerprint_without_a_keying_error", async () => {
  const t = await startedTab();
  const repeated: Changed = { ...prompt, old: ["SHA256:SAME", "SHA256:SAME"] };
  t.emit({ kind: "prompt", id: 3, prompt: repeated });
  mounted = mount(HostKeyChangedPrompt, {
    target: document.body,
    props: { tab: t.tab, sessions: t.sessions, prompt: repeated },
  });
  flushSync();
  await tick();
  const codes = [...document.querySelectorAll<HTMLElement>("code")].map((c) => c.textContent);
  expect(codes).toEqual(["SHA256:SAME", "SHA256:SAME", "SHA256:NEWKEY"]);
});
