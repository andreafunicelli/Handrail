/** Pure split-tree operations for a tab's pane layout. Each leaf hosts one terminal session. */
export type SplitDir = "row" | "col"; // row = side by side (split right), col = stacked (split down)

export interface LeafNode {
  type: "leaf";
  id: string;
  sessionId: string;
}
export interface SplitNode {
  type: "split";
  id: string;
  dir: SplitDir;
  /** Size of the first child, 0..1. */
  ratio: number;
  a: LayoutNode;
  b: LayoutNode;
}
export type LayoutNode = LeafNode | SplitNode;

let counter = 0;
export const nodeId = () => `n${Date.now().toString(36)}${(counter++).toString(36)}`;

export const leaf = (sessionId: string, id = nodeId()): LeafNode => ({ type: "leaf", id, sessionId });

export function leaves(node: LayoutNode): LeafNode[] {
  return node.type === "leaf" ? [node] : [...leaves(node.a), ...leaves(node.b)];
}

export function findLeaf(node: LayoutNode, leafId: string): LeafNode | null {
  return leaves(node).find((l) => l.id === leafId) ?? null;
}

export function findLeafBySession(node: LayoutNode, sessionId: string): LeafNode | null {
  return leaves(node).find((l) => l.sessionId === sessionId) ?? null;
}

/** Replace leaf `leafId` with a split of [leaf, inserted] (or [inserted, leaf] when `before`). */
export function splitLeaf(
  node: LayoutNode,
  leafId: string,
  inserted: LayoutNode,
  dir: SplitDir,
  before = false,
): LayoutNode {
  if (node.type === "leaf") {
    if (node.id !== leafId) return node;
    return {
      type: "split",
      id: nodeId(),
      dir,
      ratio: 0.5,
      a: before ? inserted : node,
      b: before ? node : inserted,
    };
  }
  return { ...node, a: splitLeaf(node.a, leafId, inserted, dir, before), b: splitLeaf(node.b, leafId, inserted, dir, before) };
}

/** Remove a leaf; its sibling takes the parent's place. Returns null when the tree becomes empty. */
export function removeLeaf(node: LayoutNode, leafId: string): LayoutNode | null {
  if (node.type === "leaf") return node.id === leafId ? null : node;
  const a = removeLeaf(node.a, leafId);
  const b = removeLeaf(node.b, leafId);
  if (!a) return b;
  if (!b) return a;
  if (a === node.a && b === node.b) return node;
  return { ...node, a, b };
}

const clamp = (r: number) => Math.min(0.9, Math.max(0.1, r));

export function setRatio(node: LayoutNode, splitId: string, ratio: number): LayoutNode {
  if (node.type === "leaf") return node;
  if (node.id === splitId) return { ...node, ratio: clamp(ratio) };
  return { ...node, a: setRatio(node.a, splitId, ratio), b: setRatio(node.b, splitId, ratio) };
}

/** Map every leaf (e.g. to swap session ids when re-opening a saved workspace). */
export function mapLeaves(node: LayoutNode, fn: (l: LeafNode) => LeafNode): LayoutNode {
  if (node.type === "leaf") return fn(node);
  return { ...node, a: mapLeaves(node.a, fn), b: mapLeaves(node.b, fn) };
}
