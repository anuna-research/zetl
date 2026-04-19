// walk() — generator-based traversal.
//
// CON-3210 sample:
//   for (const node of walk(ast, { type: BlockQuote })) { ... }
//
// The low-level API; `dispatch()` is the recommended 80% path (REQ-3218).

import type { AnyNode, BlockNode, DocumentNode, InlineNode, ListItemNode, NodeOf, NodeType } from "./ast.ts";

export interface WalkOptions<T extends NodeType = NodeType> {
    /** Restrict the generator to nodes whose `type` matches. */
    type?: T | readonly T[];
}

/**
 * Depth-first pre-order traversal. Visits the root itself, then each
 * child in order, recursing into children-bearing nodes. Skips inside
 * `CodeBlock`, `HtmlBlock`, `HtmlInline`, `SplBlock`, etc. since those
 * have no child nodes.
 */
export function walk<T extends NodeType = NodeType>(
    root: AnyNode,
    opts?: WalkOptions<T>,
): Generator<NodeOf<T>> {
    const filter = opts?.type;
    const matcher = buildMatcher(filter);
    return walkImpl(root, matcher) as Generator<NodeOf<T>>;
}

function* walkImpl(node: AnyNode, matcher: (t: string) => boolean): Generator<AnyNode> {
    if (matcher(node.type)) yield node;
    const children = childrenOf(node);
    for (const child of children) {
        yield* walkImpl(child, matcher);
    }
}

function buildMatcher(filter: NodeType | readonly NodeType[] | undefined): (t: string) => boolean {
    if (filter === undefined) return () => true;
    if (Array.isArray(filter)) {
        const set = new Set<string>(filter as readonly string[]);
        return (t) => set.has(t);
    }
    return (t) => t === (filter as string);
}

/**
 * Child list for a node in document order. Returns `[]` for leaf nodes
 * (Text, Code, ThematicBreak, LineBreak, Embed, Wikilink, ...).
 */
export function childrenOf(node: AnyNode): ReadonlyArray<AnyNode> {
    switch (node.type) {
        case "Document":
        case "Paragraph":
        case "Heading":
        case "Emphasis":
        case "Strong":
        case "Link":
        case "Image":
            return (node as DocumentNode | ParagraphLike).children;
        case "BlockQuote":
        case "ListItem":
            return (node as BlockQuoteLike).children;
        case "List":
            return (node as ListLike).children;
        default:
            return [];
    }
}

// Local structural aliases keep the switch readable without depending
// on the full discriminated union.
type ParagraphLike = { children: InlineNode[] };
type BlockQuoteLike = { children: BlockNode[] };
type ListLike = { children: ListItemNode[] };

/**
 * In-place replacement of every node for which `predicate` returns a
 * new node. The input tree is mutated; `null`/`undefined` returns leave
 * the node unchanged. Runs one pass (does not re-visit substitutes).
 *
 * Exposed as a building-block for dispatch handlers that want to swap
 * a subtree without walking manually.
 */
export function mapNodes(
    root: AnyNode,
    replacer: (node: AnyNode) => AnyNode | null | undefined,
): AnyNode {
    const children = mutableChildren(root);
    if (children) {
        for (let i = 0; i < children.length; i++) {
            const curr = children[i]!;
            const next = replacer(curr);
            if (next != null && next !== curr) {
                children[i] = next as never;
                // Descend into the replacement, not the original.
                mapNodes(next, replacer);
            } else {
                mapNodes(curr, replacer);
            }
        }
    }
    return root;
}

function mutableChildren(node: AnyNode): AnyNode[] | null {
    switch (node.type) {
        case "Document":
        case "Paragraph":
        case "Heading":
        case "Emphasis":
        case "Strong":
        case "Link":
        case "Image":
        case "BlockQuote":
        case "ListItem":
        case "List":
            // Narrowed at callsite — the schema guarantees `children` exists.
            return (node as { children: AnyNode[] }).children;
        default:
            return null;
    }
}
