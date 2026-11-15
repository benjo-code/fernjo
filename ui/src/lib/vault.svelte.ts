// View state for the vault: what Rust last reported about it and the credential list (without
// secrets). Every call goes through one queue so commands never overlap. Errors are returned to
// the caller, which shows them inline; only `restart_required` also goes to the global error bar.
import { toApiError } from "./api";
import type { Api, ApiError, CredentialInfo, VaultStatus } from "./types";

export class VaultStore {
  /** `null` until the first answer from Rust. */
  status = $state<VaultStatus | null>(null);
  /** Empty unless the vault is unlocked. */
  credentials = $state<CredentialInfo[]>([]);
  /** Set when the status or the list could not be loaded. */
  error = $state<string | null>(null);

  #api: Api;
  #onGlobalError: (message: string) => void;
  #queue: Promise<unknown> = Promise.resolve();

  constructor(api: Api, onGlobalError: (message: string) => void = () => {}) {
    this.#api = api;
    this.#onGlobalError = onGlobalError;
  }

  async init(): Promise<void> {
    const err = await this.#run(async () => {
      this.status = await this.#api.vaultStatus();
      await this.#loadCredentials();
    });
    this.error = err ? err.message : null;
  }

  create(password: string, remember: boolean) {
    return this.#run(async () => {
      this.status = await this.#api.vaultCreate(password, remember);
      await this.#loadCredentials();
    });
  }

  unlock(password: string, remember: boolean) {
    return this.#run(async () => {
      this.status = await this.#api.vaultUnlock(password, remember);
      await this.#loadCredentials();
    });
  }

  lock() {
    return this.#run(async () => {
      this.status = await this.#api.vaultLock();
      await this.#loadCredentials();
    });
  }

  changePassword(old: string, next: string) {
    return this.#run(async () => {
      this.status = await this.#api.vaultChangePassword(old, next);
    });
  }

  setAutoUnlock(on: boolean) {
    return this.#run(async () => {
      this.status = await this.#api.vaultSetAutoUnlock(on);
    });
  }

  refreshCredentials() {
    return this.#run(() => this.#loadCredentials());
  }

  /** The list exists only while unlocked; any other state empties it. */
  async #loadCredentials(): Promise<void> {
    if (this.status?.state === "unlocked") {
      this.credentials = await this.#api.listCredentials();
    } else {
      this.credentials = [];
    }
  }

  /** Runs `job` after every earlier job finished. Resolves to the error, or `null`. */
  #run(job: () => Promise<void>): Promise<ApiError | null> {
    const result = this.#queue.then(async () => {
      try {
        await job();
        return null;
      } catch (e) {
        const err = toApiError(e);
        if (err.code === "restart_required") this.#onGlobalError(err.message);
        return err;
      }
    });
    this.#queue = result;
    return result;
  }
}
