// View state of automatic updates. Rust decides everything (state, versions, sizes, errors): this
// store holds the last `UpdateStatus` it was told and walks the commands. The webview never
// supplies a URL or a version; "Open release page" is a command without arguments.
import { toApiError } from "./api";
import type { Api, ApiError, UpdateStatus } from "./types";

export class UpdateStore {
  /** `null` until the first answer from Rust. */
  status = $state<UpdateStatus | null>(null);
  /** The error of the last action, for the window to show; cleared by the next action. */
  error = $state<ApiError | null>(null);

  #api: Api;
  #busy = $state(false);
  /** Bumped by every adopted status, so a slow initial read never replaces a newer event. */
  #seq = 0;
  #started = false;
  #disposed = false;
  #unlisten: (() => void) | null = null;

  #onInstallFailed: () => void;

  /** `onInstallFailed` runs when an install that had already ended the sessions failed. */
  constructor(api: Api, onInstallFailed: () => void = () => {}) {
    this.#api = api;
    this.#onInstallFailed = onInstallFailed;
  }

  /** True while a command of this store is in flight; the UI disables its buttons on it. */
  get busy(): boolean {
    return this.#busy;
  }

  /** Subscribes to `update-status` and reads the status once. */
  async init(): Promise<void> {
    if (this.#started || this.#disposed) return;
    this.#started = true;
    try {
      const off = await this.#api.onUpdateStatus((s) => this.#adopt(s));
      if (this.#disposed) off();
      else this.#unlisten = off;
    } catch {
      // without the event the status still follows the answers of the commands
    }
    const seq = this.#seq;
    try {
      const status = await this.#api.updateStatus();
      if (seq === this.#seq) this.#adopt(status);
    } catch {
      // the status stays empty; the next event or command fills it
    }
  }

  dispose(): void {
    this.#disposed = true;
    this.#unlisten?.();
    this.#unlisten = null;
  }

  /** A manual check ("Check now", "Retry"): the only place that sends `manual: true`. */
  check(): Promise<boolean> {
    return this.#act(() => this.#api.updateCheck(true));
  }

  install(): Promise<boolean> {
    return this.#act(() => this.#api.updateInstall());
  }

  skip(): Promise<boolean> {
    return this.#act(() => this.#api.updateSkip());
  }

  /**
   * Turns the background check on or off. It is a settings write, not a step of the update, so it
   * also works while a download is running (the other commands wait for that one, and an update
   * event that arrived meanwhile is newer than this answer, so only the flag is taken from it).
   */
  async setEnabled(on: boolean): Promise<boolean> {
    this.error = null;
    const seq = this.#seq;
    try {
      const status = await this.#api.setCheckForUpdates(on);
      if (seq === this.#seq || this.status === null) this.#adopt(status);
      else this.status = { ...this.status, checkEnabled: status.checkEnabled };
      return true;
    } catch (e) {
      this.error = toApiError(e);
      return false;
    }
  }

  /** Opens the release page of the version on offer (Rust builds the URL). */
  async openReleasePage(): Promise<boolean> {
    if (this.#busy) return false;
    this.error = null;
    try {
      await this.#api.updateOpenReleasePage();
      return true;
    } catch (e) {
      this.error = toApiError(e);
      return false;
    }
  }

  /** Runs one command unless another one is in flight; resolves to whether it succeeded. */
  async #act(call: () => Promise<UpdateStatus>): Promise<boolean> {
    if (this.#busy) return false;
    this.#busy = true;
    this.error = null;
    try {
      this.#adopt(await call());
      return true;
    } catch (e) {
      this.error = toApiError(e);
      return false;
    } finally {
      this.#busy = false;
    }
  }

  #adopt(status: UpdateStatus): void {
    this.#seq++;
    const installFailed = this.status?.state === "installing" && status.state === "failed";
    this.status = status;
    if (installFailed) this.#onInstallFailed();
  }
}
