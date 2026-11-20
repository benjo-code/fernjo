import { expect, test } from "vitest";
import { unsavedNote } from "./unsaved";

test("each_reason_has_its_own_note", () => {
  expect(unsavedNote("env")).toBe(
    "Applies to this run only; settings are not saved while FERNJO_DATA_DIR is set",
  );
  expect(unsavedNote("settingsUnusable")).toBe(
    "Not saved: settings.json cannot be read or was written by a newer version of fernjo. Fix or remove that file and restart.",
  );
  expect(unsavedNote("noConfigDir")).toBe("Not saved: this device has no settings folder.");
});
