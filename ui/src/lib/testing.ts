// Test helpers shared by the UI unit tests (not shipped: nothing in the app imports this file).
import type {
  Api,
  ApiError,
  ApplyResult,
  ConnectionNode,
  CredentialInfo,
  DataFolderInfo,
  FolderNode,
  ImportPreview,
  Mutation,
  Protocol,
  Report,
  ResolvedNode,
  SessionEvent,
  Snapshot,
  TerminalSettings,
  Theme,
  ThemeInfo,
  TreeNode,
  VaultStatus,
} from "./types";
import { SessionStore } from "./sessions.svelte";

export function folder(name: string, children: TreeNode[] = [], extra: Partial<FolderNode> = {}) {
  const node: FolderNode = {
    id: name,
    type: "folder",
    name,
    description: "",
    settings: {},
    children,
    ...extra,
  };
  return node;
}

export function conn(
  name: string,
  protocol: Protocol = "ssh",
  extra: Partial<ConnectionNode> = {},
): ConnectionNode {
  return {
    id: name,
    type: "connection",
    name,
    description: "",
    settings: {},
    protocol,
    host: `${name}.lab`,
    ...extra,
  };
}

export function snapshot(children: TreeNode[], notices: Snapshot["notices"] = []): Snapshot {
  return { children, notices };
}

export function apiError(code: string, message = code): ApiError {
  return { code, message };
}

export function vaultStatus(partial: Partial<VaultStatus> = {}): VaultStatus {
  return {
    state: "none",
    reason: null,
    path: null,
    autoUnlock: false,
    autoUnlockIssue: null,
    ...partial,
  };
}

export function credential(partial: Partial<CredentialInfo> = {}): CredentialInfo {
  return {
    id: "cred",
    name: "cred",
    kind: "password",
    username: null,
    domain: null,
    notes: "",
    ...partial,
  };
}

export function dataFolder(partial: Partial<DataFolderInfo> = {}): DataFolderInfo {
  return {
    path: "/data",
    source: "default",
    changeable: true,
    restartRequired: false,
    existingFiles: [],
    ...partial,
  };
}

export function terminalSettings(partial: Partial<TerminalSettings> = {}): TerminalSettings {
  return { fontFamily: null, fontSize: 14, scrollback: 10000, persistent: true, ...partial };
}

export function themeInfo(partial: Partial<ThemeInfo> = {}): ThemeInfo {
  return { theme: "system", persistent: true, ...partial };
}

export function report(partial: Partial<Report> = {}): Report {
  return {
    counts: {
      folders: 0,
      ssh: 0,
      rdp: 0,
      credentials: 0,
      sharedCredentials: 0,
      skipped: 0,
      partial: 0,
      problems: 0,
    },
    skipped: [],
    partial: [],
    problems: [],
    ...partial,
  };
}

export function preview(partial: Partial<ImportPreview> = {}): ImportPreview {
  return {
    kind: "mremoteng",
    fileName: "confCons.xml",
    report: report(),
    backup: null,
    needsVault: false,
    vault: vaultStatus(),
    ...partial,
  };
}

function applyResult(children: TreeNode[]): ApplyResult {
  return { snapshot: snapshot(children), report: report(), restored: null };
}

type Calls = Record<keyof Api, unknown[][]>;

