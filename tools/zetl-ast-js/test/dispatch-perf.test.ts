// Dispatch overhead benchmark (IMPL-032-033 task-node-dispatch).
//
// Plan acceptance: "dispatch overhead per node <50µs".
//
// We walk a big synthetic AST through `dispatch` with a realistic
// table (per-type + Block / Inline / Blocks / Inlines) and divide the
// total wall time by the node count. The budget is 50µs per visited
// node on CI hardware — trivially satisfied by v8 but still worth
// pinning as a regression gate.

import { describe, it } from "node:test";
import assert from "node:assert/strict";

import { dispatch } from "../src/dispatch.ts";
import { buildContext } from "../src/context.ts";
import { walk } from "../src/walk.ts";
import { AST_VERSION, POSITION_ORIGIN, type DocumentNode } from "../src/ast.ts";

const BUDGET_US_PER_NODE = 50;

function pos() {
    return { ...POSITION_ORIGIN };
}

function bigAst(paragraphs: number): DocumentNode {
    const children: DocumentNode["children"] = [];
    for (let i = 0; i < paragraphs; i++) {
        children.push({
            type: "Paragraph",
            position: pos(),
            children: [
                { type: "Text", position: pos(), text: `lead ${i} ` },
                {
                    type: "Wikilink",
                    position: pos(),
                    target: `Page ${i}`,
                    alias: null,
                    heading: null,
                    block_id: null,
                },
                {
                    type: "Emphasis",
                    position: pos(),
                    children: [{ type: "Text", position: pos(), text: "em" }],
                },
                { type: "Text", position: pos(), text: " tail" },
            ],
        });
    }
    return {
        type: "Document",
        ast_version: AST_VERSION,
        position: pos(),
        frontmatter: {},
        children,
    };
}

function mkCtx() {
    return buildContext({
        pageSlug: "bench",
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
            extension_id: "bench",
        },
        buildData: {},
    }).ctx;
}

describe("dispatch overhead", () => {
    it(`stays under ${BUDGET_US_PER_NODE}µs per node`, () => {
        if (process.env.ZETL_SKIP_BENCH) return;

        const ast = bigAst(2000);
        const nodeCount = [...walk(ast)].length;
        assert.ok(nodeCount > 5000, `nodeCount=${nodeCount}`);

        const ctx = mkCtx();
        const table = {
            Wikilink: () => undefined,
            Text: () => undefined,
            BlockQuote: () => undefined,
            Block: () => undefined,
            Inline: () => undefined,
            Blocks: () => undefined,
            Inlines: () => undefined,
        } as const;

        // Warm-up so v8 has a hot-path to measure.
        dispatch(ast, ctx, table);

        const samples: number[] = [];
        for (let run = 0; run < 3; run++) {
            const fresh = bigAst(2000);
            const t0 = performance.now();
            dispatch(fresh, ctx, table);
            samples.push(((performance.now() - t0) * 1000) / nodeCount);
        }
        samples.sort((a, b) => a - b);
        const medianUs = samples[1]!;
        assert.ok(
            medianUs < BUDGET_US_PER_NODE,
            `dispatch overhead ${medianUs.toFixed(3)}µs/node exceeds ` +
                `${BUDGET_US_PER_NODE}µs/node budget (samples=${JSON.stringify(samples)})`,
        );
    });
});
