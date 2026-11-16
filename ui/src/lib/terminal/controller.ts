// The xterm.js side of a terminal tab: rendering, fit, search, links, clipboard. It knows nothing
// about sessions; the component wires `deps` to the `SessionStore` bridge.
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { WebglAddon } from "@xterm/addon-webgl";
import { Terminal } from "@xterm/xterm";
import { isMac, routeKey } from "../shortcuts";
import { terminalTheme } from "./theme";

export interface TerminalOptions {
  /** `null` means the default monospace stack (`--font-mono`). */
  fontFamily: string | null;
  fontSize: number;
  scrollback: number;
}

export interface TerminalDeps {
  onData(bytes: Uint8Array): void;
  onResize(cols: number, rows: number): void;
  onAck(bytes: number): void;
  openUrl(url: string): void;
  /** Ctrl+Shift+F inside the terminal. */
  onSearchRequest?(): void;
}

const DEFAULT_CONTRAST = 4.5;
/** DESIGN.md: 13px text on a 19px line. */
const LINE_HEIGHT = 19 / 13;

/** How long a menu paste waits for the clipboard before giving up. */
const PASTE_TIMEOUT_MS = 2500;

const encoder = new TextEncoder();

function rootStyle(): CSSStyleDeclaration {
  return getComputedStyle(document.documentElement);
}

function fontStack(family: string | null): string {
  const mono = rootStyle().getPropertyValue("--font-mono").trim() || "monospace";
  if (!family?.trim()) return mono;
  return `"${family.trim().replace(/"/g, "")}", ${mono}`;
}

function minContrast(): number {
  const n = Number.parseFloat(rootStyle().getPropertyValue("--terminal-min-contrast"));
  return Number.isFinite(n) && n >= 1 ? n : DEFAULT_CONTRAST;
}

/** xterm hands binary input (mouse reports and the like) as a string of code points below 256. */
function latin1(s: string): Uint8Array {
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i) & 0xff;
  return out;
}

export class TerminalController {
  #term: Terminal;
  #fit = new FitAddon();
  #search = new SearchAddon();
  #webgl: WebglAddon | null = null;
  #el: HTMLElement | null = null;
  #visible = false;
  #lastSent: { cols: number; rows: number } | null = null;
  #deps: TerminalDeps;
  #mac = isMac();

