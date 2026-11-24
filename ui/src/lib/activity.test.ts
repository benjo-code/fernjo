import { afterEach, describe, expect, test, vi } from "vitest";
import { startActivityHeartbeat } from "./activity";

let stop: (() => void) | null = null;
afterEach(() => {
  stop?.();
  stop = null;
  document.body.innerHTML = "";
});

describe("startActivityHeartbeat", () => {
  test("a_keydown_on_a_view_that_stops_propagation_is_still_counted", () => {
    const send = vi.fn();
    stop = startActivityHeartbeat(send, window, () => 0);
    const view = document.createElement("div");
    view.addEventListener("keydown", (e) => e.stopPropagation());
    document.body.append(view);
    view.dispatchEvent(new KeyboardEvent("keydown", { key: "a", bubbles: true }));
    expect(send).toHaveBeenCalledTimes(1);
  });

  test("activity_is_throttled_to_one_call_per_gap", () => {
    const send = vi.fn();
    let t = 1000;
    stop = startActivityHeartbeat(send, window, () => t, 5000);
    const move = () => window.dispatchEvent(new Event("pointermove"));
    move();
    t += 4999;
    move();
    window.dispatchEvent(new Event("wheel"));
    expect(send).toHaveBeenCalledTimes(1);
    t += 1;
    window.dispatchEvent(new Event("pointerdown"));
    expect(send).toHaveBeenCalledTimes(2);
  });

  test("stop_removes_the_listeners", () => {
    const send = vi.fn();
    const off = startActivityHeartbeat(send, window, () => 0);
    off();
    window.dispatchEvent(new Event("keydown"));
    window.dispatchEvent(new Event("pointermove"));
    expect(send).not.toHaveBeenCalled();
  });
});
