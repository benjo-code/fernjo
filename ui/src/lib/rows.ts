// Presentation helpers over a snapshot: flattening into visible rows, filtering, counting.
// (Validity and inheritance are Rust's business and do not appear here.)
import type { TreeNode } from "./types";

export interface Row {
  node: TreeNode;
  depth: number;
  /** Id of the parent folder, `null` at the top level. */
  parentId: string | null;
  /** Position among the parent's children in the real (unfiltered) tree. */
  index: number;
  expanded: boolean;
  hasChildren: boolean;
}

const normalise = (query: string) => query.trim().toLowerCase();

function matches(node: TreeNode, q: string): boolean {
  if (q === "") return true;
  return (
    node.name.toLowerCase().includes(q) ||
    node.description.toLowerCase().includes(q) ||
    (node.type === "connection" && node.host.toLowerCase().includes(q))
  );
}

/** Case-insensitive substring match over name, description and (connections) host. */
export function matchesFilter(node: TreeNode, query: string): boolean {
  return matches(node, normalise(query));
}

/** Number of nodes below `node` (the node itself is not counted). */
export function countDescendants(node: TreeNode): number {
  if (node.type === "connection") return 0;
  return node.children.reduce((sum, child) => sum + 1 + countDescendants(child), 0);
}

/** Ids of every node and of every folder in the tree. */
export function collectIds(children: TreeNode[]): { all: Set<string>; folders: Set<string> } {
  const all = new Set<string>();
  const folders = new Set<string>();
  const walk = (nodes: TreeNode[]) => {
    for (const node of nodes) {
      all.add(node.id);
      if (node.type === "folder") {
        folders.add(node.id);
        walk(node.children);
      }
    }
  };
  walk(children);
  return { all, folders };
}

function unfiltered(
  nodes: TreeNode[],
  expanded: ReadonlySet<string>,
  depth: number,
  parentId: string | null,
  out: Row[],
) {
  nodes.forEach((node, index) => {
    const isOpen = node.type === "folder" && expanded.has(node.id);
    out.push({
      node,
      depth,
      parentId,
      index,
      expanded: isOpen,
      hasChildren: node.type === "folder" && node.children.length > 0,
    });
    if (node.type === "folder" && isOpen)
      unfiltered(node.children, expanded, depth + 1, node.id, out);
  });
}

/** Matches and the ancestors of matches; ancestors are shown expanded. */
function filtered(nodes: TreeNode[], q: string, depth: number, parentId: string | null): Row[] {
  const out: Row[] = [];
  nodes.forEach((node, index) => {
    if (node.type === "folder") {
      const inside = filtered(node.children, q, depth + 1, node.id);
      if (inside.length === 0 && !matches(node, q)) return;
      out.push({
        node,
        depth,
        parentId,
        index,
        expanded: inside.length > 0,
        hasChildren: node.children.length > 0,
      });
      out.push(...inside);
    } else if (matches(node, q)) {
      out.push({ node, depth, parentId, index, expanded: false, hasChildren: false });
    }
  });
  return out;
}

/** The rows currently visible in the tree for the given expansion state and filter text. */
export function flattenRows(
  children: TreeNode[],
  expanded: ReadonlySet<string>,
  filter: string,
): Row[] {
  const q = normalise(filter);
  if (q === "") {
    const out: Row[] = [];
    unfiltered(children, expanded, 0, null, out);
    return out;
  }
  return filtered(children, q, 0, null);
}
