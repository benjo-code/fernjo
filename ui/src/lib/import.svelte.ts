// View state of the import, restore and export dialogs. Everything is decided in Rust: this
// store only walks the steps Rust's answers imply and never validates a file or a secret. A
// secret (mRemoteNG key, backup password) is handed to the api and not stored.
import { toApiError } from "./api";
import type {
  Api,
  ApiError,
  ApplyResult,
  ExportInfo,
  ImportKind,
  ImportPreview,
  Snapshot,
} from "./types";

/**
 * - `key`: the file needs a key or password (also after a wrong one: `error` says so).
 * - `vault`: applying needs the vault; the dialog unlocks or creates it, then calls `apply` again.
 * - `done`: `result` holds an import or restore, `exportResult` an export.
 * - `error`: a final error; the plan may be gone, `reset()` returns to `idle`.
 */
export type ImportStep = "idle" | "key" | "preview" | "vault" | "applying" | "done" | "error";

/** Codes after which the same file can be inspected again with another secret. */
const ASKS_FOR_SECRET = new Set(["key_needed", "wrong_key", "password_needed", "wrong_password"]);

export class ImportStore {
  step = $state<ImportStep>("idle");
  kind = $state<ImportKind | null>(null);
  fileName = $state("");
  preview = $state<ImportPreview | null>(null);
  result = $state<ApplyResult | null>(null);
  exportResult = $state<ExportInfo | null>(null);
  error = $state<ApiError | null>(null);

  #api: Api;
  #onSnapshot: (snapshot: Snapshot) => void;
  #busy = $state(false);
  /** Bumped by `reset()`: an async call that started before it drops its result. */
  #generation = 0;

  constructor(api: Api, onSnapshot: (snapshot: Snapshot) => void) {
    this.#api = api;
    this.#onSnapshot = onSnapshot;
  }

  /** True while a call to Rust is in flight; the dialog disables its buttons on it. */
  get busy(): boolean {
    return this.#busy;
  }

  /**
   * Picks a file and inspects it without a secret. A cancelled dialog changes nothing (the view
   * of a previous import stays); a pick replaces it. Ignored while a call is in flight.
   */
  async start(kind: ImportKind): Promise<void> {
    if (this.#busy) return;
    const gen = this.#generation;
    this.#busy = true;
    try {
      const picked = await this.#api.importPick(kind);
      if (gen !== this.#generation || picked === null) return;
      this.#clear();
      this.kind = kind;
      this.fileName = picked.fileName;
      await this.#inspect(null, gen);
    } catch (e) {
      if (gen === this.#generation) this.#fail(toApiError(e));
    } finally {
      this.#release(gen);
    }
  }

  /** Inspects the picked file again with the typed key or password. Ignored while busy. */
  async submitSecret(secret: string): Promise<void> {
    if (this.#busy) return;
    const gen = this.#generation;
    this.#busy = true;
    try {
      await this.#inspect(secret, gen);
    } finally {
      this.#release(gen);
    }
  }

  /** Applies the inspected plan; callable again from the `vault` step. Ignored while busy. */
  async apply(options: { withoutPasswords: boolean }): Promise<void> {
    if (this.#busy) return;
    const gen = this.#generation;
    this.#busy = true;
    this.step = "applying";
    this.error = null;
    let result: ApplyResult;
    try {
      result = await this.#api.importApply(options);
    } catch (e) {
      this.#release(gen);
      if (gen !== this.#generation) return;
      const err = toApiError(e);
      if (err.code === "vault_needed") {
        this.error = err;
        this.step = "vault";
      } else {
        this.#fail(err);
      }
      return;
    }
    this.#release(gen);
    if (gen === this.#generation) {
      this.result = result;
      this.step = "done";
    }
    // Rust applied it, so the tree must follow, even when the dialog was cancelled meanwhile.
    // Outside the try: a failing callback is not an api error and cannot undo "done".
    this.#onSnapshot(result.snapshot);
  }

  /** Drops the pending plan in Rust and returns to `idle`, also while a call is in flight. */
  async cancel(): Promise<void> {
    this.reset();
    try {
      await this.#api.importCancel();
    } catch {
      // nothing is pending either way
    }
  }

  /** `null` from the save dialog (cancelled) leaves the store idle. */
  async exportConnections(): Promise<void> {
    await this.#export(() => this.#api.exportConnections());
  }

  async exportBackup(password: string): Promise<void> {
    await this.#export(() => this.#api.exportBackup(password));
  }

  /** Back to `idle`, forgets everything shown and drops results of calls still in flight. */
  reset(): void {
    this.#generation++;
    this.#busy = false;
    this.#clear();
  }

  #clear(): void {
    this.step = "idle";
    this.kind = null;
    this.fileName = "";
    this.preview = null;
    this.result = null;
    this.exportResult = null;
    this.error = null;
  }

  #release(gen: number): void {
    if (gen === this.#generation) this.#busy = false;
  }

  async #inspect(secret: string | null, gen: number): Promise<void> {
    this.error = null;
    try {
      const preview = await this.#api.importInspect(secret);
      if (gen !== this.#generation) return;
      this.preview = preview;
      this.step = "preview";
    } catch (e) {
      if (gen !== this.#generation) return;
      const err = toApiError(e);
      if (!ASKS_FOR_SECRET.has(err.code)) {
        this.#fail(err);
        return;
      }
      this.step = "key";
      // asking for the first time is not an error to show; a rejected secret is
      this.error = secret === null ? null : err;
    }
  }

  /**
   * Exports only from `idle`, `done` and `error`: while an import is in `key`, `preview`,
   * `vault` or `applying` the call is ignored, so a pending import is never hidden without
   * `importCancel`.
   */
  async #export(run: () => Promise<ExportInfo | null>): Promise<void> {
    if (this.#busy || ["key", "preview", "vault", "applying"].includes(this.step)) return;
    this.reset();
    const gen = this.#generation;
    this.#busy = true;
    try {
      const info = await run();
      if (gen !== this.#generation || info === null) return;
      this.exportResult = info;
      this.step = "done";
    } catch (e) {
      if (gen === this.#generation) this.#fail(toApiError(e));
    } finally {
      this.#release(gen);
    }
  }

  #fail(err: ApiError): void {
    this.error = err;
    this.step = "error";
  }
}
