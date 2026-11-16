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

  constructor(api: Api, onSnapshot: (snapshot: Snapshot) => void) {
    this.#api = api;
    this.#onSnapshot = onSnapshot;
  }

  /** Picks a file and inspects it without a secret. A cancelled dialog changes nothing. */
  async start(kind: ImportKind): Promise<void> {
    this.reset();
    try {
      const picked = await this.#api.importPick(kind);
      if (picked === null) return;
      this.kind = kind;
      this.fileName = picked.fileName;
      await this.#inspect(null);
    } catch (e) {
      this.#fail(toApiError(e));
    }
  }

  /** Inspects the picked file again with the typed key or password. */
  async submitSecret(secret: string): Promise<void> {
    await this.#inspect(secret);
  }

  /** Applies the inspected plan; callable again from the `vault` step. */
  async apply(options: { withoutPasswords: boolean }): Promise<void> {
    this.step = "applying";
    this.error = null;
    try {
      const result = await this.#api.importApply(options);
      this.result = result;
      this.#onSnapshot(result.snapshot);
      this.step = "done";
    } catch (e) {
      const err = toApiError(e);
      if (err.code === "vault_needed") {
        this.error = err;
        this.step = "vault";
      } else {
        this.#fail(err);
      }
    }
  }

  /** Drops the pending plan in Rust and returns to `idle`. */
  async cancel(): Promise<void> {
    try {
      await this.#api.importCancel();
    } catch {
      // nothing is pending either way
    }
    this.reset();
  }

  /** `null` from the save dialog (cancelled) leaves the store idle. */
  async exportConnections(): Promise<void> {
    await this.#export(() => this.#api.exportConnections());
  }

  async exportBackup(password: string): Promise<void> {
    await this.#export(() => this.#api.exportBackup(password));
  }

  /** Back to `idle` and forgets everything shown. */
  reset(): void {
    this.step = "idle";
    this.kind = null;
    this.fileName = "";
    this.preview = null;
    this.result = null;
    this.exportResult = null;
    this.error = null;
  }

  async #inspect(secret: string | null): Promise<void> {
    this.error = null;
    try {
      this.preview = await this.#api.importInspect(secret);
      this.step = "preview";
    } catch (e) {
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

  async #export(run: () => Promise<ExportInfo | null>): Promise<void> {
    this.reset();
    try {
      const info = await run();
      if (info === null) return;
      this.exportResult = info;
      this.step = "done";
    } catch (e) {
      this.#fail(toApiError(e));
    }
  }

  #fail(err: ApiError): void {
    this.error = err;
    this.step = "error";
  }
}
