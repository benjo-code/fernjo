import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, describe, expect, test, vi } from "vitest";
import { SessionStore, type Tab } from "../sessions.svelte";
import { mockApi } from "../testing";
import type { SessionState } from "../types";
import TabStrip from "./TabStrip.svelte";

let mounted: ReturnType<typeof mount> | null = null;

afterEach(() => {
  if (mounted) unmount(mounted);
  mounted = null;
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

/** Tabs for nodes `a`, `b`, ... with the given states; the last one opened is active. */
function setup(states: SessionState[], opts: { protocol?: (id: string) => "ssh" | "rdp" } = {}) {
  const mock = mockApi();
  const sessions = new SessionStore(
    mock.api,
    (id) => `Name ${id}`,
    () => true,
  );
  states.forEach((state, i) => {
    sessions.open(String.fromCharCode(97 + i));
    sessions.tabs[i]!.state = state;
  });
  mounted = mount(TabStrip, {
    target: document.body,
    props: { sessions, protocolOf: opts.protocol },
  });
  flushSync();
  return { sessions, ...mock };
}

const tabs = () => [...document.querySelectorAll<HTMLElement>("[data-tab-id]")];
const tabOf = (t: Tab) => tabs().find((el) => el.dataset.tabId === t.id)!;
const titles = () => tabs().map((el) => el.querySelector(".title")?.textContent);
const dialog = () => document.querySelector<HTMLElement>('[role="alertdialog"]');

function pointer(type: string, el: Element, x: number, init: PointerEventInit = {}) {
  const Ctor = typeof PointerEvent === "function" ? PointerEvent : MouseEvent;
  el.dispatchEvent(
    new Ctor(type, {
      bubbles: true,
      cancelable: true,
      clientX: x,
      clientY: 10,
      button: 0,
      buttons: type === "pointermove" ? 1 : 0,
      ...init,
    }),
  );
  flushSync();
}

/** jsdom has no layout: every tab is 200 px wide, side by side from x = 0. */
function layout() {
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement,
  ) {
    const i = tabs().indexOf(this);
    const left = i < 0 ? 0 : i * 200;
    return {
      left,
      right: left + 200,
      width: 200,
      top: 0,
      bottom: 36,
      height: 36,
      x: left,
      y: 0,
    } as DOMRect;
  });
}

