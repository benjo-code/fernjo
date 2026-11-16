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
    attachCustomKeyEventHandler() {}
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
    onData: vi.fn<(b: Uint8Array) => void>(),
    onResize: vi.fn<(c: number, r: number) => void>(),
    onAck: vi.fn<(n: number) => void>(),
    openUrl: vi.fn<(u: string) => void>(),
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

  test("copy_and_paste_use_the_browser_clipboard", async () => {
    const { c } = setup();
    const writeText = vi.fn(async () => {});
    const readText = vi.fn(async () => "pasted");
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText, readText },
    });
    term().selection = "sel";
    await c.copySelection();
    expect(writeText).toHaveBeenCalledWith("sel");
    await c.paste();
    expect(term().pasted).toEqual(["pasted"]);
  });
});
