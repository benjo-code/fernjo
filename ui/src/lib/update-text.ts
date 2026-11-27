// Wording of the update window, the footer marker and the Settings section. It words what
// `update_status` says and decides nothing: Rust owns the state, the sizes and the codes.
import type { UnsavedReason, UpdateStatus } from "./types";

const MB = 1024 * 1024;

/** Whole megabytes; received rounds down and the total up, so the bar never reads past its end. */
const mbDown = (bytes: number) => Math.floor(bytes / MB);
const mbUp = (bytes: number) => Math.ceil(bytes / MB);

/** The line under the progress bar; empty for states that have no stage. */
export function stageText(status: UpdateStatus): string {
  switch (status.state) {
    case "checking":
      return "Checking for updates…";
    case "downloading": {
      const received = status.received ?? 0;
      const progress =
        status.total !== null
          ? `Downloading… ${mbDown(received)} of ${mbUp(status.total)} MB`
          : `Downloading… ${mbDown(received)} MB`;
      // closing the window does not cancel the download; the install follows on its own unless
      // sessions are connected then
      return `${progress}. fernjo then restarts; with sessions connected it waits for you.`;
    }
    case "ready":
      return "Download finished, ready to install";
    case "installing":
      return "Installing… the app restarts when done";
    default:
      return "";
  }
}

/** Plain text for an error code of a failed check or install, and for a release-page reason. */
export function reasonText(code: string): string {
  switch (code) {
    case "feed_unreachable":
      return "Could not reach the update server. Check your connection and try again.";
    case "feed_invalid":
      return "The update information could not be read.";
    case "download_refused":
      return "The update download was refused because it did not look right.";
    case "signature_invalid":
      return "The update's signature did not match, so it was not installed.";
    case "install_failed":
      return "The update could not be installed.";
    case "package_install":
      return "fernjo was installed from a package, so it cannot update itself. Get the new version from the release page.";
    case "not_writable":
      return "The folder fernjo runs from is not writable, so it cannot update itself. Get the new version from the release page.";
    default:
      return "The update failed.";
  }
}

/**
 * A sentence for the causes that stop the background check altogether (the settings dialog shows
 * it next to the checkbox, which would otherwise read as "on"); `null` when the checks do run.
 * "No settings folder" only means the choice is not remembered, the checks still run.
 */
export function automaticChecksNote(reason: UnsavedReason | null): string | null {
  switch (reason) {
    case "env":
      return "Automatic checks do not run while FERNJO_DATA_DIR is set; use Check now.";
    case "settingsUnusable":
      return "Automatic checks do not run until settings.json is fixed; use Check now.";
    default:
      return null;
  }
}
