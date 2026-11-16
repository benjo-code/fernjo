import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, expect, test } from "vitest";
import { startedTab } from "../testing";
import type { Prompt } from "../types";
import HostKeyPrompt from "./HostKeyPrompt.svelte";

let mounted: ReturnType<typeof mount> | null = null;

afterEach(() => {
  if (mounted) unmount(mounted);
  mounted = null;
  document.body.innerHTML = "";
});

const FINGERPRINT = "SHA256:abcDEF123+/xyz";
type HostKey = Extract<Prompt, { kind: "hostKey" }>;
const prompt: HostKey = {
  kind: "hostKey",
  host: "files.example.org",
  port: 2222,
  keyType: "ssh-ed25519",
  fingerprint: FINGERPRINT,
};

async function setup() {
  const t = await startedTab();
  t.emit({ kind: "prompt", id: 7, prompt });
  mounted = mount(HostKeyPrompt, {
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

test("host_key_prompt_shows_fingerprint_and_maps_buttons_to_answers", async () => {
  const t = await setup();
  const text = document.body.textContent!.replace(/\s+/g, " ");
  expect(text).toContain("files.example.org");
  expect(text).toContain("2222");
  expect(text).toContain("ssh-ed25519");
  const fp = [...document.querySelectorAll<HTMLElement>(".fingerprint")].find((e) =>
    e.textContent?.includes(FINGERPRINT),
  );
  expect(fp).toBeDefined();
  // the fingerprint is real text (selectable), not an image or a pseudo element
  expect(fp!.tagName).toBe("CODE");

  // exactly one yellow key, on "Trust and connect"
  expect(document.querySelectorAll(".primary")).toHaveLength(1);
  expect(button("Trust and connect").classList.contains("primary")).toBe(true);

  button("Trust and connect").click();
  expect(t.calls.answerPrompt).toEqual([["sid", 7, { kind: "hostKey", decision: "always" }]]);
});

test("host_key_prompt_trust_once_cancel_and_escape_map_to_answers", async () => {
  let t = await setup();
  button("Trust once").click();
  expect(t.calls.answerPrompt[0]![2]).toEqual({ kind: "hostKey", decision: "once" });
  remount();

  t = await setup();
  button("Cancel").click();
  expect(t.calls.answerPrompt[0]![2]).toEqual({ kind: "hostKey", decision: "reject" });
  remount();

  t = await setup();
  document
    .querySelector('[role="dialog"]')!
    .dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  expect(t.calls.answerPrompt[0]![2]).toEqual({ kind: "hostKey", decision: "reject" });
});

test("host_key_prompt_is_a_labelled_dialog_and_the_keyboard_does_not_start_on_the_yellow_key", async () => {
  await setup();
  const dialog = document.querySelector('[role="dialog"]')!;
  const label = document.getElementById(dialog.getAttribute("aria-labelledby")!)!;
  expect(label.textContent).toContain("Trust this host key");
  // a stray Enter (typed into the terminal a moment ago) must not trust the key
  expect(document.activeElement).toBe(dialog);
});
