// Declarative node-type dispatch (REQ-3218).
//
// Wraps `walk` with a table keyed by node type. Reserved keys:
//   - `Block`      — any block not otherwise covered
//   - `Inline`     — any inline not otherwise covered
//   - `_fallback`  — any node not otherwise covered
//   - `Blocks`     — sequence-level; receives each block-children array
//   - `Inlines`    — sequence-level; receives each inline-children array
//
// Visit order is depth-first pre-order: per-node handler fires on the
// node, then the sequence handler fires on its children array (if any),
// then each child is visited in turn. Handlers may mutate in place and
// return `undefined`/`null` to keep the existing node, or return a
// replacement node that will be swapped into the parent's children.

import type { AnyNode, BlockNode, InlineNode, ListItemNode, NodeOf, NodeType } from "./ast.ts";
import { isBlock, isInline } from "./ast.ts";
import type { Context } from "./context.ts";

export type NodeHandler<T extends AnyNode = AnyNode> = (
    node: T,
    ctx: Context,
) => AnyNode | undefined | null | void;

export type SequenceHandler<T extends AnyNode = AnyNode> = (
    children: T[],
    parent: AnyNode,
    ctx: Context,
) => T[] | undefined | void;

export type DispatchTable = {
    [K in NodeType]?: NodeHandler<NodeOf<K>>;
} & {
    Block?: NodeHandler<BlockNode>;
    Inline?: NodeHandler<InlineNode>;
    _fallback?: NodeHandler<AnyNode>;
    Blocks?: SequenceHandler<BlockNode | ListItemNode>;
    Inlines?: SequenceHandler<InlineNode>;
};

/**
 * Walk `root` and invoke handlers from `table` against each matching
 * node. Returns `root` (possibly a replacement produced by a root-level
 * handler).
 */
export function dispatch<T extends AnyNode>(root: T, ctx: Context, table: DispatchTable): T {
    return visit(root, ctx, table) as T;
}

function visit(node: AnyNode, ctx: Context, table: DispatchTable): AnyNode {
    // Per-node handler first — may replace the subtree before descent.
    const handler = resolveHandler(node, table);
    if (handler) {
        const r = handler(node as never, ctx);
        if (r != null && r !== (node as AnyNode)) node = r;
    }

    // Sequence-level handler on children (pre-descent).
    runSequence(node, ctx, table);

    // Descend — replacements bubble up via `visit()`'s return value.
    const arr = mutableChildren(node);
    if (arr) {
        for (let i = 0; i < arr.length; i++) {
            arr[i] = visit(arr[i]!, ctx, table) as never;
        }
    }
    return node;
}

function resolveHandler(node: AnyNode, table: DispatchTable): NodeHandler | undefined {
    const direct = (table as Record<string, NodeHandler | undefined>)[node.type];
    if (direct) return direct;
    if (isBlock(node) && table.Block) return table.Block as NodeHandler;
    if (isInline(node) && table.Inline) return table.Inline as NodeHandler;
    return table._fallback;
}

function runSequence(node: AnyNode, ctx: Context, table: DispatchTable): void {
    const blocksHandler = table.Blocks;
    const inlinesHandler = table.Inlines;
    if (!blocksHandler && !inlinesHandler) return;

    const kind = containerKind(node);
    if (kind === "block" && blocksHandler) {
        const arr = (node as { children: (BlockNode | ListItemNode)[] }).children;
        const next = blocksHandler(arr, node, ctx);
        if (next !== undefined) {
            (node as { children: (BlockNode | ListItemNode)[] }).children = next;
        }
    } else if (kind === "inline" && inlinesHandler) {
        const arr = (node as { children: InlineNode[] }).children;
        const next = inlinesHandler(arr, node, ctx);
        if (next !== undefined) {
            (node as { children: InlineNode[] }).children = next;
        }
    }
}

function containerKind(node: AnyNode): "block" | "inline" | null {
    switch (node.type) {
        case "Document":
        case "BlockQuote":
        case "ListItem":
        case "List":
            return "block";
        case "Paragraph":
        case "Heading":
        case "Emphasis":
        case "Strong":
        case "Link":
        case "Image":
            return "inline";
        default:
            return null;
    }
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
            return (node as { children: AnyNode[] }).children;
        default:
            return null;
    }
}

// ── onNode factory (REQ-3218) ───────────────────────────────────────────────

/**
 * Register a handler for a single node type. Python's helper uses
 * `@on_node("Wikilink")` as a decorator; JS has no decorator language
 * feature here, so this returns a `[type, handler]` tuple that's cheap
 * to compose into a dispatch table.
 *
 * ```ts
 * const links = onNode(Wikilink, (node, ctx) => { ... });
 * const notes = onNode(BlockQuote, (node, ctx) => { ... });
 * dispatch(ast, ctx, Object.fromEntries([links, notes]));
 * ```
 *
 * A three-argument form extends an existing table in-place:
 * ```ts
 * const table: DispatchTable = {};
 * onNode(table, Wikilink, handleLinks);
 * onNode(table, BlockQuote, handleQuotes);
 * ```
 */
export function onNode<T extends NodeType>(
    type: T,
    handler: NodeHandler<NodeOf<T>>,
): readonly [T, NodeHandler<NodeOf<T>>];
export function onNode<T extends NodeType>(
    table: DispatchTable,
    type: T,
    handler: NodeHandler<NodeOf<T>>,
): DispatchTable;
export function onNode(
    a: NodeType | DispatchTable,
    b: NodeType | NodeHandler,
    c?: NodeHandler,
): unknown {
    if (typeof a === "string") {
        return [a, b as NodeHandler] as const;
    }
    const table = a as DispatchTable;
    const type = b as NodeType;
    (table as Record<string, NodeHandler>)[type] = c as NodeHandler;
    return table;
}
