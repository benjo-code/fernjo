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
  vaultStatus: () => call("vault_status"),
  vaultCreate: (password, remember) => call("vault_create", { password, remember }),
  vaultUnlock: (password, remember) => call("vault_unlock", { password, remember }),
  vaultLock: () => call("vault_lock"),
  // the Rust parameters are named `old` and `new`
  vaultChangePassword: (old, next) => call("vault_change_password", { old, new: next }),
  vaultSetAutoUnlock: (enabled) => call("vault_set_auto_unlock", { enabled }),
  listCredentials: () => call("list_credentials"),
  getCredential: (id) => call("get_credential", { id }),
  createCredential: (draft) => call("create_credential", { draft }),
  updateCredential: (id, update) => call("update_credential", { id, update }),
  credentialUsage: (id) => call("credential_usage", { id }),
  deleteCredential: (id) => call("delete_credential", { id }),
  revealCredential: (id) => call("reveal_credential", { id }),
  getDataFolder: () => call("get_data_folder"),
  chooseDataFolder: () => call("choose_data_folder"),
  restartApp: () => call("restart_app"),
};
