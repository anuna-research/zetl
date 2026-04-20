// Typed mirror of tools/zetl-ast-schema-v1.json (v1.0).
//
// The schema is normative (SPEC-032 REQ-3202 / CON-3202). Any disagreement
// between these types and the schema is a bug in this file. Validation
// (via `validate()`) is structural, not draft-2020-12 JSON Schema.

/** Major schema version this helper is pinned to (REQ-3210). */
export const AST_MAJOR = 1 as const;

/** Exact two-component schema version emitted by zetl at `Document.ast_version`. */
export const AST_VERSION = "1.0" as const;

export interface Position {
    start_line: number;
    start_col: number;
    end_line: number;
    end_col: number;
}

export const POSITION_ORIGIN: Position = Object.freeze({
    start_line: 1,
    start_col: 1,
    end_line: 1,
    end_col: 1,
});

export type Frontmatter = Record<string, unknown>;

// ── Node-name constants ─────────────────────────────────────────────────────
//
// Exported as value AND type. CON-3210 samples use them as values
// (`walk(ast, { type: BlockQuote })`); the compiler also sees them as
// the string-literal discriminator of their corresponding interface.

export const Document = "Document" as const;
export const Heading = "Heading" as const;
export const Paragraph = "Paragraph" as const;
export const BlockQuote = "BlockQuote" as const;
export const List = "List" as const;
export const ListItem = "ListItem" as const;
export const CodeBlock = "CodeBlock" as const;
export const ThematicBreak = "ThematicBreak" as const;
export const HtmlBlock = "HtmlBlock" as const;
export const SplBlock = "SplBlock" as const;
export const Embed = "Embed" as const;
export const Text = "Text" as const;
export const Emphasis = "Emphasis" as const;
export const Strong = "Strong" as const;
export const Code = "Code" as const;
export const Link = "Link" as const;
export const Image = "Image" as const;
export const LineBreak = "LineBreak" as const;
export const SoftBreak = "SoftBreak" as const;
export const HtmlInline = "HtmlInline" as const;
export const Wikilink = "Wikilink" as const;

export type BlockType =
    | typeof Heading
    | typeof Paragraph
    | typeof BlockQuote
    | typeof List
    | typeof CodeBlock
    | typeof ThematicBreak
    | typeof HtmlBlock
    | typeof SplBlock
    | typeof Embed;

export type InlineType =
    | typeof Text
    | typeof Emphasis
    | typeof Strong
    | typeof Code
    | typeof Link
    | typeof Image
    | typeof LineBreak
    | typeof SoftBreak
    | typeof HtmlInline
    | typeof Wikilink;

export type NodeType = typeof Document | BlockType | InlineType | typeof ListItem;

// ── Node shapes ─────────────────────────────────────────────────────────────

export interface DocumentNode {
    type: typeof Document;
    ast_version: string;
    position: Position;
    frontmatter?: Frontmatter;
    children: BlockNode[];
}

export interface HeadingNode {
    type: typeof Heading;
    position: Position;
    level: 1 | 2 | 3 | 4 | 5 | 6;
    children: InlineNode[];
}

export interface ParagraphNode {
    type: typeof Paragraph;
    position: Position;
    children: InlineNode[];
}

export interface BlockQuoteNode {
    type: typeof BlockQuote;
    position: Position;
    children: BlockNode[];
}

export interface ListNode {
    type: typeof List;
    position: Position;
    ordered: boolean;
    tight: boolean;
    start: number | null;
    children: ListItemNode[];
}

export interface ListItemNode {
    type: typeof ListItem;
    position: Position;
    children: BlockNode[];
}

export interface CodeBlockNode {
    type: typeof CodeBlock;
    position: Position;
    fenced: boolean;
    lang: string | null;
    info: string | null;
    text: string;
}

export interface ThematicBreakNode {
    type: typeof ThematicBreak;
    position: Position;
}

export interface HtmlBlockNode {
    type: typeof HtmlBlock;
    position: Position;
    text: string;
}

export interface SplBlockNode {
    type: typeof SplBlock;
    position: Position;
    info?: string | null;
    text: string;
}

export interface EmbedNode {
    type: typeof Embed;
    position: Position;
    target: string;
    heading: string | null;
    block_id: string | null;
}

export interface TextNode {
    type: typeof Text;
    position: Position;
    text: string;
}

export interface EmphasisNode {
    type: typeof Emphasis;
    position: Position;
    children: InlineNode[];
}

export interface StrongNode {
    type: typeof Strong;
    position: Position;
    children: InlineNode[];
}

export interface CodeNode {
    type: typeof Code;
    position: Position;
    text: string;
}

export interface LinkNode {
    type: typeof Link;
    position: Position;
    url: string;
    title: string | null;
    children: InlineNode[];
}

export interface ImageNode {
    type: typeof Image;
    position: Position;
    url: string;
    title: string | null;
    alt: string;
    children: InlineNode[];
}

export interface LineBreakNode {
    type: typeof LineBreak;
    position: Position;
}

export interface SoftBreakNode {
    type: typeof SoftBreak;
    position: Position;
}

export interface HtmlInlineNode {
    type: typeof HtmlInline;
    position: Position;
    text: string;
}

export interface WikilinkNode {
    type: typeof Wikilink;
    position: Position;
    target: string;
    alias: string | null;
    heading: string | null;
    block_id: string | null;
}

