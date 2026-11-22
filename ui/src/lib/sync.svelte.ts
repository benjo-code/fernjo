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
  #onJoined: () => void;
  /**
   * Bumped whenever the pairing code's panel closes (hide, dispose, leave, status off): a reveal
   * that started before the bump drops its answer, so the code never outlives its panel.
   */
  #codeGen = 0;
  #busy = $state(false);
  #applied: number | null = null;
  #unlisten: (() => void) | null = null;
  #disposed = false;

  /**
   * `onApplied` runs when a run wrote remote changes locally: fetch the tree and the vault status
   * again. `onJoined` runs after a join replaced the local data: Rust closed every session
   * without an event, so the owner of the tabs must drop them, and the tree and vault follow.
   */
  constructor(api: Api, onApplied: () => void = () => {}, onJoined: () => void = () => {}) {
    this.#api = api;
    this.#onApplied = onApplied;
    this.#onJoined = onJoined;
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
    this.hidePairingCode();
  }

  async refresh(): Promise<void> {
    try {
      this.#adopt(await this.#api.syncStatus());
    } catch {
      // the previous status stays; the next event or refresh replaces it
    }
  }

  async setup(url: string, secret: string, caPem: string | null, deviceName: string) {
    // the panel that may show the code is the one open now; closing it during the call drops the code
    const gen = this.#codeGen;
    const status = await this.#act(() => this.#api.syncSetup(url, secret, caPem, deviceName));
    if (status === null) return false;
    // the one-time reveal of the code the other devices need
    await this.#reveal(gen);
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
    this.#onJoined();
    return "joined" as const;
  }

  dropConfirmation(): void {
    this.confirmation = null;
  }

  async showPairingCode(): Promise<void> {
    await this.#reveal(this.#codeGen);
  }

  async #reveal(gen: number): Promise<void> {
    try {
      const code = await this.#api.syncPairingCode();
      if (gen === this.#codeGen) this.pairingCode = code;
    } catch (e) {
      if (gen !== this.#codeGen) return;
      this.pairingCode = null;
      this.error = toApiError(e);
    }
  }

  hidePairingCode(): void {
    this.#codeGen++;
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
    // a reveal still in flight must not bring the code back after leaving
    this.#codeGen++;
    this.pairingCode = null;
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
      this.hidePairingCode();
      this.confirmation = null;
    }
    // The first status counts too when runs already applied something: the tree was listed before
    // this store was told anything, so a startup run may have changed the data since.
    const changed = previous === null ? status.applied > 0 : previous !== status.applied;
    if (changed) this.#onApplied();
  }
}
