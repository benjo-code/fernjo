import { describe, expect, test } from "vitest";
import { SessionStore } from "./sessions.svelte";
import { apiError, deferred, mockApi } from "./testing";
import type { SessionEvent } from "./types";

type Emit = (e: SessionEvent | ArrayBuffer) => void;

const dec = new TextDecoder();
const enc = new TextEncoder();
const buf = (s: string) => enc.encode(s).buffer as ArrayBuffer;
const tick = () => new Promise<void>((r) => setTimeout(r, 0));

function setup(impl: Parameters<typeof mockApi>[1] = {}) {
  const m = mockApi([], impl);
  const store = new SessionStore(m.api, (id) => `host-${id}`);
  const emits: Emit[] = [];
  const origOpen = m.api.openSession;
  m.api.openSession = (nodeId, onEvent, cols, rows) => {
    emits.push(onEvent);
    return origOpen(nodeId, onEvent, cols, rows);
  };
  return { ...m, store, emits };
}

function bridge(cols = 80, rows = 24) {
  const out: string[] = [];
  const b = {
    out,
    text: () => out.join(""),
    size: { cols, rows },
    api: () => ({
      write: (bytes: Uint8Array) => {
        out.push(dec.decode(bytes));
      },
      size: () => b.size,
    }),
  };
  return b;
}

async function openAttached(s: ReturnType<typeof setup>, nodeId = "a") {
  s.store.open(nodeId, { another: true });
  const tab = s.store.tabs[s.store.tabs.length - 1]!;
  const b = bridge();
  s.store.attachTerminal(tab.id, b.api());
  await tick();
  return { tab, b, emit: s.emits[s.emits.length - 1]! };
}

