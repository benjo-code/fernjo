// Shapes of the data that crosses the IPC boundary. They mirror the serde output of
// `fernjo-core` and `src-tauri/src/commands.rs`; TypeScript never derives validity or
// inherited values itself.

export type Protocol = "ssh" | "rdp";

/** Local settings of a node; a missing key means "inherit". */
export interface Settings {
  username?: string;
  port?: number;
  keepaliveSecs?: number;
  credential?: string;
}

interface NodeBase {
  id: string;
  name: string;
  description: string;
  settings: Settings;
}

export interface FolderNode extends NodeBase {
  type: "folder";
  children: TreeNode[];
}

export interface ConnectionNode extends NodeBase {
  type: "connection";
  protocol: Protocol;
  host: string;
}

export type TreeNode = FolderNode | ConnectionNode;

export type Source =
  { kind: "local" } | { kind: "folder"; id: string; name: string } | { kind: "default" };

export interface FieldInfo<T> {
  local: T | null;
  effective: T | null;
  source: Source;
}

export interface ResolvedNode {
  id: string;
  kind: "folder" | "connection";
  name: string;
  description: string;
  protocol: Protocol | null;
  host: string | null;
  username: FieldInfo<string>;
  port: FieldInfo<number>;
  keepaliveSecs: FieldInfo<number>;
  /** A vault entry id; the name is looked up in the unlocked credential list. */
  credential: FieldInfo<string>;
}

/** Partial update: a key left out keeps the value, `null` resets to inherit. */
export interface NodePatch {
  name?: string;
  description?: string;
  host?: string;
  protocol?: Protocol;
  username?: string | null;
  port?: number | null;
  keepaliveSecs?: number | null;
  /** A vault entry id sets it; `null` resets to inherit. */
  credential?: string | null;
}

export type Notice =
  | { kind: "restoredFromBackup"; badFile: string | null }
  | { kind: "syncConflicts"; files: string[] }
  | { kind: "settingsUnreadable"; message: string };

export interface Snapshot {
  children: TreeNode[];
  notices: Notice[];
}

export interface Mutation {
  id: string | null;
  snapshot: Snapshot;
}

/** Error from a command: a stable `code` and a message fit for display. */
export interface ApiError {
  code: string;
  message: string;
}

/** What the vault commands report. `reason` and `path` are only set where Rust has them. */
export interface VaultStatus {
  state: "none" | "locked" | "unlocked" | "unusable";
  reason: string | null;
  path: string | null;
  /** The key store holds this vault's key. */
  autoUnlock: boolean;
  /**
   * Why the automatic unlock at startup did not work (locked only). Rust never deletes the stored
   * key for any of these, except that `keychain_unavailable` means nothing is known about one.
   */
  autoUnlockIssue: AutoUnlockIssue | null;
}

/** Wire codes of the startup auto-unlock failure; Rust may add more; the UI falls back to a neutral line for an unknown code and shows no line for `keychain_unavailable`. */
export type AutoUnlockIssue =
  "key_not_accepted" | "keychain_unavailable" | "unavailable" | "rolled_back";

/** A credential as the UI sees it: never a secret field. */
export interface CredentialInfo {
  id: string;
  name: string;
  kind: "password" | "privateKey";
  username: string | null;
  domain: string | null;
  notes: string;
}

interface DraftBase {
  name: string;
  username?: string;
  domain?: string;
  notes?: string;
}

/** A new credential. Secret fields that do not belong to the kind are left out entirely. */
export type CredentialDraft =
  | (DraftBase & { kind: "password"; password: string })
  | (DraftBase & { kind: "privateKey"; privateKey: string; passphrase?: string });

/** Partial update: a key left out keeps the value; a present secret field replaces the secret. */
export interface CredentialUpdate {
  name?: string;
  username?: string;
  domain?: string;
  notes?: string;
  password?: string;
  privateKey?: string;
  /** Sets or replaces the passphrase; a new `privateKey` without it keeps the stored one. */
  passphrase?: string;
  /** Removes the stored passphrase (never sent together with `passphrase`). */
  clearPassphrase?: boolean;
}

