import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, test } from "vitest";
import { SessionStore } from "../sessions.svelte";
import { apiError, mockApi } from "../testing";
import QuickConnect from "./QuickConnect.svelte";

let mounted: ReturnType<typeof mount> | null = null;

afterEach(() => {
  if (mounted) unmount(mounted);
  mounted = null;
  document.body.innerHTML = "";
});

const tick = () => new Promise<void>((r) => setTimeout(r, 0));

function setup(impl: Parameters<typeof mockApi>[1] = {}, mac = false) {
  const mock = mockApi([], impl);
  const sessions = new SessionStore(
    mock.api,
    (id) => id,
    () => true,
  );
  mounted = mount(QuickConnect, { target: document.body, props: { sessions, mac } });
  flushSync();
  const input = document.querySelector<HTMLInputElement>("input")!;
  const type = (text: string) => {
    input.value = text;
    input.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
  };
  const press = async (key: string) => {
    input.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
    await tick();
    flushSync();
  };
  return { ...mock, sessions, input, type, press };
}

describe("QuickConnect", () => {
  test("the_placeholder_shows_the_target_form_and_the_field_has_a_label", () => {
    const { input } = setup();
    expect(input.placeholder).toBe("user@host[:port]");
    expect(input.getAttribute("aria-label")).toBe("Quick connect");
    expect(input.title).toBe("Quick connect (Ctrl+Shift+N)");
  });

  test("the_tooltip_uses_cmd_on_a_mac", () => {
    expect(setup({}, true).input.title).toBe("Quick connect (Cmd+N)");
  });

  test("enter_opens_the_target_and_clears_the_field", async () => {
    const { input, type, press, sessions, calls } = setup();
    type("root@h.lab");
    await press("Enter");
    expect(calls.quickParse).toEqual([["root@h.lab"]]);
    expect(sessions.tabs.map((t) => t.title)).toEqual(["root@h.lab"]);
    expect(input.value).toBe("");
  });

  test("a_bad_target_shows_the_error_text_and_keeps_the_input", async () => {
    const { input, type, press, sessions } = setup({
      quickParse: async () => {
        throw apiError("quick_bad_port", "The port must be between 1 and 65535");
      },
    });
    type("h:0");
    await press("Enter");
    expect(sessions.tabs).toHaveLength(0);
    expect(input.value).toBe("h:0");
    expect(document.querySelector('[role="alert"]')?.textContent).toBe(
      "The port must be between 1 and 65535",
    );
    expect(input.getAttribute("aria-invalid")).toBe("true");
    type("h:22");
    expect(document.querySelector('[role="alert"]')).toBeNull();
  });

  test("escape_clears_and_leaves_the_field", async () => {
    const { input, type, press } = setup();
    input.focus();
    type("root@h");
    await press("Escape");
    expect(input.value).toBe("");
    expect(document.activeElement).not.toBe(input);
  });

  test("an_empty_field_does_nothing_on_enter", async () => {
    const { press, calls } = setup();
    await press("Enter");
    expect(calls.quickParse).toEqual([]);
  });
});
