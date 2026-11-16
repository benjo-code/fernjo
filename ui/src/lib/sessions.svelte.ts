// View state for terminal sessions: one tab per session, the prompt it waits on and the bridge to
// its terminal. Everything about a connection (credentials, host keys, state changes) is decided
// in Rust; this store only mirrors the events. Typed prompt answers go straight to the api and are
// never kept here.
import { toApiError } from "./api";
import { reorder } from "./tabs";
import type { Api, Prompt, PromptAnswer, SessionEvent, SessionState } from "./types";

export interface Tab {
  /** Local key, stable from the click on; the session id only exists once Rust answered. */
  id: string;
  sessionId: string | null;
  nodeId: string;
  title: string;
  state: SessionState;
  detail: string | null;
  prompt: { id: number; prompt: Prompt } | null;
}

/** What the terminal component hands over: where output goes and how big the terminal is. */
export interface TerminalBridge {
  write(bytes: Uint8Array): void;
  size(): { cols: number; rows: number };
}

export type Lamp = "live" | "connecting" | "dropped";

const RECONNECTED = "\r\n\x1b[2m— reconnected —\x1b[0m\r\n";

/** Per-tab plumbing that is not view state. */
interface Runtime {
  bridge: TerminalBridge | null;
  /** Output that arrived while no terminal was attached. */
  pending: Uint8Array[];
  started: boolean;
  closed: boolean;
  sessionId: string | null;
}

/** `instanceof ArrayBuffer` would fail across realms (jsdom in tests); the tag does not. */
function isBuffer(e: SessionEvent | ArrayBuffer): e is ArrayBuffer {
  return Object.prototype.toString.call(e) === "[object ArrayBuffer]";
}

export class SessionStore {
  tabs = $state<Tab[]>([]);
  activeId = $state<string | null>(null);
  /** The last failure to start a session; shown by the app, replaced by the next one. */
  error = $state<string | null>(null);
  /**
   * Counts the moments a tab was asked to come to the front (open, activate, next, previous), even
   * when it already was the active one. Closing a tab does not count. The app leaves other views
   * (the vault manager) when it changes.
   */
  revealed = $state(0);
  /** A close that waits for the user's answer (only a running session is worth a question). */
  closeRequest = $state<{ id: string; title: string } | null>(null);

  #api: Api;
  #nameOf: (nodeId: string) => string;
  // A plain Map on purpose: plumbing, not view state; effects that call the bridge must not subscribe to it.
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  #runtime = new Map<string, Runtime>();
  #counter = 0;

  constructor(api: Api, nameOf: (nodeId: string) => string) {
    this.#api = api;
    this.#nameOf = nameOf;
  }