export type BlockNode =
    | HeadingNode
    | ParagraphNode
    | BlockQuoteNode
    | ListNode
    | CodeBlockNode
    | ThematicBreakNode
    | HtmlBlockNode
    | SplBlockNode
    | EmbedNode;

export type InlineNode =
    | TextNode
    | EmphasisNode
    | StrongNode
    | CodeNode
    | LinkNode
    | ImageNode
    | LineBreakNode
    | SoftBreakNode
    | HtmlInlineNode
    | WikilinkNode;

export type AnyNode = DocumentNode | BlockNode | InlineNode | ListItemNode;

/** Map node-name constant to node shape (for generic `walk(ast, {type: T})`). */
export type NodeOf<T extends NodeType> = Extract<AnyNode, { type: T }>;

// ── Predicates ──────────────────────────────────────────────────────────────

const BLOCK_TYPES = new Set<string>([
    Heading,
    Paragraph,
    BlockQuote,
    List,
    CodeBlock,
    ThematicBreak,
    HtmlBlock,
    SplBlock,
    Embed,
]);
const INLINE_TYPES = new Set<string>([
    Text,
    Emphasis,
    Strong,
    Code,
    Link,
    Image,
    LineBreak,
    SoftBreak,
    HtmlInline,
    Wikilink,
]);

export function isBlock(node: AnyNode): node is BlockNode {
    return BLOCK_TYPES.has(node.type);
}

export function isInline(node: AnyNode): node is InlineNode {
    return INLINE_TYPES.has(node.type);
}

export function isDocument(node: AnyNode): node is DocumentNode {
    return node.type === Document;
}

/** All node-name constants known to this helper version. Grep-friendly. */
export const KNOWN_NODE_TYPES: ReadonlyArray<NodeType> = Object.freeze([
    Document,
    Heading,
    Paragraph,
    BlockQuote,
    List,
    ListItem,
    CodeBlock,
    ThematicBreak,
    HtmlBlock,
    SplBlock,
    Embed,
    Text,
    Emphasis,
    Strong,
    Code,
    Link,
    Image,
    LineBreak,
    SoftBreak,
    HtmlInline,
    Wikilink,
]);

// ── Validation ──────────────────────────────────────────────────────────────

export class SchemaError extends Error {
    readonly path: string;
    constructor(message: string, path: string) {
        super(`${path}: ${message}`);
        this.name = "SchemaError";
        this.path = path;
    }
}

/**
 * Structural validation against the v1 schema. Intended as a fast-fail
 * guard at the protocol boundary — full JSON Schema Draft 2020-12
 * validation is out of scope (the Rust side owns that).
 *
 * Returns the same value (narrowed to `DocumentNode`) on success;
 * throws [`SchemaError`] on any shape mismatch.
 */
export function validateDocument(value: unknown): DocumentNode {
    if (!isObject(value)) throw new SchemaError("expected object", "$");
    const v = value as Record<string, unknown>;
    if (v.type !== Document) throw new SchemaError(`expected type="Document", got ${JSON.stringify(v.type)}`, "$.type");
    if (typeof v.ast_version !== "string" || !/^\d+\.\d+$/.test(v.ast_version)) {
        throw new SchemaError(`expected ast_version matching \\d+\\.\\d+, got ${JSON.stringify(v.ast_version)}`, "$.ast_version");
    }
    const major = Number.parseInt(v.ast_version.split(".")[0] ?? "", 10);
    if (major !== AST_MAJOR) {
        throw new SchemaError(
            `ast_version major ${major} does not match this helper's pinned major ${AST_MAJOR}. ` +
                `Upgrade zetl-ast-js, or use the version of the helper pinned to major ${major}.`,
            "$.ast_version",
        );
    }
    validatePosition(v.position, "$.position");
    if ("frontmatter" in v && v.frontmatter !== undefined && !isPlainObject(v.frontmatter)) {
        throw new SchemaError("frontmatter must be an object when present", "$.frontmatter");
    }
    if (!Array.isArray(v.children)) throw new SchemaError("children must be an array", "$.children");
    v.children.forEach((child, i) => validateBlock(child, `$.children[${i}]`));
    return value as DocumentNode;
}

function validatePosition(value: unknown, path: string): void {
    if (!isObject(value)) throw new SchemaError("expected object", path);
    const p = value as Record<string, unknown>;
    for (const k of ["start_line", "start_col", "end_line", "end_col"] as const) {
        if (typeof p[k] !== "number" || !Number.isInteger(p[k]) || (p[k] as number) < 1) {
            throw new SchemaError(`${k} must be an integer ≥ 1`, `${path}.${k}`);
        }
    }
}

function validateBlock(value: unknown, path: string): void {
    if (!isObject(value)) throw new SchemaError("expected object", path);
    const v = value as Record<string, unknown>;
    if (typeof v.type !== "string") throw new SchemaError("missing type", `${path}.type`);
    if (!BLOCK_TYPES.has(v.type)) {
        throw new SchemaError(`expected a block type, got ${JSON.stringify(v.type)}`, `${path}.type`);
    }
    validatePosition(v.position, `${path}.position`);
    // Deep validation of children is intentionally shallow — we catch
    // gross shape errors, not semantic ones. The v1 schema is additive
    // within major 1 per NFR-3206, so we tolerate unknown fields here
    // rather than rejecting forward-compatible extensions.
}

function isObject(v: unknown): v is object {
    return v !== null && typeof v === "object";
}

function isPlainObject(v: unknown): v is Record<string, unknown> {
    return isObject(v) && !Array.isArray(v);
}
