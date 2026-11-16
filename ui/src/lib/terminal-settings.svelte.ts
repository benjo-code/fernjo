// View state for the terminal settings. Validation lives in Rust; a rejected save comes back as
// the api error and the stored values stay as they were.
import { toApiError } from "./api";
import type { Api, ApiError, TerminalSettings } from "./types";

export class TerminalSettingsStore {
  info = $state<TerminalSettings | null>(null);
  #api: Api;

  constructor(api: Api) {
    this.#api = api;
  }

  /** Reads the settings from Rust; a failure leaves `info` as it was. */
  async load(): Promise<void> {
    try {
      this.info = await this.#api.getTerminalSettings();
    } catch {
      // callers fall back to defaults
    }
  }

  /** `null` when saved, otherwise the error Rust answered with. */
  async save(input: {
    fontFamily: string | null;
    fontSize: number;
    scrollback: number;
  }): Promise<ApiError | null> {
    try {
      this.info = await this.#api.setTerminalSettings(input);
      return null;
    } catch (e) {
      return toApiError(e);
    }
  }
}
