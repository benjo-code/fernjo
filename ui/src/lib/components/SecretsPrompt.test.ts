import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, expect, test } from "vitest";
import { startedTab } from "../testing";
import type { Prompt } from "../types";
import SecretsPrompt from "./SecretsPrompt.svelte";

let mounted: ReturnType<typeof mount> | null = null;

afterEach(() => {
  if (mounted) unmount(mounted);
  mounted = null;
  document.body.innerHTML = "";
});

type Secretish = Extract<
  Prompt,
  { kind: "keyboardInteractive" | "password" | "keyPassphrase" | "username" }
>;

async function setup(prompt: Secretish) {
  const t = await startedTab();
  t.emit({ kind: "prompt", id: 5, prompt });
  mounted = mount(SecretsPrompt, {
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

const inputs = () =>
  [...document.querySelectorAll<HTMLInputElement>("input")].filter((i) => i.type !== "checkbox");
const button = (text: string) =>
  [...document.querySelectorAll<HTMLButtonElement>("button")].find(
    (b) => b.textContent?.trim() === text,
  )!;

async function type(el: HTMLInputElement, text: string) {
  el.value = text;
  el.dispatchEvent(new Event("input", { bubbles: true }));
  await tick();
}

const keyboardInteractive: Secretish = {
  kind: "keyboardInteractive",
  host: "bastion.example.org",
  name: "Two-factor",
  instructions: "Use your authenticator app.",
  prompts: [
    { text: "Password: ", echo: false },
    { text: "Verification code: ", echo: true },
  ],
};

test("secrets_prompt_masks_unless_echo_and_sends_values_in_order", async () => {
  const t = await setup(keyboardInteractive);
  expect(document.body.textContent).toContain("Use your authenticator app.");
  const [first, second] = inputs();
  expect(inputs()).toHaveLength(2);
  expect(first!.type).toBe("password");
  expect(second!.type).toBe("text");
  // only the masked field has a show toggle
  expect(document.querySelectorAll("button[aria-pressed]")).toHaveLength(1);
  expect(document.querySelector(`label[for="${first!.id}"]`)!.textContent).toContain("Password");
  expect(document.activeElement).toBe(first);
  // one yellow key
  expect(document.querySelectorAll(".primary")).toHaveLength(1);
  expect(button("Continue").classList.contains("primary")).toBe(true);

  await type(first!, "hunter2");
  await type(second!, "123456");
  button("Continue").click();
  expect(t.calls.answerPrompt).toEqual([
    ["sid", 5, { kind: "secrets", values: ["hunter2", "123456"] }],
  ]);
});

test("secrets_prompt_cancel_sends_cancel_and_escape_does_the_same", async () => {
  let t = await setup({ kind: "password", user: "deploy", host: "files.example.org" });
  expect(document.body.textContent).toContain("deploy@files.example.org");
  expect(inputs()[0]!.type).toBe("password");
  button("Cancel").click();
  expect(t.calls.answerPrompt).toEqual([["sid", 5, { kind: "cancel" }]]);
  remount();

  t = await setup({ kind: "keyPassphrase" });
  expect(inputs()[0]!.type).toBe("password");
  await type(inputs()[0]!, "typed then abandoned");
  inputs()[0]!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  expect(t.calls.answerPrompt).toEqual([["sid", 5, { kind: "cancel" }]]);
});

test("typed_values_are_cleared_after_submit", async () => {
  const t = await setup(keyboardInteractive);
  await type(inputs()[0]!, "very-secret-value");
  await type(inputs()[1]!, "424242");
  const before = inputs();
  button("Continue").click();
  // cleared synchronously, before the api call settles
  flushSync();
  for (const el of before) expect(el.value).toBe("");
  await tick();
  expect(document.body.innerHTML).not.toContain("very-secret-value");
  expect(document.body.innerHTML).not.toContain("424242");
  // the store holds none of it
  expect(t.tab.prompt).toBeNull();
  expect(JSON.stringify(t.sessions.tabs)).not.toContain("very-secret-value");
});

test("username_prompt_sends_text", async () => {
  const t = await setup({ kind: "username", host: "files.example.org" });
  const [field] = inputs();
  expect(inputs()).toHaveLength(1);
  expect(field!.type).toBe("text");
  expect(document.querySelectorAll("button[aria-pressed]")).toHaveLength(0);
  expect(document.body.textContent).toContain("files.example.org");
  await type(field!, "alice");
  field!.form!.requestSubmit();
  expect(t.calls.answerPrompt).toEqual([["sid", 5, { kind: "text", value: "alice" }]]);
});

// M11 review #39: a server's keyboard-interactive round must not pass for fernjo's own prompt.
test("keyboard_interactive_names_the_server_and_marks_its_text_as_the_servers", async () => {
  // a server that copies fernjo's own vault prompt word for word
  await setup({
    kind: "keyboardInteractive",
    host: "bastion.example.org",
    name: "Vault locked",
    instructions: "Enter your vault master password to continue.",
    prompts: [{ text: "Master password: ", echo: false }],
  });
  const title = document.querySelector("h2")!.textContent!;
  expect(title).not.toContain("Vault");
  expect(title).toMatch(/server/i);
  expect(document.body.textContent).toContain("The server bastion.example.org asks:");
  // everything the server wrote, its field labels included, sits in the marked server block
  const quoted = document.querySelector('[aria-label="Text from the server bastion.example.org"]')!;
  expect(quoted).not.toBeNull();
  expect(quoted.textContent).toContain("Vault locked");
  expect(quoted.textContent).toContain("Enter your vault master password to continue.");
  expect(quoted.querySelector("label")!.textContent).toContain("Master password");
  expect(quoted.contains(inputs()[0]!)).toBe(true);
  // fernjo's own buttons stay outside it
  expect(quoted.contains(button("Continue"))).toBe(false);
  // and fernjo says, in its own words, where the answer goes
  expect(document.body.textContent).toContain("Your answer goes to bastion.example.org");
});

test("other_secret_prompts_have_no_server_block", async () => {
  await setup({ kind: "password", user: "deploy", host: "files.example.org" });
  expect(document.querySelector('[aria-label^="Text from the server"]')).toBeNull();
  expect(document.body.textContent).not.toContain("asks:");
});
