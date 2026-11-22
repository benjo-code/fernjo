// The one-line sync status of the sidebar footer. It words what `sync_status` says and decides
// nothing: Rust owns the state, the counts and the time.
import type { SyncStatus } from "./types";
import { syncErrorText } from "./sync-errors";

export type SyncLine = {
  text: string;
  /** Longer explanation for the tooltip. */
  title: string;
  /** `error` and `rolled_back` get the attention colour; offline never does. */
  tone: "calm" | "attention";
  /** The state has an action next to the line. */
  action: "none" | "reseed" | "details";
};

const HIDDEN: ReadonlySet<string> = new Set(["off", "needs_confirmation"]);

/** Local time of an RFC 3339 instant, in the locale's own form; empty when unparsable. */
export function formatTime(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/** `null` while sync is off: the footer shows nothing. */
export function syncLine(status: SyncStatus | null): SyncLine | null {
  if (status === null || HIDDEN.has(status.state)) return null;
  if (status.running) {
    return { text: "Syncing…", title: "Sync is running", tone: "calm", action: "none" };
  }
  switch (status.state) {
    case "ok": {
      const time = status.lastSyncedAt ? formatTime(status.lastSyncedAt) : "";
      return {
        text: time ? `Synced ${time}` : "Synced",
        title: status.hubUrl ? `Synced with ${status.hubUrl}` : "Synced",
        tone: "calm",
        action: "none",
      };
    }
    case "offline": {
      const n = status.pendingChanges;
      return {
        text: n > 0 ? `Offline, ${n} ${n === 1 ? "change" : "changes"} waiting` : "Offline",
        title: "The hub cannot be reached. Changes are kept and sent when it is back.",
        tone: "calm",
        action: "none",
      };
    }
    case "waiting_for_unlock":
      return {
        text: "Sync paused: unlock the vault",
        title: "Both sides changed the vault. Unlock it so the changes can be merged.",
        tone: "calm",
        action: "none",
      };
    case "rolled_back":
      return {
        text: "Sync: hub older than this device",
        title: status.error?.message ?? "The hub shows an older state than this device has seen.",
        tone: "attention",
        action: "reseed",
      };
    default:
      return {
        text: "Sync problem",
        title: status.error ? syncErrorText(status.error).text : "The last sync run failed.",
        tone: "attention",
        action: "details",
      };
  }
}

/** The confirmation of a re-seed, shared by the Settings section and the footer's "Re-seed…". */
export const RESEED_CONFIRM = {
  title: "Re-seed the hub from this device?",
  detail:
    "The hub's data is replaced with what this device has now. Other devices then follow this device's state.",
  confirmLabel: "Re-seed",
} as const;