/** A node that references a credential. */
export interface UsageItem {
  id: string;
  name: string;
  kind: "folder" | "connection";
}

/** The only response that carries secret values (the explicit "reveal" action). */
export interface RevealedSecret {
  password?: string;
  privateKey?: string;
  passphrase?: string;
}

export interface DataFolderInfo {
  /** The folder in use; while a restart is pending, the folder chosen for the next start. */
  path: string;
  source: "env" | "settings" | "default";
  changeable: boolean;
  restartRequired: boolean;
  /** Data files that were already in the chosen folder and were kept. */
  existingFiles: string[];
}

/** The Tauri commands as a typed interface (replaced by a mock in tests). */
export interface Api {
  listTree(): Promise<Snapshot>;
  /** Re-reads connections.json after it changed on disk and returns the adopted tree. */
  reloadTree(): Promise<Snapshot>;
  getNode(id: string): Promise<ResolvedNode>;
  createFolder(parentId: string | null, index: number | null, name: string): Promise<Mutation>;
  createConnection(
    parentId: string | null,
    index: number | null,
    name: string,
    protocol: Protocol,
    host: string,
  ): Promise<Mutation>;
  updateNode(id: string, patch: NodePatch): Promise<Mutation>;
  moveNode(id: string, parentId: string | null, index: number): Promise<Mutation>;
  duplicateNode(id: string): Promise<Mutation>;
  deleteNode(id: string): Promise<Mutation>;
  vaultStatus(): Promise<VaultStatus>;
  vaultCreate(password: string, remember: boolean): Promise<VaultStatus>;
  vaultUnlock(password: string, remember: boolean): Promise<VaultStatus>;
  /** Unlocks a vault file that is older than the last version this device opened. */
  vaultUnlockOlder(password: string, remember: boolean): Promise<VaultStatus>;
  vaultLock(): Promise<VaultStatus>;
  vaultChangePassword(old: string, next: string): Promise<VaultStatus>;
  vaultSetAutoUnlock(enabled: boolean): Promise<VaultStatus>;
  listCredentials(): Promise<CredentialInfo[]>;
  getCredential(id: string): Promise<CredentialInfo>;
  createCredential(draft: CredentialDraft): Promise<CredentialInfo>;
  updateCredential(id: string, update: CredentialUpdate): Promise<CredentialInfo>;
  credentialUsage(id: string): Promise<UsageItem[]>;
  deleteCredential(id: string): Promise<Mutation>;
  revealCredential(id: string): Promise<RevealedSecret>;
  getDataFolder(): Promise<DataFolderInfo>;
  /** Opens the folder picker; `null` when it was cancelled. */
  chooseDataFolder(): Promise<DataFolderInfo | null>;
  restartApp(): Promise<void>;
  /**
   * Starts a session. Terminal output reaches `onEvent` as an `ArrayBuffer`, events as parsed
   * objects. Resolves with the session id.
   */
  openSession(
    nodeId: string,
    onEvent: (e: SessionEvent | ArrayBuffer) => void,
    cols: number,
    rows: number,
  ): Promise<string>;
  sessionInput(id: string, data: Uint8Array): Promise<void>;
  sessionResize(id: string, cols: number, rows: number): Promise<void>;
  sessionAck(id: string, bytes: number): Promise<void>;
  closeSession(id: string): Promise<void>;
  reconnectSession(id: string, cols: number, rows: number): Promise<void>;
  answerPrompt(id: string, promptId: number, answer: PromptAnswer): Promise<void>;
  resumeSession(id: string, skipCredential: boolean): Promise<void>;
  openExternalUrl(url: string): Promise<void>;
  getTerminalSettings(): Promise<TerminalSettings>;
  setTerminalSettings(s: {
    fontFamily: string | null;
    fontSize: number;
    scrollback: number;
  }): Promise<TerminalSettings>;
  getTheme(): Promise<ThemeInfo>;
  setTheme(theme: Theme): Promise<ThemeInfo>;
  /** Opens the native file picker; `null` when it was cancelled. */
  importPick(kind: ImportKind): Promise<PickedFile | null>;
  /** The secret is an mRemoteNG key or a backup password; the only secret that goes in. */
  importInspect(secret: string | null): Promise<ImportPreview>;
  importApply(options: { withoutPasswords: boolean }): Promise<ApplyResult>;
  importCancel(): Promise<void>;
  /** Opens the save dialog and writes the file; `null` when it was cancelled. */
  exportConnections(): Promise<ExportInfo | null>;
  exportBackup(password: string): Promise<ExportInfo | null>;
}