describe("TabStrip", () => {
  test("shows_state_mark_icon_title_and_caption", () => {
    setup(["connecting", "connected", "needsVault", "disconnected"]);
    const [connecting, connected, needsVault, dropped] = tabs();

    expect(connecting!.querySelector(".lamp")?.getAttribute("data-state")).toBe("connecting");
    expect(connecting!.querySelector(".caption")?.textContent).toBe("Connecting…");
    expect(connecting!.querySelector(".title")?.textContent).toBe("Name a");
    expect(connecting!.querySelector(".protocol svg")).not.toBeNull();

    expect(connected!.querySelector(".lamp")?.getAttribute("data-state")).toBe("live");
    expect(connected!.querySelector(".caption")).toBeNull();

    expect(needsVault!.querySelector(".caption")?.textContent).toBe("Unlock vault");
    expect(needsVault!.querySelector(".lamp")).toBeNull();

    expect(dropped!.querySelector(".lamp")?.getAttribute("data-state")).toBe("dropped");
    expect(dropped!.querySelector("button.close")).not.toBeNull();
  });

  test("a_tab_with_a_pending_prompt_says_what_it_waits_for", () => {
    const { sessions } = setup([
      "connecting",
      "connecting",
      "connecting",
      "needsVault",
      "connecting",
    ]);
    const ask = (i: number, prompt: Tab["prompt"]) => (sessions.tabs[i]!.prompt = prompt);
    ask(0, {
      id: 1,
      prompt: {
        kind: "hostKey",
        host: "h",
        port: 22,
        keyType: "ssh-ed25519",
        fingerprint: "SHA256:x",
      },
    });
    ask(1, {
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
    ask(2, { id: 3, prompt: { kind: "password", user: "u", host: "h" } });
    ask(4, {
      id: 4,
      prompt: {
        kind: "hostKeySynced",
        host: "h",
        port: 22,
        keyType: "ssh-ed25519",
        fingerprint: "SHA256:x",
        previous: [],
      },
    });
    flushSync();
    const caption = (i: number) => tabs()[i]!.querySelector(".caption")?.textContent;
    expect(caption(0)).toBe("Confirm host key");
    expect(caption(1)).toBe("Confirm host key");
    expect(caption(2)).toBe("Needs input");
    expect(caption(3)).toBe("Unlock vault");
    expect(caption(4)).toBe("Confirm host key");
    // answered: back to the plain state caption
    sessions.tabs[2]!.prompt = null;
    flushSync();
    expect(caption(2)).toBe("Connecting…");
  });

  test("a_disconnected_tab_has_no_caption_but_keeps_title_mark_reconnect_and_the_reason_as_tooltip", () => {
    const { sessions } = setup(["disconnected"]);
    sessions.tabs[0]!.detail = "The remote shell ended (exit status 3).";
    flushSync();
    const tab = tabs()[0]!;
    // the reason lives in the bar above the terminal; a caption would cut the title short
    expect(tab.querySelector(".caption")).toBeNull();
    expect(tab.querySelector(".title")?.textContent).toBe("Name a");
    expect(tab.querySelector(".lamp")?.getAttribute("data-state")).toBe("dropped");
    expect(tab.querySelector("button.reconnect")).not.toBeNull();
    expect(tab.querySelector("button.main")?.getAttribute("title")).toBe(
      "Name a: The remote shell ended (exit status 3).",
    );
  });

  test("the_protocol_icon_follows_the_connection", () => {
    setup(["connected"], { protocol: () => "rdp" });
    // the monitor on a stand has the stand path
    expect(tabs()[0]!.querySelector(".protocol path")?.getAttribute("d")).toContain("M8 11v2.5");
  });

  test("reconnect_button_only_on_disconnected_tabs", () => {
    setup(["connecting", "connected", "needsVault", "disconnected", "disconnected"]);
    const withButton = tabs().map((el) => el.querySelector("button.reconnect") !== null);
    expect(withButton).toEqual([false, false, false, true, true]);
    // icon only: the labelled button is in the bar above the terminal, so the title keeps the room
    const button = tabs()[3]!.querySelector("button.reconnect")!;
    expect(button.textContent?.trim()).toBe("");
    expect(button.querySelector("svg")).not.toBeNull();
    expect(button.getAttribute("aria-label")).toBe("Reconnect Name d");
    expect(button.getAttribute("title")).toBe("Reconnect");
  });

  test("clicking_reconnect_calls_the_store", () => {
    const { sessions } = setup(["connected", "disconnected"]);
    const reconnect = vi.spyOn(sessions, "reconnect").mockImplementation(() => {});
    const second = sessions.tabs[1]!;
    tabOf(second).querySelector<HTMLButtonElement>("button.reconnect")!.click();
    expect(reconnect).toHaveBeenCalledWith(second.id);
  });

  test("closing_a_connected_tab_asks_first_and_cancel_keeps_it", async () => {
    const { sessions } = setup(["connected"]);
    const tab = sessions.tabs[0]!;
    const close = vi.spyOn(sessions, "close");
    tabOf(tab).querySelector<HTMLButtonElement>("button.close")!.click();
    await tick();
    flushSync();
    expect(dialog()).not.toBeNull();
    expect(close).not.toHaveBeenCalled();
    // Cancel has the focus, Escape keeps the tab
    const cancel = [...dialog()!.querySelectorAll("button")].find(
      (b) => b.textContent?.trim() === "Cancel",
    )!;
    expect(document.activeElement).toBe(cancel);
    dialog()!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    flushSync();
    expect(dialog()).toBeNull();
    expect(sessions.tabs).toHaveLength(1);

    // the confirm button closes it
    tabOf(tab).querySelector<HTMLButtonElement>("button.close")!.click();
    await tick();
    flushSync();
    [...dialog()!.querySelectorAll("button")]
      .find((b) => b.textContent?.trim() === "Close")!
      .click();
    flushSync();
    expect(close).toHaveBeenCalledWith(tab.id);
    expect(sessions.tabs).toHaveLength(0);
  });

  test("closing_a_dropped_tab_does_not_ask", () => {
    const { sessions } = setup(["disconnected", "connecting", "needsVault"]);
    for (let n = 3; n > 0; n--) {
      tabs()[0]!.querySelector<HTMLButtonElement>("button.close")!.click();
      flushSync();
      expect(dialog()).toBeNull();
      expect(sessions.tabs).toHaveLength(n - 1);
    }
  });

  test("pointer_drag_reorders_and_escape_cancels", () => {
    const { sessions } = setup(["connected", "connected", "connected"]);
    layout();
    const [a, b, c] = sessions.tabs.map((t) => t.id);
    const grab = () => tabs()[0]!.querySelector(".main")!;
    const order = () => sessions.tabs.map((t) => t.id);

    // below the 4 px threshold nothing is dragged
    pointer("pointerdown", grab(), 50);
    pointer("pointermove", grab(), 53);
    pointer("pointerup", grab(), 53);
    expect(order()).toEqual([a, b, c]);

    // dragging the first tab past the middle of the last one puts it at the end
    pointer("pointerdown", grab(), 50);
    pointer("pointermove", grab(), 580);
    pointer("pointerup", grab(), 580);
    expect(order()).toEqual([b, c, a]);

    // Escape during a drag leaves the order alone, and the release does not reorder
    const first = () => tabs()[0]!.querySelector(".main")!;
    pointer("pointerdown", first(), 50);
    pointer("pointermove", first(), 580);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    flushSync();
    pointer("pointerup", first(), 580);
    expect(order()).toEqual([b, c, a]);
  });

  test("a_drag_does_not_activate_the_tab_it_ends_on", () => {
    const { sessions } = setup(["connected", "connected", "connected"]);
    layout();
    const active = sessions.activeId;
    const first = tabs()[0]!.querySelector<HTMLElement>(".main")!;
    pointer("pointerdown", first, 50);
    pointer("pointermove", first, 300);
    pointer("pointerup", first, 300);
    first.click();
    expect(sessions.activeId).toBe(active);
  });

  test("active_tab_is_marked_for_assistive_tech", () => {
    const { sessions } = setup(["connected", "connected"]);
    expect(document.querySelector('[role="tablist"]')).not.toBeNull();
    const marks = () =>
      tabs().map((el) => el.querySelector('[role="tab"]')?.getAttribute("aria-selected"));
    expect(marks()).toEqual(["false", "true"]);
    tabs()[0]!.querySelector<HTMLElement>('[role="tab"]')!.click();
    flushSync();
    expect(sessions.activeId).toBe(sessions.tabs[0]!.id);
    expect(marks()).toEqual(["true", "false"]);
    expect(titles()).toEqual(["Name a", "Name b"]);
  });

  test("a_restored_disconnected_tab_shows_no_lamp_and_a_reconnected_one_is_dropped_again", () => {
    const mock = mockApi();
    const sessions = new SessionStore(
      mock.api,
      (id) => `Name ${id}`,
      () => true,
    );
    sessions.restore([{ target: { node: "r" }, protocol: "ssh", title: "R", usesVault: false }], 0);
    mounted = mount(TabStrip, { target: document.body, props: { sessions } });
    flushSync();
    const tab = () => document.querySelector<HTMLElement>("[data-tab-id]")!;
    // nothing connected yet: neither a lamp nor the vault lock
    expect(tab().querySelector(".lamp")).toBeNull();
    expect(tab().querySelector(".mark svg")).toBeNull();
    // the flag is cleared when the user connects; a later drop is a real drop
    sessions.tabs[0]!.restored = false;
    flushSync();
    expect(tab().querySelector(".lamp")?.getAttribute("data-state")).toBe("dropped");
  });

  test("reconnect_all_shows_only_while_a_restored_tab_is_disconnected", async () => {
    const onreconnectall = vi.fn();
    const mock = mockApi();
    const sessions = new SessionStore(
      mock.api,
      (id) => `Name ${id}`,
      () => true,
    );
    sessions.open("plain");
    sessions.tabs[0]!.state = "disconnected";
    mounted = mount(TabStrip, { target: document.body, props: { sessions, onreconnectall } });
    flushSync();
    const all = () =>
      document.querySelector<HTMLButtonElement>('button[aria-label="Reconnect all restored tabs"]');
    // a dropped tab that was not restored does not offer it
    expect(all()).toBeNull();

    sessions.restore([{ target: { node: "r" }, protocol: "ssh", title: "R", usesVault: false }], 0);
    flushSync();
    expect(all()).not.toBeNull();
    expect(all()!.textContent).toContain("Reconnect all");
    all()!.click();
    expect(onreconnectall).toHaveBeenCalledTimes(1);

    sessions.tabs.find((t) => t.restored)!.state = "connecting";
    flushSync();
    expect(all()).toBeNull();
  });
});