/** A scriptable `Api`: every method records its arguments; `impl` overrides behaviour. */
export function mockApi(initial: TreeNode[] = [], impl: Partial<Api> = {}) {
  const calls: Calls = {
    listTree: [],
    reloadTree: [],
    getNode: [],
    createFolder: [],
    createConnection: [],
    updateNode: [],
    moveNode: [],
    duplicateNode: [],
    deleteNode: [],
    vaultStatus: [],
    vaultCreate: [],
    vaultUnlock: [],
    vaultUnlockOlder: [],
    vaultLock: [],
    vaultChangePassword: [],
    vaultSetAutoUnlock: [],
    listCredentials: [],
    getCredential: [],
    createCredential: [],
    updateCredential: [],
    credentialUsage: [],
    deleteCredential: [],
    revealCredential: [],
    getDataFolder: [],
    chooseDataFolder: [],
    restartApp: [],
    openSession: [],
    sessionInput: [],
    sessionResize: [],
    sessionAck: [],
    closeSession: [],
    reconnectSession: [],
    answerPrompt: [],
    resumeSession: [],
    openExternalUrl: [],
    getTerminalSettings: [],
    setTerminalSettings: [],
    getTheme: [],
    setTheme: [],
    importPick: [],
    importInspect: [],
    importApply: [],
    importCancel: [],
    exportConnections: [],
    exportBackup: [],
  };
  let current = initial;
  const mutation = (id: string | null): Mutation => ({ id, snapshot: snapshot(current) });
  const api: Api = {
    listTree: async () => {
      calls.listTree.push([]);
      return impl.listTree ? impl.listTree() : snapshot(current);
    },
    reloadTree: async () => {
      calls.reloadTree.push([]);
      return impl.reloadTree ? impl.reloadTree() : snapshot(current);
    },
    getNode: async (id) => {
      calls.getNode.push([id]);
      if (impl.getNode) return impl.getNode(id);
      throw apiError("not_found");
    },
    createFolder: async (...args) => {
      calls.createFolder.push(args);
      return impl.createFolder ? impl.createFolder(...args) : mutation(null);
    },
    createConnection: async (...args) => {
      calls.createConnection.push(args);
      return impl.createConnection ? impl.createConnection(...args) : mutation(null);
    },
    updateNode: async (...args) => {
      calls.updateNode.push(args);
      return impl.updateNode ? impl.updateNode(...args) : mutation(args[0]);
    },
    moveNode: async (...args) => {
      calls.moveNode.push(args);
      return impl.moveNode ? impl.moveNode(...args) : mutation(args[0]);
    },
    duplicateNode: async (...args) => {
      calls.duplicateNode.push(args);
      return impl.duplicateNode ? impl.duplicateNode(...args) : mutation(null);
    },
    deleteNode: async (...args) => {
      calls.deleteNode.push(args);
      return impl.deleteNode ? impl.deleteNode(...args) : mutation(null);
    },
    vaultStatus: async () => {
      calls.vaultStatus.push([]);
      return impl.vaultStatus ? impl.vaultStatus() : vaultStatus();
    },
    vaultCreate: async (...args) => {
      calls.vaultCreate.push(args);
      return impl.vaultCreate ? impl.vaultCreate(...args) : vaultStatus({ state: "unlocked" });
    },
    vaultUnlock: async (...args) => {
      calls.vaultUnlock.push(args);
      return impl.vaultUnlock ? impl.vaultUnlock(...args) : vaultStatus({ state: "unlocked" });
    },
    vaultUnlockOlder: async (...args) => {
      calls.vaultUnlockOlder.push(args);
      return impl.vaultUnlockOlder
        ? impl.vaultUnlockOlder(...args)
        : vaultStatus({ state: "unlocked" });
    },
    vaultLock: async () => {
      calls.vaultLock.push([]);
      return impl.vaultLock ? impl.vaultLock() : vaultStatus({ state: "locked" });
    },
    vaultChangePassword: async (...args) => {
      calls.vaultChangePassword.push(args);
      return impl.vaultChangePassword
        ? impl.vaultChangePassword(...args)
        : vaultStatus({ state: "unlocked" });
    },
    vaultSetAutoUnlock: async (...args) => {
      calls.vaultSetAutoUnlock.push(args);
      return impl.vaultSetAutoUnlock
        ? impl.vaultSetAutoUnlock(...args)
        : vaultStatus({ state: "unlocked" });
    },
    listCredentials: async () => {
      calls.listCredentials.push([]);
      return impl.listCredentials ? impl.listCredentials() : [];
    },
    getCredential: async (...args) => {
      calls.getCredential.push(args);
      if (impl.getCredential) return impl.getCredential(...args);
      throw apiError("not_found");
    },
    createCredential: async (...args) => {
      calls.createCredential.push(args);
      if (impl.createCredential) return impl.createCredential(...args);
      return credential({ name: args[0].name, kind: args[0].kind });
    },
    updateCredential: async (...args) => {
      calls.updateCredential.push(args);
      if (impl.updateCredential) return impl.updateCredential(...args);
      throw apiError("not_found");
    },
    credentialUsage: async (...args) => {
      calls.credentialUsage.push(args);
      return impl.credentialUsage ? impl.credentialUsage(...args) : [];
    },
    deleteCredential: async (...args) => {
      calls.deleteCredential.push(args);
      return impl.deleteCredential ? impl.deleteCredential(...args) : mutation(null);
    },
    revealCredential: async (...args) => {
      calls.revealCredential.push(args);
      if (impl.revealCredential) return impl.revealCredential(...args);
      throw apiError("not_found");
    },
    getDataFolder: async () => {
      calls.getDataFolder.push([]);
      return impl.getDataFolder ? impl.getDataFolder() : dataFolder();
    },
    chooseDataFolder: async () => {
      calls.chooseDataFolder.push([]);
      return impl.chooseDataFolder ? impl.chooseDataFolder() : null;
    },
    restartApp: async () => {
      calls.restartApp.push([]);
      if (impl.restartApp) await impl.restartApp();
    },
    openSession: async (...args) => {
      calls.openSession.push(args);
      return impl.openSession ? impl.openSession(...args) : "sid";
    },
    sessionInput: async (...args) => {
      calls.sessionInput.push(args);
      if (impl.sessionInput) await impl.sessionInput(...args);
    },
    sessionResize: async (...args) => {
      calls.sessionResize.push(args);
      if (impl.sessionResize) await impl.sessionResize(...args);
    },
    sessionAck: async (...args) => {
      calls.sessionAck.push(args);
      if (impl.sessionAck) await impl.sessionAck(...args);
    },
    closeSession: async (...args) => {
      calls.closeSession.push(args);
      if (impl.closeSession) await impl.closeSession(...args);
    },
    reconnectSession: async (...args) => {
      calls.reconnectSession.push(args);
      if (impl.reconnectSession) await impl.reconnectSession(...args);
    },
    answerPrompt: async (...args) => {
      calls.answerPrompt.push(args);
      if (impl.answerPrompt) await impl.answerPrompt(...args);
    },
    resumeSession: async (...args) => {
      calls.resumeSession.push(args);
      if (impl.resumeSession) await impl.resumeSession(...args);
    },
    openExternalUrl: async (...args) => {
      calls.openExternalUrl.push(args);
      if (impl.openExternalUrl) await impl.openExternalUrl(...args);
    },
    getTerminalSettings: async () => {
      calls.getTerminalSettings.push([]);
      return impl.getTerminalSettings ? impl.getTerminalSettings() : terminalSettings();
    },
    setTerminalSettings: async (...args) => {
      calls.setTerminalSettings.push(args);
      return impl.setTerminalSettings
        ? impl.setTerminalSettings(...args)
        : terminalSettings(args[0]);
    },
    getTheme: async () => {
      calls.getTheme.push([]);
      return impl.getTheme ? impl.getTheme() : themeInfo();
    },
    setTheme: async (...args) => {
      calls.setTheme.push(args);
      return impl.setTheme ? impl.setTheme(...args) : themeInfo({ theme: args[0] as Theme });
    },
    importPick: async (...args) => {
      calls.importPick.push(args);
      return impl.importPick ? impl.importPick(...args) : { fileName: "confCons.xml" };
    },
    importInspect: async (...args) => {
      calls.importInspect.push(args);
      return impl.importInspect ? impl.importInspect(...args) : preview();
    },
    importApply: async (...args) => {
      calls.importApply.push(args);
      return impl.importApply ? impl.importApply(...args) : applyResult(current);
    },
    importCancel: async () => {
      calls.importCancel.push([]);
      if (impl.importCancel) await impl.importCancel();
    },
    exportConnections: async () => {
      calls.exportConnections.push([]);
      return impl.exportConnections
        ? impl.exportConnections()
        : { fileName: "fernjo-connections.json", droppedCredentials: 0 };
    },
    exportBackup: async (...args) => {
      calls.exportBackup.push(args);
      return impl.exportBackup
        ? impl.exportBackup(...args)
        : { fileName: "fernjo-backup.fjbk", droppedCredentials: 0 };
    },
  };
  return {
    api,
    calls,
    /** Changes what the default implementations return as the tree. */
    setTree(children: TreeNode[]) {
      current = children;
    },
  };
}

