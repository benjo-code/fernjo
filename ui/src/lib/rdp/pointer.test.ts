import { expect, test } from "vitest";
import { buttonOf, toDesktop } from "./pointer";

const desktop = { width: 1920, height: 1080 };

test("maps client to desktop pixels when scaled", () => {
  // the 1920x1080 desktop is shown in a 960x540 box at (100, 50)
  const box = { left: 100, top: 50, width: 960, height: 540 };
  expect(toDesktop(100, 50, box, desktop)).toEqual({ x: 0, y: 0 });
  expect(toDesktop(580, 320, box, desktop)).toEqual({ x: 960, y: 540 });
  expect(toDesktop(1059, 589, box, desktop)).toEqual({ x: 1918, y: 1078 });
  // unscaled: client offsets are pixels
  expect(toDesktop(110, 60, { left: 100, top: 50, width: 1920, height: 1080 }, desktop)).toEqual({
    x: 10,
    y: 10,
  });
});

test("clamps to the desktop", () => {
  const box = { left: 0, top: 0, width: 960, height: 540 };
  expect(toDesktop(-50, -1, box, desktop)).toEqual({ x: 0, y: 0 });
  expect(toDesktop(960, 540, box, desktop)).toEqual({ x: 1919, y: 1079 });
  expect(toDesktop(5000, 5000, box, desktop)).toEqual({ x: 1919, y: 1079 });
});

test("an empty box or a bad number is the origin", () => {
  expect(toDesktop(10, 10, { left: 0, top: 0, width: 0, height: 0 }, desktop)).toEqual({
    x: 0,
    y: 0,
  });
  const box = { left: 0, top: 0, width: 100, height: 100 };
  expect(toDesktop(Number.NaN, Number.POSITIVE_INFINITY, box, desktop)).toEqual({ x: 0, y: 0 });
});

test("buttons", () => {
  expect([0, 1, 2, 3, 4].map(buttonOf)).toEqual(["left", "middle", "right", "x1", "x2"]);
  expect(buttonOf(5)).toBeNull();
  expect(buttonOf(-1)).toBeNull();
});
