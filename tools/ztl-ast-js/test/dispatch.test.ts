import { describe, it } from "node:test";
import assert from "node:assert/strict";

import { dispatch, onNode, type DispatchTable } from "../src/dispatch.ts";
import { buildContext } from "../src/context.ts";
import {
    BlockQuote,
    POSITION_ORIGIN,
    Paragraph,
    Text,
    Wikilink,
    type AnyNode,
    type InlineNode,
} from "../src/ast.ts";
import { sampleAst } from "./fixtures/sample.ts";

function mkCtx() {
    return buildContext({
        pageSlug: "test",
        frontmatter: {},
        stage: "transform",
        env: {
            mode: "build",
            theme: null,
            vault_root: "/v",
            out_dir: null,
            verbose: false,
            at: null,
            hook_path: null,
            extension_id: "test",
        },
        buildData: {},
    }).ctx;
}

describe("dispatch", () => {
    it("routes by type", () => {
        const ast = sampleAst();
        const seen: string[] = [];
        dispatch(ast, mkCtx(), {
            Wikilink(node) {
                seen.push(`wl:${node.target}`);
            },
            BlockQuote(node) {
                seen.push(`bq:${node.children.length}`);
            },
        });
        assert.deepStrictEqual(seen, ["wl:Other page", "bq:1"]);
    });

    it("falls back to Inline / Block / _fallback", () => {
        const ast = sampleAst();
        let inlineCount = 0;
        let blockCount = 0;
        let fallbackCount = 0;
        dispatch(ast, mkCtx(), {
            Inline() {
                inlineCount++;
            },
            Block() {
                blockCount++;
            },
            _fallback() {
                fallbackCount++;
            },
        });
        // Every Text/Wikilink/Emphasis counts as inline, every
        // Heading/Paragraph/BlockQuote/CodeBlock as block, and Document
        // itself hits _fallback.
        assert.ok(inlineCount >= 5, `inlineCount=${inlineCount}`);
        assert.ok(blockCount >= 4, `blockCount=${blockCount}`);
        assert.strictEqual(fallbackCount, 1);
    });

    it("replaces a node when handler returns one", () => {
        const ast = sampleAst();
        dispatch(ast, mkCtx(), {
            Wikilink(node) {
                const replacement: InlineNode = {
                    type: "Text",
                    position: node.position,
                    text: `link:${node.target}`,
                };
                return replacement;
            },
        });
        const paragraph = ast.children.find((c) => c.type === "Paragraph")!;
        assert.ok((paragraph as { children: AnyNode[] }).children.some((n) => n.type === "Text" && /^link:/.test(n.text)));
    });

    it("sequence-level Inlines handler can mutate child array", () => {
        const ast = sampleAst();
        dispatch(ast, mkCtx(), {
            Inlines(children, parent) {
                if (parent.type === "Heading") {
                    return [{ type: "Text", position: { ...POSITION_ORIGIN }, text: "REWRITTEN" }];
                }
                return undefined;
            },
        });
        const heading = ast.children.find((c) => c.type === "Heading")!;
        assert.strictEqual((heading as { children: AnyNode[] }).children.length, 1);
        assert.strictEqual(((heading as { children: AnyNode[] }).children[0] as { text: string }).text, "REWRITTEN");
    });

    it("type handler takes precedence over Inline / _fallback", () => {
        const ast = sampleAst();
        let textCount = 0;
        let inlineCount = 0;
        dispatch(ast, mkCtx(), {
            Text() {
                textCount++;
            },
            Inline() {
                inlineCount++;
            },
        });
        assert.strictEqual(textCount, 5);
        // Wikilink + Emphasis → 2 unmatched inlines.
        assert.strictEqual(inlineCount, 2);
    });
});

describe("onNode", () => {
    it("returns a [type, handler] tuple composable via Object.fromEntries", () => {
        const entry = onNode(Wikilink, () => undefined);
        assert.strictEqual(entry[0], "Wikilink");
        assert.strictEqual(typeof entry[1], "function");
        const table = Object.fromEntries([entry]) as DispatchTable;
        assert.ok(typeof table.Wikilink === "function");
    });

    it("chains onto an existing table", () => {
        const table: DispatchTable = {};
        onNode(table, BlockQuote, () => undefined);
        onNode(table, Paragraph, () => undefined);
        assert.ok(table.BlockQuote);
        assert.ok(table.Paragraph);
    });
});