describe("SessionStore", () => {
  test("open_creates_a_tab_and_starts_the_session_after_attach", async () => {
    const s = setup({ openSession: async () => "S1" });
    s.store.open("a");
    expect(s.store.tabs).toHaveLength(1);
    expect(s.store.tabs[0]).toMatchObject({
      nodeId: "a",
      title: "host-a",
      state: "connecting",
      sessionId: null,
      prompt: null,
    });
    expect(s.store.activeId).toBe(s.store.tabs[0]!.id);
    expect(s.calls.openSession).toHaveLength(0);

    s.store.attachTerminal(s.store.tabs[0]!.id, bridge(100, 30).api());
    await tick();
    expect(s.calls.openSession).toHaveLength(1);
    expect(s.calls.openSession[0]!.slice(0, 1)).toEqual(["a"]);
    expect(s.calls.openSession[0]!.slice(2)).toEqual([100, 30]);
    expect(s.store.tabs[0]!.sessionId).toBe("S1");
  });

  test("open_again_focuses_the_existing_tab", () => {
    const s = setup();
    s.store.open("a");
    s.store.open("b");
    const first = s.store.tabs[0]!.id;
    expect(s.store.activeId).toBe(s.store.tabs[1]!.id);
    s.store.open("a");
    expect(s.store.tabs).toHaveLength(2);
    expect(s.store.activeId).toBe(first);
  });

  test("another_opens_a_second_tab_titled_name_2", () => {
    const s = setup();
    s.store.open("a");
    s.store.open("a", { another: true });
    expect(s.store.tabs.map((t) => t.title)).toEqual(["host-a", "host-a (2)"]);
    expect(s.store.activeId).toBe(s.store.tabs[1]!.id);
    s.store.open("a", { another: true });
    expect(s.store.tabs[2]!.title).toBe("host-a (3)");
  });

  test("state_events_update_the_tab_and_detail", async () => {
    const s = setup();
    const { tab, emit } = await openAttached(s);
    emit({ kind: "state", state: "connected", detail: null });
    expect(s.store.tabs[0]).toMatchObject({ id: tab.id, state: "connected", detail: null });
    emit({ kind: "state", state: "disconnected", detail: "connection reset" });
    expect(s.store.tabs[0]).toMatchObject({ state: "disconnected", detail: "connection reset" });
  });

  test("prompt_event_sets_the_prompt_and_answer_clears_it_and_calls_the_api", async () => {
    const s = setup({ openSession: async () => "S1" });
    const { tab, emit } = await openAttached(s);
    emit({ kind: "prompt", id: 7, prompt: { kind: "password", user: "u", host: "h" } });
    expect(s.store.tabs[0]!.prompt).toEqual({
      id: 7,
      prompt: { kind: "password", user: "u", host: "h" },
    });
    s.store.answer(tab.id, { kind: "secrets", values: ["hunter2"] });
    expect(s.store.tabs[0]!.prompt).toBeNull();
    expect(s.calls.answerPrompt).toEqual([["S1", 7, { kind: "secrets", values: ["hunter2"] }]]);
    // nothing of the answer stays in the store
    expect(JSON.stringify(s.store.tabs)).not.toContain("hunter2");
  });

  test("output_before_attach_is_flushed_on_attach", async () => {
    const s = setup();
    s.store.open("a");
    const tab = s.store.tabs[0]!;
    s.store.attachTerminal(tab.id, bridge().api());
    await tick();
    // a second attach must not start a second session
    s.store.attachTerminal(tab.id, bridge().api());
    await tick();
    expect(s.calls.openSession).toHaveLength(1);
    // output that arrives while no terminal is attached waits for the next one
    s.store.detachTerminal(tab.id);
    s.emits[0]!(buf("hello "));
    s.emits[0]!(buf("world"));
    const b2 = bridge();
    s.store.attachTerminal(tab.id, b2.api());
    expect(b2.text()).toBe("hello world");
  });

  test("close_calls_closeSession_and_removes_the_tab_and_activates_a_neighbour", async () => {
    const s = setup({ openSession: async (n) => `S-${n}` });
    await openAttached(s, "a");
    await openAttached(s, "b");
    await openAttached(s, "c");
    const [a, b, c] = s.store.tabs;
    s.store.activate(b!.id);
    s.store.close(b!.id);
    expect(s.calls.closeSession).toEqual([["S-b"]]);
    expect(s.store.tabs.map((t) => t.id)).toEqual([a!.id, c!.id]);
    expect(s.store.activeId).toBe(c!.id);
    s.store.close(c!.id);
    expect(s.store.activeId).toBe(a!.id);
    s.store.close(a!.id);
    expect(s.store.activeId).toBeNull();
  });

  test("close_before_the_session_id_is_known_still_ends_the_session", async () => {
    const d = deferred<string>();
    const s = setup({ openSession: () => d.promise });
    s.store.open("a");
    const tab = s.store.tabs[0]!;
    s.store.attachTerminal(tab.id, bridge().api());
    s.store.close(tab.id);
    expect(s.store.tabs).toHaveLength(0);
    expect(s.calls.closeSession).toHaveLength(0);
    d.resolve("LATE");
    await tick();
    expect(s.calls.closeSession).toEqual([["LATE"]]);
  });

  test("reconnect_calls_reconnectSession_with_the_current_size", async () => {
    const s = setup({ openSession: async () => "S1" });
    const { tab, emit, b } = await openAttached(s);
    emit({ kind: "state", state: "disconnected", detail: "gone" });
    b.size = { cols: 120, rows: 40 };
    s.store.reconnect(tab.id);
    expect(s.calls.reconnectSession).toEqual([["S1", 120, 40]]);
    expect(s.store.tabs[0]).toMatchObject({ state: "connecting", detail: null });
  });

  test("reconnect_writes_a_dim_separator_line_into_the_terminal_and_keeps_earlier_output", async () => {
    const s = setup({ openSession: async () => "S1" });
    const { tab, emit, b } = await openAttached(s);
    emit(buf("before"));
    emit({ kind: "state", state: "disconnected", detail: null });
    s.store.reconnect(tab.id);
    await tick();
    expect(b.out).toEqual(["before", "\r\n\x1b[2m— reconnected —\x1b[0m\r\n"]);
  });

  test("resize_ignores_zero_sizes", async () => {
    const s = setup({ openSession: async () => "S1" });
    const { tab } = await openAttached(s);
    s.store.resize(tab.id, 0, 24);
    s.store.resize(tab.id, 80, 0);
    expect(s.calls.sessionResize).toHaveLength(0);
    s.store.resize(tab.id, 90, 30);
    expect(s.calls.sessionResize).toEqual([["S1", 90, 30]]);
  });

  test("ack_and_input_forward_to_the_api", async () => {
    const s = setup({ openSession: async () => "S1" });
    const { tab, emit } = await openAttached(s);
    emit({ kind: "state", state: "connected", detail: null });
    const bytes = new Uint8Array([1, 2]);
    s.store.input(tab.id, bytes);
    s.store.ack(tab.id, 512);
    expect(s.calls.sessionInput).toEqual([["S1", bytes]]);
    expect(s.calls.sessionAck).toEqual([["S1", 512]]);
  });

  test("input_is_dropped_unless_the_tab_is_connected", async () => {
    const s = setup({ openSession: async () => "S1" });
    const { tab, emit } = await openAttached(s);
    const bytes = new Uint8Array([7]);
    // connecting (also while a prompt is pending), needing the vault, dropped: nothing is sent
    s.store.input(tab.id, bytes);
    emit({
      kind: "prompt",
      id: 1,
      prompt: { kind: "password", user: "u", host: "h" },
    });
    s.store.input(tab.id, bytes);
    emit({ kind: "state", state: "needsVault", detail: null });
    s.store.input(tab.id, bytes);
    emit({ kind: "state", state: "disconnected", detail: null });
    s.store.input(tab.id, bytes);
    expect(s.calls.sessionInput).toHaveLength(0);
    emit({ kind: "state", state: "connected", detail: null });
    s.store.input(tab.id, bytes);
    expect(s.calls.sessionInput).toEqual([["S1", bytes]]);
  });

  test("lamp_priority_live_over_connecting_over_dropped", async () => {
    const s = setup({ openSession: async (n) => `S-${n}` });
    const one = await openAttached(s, "a");
    const two = await openAttached(s, "a");
    expect(s.store.lampFor("a")).toBe("connecting");
    one.emit({ kind: "state", state: "disconnected", detail: null });
    expect(s.store.lampFor("a")).toBe("connecting");
    two.emit({ kind: "state", state: "needsVault", detail: null });
    expect(s.store.lampFor("a")).toBe("connecting");
    two.emit({ kind: "state", state: "disconnected", detail: null });
    expect(s.store.lampFor("a")).toBe("dropped");
    two.emit({ kind: "state", state: "connected", detail: null });
    expect(s.store.lampFor("a")).toBe("live");
  });

  test("lamp_is_null_without_tabs", () => {
    const s = setup();
    expect(s.store.lampFor("a")).toBeNull();
  });

  test("open_error_shows_a_message_and_leaves_no_tab", async () => {
    const s = setup({
      openSession: async () => {
        throw apiError("not_found", "there is no such connection");
      },
    });
    await openAttached(s);
    expect(s.store.tabs).toHaveLength(0);
    expect(s.store.activeId).toBeNull();
    expect(s.store.error).toBe("there is no such connection");
  });

  test("close_all_closes_every_session", async () => {
    const s = setup({ openSession: async (n) => `S-${n}` });
    await openAttached(s, "a");
    await openAttached(s, "b");
    s.store.closeAll();
    expect(s.calls.closeSession).toEqual([["S-a"], ["S-b"]]);
    expect(s.store.tabs).toHaveLength(0);
  });

  test("rejected_reconnect_ends_the_tab_as_disconnected_with_the_message", async () => {
    const s = setup({
      openSession: async () => "S1",
      reconnectSession: async () => {
        throw apiError("connect_failed", "no route to host");
      },
    });
    const { tab, emit, b } = await openAttached(s);
    emit({ kind: "state", state: "disconnected", detail: "gone" });
    s.store.reconnect(tab.id);
    await tick();
    expect(s.store.tabs[0]).toMatchObject({ state: "disconnected", detail: "no route to host" });
    expect(b.text()).not.toContain("reconnected");
  });

  test("rejected_answer_ends_the_tab_as_disconnected_with_the_message", async () => {
    const s = setup({
      openSession: async () => "S1",
      answerPrompt: async () => {
        throw apiError("not_found", "the prompt is gone");
      },
    });
    const { tab, emit } = await openAttached(s);
    emit({ kind: "prompt", id: 1, prompt: { kind: "username", host: "h" } });
    s.store.answer(tab.id, { kind: "text", value: "root" });
    await tick();
    expect(s.store.tabs[0]).toMatchObject({
      state: "disconnected",
      detail: "the prompt is gone",
      prompt: null,
    });
  });

  test("rejected_resume_ends_the_tab_as_disconnected_with_the_message", async () => {
    const s = setup({
      openSession: async () => "S1",
      resumeSession: async () => {
        throw apiError("vault_locked", "the vault is locked");
      },
    });
    const { tab, emit } = await openAttached(s);
    emit({ kind: "state", state: "needsVault", detail: null });
    s.store.resume(tab.id, false);
    await tick();
    expect(s.store.tabs[0]).toMatchObject({ state: "disconnected", detail: "the vault is locked" });
  });

  test("session_active_rejection_leaves_the_tab_state_unchanged", async () => {
    const active = async () => {
      throw apiError("session_active", "the session is active");
    };
    const s = setup({
      openSession: async () => "S1",
      reconnectSession: active,
      resumeSession: active,
      answerPrompt: active,
    });
    const { tab, emit, b } = await openAttached(s);
    emit({ kind: "state", state: "needsVault", detail: null });
    s.store.resume(tab.id, true);
    await tick();
    expect(s.store.tabs[0]).toMatchObject({ state: "needsVault", detail: null });

    emit({ kind: "prompt", id: 2, prompt: { kind: "keyPassphrase" } });
    s.store.answer(tab.id, { kind: "cancel" });
    await tick();
    expect(s.store.tabs[0]!.state).toBe("needsVault");

    emit({ kind: "state", state: "disconnected", detail: "gone" });
    s.store.reconnect(tab.id);
    await tick();
    // the optimistic "connecting" is undone and no separator line is written
    expect(s.store.tabs[0]).toMatchObject({ state: "disconnected", detail: "gone" });
    expect(b.out).toEqual([]);
  });

  test("reconnect_on_a_connected_or_connecting_tab_is_a_no_op", async () => {
    const s = setup({ openSession: async () => "S1" });
    const { tab, emit, b } = await openAttached(s);
    s.store.reconnect(tab.id);
    emit({ kind: "state", state: "connected", detail: null });
    s.store.reconnect(tab.id);
    await tick();
    expect(s.calls.reconnectSession).toHaveLength(0);
    expect(b.out).toEqual([]);
    expect(s.store.tabs[0]!.state).toBe("connected");
  });

  test("resume_on_a_tab_that_is_not_waiting_for_the_vault_is_a_no_op", async () => {
    const s = setup({ openSession: async () => "S1" });
    const { tab, emit } = await openAttached(s);
    s.store.resume(tab.id, false);
    emit({ kind: "state", state: "connected", detail: null });
    s.store.resume(tab.id, true);
    expect(s.calls.resumeSession).toHaveLength(0);
    emit({ kind: "state", state: "needsVault", detail: null });
    s.store.resume(tab.id, true);
    expect(s.calls.resumeSession).toEqual([["S1", true]]);
  });

  test("answer_without_a_prompt_does_nothing_and_does_not_call_the_api", async () => {
    const s = setup({ openSession: async () => "S1" });
    const { tab, emit } = await openAttached(s);
    s.store.answer(tab.id, { kind: "cancel" });
    expect(s.calls.answerPrompt).toHaveLength(0);
    emit({ kind: "prompt", id: 3, prompt: { kind: "keyPassphrase" } });
    s.store.answer(tab.id, { kind: "cancel" });
    s.store.answer(tab.id, { kind: "cancel" }); // stale: the prompt is already answered
    expect(s.calls.answerPrompt).toEqual([["S1", 3, { kind: "cancel" }]]);
  });

  test("reorder_moves_a_tab_before_another", () => {
    const s = setup();
    s.store.open("a");
    s.store.open("b");
    s.store.open("c");
    const [a, b, c] = s.store.tabs.map((t) => t.id);
    s.store.reorder(c!, a!);
    expect(s.store.tabs.map((t) => t.id)).toEqual([c, a, b]);
    s.store.reorder(c!, null);
    expect(s.store.tabs.map((t) => t.id)).toEqual([a, b, c]);
    s.store.reorder("nope", a!);
    expect(s.store.tabs.map((t) => t.id)).toEqual([a, b, c]);
  });

  test("next_and_previous_cycle_and_wrap", () => {
    const s = setup();
    s.store.next();
    s.store.previous();
    expect(s.store.activeId).toBeNull();
    s.store.open("a");
    s.store.open("b");
    s.store.open("c");
    const [a, b, c] = s.store.tabs.map((t) => t.id);
    expect(s.store.activeId).toBe(c);
    s.store.next();
    expect(s.store.activeId).toBe(a);
    s.store.next();
    expect(s.store.activeId).toBe(b);
    s.store.previous();
    s.store.previous();
    expect(s.store.activeId).toBe(c);
  });
});
