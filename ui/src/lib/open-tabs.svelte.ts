// View state for "reopen tabs on start". Rust owns the saved list (`open-tabs.json`, targets
// only) and the setting; this store restores the tabs once at start and afterwards reports every
// change the user makes to the tab list. A save is a convenience: a failed one is ignored.
import { toApiError } from "./api";
import type { SessionStore } from "./sessions.svelte";
import type { Api, ApiError, OpenTabsInfo, SavedTab } from "./types";

/** Quiet time after the last change before the list is saved. */
export const SAVE_DELAY_MS = 300;

export class OpenTabsStore {
  /** The "Reopen tabs on start" setting. */
  reopen = $state(true);
  /** Nothing is reported before the restore finished, or it would overwrite the saved list. */
  ready = $state(false);
  info = $state<OpenTabsInfo | null>(null);

  #api: Api;
  #sessions: SessionStore;
  #timer: ReturnType<typeof setTimeout> | null = null;
  /** The list that was last saved or restored; an equal list is not saved again. */
  #lastKey: string | null = null;
  #seenBulk = 0;
  #stop: () => void;

  constructor(api: Api, sessions: SessionStore) {
    this.#api = api;
    this.#sessions = sessions;
    this.#seenBulk = sessions.bulkClosed;
    this.#stop = $effect.root(() => {
      $effect(() => {
        // every field read here is a trigger: order, ids, targets, the front tab, the setting
        const { saved, active, key } = this.#snapshot();
        const bulk = sessions.bulkClosed;
        const live = this.ready && this.reopen;
        if (bulk !== this.#seenBulk) {
          // closeAll (reload, unload, a sync join) is not the user's doing: never saved
          this.#seenBulk = bulk;
          this.#cancel();
          return;
        }
        if (!live) {
          this.#cancel();
          return;
        }
        if (key === this.#lastKey) {
          this.#cancel();
          return;
        }
        this.#cancel();
        this.#timer = setTimeout(() => {
          this.#timer = null;
          this.#lastKey = key;
          void this.#api.saveOpenTabs(saved, active).catch(() => {});
        }, SAVE_DELAY_MS);
      });
    });
  }

  #snapshot(): { saved: SavedTab[]; active: number | null; key: string } {
    const { tabs, activeId } = this.#sessions;
    const saved: SavedTab[] = tabs.map((t) =>
      t.quick ? { quick: { ...t.quick } } : { node: t.nodeId! },
    );
    const front = tabs.findIndex((t) => t.id === activeId);
    const active = front < 0 ? null : front;
    return { saved, active, key: JSON.stringify([saved, active]) };
  }

  #cancel(): void {
    if (this.#timer !== null) clearTimeout(this.#timer);
    this.#timer = null;
  }

  /** Reads the saved list; with the setting on, makes the tabs. Saving starts after this. */
  async load(): Promise<void> {
    let info: OpenTabsInfo;
    try {
      info = await this.#api.getOpenTabs();
    } catch {
      // without an answer nothing is restored and, to keep the saved list intact, nothing saved
      return;
    }
    this.info = info;
    this.reopen = info.reopen;
    if (info.reopen && info.tabs.length > 0) this.#sessions.restore(info.tabs, info.active);
    // what the user sees now is what is on disk; an unchanged list is never written back
    this.#lastKey = this.#snapshot().key;
    this.ready = true;
  }

  /** `null` when saved, otherwise the error Rust answered with; nothing changes on an error. */
  async setReopen(enabled: boolean): Promise<ApiError | null> {
    try {
      this.info = await this.#api.setReopenTabs(enabled);
      this.reopen = this.info.reopen;
      // switching on writes the current tabs; switching off made Rust delete the file
      this.#lastKey = null;
      return null;
    } catch (e) {
      return toApiError(e);
    }
  }

  dispose(): void {
    this.#cancel();
    this.#stop();
  }
}
