// Shapes of the data that crosses the IPC boundary. They mirror the serde output of
// `fernjo-core` and `src-tauri/src/commands.rs`; TypeScript never derives validity or
// inherited values itself.

export type Protocol = "ssh" | "rdp";

/** Local settings of a node; a missing key means "inherit". */
export interface Settings {
  username?: string;
  port?: number;
  keepaliveSecs?: number;
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
}

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
  passphrase?: string;
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
}
