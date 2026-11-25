import { describe, expect, test } from "vitest";
import { syncErrorText } from "./sync-errors";

describe("syncErrorText", () => {
  test("known_codes_get_human_text_without_the_raw_code", () => {
    const e = syncErrorText({ code: "hub_unauthorized", message: "raw" });
    expect(e.text).toBe("The hub did not accept the secret.");
    expect(e.code).toBeNull();
  });

  test("the_vault_rollback_names_the_vault_and_the_reseed", () => {
    const e = syncErrorText({ code: "vault_rolled_back", message: "raw" });
    expect(e.text).toContain("older vault");
    expect(e.text).toContain("Re-seed");
    expect(e.code).toBeNull();
  });

  test("an_unknown_code_shows_the_message_and_the_raw_code", () => {
    expect(syncErrorText({ code: "snapshot_invalid", message: "bad snapshot" })).toEqual({
      text: "bad snapshot",
      code: "snapshot_invalid",
    });
  });
});