  constructor(opts: TerminalOptions, deps: TerminalDeps) {
    this.#deps = deps;
    this.#term = new Terminal({
      fontFamily: fontStack(opts.fontFamily),
      fontSize: opts.fontSize,
      lineHeight: LINE_HEIGHT,
      scrollback: opts.scrollback,
      minimumContrastRatio: minContrast(),
      theme: terminalTheme(rootStyle()),
      cursorBlink: true,
      allowProposedApi: true,
    });
    this.#term.onData((s) => deps.onData(encoder.encode(s)));
    this.#term.onBinary((s) => deps.onData(latin1(s)));
    this.#term.loadAddon(this.#fit);
    this.#term.loadAddon(this.#search);
    // Links only ever leave through `openUrl`; the default handler would navigate the webview.
    this.#term.loadAddon(
      new WebLinksAddon((event, uri) => {
        event.preventDefault();
        deps.openUrl(uri);
      }),
    );
    this.#term.attachCustomKeyEventHandler((e) => this.#onKey(e));
  }

  get cols(): number {
    return this.#term.cols;
  }

  get rows(): number {
    return this.#term.rows;
  }

  mount(el: HTMLElement): void {
    this.#el = el;
    this.#term.open(el);
    if (this.#visible) this.#loadWebgl();
  }

  write(bytes: Uint8Array): void {
    if (bytes.length === 0) return;
    // Flow control: Rust is told about the bytes only once xterm has parsed them.
    this.#term.write(bytes, () => this.#deps.onAck(bytes.length));
  }

  setVisible(visible: boolean): void {
    if (visible === this.#visible) return;
    this.#visible = visible;
    if (visible) {
      if (this.#el) this.#loadWebgl();
      this.fit();
    } else {
      this.#dropWebgl();
    }
  }

  setOptions(o: TerminalOptions): void {
    const t = this.#term.options;
    const family = fontStack(o.fontFamily);
    if (t.fontFamily !== family) t.fontFamily = family;
    if (t.fontSize !== o.fontSize) t.fontSize = o.fontSize;
    if (t.scrollback !== o.scrollback) t.scrollback = o.scrollback;
    this.fit();
  }

  /** Fits to the element and reports a changed size once; does nothing while the element is hidden. */
  fit(): void {
    const el = this.#el;
    if (!el || el.clientWidth === 0 || el.clientHeight === 0) return;
    const dims = this.#fit.proposeDimensions();
    if (!dims || !(dims.cols > 0) || !(dims.rows > 0)) return;
    if (dims.cols !== this.#term.cols || dims.rows !== this.#term.rows) {
      this.#term.resize(dims.cols, dims.rows);
    }
    if (this.#lastSent?.cols === dims.cols && this.#lastSent.rows === dims.rows) return;
    this.#lastSent = { cols: dims.cols, rows: dims.rows };
    this.#deps.onResize(dims.cols, dims.rows);
  }

  focus(): void {
    this.#term.focus();
  }

  search(term: string, dir: "next" | "previous"): boolean {
    if (term === "") {
      this.#search.clearDecorations();
      return false;
    }
    return dir === "next" ? this.#search.findNext(term) : this.#search.findPrevious(term);
  }

  clearSearch(): void {
    this.#search.clearDecorations();
  }

  hasSelection(): boolean {
    return this.#term.hasSelection();
  }

  async copySelection(): Promise<void> {
    if (!this.#term.hasSelection()) return;
    await navigator.clipboard.writeText(this.#term.getSelection());
  }

  /**
   * Menu paste. WebView2 shows a permission prompt for `readText` (measured: it stays pending),
   * so keyboard paste (Ctrl+V, Ctrl+Shift+V) is left to the browser's native paste event, which
   * xterm handles without a prompt.
   */
  async paste(): Promise<void> {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<null>((resolve) => {
      timer = setTimeout(() => resolve(null), PASTE_TIMEOUT_MS);
    });
    try {
      // A late answer after the timeout is dropped: it could land in whatever has focus by then.
      const text = await Promise.race([navigator.clipboard.readText(), timeout]);
      if (text) this.#term.paste(text);
    } finally {
      clearTimeout(timer);
    }
  }

  refreshTheme(): void {
    this.#term.options.theme = terminalTheme(rootStyle());
    this.#term.options.minimumContrastRatio = minContrast();
  }

  dispose(): void {
    this.#dropWebgl();
    this.#term.dispose();
    this.#el = null;
  }

  /**
   * Key contract (shared with App's window handler, which must NOT dispatch copy, paste or
   * terminalSearch): `routeKey(e, mac, true)` decides.
   * - `copy` and `terminalSearch` are handled here (preventDefault + stopPropagation) and xterm
   *   ignores them.
   * - every other app action (tabs, close, filter, and paste) returns false WITHOUT
   *   preventDefault, so xterm ignores it and the event bubbles to the window handler; for
   *   paste the native paste event then reaches xterm's own handler.
   * - everything else (plain Ctrl+C/W/B, Alt+letter, Escape) goes to the program.
   */
  #onKey(e: KeyboardEvent): boolean {
    if (e.type !== "keydown") return true;
    const action = routeKey(e, this.#mac, true);
    if (action === null) return true;
    if (action === "copy" || action === "terminalSearch") {
      e.preventDefault();
      e.stopPropagation();
      if (action === "copy") void this.copySelection().catch(() => {});
      else this.#deps.onSearchRequest?.();
    }
    return false;
  }

  #loadWebgl(): void {
    if (this.#webgl) return;
    try {
      const addon = new WebglAddon();
      addon.onContextLoss(() => {
        // The DOM renderer takes over; the user gets no message for this.
        if (this.#webgl === addon) this.#webgl = null;
        try {
          addon.dispose();
        } catch {
          // already gone with the context
        }
      });
      this.#term.loadAddon(addon);
      this.#webgl = addon;
    } catch {
      this.#webgl = null;
    }
  }

  #dropWebgl(): void {
    const addon = this.#webgl;
    this.#webgl = null;
    try {
      addon?.dispose();
    } catch {
      // nothing to free
    }
  }
}
