import { describe, it } from "node:test";
import assert from "node:assert/strict";

import { walk, childrenOf, mapNodes } from "../src/walk.ts";
import { BlockQuote, Paragraph, Text, Wikilink, validateDocument } from "../src/ast.ts";
import { sampleAst } from "./fixtures/sample.ts";

describe("walk", () => {
    it("yields every node in depth-first pre-order", () => {
        const ast = sampleAst();
        const types = [...walk(ast)].map((n) => n.type);
        assert.deepStrictEqual(types.slice(0, 6), [
            "Document",
            "Heading",
            "Text",
            "Text",
            "Paragraph",
            "Text",
        ]);
        assert.ok(types.includes("Wikilink"));
        assert.ok(types.includes("CodeBlock"));
    });

    it("filters by single type", () => {
        const ast = sampleAst();
        const texts = [...walk(ast, { type: Text })];
        assert.strictEqual(texts.length, 5);
        assert.ok(texts.every((n) => n.type === "Text"));
    });

    it("filters by array of types", () => {
        const ast = sampleAst();
        const matches = [...walk(ast, { type: [Paragraph, BlockQuote] })];
        assert.deepStrictEqual(
            matches.map((n) => n.type),
            ["Paragraph", "BlockQuote", "Paragraph"],
        );
    });

    it("walks into Wikilink target and finds zero descendants", () => {
        const ast = sampleAst();
        const wikilinks = [...walk(ast, { type: Wikilink })];
        assert.strictEqual(wikilinks.length, 1);
        assert.strictEqual(childrenOf(wikilinks[0]!).length, 0);
    });
});

describe("mapNodes", () => {
    it("replaces a node in place and descends into the replacement", () => {
        const ast = sampleAst();
        let replaced = 0;
        mapNodes(ast, (node) => {
            if (node.type === "Wikilink" && (node as { alias: string | null }).alias === null) {
                replaced++;
                return {
                    type: "Text",
                    position: node.position,
                    text: `[[${(node as { target: string }).target}]]`,
                };
            }
            return undefined;
        });
        assert.strictEqual(replaced, 1);
        const wl = [...walk(ast, { type: Wikilink })];
        assert.strictEqual(wl.length, 0);
    });
});

describe("validateDocument", () => {
    it("accepts the sample AST", () => {
        const ast = sampleAst();
        assert.strictEqual(validateDocument(ast).type, "Document");
    });

    it("rejects wrong ast_version major", () => {
        const ast = sampleAst() as unknown as Record<string, unknown>;
        ast.ast_version = "2.0";
        assert.throws(() => validateDocument(ast), /major 2 does not match/);
    });

    it("rejects non-object roots", () => {
        assert.throws(() => validateDocument(null), /expected object/);
        assert.throws(() => validateDocument("hi"), /expected object/);
    });
});
