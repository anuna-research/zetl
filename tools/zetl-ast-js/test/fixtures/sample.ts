import type { DocumentNode } from "../../src/ast.ts";
import { AST_VERSION, POSITION_ORIGIN } from "../../src/ast.ts";

/**
 * A small AST exercising a cross-section of block and inline node
 * types. Intentionally not exhaustive — extra types can be added
 * per-test where needed.
 */
export function sampleAst(): DocumentNode {
    return {
        type: "Document",
        ast_version: AST_VERSION,
        position: { ...POSITION_ORIGIN },
        frontmatter: { title: "Sample", extensions: { callouts: true } },
        children: [
            {
                type: "Heading",
                position: { ...POSITION_ORIGIN },
                level: 1,
                children: [
                    { type: "Text", position: { ...POSITION_ORIGIN }, text: "Hello" },
                    { type: "Text", position: { ...POSITION_ORIGIN }, text: " world" },
                ],
            },
            {
                type: "Paragraph",
                position: { ...POSITION_ORIGIN },
                children: [
                    { type: "Text", position: { ...POSITION_ORIGIN }, text: "See " },
                    {
                        type: "Wikilink",
                        position: { ...POSITION_ORIGIN },
                        target: "Other page",
                        alias: null,
                        heading: null,
                        block_id: null,
                    },
                    { type: "Text", position: { ...POSITION_ORIGIN }, text: " for context." },
                ],
            },
            {
                type: "BlockQuote",
                position: { ...POSITION_ORIGIN },
                children: [
                    {
                        type: "Paragraph",
                        position: { ...POSITION_ORIGIN },
                        children: [
                            {
                                type: "Emphasis",
                                position: { ...POSITION_ORIGIN },
                                children: [{ type: "Text", position: { ...POSITION_ORIGIN }, text: "quoted" }],
                            },
                        ],
                    },
                ],
            },
            {
                type: "CodeBlock",
                position: { ...POSITION_ORIGIN },
                fenced: true,
                lang: "ts",
                info: "ts",
                text: "const x = 1;\n",
            },
        ],
    };
}
