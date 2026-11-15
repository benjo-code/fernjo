// Test helpers shared by the UI unit tests (not shipped: nothing in the app imports this file).
import type {
  Api,
  ApiError,
  ConnectionNode,
  FolderNode,
  Mutation,
  Protocol,
  ResolvedNode,
  Snapshot,
  TreeNode,
} from "./types";

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

export function snapshot(children: TreeNode[], notice: Snapshot["notice"] = null): Snapshot {
  return { children, notice };
}

export function apiError(code: string, message = code): ApiError {
  return { code, message };
}

type Calls = Record<keyof Api, unknown[][]>;

/** A scriptable `Api`: every method records its arguments; `impl` overrides behaviour. */
export function mockApi(initial: TreeNode[] = [], impl: Partial<Api> = {}) {
  const calls: Calls = {
    listTree: [],
    getNode: [],
    createFolder: [],
    createConnection: [],
    updateNode: [],
    moveNode: [],
    duplicateNode: [],
    deleteNode: [],
  };
  let current = initial;
  const mutation = (id: string | null): Mutation => ({ id, snapshot: snapshot(current) });
  const api: Api = {
    listTree: async () => {
      calls.listTree.push([]);
      return impl.listTree ? impl.listTree() : snapshot(current);
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
