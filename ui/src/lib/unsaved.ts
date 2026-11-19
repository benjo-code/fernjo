import type { UnsavedReason } from "./types";

/** The note under a settings section that cannot be saved, naming the real cause. */
export function unsavedNote(reason: UnsavedReason): string {
  switch (reason) {
    case "env":
      return "Applies to this run only; settings are not saved while FERNJO_DATA_DIR is set";
    case "settingsUnusable":
      return "Not saved: settings.json cannot be read or was written by a newer version of fernjo. Fix or remove that file and restart.";
    case "noConfigDir":
      return "Not saved: this device has no settings folder.";
  }
}
