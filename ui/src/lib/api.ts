import { invoke } from "@tauri-apps/api/core";
import type { Api, ApiError } from "./types";

export type { Api } from "./types";

/** Makes whatever `invoke` rejected with look like an `ApiError`. */
export function toApiError(e: unknown): ApiError {
  if (typeof e === "object" && e !== null) {
    const { code, message } = e as Record<string, unknown>;
    if (typeof code === "string" && typeof message === "string") return { code, message };
  }
  return { code: "internal", message: typeof e === "string" ? e : "Something went wrong" };
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (e) {
    throw toApiError(e);
  }
}

export const tauriApi: Api = {
  listTree: () => call("list_tree"),
  getNode: (id) => call("get_node", { id }),
  createFolder: (parentId, index, name) => call("create_folder", { parentId, index, name }),
  createConnection: (parentId, index, name, protocol, host) =>
    call("create_connection", { parentId, index, name, protocol, host }),
  updateNode: (id, patch) => call("update_node", { id, patch }),
  moveNode: (id, parentId, index) => call("move_node", { id, parentId, index }),
  duplicateNode: (id) => call("duplicate_node", { id }),
  deleteNode: (id) => call("delete_node", { id }),
};
