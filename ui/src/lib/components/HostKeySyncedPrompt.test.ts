import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, expect, test } from "vitest";
import { startedTab } from "../testing";
import type { Prompt } from "../types";
import HostKeySyncedPrompt from "./HostKeySyncedPrompt.svelte";

let mounted: ReturnType<typeof mount> | null = null;

afterEach(() => {
  if (mounted) unmount(mounted);
  mounted = null;
  document.body.innerHTML = "";
});

const FINGERPRINT = "SHA256:abcDEF123+/xyz";
type HostKey = Extract<Prompt, { kind: "hostKeySynced" }>;
const prompt: HostKey = {
  kind: "hostKeySynced",
  host: "files.example.org",
  port: 2222,
  keyType: "ssh-ed25519",
  fingerprint: FINGERPRINT,
  previous: [],
};

async function setup() {
  const t = await startedTab();
  t.emit({ kind: "prompt", id: 7, prompt });
  mounted = mount(HostKeySyncedPrompt, {
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

test("synced_host_key_prompt_shows_fingerprint_and_maps_buttons_to_answers", async () => {
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

  // exactly one yellow key, on "Confirm and connect"
  expect(document.querySelectorAll(".primary")).toHaveLength(1);
  expect(button("Confirm and connect").classList.contains("primary")).toBe(true);

  button("Confirm and connect").click();
  expect(t.calls.answerPrompt).toEqual([["sid", 7, { kind: "hostKey", decision: "always" }]]);
});

test("synced_host_key_prompt_trust_once_cancel_and_escape_map_to_answers", async () => {
  let t = await setup();
  button("Connect once").click();
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

test("synced_host_key_prompt_is_a_labelled_dialog_and_the_keyboard_does_not_start_on_the_yellow_key", async () => {
  await setup();
  const dialog = document.querySelector('[role="dialog"]')!;
  const label = document.getElementById(dialog.getAttribute("aria-labelledby")!)!;
  expect(label.textContent).toContain("Confirm this host key");
  // a stray Enter (typed into the terminal a moment ago) must not trust the key
  expect(document.activeElement).toBe(dialog);
});

test("a_click_on_the_empty_area_refocuses_the_prompt_and_escape_still_works", async () => {
  const t = await setup();
  const dialog = document.querySelector<HTMLElement>('[role="dialog"]')!;
  button("Connect once").focus();
  const wrap = document.querySelector<HTMLElement>(".wrap")!;
  const down = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
  wrap.dispatchEvent(down);
  // the default (focus to <body>) is cancelled and the prompt has the focus
  expect(down.defaultPrevented).toBe(true);
  expect(document.activeElement).toBe(dialog);
  // a mousedown inside the column is left alone (text selection, buttons)
  const inside = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
  button("Cancel").dispatchEvent(inside);
  expect(inside.defaultPrevented).toBe(false);

  document.activeElement!.dispatchEvent(
    new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
  );
  expect(t.calls.answerPrompt[0]![2]).toEqual({ kind: "hostKey", decision: "reject" });
});

test("tab_and_shift_tab_wrap_inside_the_prompt", async () => {
  await setup();
  const first = button("Confirm and connect");
  const last = button("Cancel");
  const press = (shiftKey: boolean) => {
    const e = new KeyboardEvent("keydown", {
      key: "Tab",
      shiftKey,
      bubbles: true,
      cancelable: true,
    });
    document.activeElement!.dispatchEvent(e);
    return e;
  };
  // Shift+Tab from the dialog itself or the first button goes to the last, never out of the prompt
  const dialog = document.querySelector<HTMLElement>('[role="dialog"]')!;
  expect(document.activeElement).toBe(dialog);
  expect(press(true).defaultPrevented).toBe(true);
  expect(document.activeElement).toBe(last);
  // Tab from the last wraps to the first
  expect(press(false).defaultPrevented).toBe(true);
  expect(document.activeElement).toBe(first);
  expect(press(true).defaultPrevented).toBe(true);
  expect(document.activeElement).toBe(last);
  // in the middle the browser's own order is used
  button("Connect once").focus();
  expect(press(false).defaultPrevented).toBe(false);
});

test("synced_host_key_prompt_explains_where_the_key_came_from_and_shows_no_previous_when_empty", async () => {
  await setup();
  const text = document.body.textContent!.replace(/\s+/g, " ");
  expect(text).toContain(
    "This host key was added by another device or file. This device has not confirmed it. If you did not add this host yourself on one of your devices, cancel.",
  );
  expect(text).not.toContain("previously trusted");
});

test("synced_host_key_prompt_lists_the_previously_trusted_fingerprints", async () => {
  const t = await startedTab();
  const withPrevious: HostKey = { ...prompt, previous: ["SHA256:old1", "SHA256:old2"] };
  t.emit({ kind: "prompt", id: 7, prompt: withPrevious });
  mounted = mount(HostKeySyncedPrompt, {
    target: document.body,
    props: { tab: t.tab, sessions: t.sessions, prompt: withPrevious },
  });
  flushSync();
  await tick();
  expect(document.body.textContent).toContain("This device previously trusted");
  const codes = [...document.querySelectorAll<HTMLElement>("code")].map((c) => c.textContent);
  expect(codes).toEqual(expect.arrayContaining([FINGERPRINT, "SHA256:old1", "SHA256:old2"]));
  expect(document.querySelectorAll("code")).toHaveLength(3);
});
