import { describe, expect, test } from "vitest";
import { syncErrorText } from "./sync-errors";

describe("syncErrorText", () => {
  test("known_codes_get_human_text_without_the_raw_code", () => {
    const e = syncErrorText({ code: "hub_unauthorized", message: "raw" });
    expect(e.text).toBe("The hub did not accept the secret.");
    expect(e.code).toBeNull();
  });

  test("the_vault_rollback_names_the_vault_the_likely_cause_and_the_reseed", () => {
    const e = syncErrorText({ code: "vault_rolled_back", message: "raw" });
    expect(e.text).toBe(
      "The hub holds an older vault than this device, typically because another device re-seeded the hub from an older copy. Re-seed replaces it with this device's vault.",
    );
    expect(e.code).toBeNull();
  });

  test("a_hub_rollback_keeps_the_restored_on_purpose_wording", () => {
    const e = syncErrorText({ code: "hub_rolled_back", message: "raw" });
    expect(e.text).toContain("restored on purpose");
    expect(e.code).toBeNull();
  });

  test("a_local_vault_below_the_mark_says_how_to_recover_and_offers_no_reseed", () => {
    const e = syncErrorText({ code: "vault_below_mark", message: "raw" });
    expect(e.text).toContain("Save the vault once on this device");
    expect(e.text).not.toContain("Re-seed");
    expect(e.code).toBeNull();
  });

  test("an_exhausted_sequence_number_gets_human_text", () => {
    const e = syncErrorText({ code: "seq_exhausted", message: "raw" });
    expect(e.text).toContain("sequence number is at its maximum");
    expect(e.code).toBeNull();
  });

  test("an_unknown_code_shows_the_message_and_the_raw_code", () => {
    expect(syncErrorText({ code: "snapshot_invalid", message: "bad snapshot" })).toEqual({
      text: "bad snapshot",
      code: "snapshot_invalid",
    });
  });
});