export function resolved(partial: Partial<ResolvedNode> & { id: string; name: string }) {
  const none = { local: null, effective: null, source: { kind: "default" as const } };
  const node: ResolvedNode = {
    kind: "connection",
    description: "",
    protocol: "ssh",
    host: "h.lab",
    username: none,
    port: { local: null, effective: 22, source: { kind: "default" } },
    keepaliveSecs: { local: null, effective: 30, source: { kind: "default" } },
    credential: none,
    ...partial,
  };
  return node;
}

/** A promise that is resolved or rejected from the outside. */
export function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/**
 * A session store with one started tab whose events the test drives (prompts, state changes).
 * `emit` plays the part of Rust's session channel.
 */
export async function startedTab(impl: Partial<Api> = {}) {
  let handler: (e: SessionEvent | ArrayBuffer) => void = () => {};
  const mock = mockApi([], {
    ...impl,
    openSession: async (_nodeId, onEvent) => {
      handler = onEvent;
      return "sid";
    },
  });
  const sessions = new SessionStore(mock.api, (id) => id);
  sessions.open("n1");
  const tab = sessions.tabs[0]!;
  sessions.attachTerminal(tab.id, { write() {}, size: () => ({ cols: 80, rows: 24 }) });
  await new Promise((r) => setTimeout(r, 0));
  return { sessions, tab, emit: (e: SessionEvent) => handler(e), ...mock };
}
