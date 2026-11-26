import { describe, expect, test } from "vitest";
import { reasonText, stageText } from "./update-text";
import { updateStatus } from "./testing";

describe("update texts", () => {
  test("stage_texts", () => {
    expect(stageText(updateStatus({ state: "checking" }))).toBe("Checking for updates…");
    expect(
      stageText(
        updateStatus({ state: "downloading", received: 12 * 1048576, total: 48 * 1048576 }),
      ),
    ).toBe("Downloading… 12 of 48 MB");
    expect(stageText(updateStatus({ state: "downloading", received: 5 * 1048576 }))).toBe(
      "Downloading… 5 MB",
    );
    expect(stageText(updateStatus({ state: "installing" }))).toBe(
      "Installing… the app restarts when done",
    );
    expect(stageText(updateStatus({ state: "ready" }))).toBe("Download finished, ready to install");
    expect(stageText(updateStatus({ state: "idle" }))).toBe("");
  });

  test("every_known_code_has_its_own_text", () => {
    const codes = [
      "feed_unreachable",
      "feed_invalid",
      "download_refused",
      "signature_invalid",
      "install_failed",
      "package_install",
      "not_writable",
    ];
    const texts = codes.map(reasonText);
    expect(new Set(texts).size).toBe(codes.length);
    expect(reasonText("something_new")).toBe("The update failed.");
  });
});
