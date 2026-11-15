// View state for the vault: what Rust last reported about it and the credential list (without
// secrets). Every call goes through one queue so commands never overlap. Errors are returned to
// the caller, which shows them inline; only `restart_required` also goes to the global error bar.
import { toApiError } from "./api";
import type {
  Api,
  ApiError,
  CredentialDraft,
  CredentialInfo,
  CredentialUpdate,
  Mutation,
  RevealedSecret,
  UsageItem,
  VaultStatus,
} from "./types";

/** The result of a call whose answer the caller needs: the value, or the error to show. */
export type Outcome<T> = { value: T; error: null } | { value: null; error: ApiError };

export class VaultStore {
  /** `null` until the first answer from Rust. */
  status = $state<VaultStatus | null>(null);
  /** Empty unless the vault is unlocked and the list is loaded. */
  credentials = $state<CredentialInfo[]>([]);
  /**
   * Whether `credentials` can be trusted: `loading` until the list of an unlocked vault
   * arrived (also while locked), `failed` when listing it failed. An empty list means
   * "no credentials" only in `loaded`.
   */
  credentialsState = $state<"loading" | "loaded" | "failed">("loading");
  /** Set when the first status could not be loaded. */
  error = $state<string | null>(null);
  /** True when the first status could not be loaded: there is nothing to unlock or create. */
  unavailable = $state(false);

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
    this.unavailable = err !== null && this.status === null;
    // restart_required was already reported by #run
    if (this.unavailable && err && err.code !== "restart_required") {
      this.#onGlobalError(`Vault unavailable: ${err.message}`);
    }
  }

  create(password: string, remember: boolean) {
    return this.#run(async () => {
      this.status = await this.#api.vaultCreate(password, remember);
      await this.#loadCredentials();
    });
  }

  /**
   * Unlocks. Rust keeps the key-store item when `remember` is false, so a user who unticked
   * "Unlock automatically" while it was on gets it turned off here after the unlock worked;
   * a failure of that second call is shown, the unlock stands.
   */
  unlock(password: string, remember: boolean) {
    return this.#run(async () => {
      const hadAutoUnlock = this.status?.autoUnlock === true;
      this.status = await this.#api.vaultUnlock(password, remember);
      await this.#loadCredentials();
      if (!remember && hadAutoUnlock && this.status.autoUnlock) {
        try {
          this.status = await this.#api.vaultSetAutoUnlock(false);
        } catch (e) {
          this.#onGlobalError(toApiError(e).message);
        }
      }
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

  createCredential(draft: CredentialDraft) {
    return this.#call(async () => {
      const info = await this.#api.createCredential(draft);
      await this.#loadCredentials();
      return info;
    });
  }

  updateCredential(id: string, update: CredentialUpdate) {
    return this.#call(async () => {
      const info = await this.#api.updateCredential(id, update);
      await this.#loadCredentials();
      return info;
    });
  }

  credentialUsage(id: string): Promise<Outcome<UsageItem[]>> {
    return this.#call(() => this.#api.credentialUsage(id));
  }

  /** Returns the tree snapshot: nodes that used the credential were reset to inherit. */
  deleteCredential(id: string): Promise<Outcome<Mutation>> {
    return this.#call(async () => {
      const m = await this.#api.deleteCredential(id);
      await this.#loadCredentials();
      return m;
    });
  }

  /** The only call that brings secret values into the UI; the caller hides them again. */
  revealCredential(id: string): Promise<Outcome<RevealedSecret>> {
    return this.#call(() => this.#api.revealCredential(id));
  }

  refreshCredentials() {
    return this.#run(() => this.#loadCredentials());
  }

  /**
   * The list exists only while unlocked; any other state empties it. A refresh of an
   * already loaded list keeps showing the old one until the new one arrives.
   */
  async #loadCredentials(): Promise<void> {
    if (this.status?.state !== "unlocked") {
      this.credentials = [];
      this.credentialsState = "loading";
      return;
    }
    if (this.credentialsState === "failed") this.credentialsState = "loading";
    try {
      this.credentials = await this.#api.listCredentials();
      this.credentialsState = "loaded";
    } catch (e) {
      // the status change itself worked: keep it and tell the user about the list
      this.credentials = [];
      this.credentialsState = "failed";
      const err = toApiError(e);
      this.#onGlobalError(`Could not load the credentials: ${err.message}`);
    }
  }

  async #call<T>(job: () => Promise<T>): Promise<Outcome<T>> {
    let value: T | undefined;
    const error = await this.#run(async () => {
      value = await job();
    });
    return error ? { value: null, error } : { value: value as T, error: null };
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
