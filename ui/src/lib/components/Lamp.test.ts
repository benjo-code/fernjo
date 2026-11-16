import { flushSync, mount, unmount } from "svelte";
import { afterEach, expect, test } from "vitest";
import Lamp from "./Lamp.svelte";

let mounted: ReturnType<typeof mount> | null = null;

afterEach(() => {
  if (mounted) unmount(mounted);
  mounted = null;
  document.body.innerHTML = "";
});

function lamp(state: "connecting" | "live" | "dropped", size: "tree" | "tab" = "tree") {
  document.body.innerHTML = "";
  mounted = mount(Lamp, { target: document.body, props: { state, size } });
  flushSync();
  return document.querySelector(".lamp")!;
}

test("renders_a_distinct_shape_per_state", () => {
  const live = lamp("live");
  expect(live.getAttribute("data-state")).toBe("live");
  expect(live.querySelector("circle.dot")).not.toBeNull();
  expect(live.querySelector(".ring, .strike")).toBeNull();

  const connecting = lamp("connecting");
  expect(connecting.getAttribute("data-state")).toBe("connecting");
  expect(connecting.querySelector("circle.ring")).not.toBeNull();
  expect(connecting.querySelector(".dot, .strike")).toBeNull();

  const dropped = lamp("dropped");
  expect(dropped.getAttribute("data-state")).toBe("dropped");
  expect(dropped.querySelector("circle.ring")).not.toBeNull();
  expect(dropped.querySelector("line.strike")).not.toBeNull();
  expect(dropped.querySelector(".dot")).toBeNull();
});

test("the_size_prop_selects_the_tab_mark", () => {
  expect(lamp("live", "tab").classList.contains("tab")).toBe(true);
  expect(lamp("live", "tree").classList.contains("tab")).toBe(false);
});

test("every_state_has_an_accessible_name", () => {
  expect(lamp("live").getAttribute("aria-label")).toBe("Connected");
  expect(lamp("connecting").getAttribute("aria-label")).toBe("Connecting");
  expect(lamp("dropped").getAttribute("aria-label")).toBe("Disconnected");
});
