import { mount, unmount } from "svelte";
import { afterEach, expect, test, vi } from "vitest";
import Splitter from "./Splitter.svelte";

afterEach(() => {
  document.body.innerHTML = "";
});

function setup(extra: Record<string, unknown> = {}) {
  const onchange = vi.fn();
  const onreset = vi.fn();
  const app = mount(Splitter, {
    target: document.body,
    props: {
      orientation: "vertical",
      value: 300,
      min: 240,
      max: 560,
      label: "Resize sidebar",
      onchange,
      onreset,
      ...extra,
    },
  });
  const el = document.querySelector<HTMLElement>("[role='separator']")!;
  return { app, el, onchange, onreset };
}

const key = (el: HTMLElement, k: string, shiftKey = false) =>
  el.dispatchEvent(new KeyboardEvent("keydown", { key: k, shiftKey, bubbles: true }));

test("ArrowRight grows a vertical separator by 16 and Shift by 64", () => {
  const { app, el, onchange } = setup();
  key(el, "ArrowRight");
  key(el, "ArrowRight", true);
  key(el, "ArrowLeft");
  expect(onchange.mock.calls.map((c) => c[0])).toEqual([316, 364, 284]);
  unmount(app);
});

test("a horizontal separator above its panel grows it with ArrowUp", () => {
  const { app, el, onchange } = setup({
    orientation: "horizontal",
    invert: true,
    value: 200,
    min: 26,
    max: 400,
  });
  key(el, "ArrowUp");
  key(el, "ArrowDown");
  expect(onchange.mock.calls.map((c) => c[0])).toEqual([216, 184]);
  unmount(app);
});

test("Home and End jump to min and max", () => {
  const { app, el, onchange } = setup();
  key(el, "Home");
  key(el, "End");
  expect(onchange.mock.calls.map((c) => c[0])).toEqual([240, 560]);
  unmount(app);
});

test("aria values reflect the props", () => {
  const { app, el } = setup();
  expect(el.getAttribute("aria-orientation")).toBe("vertical");
  expect(el.getAttribute("aria-valuenow")).toBe("300");
  expect(el.getAttribute("aria-valuemin")).toBe("240");
  expect(el.getAttribute("aria-valuemax")).toBe("560");
  expect(el.getAttribute("aria-label")).toBe("Resize sidebar");
  expect(el.getAttribute("tabindex")).toBe("0");
  unmount(app);
});

test("double click calls onreset", () => {
  const { app, el, onreset } = setup();
  el.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
  expect(onreset).toHaveBeenCalledOnce();
  unmount(app);
});

test("pointer drag reports the new size", () => {
  const { app, el, onchange } = setup();
  const ev = (type: string, clientX: number) =>
    el.dispatchEvent(new MouseEvent(type, { clientX, button: 0, bubbles: true, cancelable: true }));
  ev("pointerdown", 100);
  ev("pointermove", 150);
  ev("pointermove", 900);
  ev("pointerup", 900);
  ev("pointermove", 120);
  expect(onchange.mock.calls.map((c) => c[0])).toEqual([350, 560]);
  unmount(app);
});
