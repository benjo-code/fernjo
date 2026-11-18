// View state for the theme. Rust owns the stored choice (settings.json); localStorage only mirrors
// it so that `main.ts` can set the attribute as soon as the page script runs, before Rust has answered.
import { toApiError } from "./api";
import type { Api, ApiError, Theme, ThemeInfo } from "./types";

export const THEME_KEY = "fernjo.theme";

const THEMES: readonly Theme[] = ["system", "light", "dark"];

const isTheme = (v: unknown): v is Theme => THEMES.includes(v as Theme);

/** `light` and `dark` pin the palette; `system` removes the pin so the media query decides. */
export function applyTheme(theme: Theme): void {
  if (theme === "system") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = theme;
}

export function readStoredTheme(): Theme {
  try {
    const v = localStorage.getItem(THEME_KEY);
    return isTheme(v) ? v : "system";
  } catch {
    return "system";
  }
}

function mirror(theme: Theme): void {
  try {
    localStorage.setItem(THEME_KEY, theme);
  } catch {
    // the mirror is only a flash guard
  }
}

export class ThemeStore {
  info = $state<ThemeInfo | null>(null);
  #api: Api;

  constructor(api: Api) {
    this.#api = api;
  }

  /** Reads the choice from Rust; a failure leaves everything as it was. */
  async load(): Promise<void> {
    try {
      this.#take(await this.#api.getTheme());
    } catch {
      // the stored mirror (or the system theme) stays in effect
    }
  }

  /** `null` when saved, otherwise the error Rust answered with; nothing changes on an error. */
  async set(theme: Theme): Promise<ApiError | null> {
    try {
      this.#take(await this.#api.setTheme(theme));
      return null;
    } catch (e) {
      return toApiError(e);
    }
  }

  #take(info: ThemeInfo) {
    this.info = info;
    applyTheme(info.theme);
    // A run that cannot store the choice (FERNJO_DATA_DIR) answers System; writing that would
    // overwrite the mirror of the real settings and flash once on the next real start.
    if (info.persistent) mirror(info.theme);
  }
}