export type ImportKind = "mremoteng" | "connections" | "backup";

export interface PickedFile {
  fileName: string;
}

/** Names and paths only, never a password. */
export interface ReportLine {
  path: string;
  text: string;
}

export interface Counts {
  folders: number;
  ssh: number;
  rdp: number;
  credentials: number;
  sharedCredentials: number;
  skipped: number;
  partial: number;
  problems: number;
}

export interface Report {
  counts: Counts;
  skipped: ReportLine[];
  partial: ReportLine[];
  problems: ReportLine[];
}

export interface BackupSummary {
  createdAt: string;
  app: string;
  folders: number;
  connections: number;
  credentials: number;
}

export interface ImportPreview {
  kind: ImportKind;
  fileName: string;
  /** mRemoteNG and connections imports. */
  report: Report | null;
  /** Restores. */
  backup: BackupSummary | null;
  /** Applying needs an unlocked vault. */
  needsVault: boolean;
  vault: VaultStatus;
}

export interface RestoreSummary {
  createdAt: string;
  folders: number;
  connections: number;
  credentials: number;
}

export interface ApplyResult {
  snapshot: Snapshot;
  report: Report | null;
  restored: RestoreSummary | null;
}

export interface ExportInfo {
  fileName: string;
  /** Credential references left out of a connections export. */
  droppedCredentials: number;
}

export type SessionState = "connecting" | "needsVault" | "connected" | "disconnected";

/** What the user is asked. Never contains a secret. */
export type Prompt =
  | { kind: "hostKey"; host: string; port: number; keyType: string; fingerprint: string }
  | {
      kind: "hostKeySynced";
      host: string;
      port: number;
      keyType: string;
      fingerprint: string;
      previous: string[];
    }
  | {
      kind: "hostKeyChanged";
      host: string;
      port: number;
      keyType: string;
      old: string[];
      new: string;
    }
  | {
      kind: "keyboardInteractive";
      name: string;
      instructions: string;
      prompts: { text: string; echo: boolean }[];
    }
  | { kind: "password"; user: string; host: string }
  | { kind: "username"; host: string }
  | { kind: "keyPassphrase" };

/** A JSON message on a session channel. */
export type SessionEvent =
  | { kind: "state"; state: SessionState; detail: string | null }
  | { kind: "prompt"; id: number; prompt: Prompt };

/** The user's answer to a prompt (UI to Rust only). */
export type PromptAnswer =
  | { kind: "hostKey"; decision: "once" | "always" | "reject" }
  | { kind: "hostKeyChanged"; decision: "replace" | "reject" }
  | { kind: "secrets"; values: string[] }
  | { kind: "text"; value: string }
  | { kind: "cancel" };

export type Theme = "system" | "light" | "dark";

/** Why settings only live in memory. */
export type UnsavedReason = "env" | "settingsUnusable" | "noConfigDir";

export interface ThemeInfo {
  theme: Theme;
  /** `false` when the choice only lives in memory. */
  persistent: boolean;
  /** Why it is not saved; `null` when it is. */
  unsavedReason: UnsavedReason | null;
}

export interface TerminalSettings {
  /** `null` means the default monospace stack. */
  fontFamily: string | null;
  fontSize: number;
  scrollback: number;
  /** `false` when the values only live in memory. */
  persistent: boolean;
  /** Why they are not saved; `null` when they are. */
  unsavedReason: UnsavedReason | null;
}
