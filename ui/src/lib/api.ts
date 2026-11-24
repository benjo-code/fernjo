import { listen } from "@tauri-apps/api/event";
import { Channel, invoke, type InvokeArgs, type InvokeOptions } from "@tauri-apps/api/core";
import type { Api, ApiError, SessionEvent, SyncStatus, VaultStatus } from "./types";

export type { Api } from "./types";

/** Makes whatever `invoke` rejected with look like an `ApiError`. */
export function toApiError(e: unknown): ApiError {
  if (typeof e === "object" && e !== null) {
    const { code, message } = e as Record<string, unknown>;
    if (typeof code === "string" && typeof message === "string") return { code, message };
  }
  return { code: "internal", message: typeof e === "string" ? e : "Something went wrong" };
}

async function call<T>(command: string, args?: InvokeArgs, options?: InvokeOptions): Promise<T> {
  try {
    return await invoke<T>(command, args, options);
  } catch (e) {
    throw toApiError(e);
  }
}

export const tauriApi: Api = {
  listTree: () => call("list_tree"),
  reloadTree: () => call("reload_tree"),
  getNode: (id) => call("get_node", { id }),
  createFolder: (parentId, index, name) => call("create_folder", { parentId, index, name }),
  createConnection: (parentId, index, name, protocol, host) =>
    call("create_connection", { parentId, index, name, protocol, host }),
  updateNode: (id, patch) => call("update_node", { id, patch }),
  moveNode: (id, parentId, index) => call("move_node", { id, parentId, index }),
  moveNodes: (ids, parentId, index) => call("move_nodes", { ids, parentId, index }),
  duplicateNode: (id) => call("duplicate_node", { id }),
  deleteNode: (id) => call("delete_node", { id }),
  vaultStatus: () => call("vault_status"),
  vaultCreate: (password, remember) => call("vault_create", { password, remember }),
  vaultUnlock: (password, remember) => call("vault_unlock", { password, remember }),
  vaultUnlockOlder: (password, remember) => call("vault_unlock_older", { password, remember }),
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
  openSession: (nodeId, onEvent, cols, rows) => {
    // Raw output arrives as an ArrayBuffer, JSON events as parsed objects.
    const channel = new Channel<ArrayBuffer | SessionEvent>();
    channel.onmessage = onEvent;
    return call("open_session", { nodeId, onEvent: channel, cols, rows });
  },
  quickParse: (input) => call("quick_parse", { input }),
  clipboardReadText: () => call("clipboard_read_text"),
  openQuickSession: (target, onEvent, cols, rows) => {
    const channel = new Channel<ArrayBuffer | SessionEvent>();
    channel.onmessage = onEvent;
    return call("open_quick_session", { target, onEvent: channel, cols, rows });
  },
  restoreSession: (target, onEvent, cols, rows) => {
    const channel = new Channel<ArrayBuffer | SessionEvent>();
    channel.onmessage = onEvent;
    return call("restore_session", { target, onEvent: channel, cols, rows });
  },
  getOpenTabs: () => call("get_open_tabs"),
  setReopenTabs: (enabled) => call("set_reopen_tabs", { enabled }),
  saveOpenTabs: (tabs, active) => call("save_open_tabs", { tabs, active }),
  // the input is the raw request body, the session id travels in a header
  sessionInput: (id, data) => call("session_input", data, { headers: { "x-fernjo-session": id } }),
  rdpInput: (id, events) => call("rdp_input", { id, events }),
  rdpClipboard: (id, text) => call("rdp_clipboard", { id, text }),
  sessionResize: (id, cols, rows) => call("session_resize", { id, cols, rows }),
  sessionAck: (id, bytes) => call("session_ack", { id, bytes }),
  closeSession: (id) => call("close_session", { id }),
  reconnectSession: (id, cols, rows) => call("reconnect_session", { id, cols, rows }),
  answerPrompt: (id, promptId, answer) => call("answer_prompt", { id, promptId, answer }),
  resumeSession: (id, skipCredential) => call("resume_session", { id, skipCredential }),
  openExternalUrl: (url) => call("open_external_url", { url }),
  getTerminalSettings: () => call("get_terminal_settings"),
  setTerminalSettings: (settings) => call("set_terminal_settings", { settings }),
  getTheme: () => call("get_theme"),
  setTheme: (theme) => call("set_theme", { theme }),
  getVaultAutoLock: () => call("get_vault_auto_lock"),
  setVaultAutoLock: (minutes) => call("set_vault_auto_lock", { minutes }),
  noteActivity: () => call("note_activity"),
  importPick: (kind) => call("import_pick", { kind }),
  importInspect: (secret) => call("import_inspect", { secret }),
  importApply: (options) => call("import_apply", { options }),
  importCancel: () => call("import_cancel"),
  exportConnections: () => call("export_connections"),
  exportBackup: (password) => call("export_backup", { password }),
  syncStatus: () => call("sync_status"),
  syncSetup: (url, secret, caPem, deviceName) =>
    call("sync_setup", { url, secret, caPem, deviceName }),
  syncPairingCode: () => call("sync_pairing_code"),
  syncJoin: (code, deviceName, confirmReplace) =>
    call("sync_join", { code, deviceName, confirmReplace }),
  syncNow: () => call("sync_now"),
  syncReseed: () => call("sync_reseed"),
  syncEditSecret: (secret) => call("sync_edit_secret", { secret }),
  syncLeave: () => call("sync_leave"),
  syncDismissFindings: () => call("sync_dismiss_findings"),
  onSyncStatus: (handler) => listen<SyncStatus>("sync-status", (e) => handler(e.payload)),
  onVaultStatus: (handler) => listen<VaultStatus>("vault-status", (e) => handler(e.payload)),
};
