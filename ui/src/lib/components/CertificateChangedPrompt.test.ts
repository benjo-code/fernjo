import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, expect, test } from "vitest";
import { startedTab } from "../testing";
import type { Prompt } from "../types";
import CertificateChangedPrompt from "./CertificateChangedPrompt.svelte";

let mounted: ReturnType<typeof mount> | null = null;

afterEach(() => {
  if (mounted) unmount(mounted);
  mounted = null;
  document.body.innerHTML = "";
});

type Changed = Extract<Prompt, { kind: "certificateChanged" }>;
const prompt: Changed = {
  kind: "certificateChanged",
  host: "dc01.example.org",
  port: 3389,
  old: "sha256:OLDOLD",
  new: "sha256:NEWNEW",
};

async function setup() {
  const t = await startedTab();
  t.emit({ kind: "prompt", id: 3, prompt });
  mounted = mount(CertificateChangedPrompt, {
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

test("changed shows old and new and defaults to cancel", async () => {
  const t = await setup();
  expect(document.activeElement).toBe(button("Cancel"));
  expect(document.querySelector('[role="alertdialog"]')).not.toBeNull();
  const text = document.body.textContent!.replace(/\s+/g, " ");
  expect(text).toContain("The server certificate has changed");
  expect(text).toContain("sha256:OLDOLD");
  expect(text).toContain("sha256:NEWNEW");
  expect(text).toContain("dc01.example.org:3389");
  // no yellow key: replacing a pin is never the default action
  expect(document.querySelectorAll(".primary")).toHaveLength(0);
  expect(button("Replace and connect").classList.contains("danger")).toBe(true);
  button("Cancel").click();
  expect(t.calls.answerPrompt).toEqual([
    ["sid", 3, { kind: "hostKeyChanged", decision: "reject" }],
  ]);
});

test("replace sends replace and escape rejects", async () => {
  let t = await setup();
  button("Replace and connect").click();
  expect(t.calls.answerPrompt[0]![2]).toEqual({ kind: "hostKeyChanged", decision: "replace" });
  remount();

  t = await setup();
  keydown("Escape");
  expect(t.calls.answerPrompt[0]![2]).toEqual({ kind: "hostKeyChanged", decision: "reject" });
});

test("tab cycles between the two buttons only", async () => {
  await setup();
  const cancel = button("Cancel");
  const replace = button("Replace and connect");
  expect(document.activeElement).toBe(cancel);
  keydown("Tab");
  expect(document.activeElement).toBe(replace);
  keydown("Tab");
  expect(document.activeElement).toBe(cancel);
  keydown("Tab", true);
  expect(document.activeElement).toBe(replace);
});
