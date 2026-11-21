import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, expect, test } from "vitest";
import { startedTab } from "../testing";
import type { Prompt } from "../types";
import CertificatePrompt from "./CertificatePrompt.svelte";

let mounted: ReturnType<typeof mount> | null = null;

afterEach(() => {
  if (mounted) unmount(mounted);
  mounted = null;
  document.body.innerHTML = "";
});

const FINGERPRINT = "sha256:0123456789abcdef";
type Cert = Extract<Prompt, { kind: "certificate" }>;
const prompt: Cert = {
  kind: "certificate",
  host: "dc01.example.org",
  port: 3390,
  fingerprint: FINGERPRINT,
};

async function setup() {
  const t = await startedTab();
  t.emit({ kind: "prompt", id: 7, prompt });
  mounted = mount(CertificatePrompt, {
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

test("shows host port and fingerprint", async () => {
  await setup();
  const text = document.body.textContent!.replace(/\s+/g, " ");
  expect(text).toContain("dc01.example.org");
  expect(text).toContain("3390");
  const fp = [...document.querySelectorAll<HTMLElement>(".fingerprint")].find((e) =>
    e.textContent?.includes(FINGERPRINT),
  );
  expect(fp?.tagName).toBe("CODE");
  const dialog = document.querySelector('[role="dialog"]')!;
  const label = document.getElementById(dialog.getAttribute("aria-labelledby")!)!;
  expect(label.textContent).toContain("Trust this server certificate?");
  // each choice is explained
  expect(text).toContain("Trust always remembers this certificate on this device");
  expect(text).toContain("Trust once asks again next time");
});

test("choices answer with the decision", async () => {
  let t = await setup();
  expect(document.querySelectorAll(".primary")).toHaveLength(1);
  expect(button("Trust always").classList.contains("primary")).toBe(true);
  button("Trust always").click();
  expect(t.calls.answerPrompt).toEqual([["sid", 7, { kind: "hostKey", decision: "always" }]]);
  remount();

  t = await setup();
  button("Trust once").click();
  expect(t.calls.answerPrompt[0]![2]).toEqual({ kind: "hostKey", decision: "once" });
  remount();

  t = await setup();
  button("Cancel").click();
  expect(t.calls.answerPrompt[0]![2]).toEqual({ kind: "hostKey", decision: "reject" });
});

test("escape cancels and a stray Enter does not trust", async () => {
  const t = await setup();
  // the keyboard starts on the dialog, not on a button
  expect(document.activeElement).toBe(document.querySelector('[role="dialog"]'));
  document.activeElement!.dispatchEvent(
    new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }),
  );
  expect(t.calls.answerPrompt).toEqual([["sid", 7, { kind: "hostKey", decision: "reject" }]]);
});
