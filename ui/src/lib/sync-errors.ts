// Human text for the codes a sync command can fail with. Rust owns the codes and the validation;
// this only words them. A code without text shows Rust's message with the raw code beside it.
import type { ApiError } from "./types";

const TEXT: Record<string, string> = {
  hub_unreachable: "The hub could not be reached. Check the address and your network.",
  hub_unauthorized: "The hub did not accept the secret.",
  hub_tls_untrusted:
    "The hub's certificate is not trusted. If it comes from your own CA, paste that CA certificate.",
  invalid_pairing_code: "That is not a valid pairing code.",
  newer_pairing_code: "That pairing code is from a newer version of fernjo. Update this app first.",
  hub_not_empty: "The hub already holds a sync group. Join it with a pairing code instead.",
  hub_empty: "The hub holds no sync group yet. Set up a device first.",
  no_device_folder: "This device has no device-local folder, so sync cannot be set up here.",
  keychain_unavailable: "The OS keychain is not available, so the sync keys cannot be stored.",
  invalid_device_name: "Enter a device name (up to 100 characters).",
  sync_busy: "A sync run is in progress. Try again in a moment.",
  not_paired: "This device is not part of a sync group.",
  already_paired: "This device is already part of a sync group.",
  changed_on_disk:
    "The data changed on disk while syncing. Reload and try again, or lock and unlock the vault.",
  hub_rolled_back: "The hub shows an older state than this device has seen. Re-seed it from here.",
  rolled_back: "The hub shows an older state than this device has seen.",
  seq_exhausted:
    "The hub's sequence number is at its maximum, so nothing can be sent to it, not even by a re-seed. The hub's admin has to remove its head.",
  vault_rolled_back:
    "The hub holds an older vault than this device has seen. Re-seed it from here if that is expected.",
};

/** The text to show for an error, with the raw code appended when there is no wording for it. */
export function syncErrorText(error: ApiError): { text: string; code: string | null } {
  const text = TEXT[error.code];
  if (text !== undefined) return { text, code: null };
  return { text: error.message, code: error.code };
}
