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
}
