// View state of multi-device sync. Rust decides everything (state, counts, findings, errors):
// this store holds the last `SyncStatus` it was told and walks the steps of the commands. The hub
// secret is handed to the api and not stored; the pairing code is a view-state copy that lives
// only while its panel is open. Nothing here goes to `localStorage`.
import { toApiError } from "./api";
import type { Api, ApiError, SyncConfirmation, SyncStatus } from "./types";

export class SyncStore {
  /** `null` until the first answer from Rust. */
  status = $state<SyncStatus | null>(null);
  /** The pairing code while its reveal panel is open, else `null`. */
  pairingCode = $state<string | null>(null);
  /** What a join without `confirmReplace` would replace; set until the join is confirmed or dropped. */
  confirmation = $state<SyncConfirmation | null>(null);
  /** The error of the last action, for the form to show; cleared by the next action. */
  error = $state<ApiError | null>(null);

  #api: Api;
  #onApplied: () => void;
  #busy = $state(false);
  #applied: number | null = null;
  #unlisten: (() => void) | null = null;
  #disposed = false;

  /** `onApplied` runs when a run wrote remote changes locally: reload the tree and the vault status. */
  constructor(api: Api, onApplied: () => void = () => {}) {
    this.#api = api;
    this.#onApplied = onApplied;
  }

  /** True while a command of this store is in flight; the UI disables its buttons on it. */
  get busy(): boolean {
    return this.#busy;
  }

  /** Loads the status and starts listening to the `sync-status` event. */
  async init(): Promise<void> {
    if (this.#unlisten === null && !this.#disposed) {
      try {
        const off = await this.#api.onSyncStatus((s) => this.#adopt(s));
        if (this.#disposed) off();
        else this.#unlisten = off;
      } catch {
        // without the event the status still refreshes when the UI asks
      }
    }
    await this.refresh();
  }

  dispose(): void {
    this.#disposed = true;
    this.#unlisten?.();
    this.#unlisten = null;
    this.pairingCode = null;
  }

  async refresh(): Promise<void> {
    try {
      this.#adopt(await this.#api.syncStatus());
    } catch {
      // the previous status stays; the next event or refresh replaces it
    }
  }

  async setup(url: string, secret: string, caPem: string | null, deviceName: string) {
    const status = await this.#act(() => this.#api.syncSetup(url, secret, caPem, deviceName));
    if (status === null) return false;
    // the one-time reveal of the code the other devices need
    await this.showPairingCode();
    return true;
  }

  /**
   * First call (`confirmReplace` false): Rust answers `needs_confirmation` with the counts and
   * changes nothing; they land in `confirmation`. The second call (`true`) joins. Returns
   * `"confirm"`, `"joined"` or `null` on an error (`error` says which).
   */
  async join(code: string, deviceName: string, confirmReplace: boolean) {
    const status = await this.#act(() => this.#api.syncJoin(code, deviceName, confirmReplace));
    if (status === null) return null;
    if (status.state === "needs_confirmation") {
      this.confirmation = status.confirmation;
      return "confirm" as const;
    }
    this.confirmation = null;
    return "joined" as const;
  }

  dropConfirmation(): void {
    this.confirmation = null;
  }

  async showPairingCode(): Promise<void> {
    try {
      this.pairingCode = await this.#api.syncPairingCode();
    } catch (e) {
      this.pairingCode = null;
      this.error = toApiError(e);
    }
  }

  hidePairingCode(): void {
    this.pairingCode = null;
  }

  async syncNow() {
    return (await this.#act(() => this.#api.syncNow())) !== null;
  }

  async reseed() {
    return (await this.#act(() => this.#api.syncReseed())) !== null;
  }

  async editSecret(secret: string) {
    return (await this.#act(() => this.#api.syncEditSecret(secret))) !== null;
  }

  async leave() {
    return (await this.#act(() => this.#api.syncLeave())) !== null;
  }

  /** Clears the merge notices in Rust and here. */
  async dismissFindings(): Promise<void> {
    try {
      await this.#api.syncDismissFindings();
    } catch (e) {
      this.error = toApiError(e);
      return;
    }
    if (this.status) this.status = { ...this.status, findings: [] };
  }

  /** Runs one command unless another one is in flight; resolves to its status or `null`. */
  async #act(call: () => Promise<SyncStatus>): Promise<SyncStatus | null> {
    if (this.#busy) return null;
    this.#busy = true;
    this.error = null;
    try {
      const status = await call();
      // a join that waits for its confirmation is an answer, not a state to show
      if (status.state !== "needs_confirmation") this.#adopt(status);
      return status;
    } catch (e) {
      this.error = toApiError(e);
      return null;
    } finally {
      this.#busy = false;
    }
  }

  #adopt(status: SyncStatus): void {
    const previous = this.#applied;
    this.#applied = status.applied;
    this.status = status;
    if (status.state === "off") {
      this.pairingCode = null;
      this.confirmation = null;
    }
    if (previous !== null && previous !== status.applied) this.#onApplied();
  }
}