  open(nodeId: string, opts: { another?: boolean } = {}): void {
    if (!opts.another) {
      const existing = this.tabs.find((t) => t.nodeId === nodeId);
      if (existing) {
        this.activeId = existing.id;
        this.revealed++;
        return;
      }
    }
    const name = this.#nameOf(nodeId);
    const taken = this.tabs.filter((t) => t.nodeId === nodeId).map((t) => t.title);
    let title = name;
    for (let n = 2; taken.includes(title); n++) title = `${name} (${n})`;
    const id = `tab-${++this.#counter}`;
    this.#runtime.set(id, {
      bridge: null,
      pending: [],
      started: false,
      closed: false,
      sessionId: null,
    });
    this.tabs.push({
      id,
      sessionId: null,
      nodeId,
      title,
      state: "connecting",
      detail: null,
      prompt: null,
    });
    this.activeId = id;
    this.revealed++;
  }

  activate(id: string): void {
    if (!this.tabs.some((t) => t.id === id)) return;
    this.activeId = id;
    this.revealed++;
  }

  next(): void {
    this.#step(1);
  }

  previous(): void {
    this.#step(-1);
  }

  #step(delta: number): void {
    if (this.tabs.length === 0) return;
    const at = this.tabs.findIndex((t) => t.id === this.activeId);
    const to = (at + delta + this.tabs.length) % this.tabs.length;
    this.activeId = this.tabs[to]!.id;
    this.revealed++;
  }

  /** The one closing rule (× button and shortcut): a connected tab asks first, others close. */
  requestClose(id: string): void {
    const tab = this.tabs.find((t) => t.id === id);
    if (!tab) return;
    if (tab.state === "connected") this.closeRequest = { id: tab.id, title: tab.title };
    else this.close(id);
  }

  confirmClose(): void {
    const request = this.closeRequest;
    this.closeRequest = null;
    if (request) this.close(request.id);
  }

  cancelClose(): void {
    this.closeRequest = null;
  }

  close(id: string): void {
    if (this.closeRequest?.id === id) this.closeRequest = null;
    const at = this.tabs.findIndex((t) => t.id === id);
    if (at < 0) return;
    const rt = this.#runtime.get(id);
    if (rt) {
      rt.closed = true;
      // Without an id the session may still be starting: the open call ends it on arrival.
      if (rt.sessionId) this.#fire(this.#api.closeSession(rt.sessionId));
    }
    this.#runtime.delete(id);
    this.tabs.splice(at, 1);
    if (this.activeId === id) {
      this.activeId = (this.tabs[at] ?? this.tabs[at - 1])?.id ?? null;
    }
  }

  /** Ends every session (the window is going away or reloading). */
  closeAll(): void {
    for (const t of [...this.tabs]) this.close(t.id);
  }

  reconnect(id: string): void {
    const tab = this.tabs.find((t) => t.id === id);
    const rt = this.#runtime.get(id);
    if (!tab || !rt?.sessionId || tab.state !== "disconnected") return;
    const { cols, rows } = rt.bridge?.size() ?? { cols: 80, rows: 24 };
    const previous = { state: tab.state, detail: tab.detail };
    tab.state = "connecting";
    tab.detail = null;
    tab.prompt = null;
    this.#guard(
      tab,
      // The separator is written only once Rust accepted the reconnect.
      this.#api.reconnectSession(rt.sessionId, cols, rows).then(() => {
        this.#emit(rt, new TextEncoder().encode(RECONNECTED));
      }),
      () => {
        // The click raced the state: undo the optimistic change, the next event decides.
        tab.state = previous.state;
        tab.detail = previous.detail;
      },
    );
  }

  reorder(fromId: string, beforeId: string | null): void {
    const next = reorder(this.tabs, fromId, beforeId);
    if (next !== this.tabs) this.tabs = next;
  }

  answer(id: string, answer: PromptAnswer): void {
    const tab = this.tabs.find((t) => t.id === id);
    const rt = this.#runtime.get(id);
    if (!tab || !rt?.sessionId || !tab.prompt) return;
    const promptId = tab.prompt.id;
    tab.prompt = null;
    // `answer` is not stored anywhere: it goes to the api and this frame ends.
    this.#guard(tab, this.#api.answerPrompt(rt.sessionId, promptId, answer));
  }

  resume(id: string, skipCredential: boolean): void {
    const tab = this.tabs.find((t) => t.id === id);
    const rt = this.#runtime.get(id);
    if (!tab || !rt?.sessionId || tab.state !== "needsVault") return;
    this.#guard(tab, this.#api.resumeSession(rt.sessionId, skipCredential));
  }

  lampFor(nodeId: string): Lamp | null {
    let lamp: Lamp | null = null;
    for (const t of this.tabs) {
      if (t.nodeId !== nodeId) continue;
      // "needsVault" counts as connecting here (the tab shows a lock icon, the tree a pulsing ring).
      if (t.state === "connected") return "live";
      if (t.state === "disconnected") lamp ??= "dropped";
      else lamp = "connecting";
    }
    return lamp;
  }

  // --- terminal bridge ---------------------------------------------------------------------

  /** Connects the terminal; the first attach starts the session at the fitted size. */
  attachTerminal(id: string, bridge: TerminalBridge): void {
    const tab = this.tabs.find((t) => t.id === id);
    const rt = this.#runtime.get(id);
    if (!tab || !rt) return;
    rt.bridge = bridge;
    const pending = rt.pending;
    rt.pending = [];
    for (const chunk of pending) bridge.write(chunk);
    if (rt.started) return;
    rt.started = true;
    const { cols, rows } = bridge.size();
    this.#api
      .openSession(tab.nodeId, (e) => this.#onMessage(id, e), cols, rows)
      .then(
        (sessionId) => {
          if (rt.closed) {
            this.#fire(this.#api.closeSession(sessionId));
            return;
          }
          rt.sessionId = sessionId;
          tab.sessionId = sessionId;
        },
        (e: unknown) => {
          if (rt.closed) return;
          this.error = toApiError(e).message;
          this.close(id);
        },
      );
  }

  /** The terminal component is gone (for example a remount); output waits for the next one. */
  detachTerminal(id: string): void {
    const rt = this.#runtime.get(id);
    if (rt) rt.bridge = null;
  }

  input(id: string, bytes: Uint8Array): void {
    // Keystrokes belong to a running shell only: while a prompt or the vault is pending (or the
    // tab dropped) Rust has no shell and rejects them anyway, so they are not even sent.
    if (this.tabs.find((t) => t.id === id)?.state !== "connected") return;
    const sessionId = this.#runtime.get(id)?.sessionId;
    if (sessionId) this.#fire(this.#api.sessionInput(sessionId, bytes));
  }

  resize(id: string, cols: number, rows: number): void {
    if (cols < 1 || rows < 1) return;
    const sessionId = this.#runtime.get(id)?.sessionId;
    if (sessionId) this.#fire(this.#api.sessionResize(sessionId, cols, rows));
  }

  ack(id: string, bytes: number): void {
    const sessionId = this.#runtime.get(id)?.sessionId;
    if (sessionId) this.#fire(this.#api.sessionAck(sessionId, bytes));
  }

  // --- internals ---------------------------------------------------------------------------

  #onMessage(id: string, e: SessionEvent | ArrayBuffer): void {
    const rt = this.#runtime.get(id);
    const tab = this.tabs.find((t) => t.id === id);
    if (!rt || !tab) return;
    if (isBuffer(e)) {
      this.#emit(rt, new Uint8Array(e));
    } else if (e.kind === "state") {
      tab.state = e.state;
      tab.detail = e.detail;
      if (e.state === "disconnected") tab.prompt = null;
    } else {
      tab.prompt = { id: e.id, prompt: e.prompt };
    }
  }

  #emit(rt: Runtime, bytes: Uint8Array): void {
    if (rt.bridge) rt.bridge.write(bytes);
    else rt.pending.push(bytes);
  }

  /**
   * A rejected call on a live session ends the tab as disconnected with the message. Exception:
   * `session_active` only means the call raced the state; Rust's state is authoritative and the
   * next event updates the tab (`onActive` can undo an optimistic change).
   */
  #guard(tab: Tab, p: Promise<unknown>, onActive?: () => void): void {
    p.catch((e: unknown) => {
      if (!this.tabs.some((t) => t.id === tab.id)) return;
      const err = toApiError(e);
      if (err.code === "session_active") {
        onActive?.();
        return;
      }
      tab.state = "disconnected";
      tab.detail = err.message;
      tab.prompt = null;
    });
  }

  /** Fire and forget: the session may already be gone, which is not worth a message. */
  #fire(p: Promise<unknown>): void {
    p.catch(() => {});
  }
}
