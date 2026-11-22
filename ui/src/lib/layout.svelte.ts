// View state for the panel sizes. Rust knows nothing about them: they are a per-device view
// preference, so localStorage holds them and every access is guarded (it can be blocked or corrupt).

export const LAYOUT_KEY = "fernjo.layout.v1";
export const SIDEBAR_DEFAULT = 300;
export const SIDEBAR_MIN = 240;
export const SIDEBAR_MAX = 560;
/** The header row of the properties panel: the smallest it can be. */
export const PANEL_MIN = 26;
/** Upper bound for a stored panel height before the window clamps it further. */
const PANEL_STORE_MAX = 4000;

/** Tree rows kept visible while the panel is content-sized: the panel gives way first. */
export const TREE_ROWS_DEFAULT = 23;
/** Tree rows kept visible once the user has set a panel height: they chose the trade. */
export const TREE_ROWS_SIZED = 6;

/**
 * The tree slot's minimum height. Sidebar.svelte mirrors it in CSS: `.tree-slot` is the default
 * (`panelSized` false), and `.panel-sized`, `.note-open` and `.bars-open` (a notice bar or the sync
 * line is shown) use the small one; the two must stay in step or the sidebar overflows.
 */
export function treeMinHeight(
  panelSized: boolean,
  rowHeight: number,
  viewportHeight: number,
): number {
  const rows = panelSized ? TREE_ROWS_SIZED : TREE_ROWS_DEFAULT;
  return Math.min(rowHeight * rows, viewportHeight - 120);
}

/** The widest the sidebar may be in a window of this width. */
export function sidebarMax(viewportWidth: number): number {
  return Math.max(SIDEBAR_MIN, Math.min(SIDEBAR_MAX, Math.floor(viewportWidth / 2)));
}

export function clampSidebarWidth(value: number, viewportWidth: number): number {
  if (!Number.isFinite(value)) return SIDEBAR_DEFAULT;
  return Math.round(Math.min(sidebarMax(viewportWidth), Math.max(SIDEBAR_MIN, value)));
}

/**
 * `available` is the sidebar height minus the fixed chrome (filter, notices, footer), `treeMin`
 * the tree slot's minimum height. The panel never gets so tall that the tree drops below it.
 */
export function clampPanelHeight(value: number, available: number, treeMin: number): number {
  const max = Math.max(PANEL_MIN, available - treeMin);
  if (!Number.isFinite(value)) return PANEL_MIN;
  return Math.round(Math.min(max, Math.max(PANEL_MIN, value)));
}

const isSize = (v: unknown, min: number, max: number): v is number =>
  typeof v === "number" && Number.isFinite(v) && v >= min && v <= max;

export class LayoutStore {
  sidebarWidth = $state(SIDEBAR_DEFAULT);
  /** `null`: the panel is as tall as its content, as before this setting existed. */
  propertiesHeight = $state<number | null>(null);
  viewportWidth = $state(typeof window === "undefined" ? 1280 : window.innerWidth);

  constructor() {
    try {
      const raw = localStorage.getItem(LAYOUT_KEY);
      if (raw === null) return;
      const v: unknown = JSON.parse(raw);
      if (typeof v !== "object" || v === null) return;
      const o = v as Record<string, unknown>;
      if (isSize(o.sidebarWidth, SIDEBAR_MIN, SIDEBAR_MAX)) this.sidebarWidth = o.sidebarWidth;
      if (isSize(o.propertiesHeight, PANEL_MIN, PANEL_STORE_MAX)) {
        this.propertiesHeight = o.propertiesHeight;
      }
    } catch {
      // blocked or corrupt storage: the defaults stay
    }
  }

  /** The width the shell actually uses: the stored one, held inside what this window allows. */
  get effectiveSidebarWidth(): number {
    return clampSidebarWidth(this.sidebarWidth, this.viewportWidth);
  }

  setSidebarWidth(px: number): void {
    this.sidebarWidth = clampSidebarWidth(px, SIDEBAR_MAX * 2);
    this.#save();
  }

  setPropertiesHeight(px: number | null): void {
    this.propertiesHeight =
      px === null || !Number.isFinite(px)
        ? null
        : Math.round(Math.min(PANEL_STORE_MAX, Math.max(PANEL_MIN, px)));
    this.#save();
  }

  resetSidebarWidth(): void {
    this.setSidebarWidth(SIDEBAR_DEFAULT);
  }

  resetPropertiesHeight(): void {
    this.setPropertiesHeight(null);
  }

  #save(): void {
    try {
      localStorage.setItem(
        LAYOUT_KEY,
        JSON.stringify({
          sidebarWidth: this.sidebarWidth,
          propertiesHeight: this.propertiesHeight,
        }),
      );
    } catch {
      // the size only lasts for this run
    }
  }
}
