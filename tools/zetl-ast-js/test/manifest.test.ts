import { describe, it } from "node:test";
import assert from "node:assert/strict";

import {
    parseManifest,
    renderManifest,
    resolvedBefore,
    resolvedAfter,
    defaultExtensionId,
    ManifestError,
} from "../src/manifest.ts";

describe("parseManifest", () => {
    it("reads a minimal manifest", () => {
        const m = parseManifest(`
extension_id = "callouts"
optional = true
ast_type = "zetl-ext"
ast_version = "1.0"
`);
        assert.strictEqual(m.extension_id, "callouts");
        assert.strictEqual(m.optional, true);
        assert.strictEqual(m.ast_type, "zetl-ext");
        assert.strictEqual(m.ast_version, "1.0");
    });

    it("merges [ordering] and top-level aliases via resolvedBefore / resolvedAfter", () => {
        const m = parseManifest(`
before = ["mid"]

[ordering]
before = ["early"]
after  = ["late"]
`);
        assert.deepStrictEqual(resolvedBefore(m), ["early", "mid"]);
        assert.deepStrictEqual(resolvedAfter(m), ["late"]);
    });

    it("parses [contract].preserves", () => {
        const m = parseManifest(`
[contract]
preserves = ["Wikilink", "Embed", "SplBlock"]
`);
        assert.deepStrictEqual(m.contract?.preserves, ["Wikilink", "Embed", "SplBlock"]);
    });

    it("ignores comments and blank lines", () => {
        const m = parseManifest(`
# heading comment
extension_id = "x"   # trailing

# spacer

optional = false
`);
        assert.strictEqual(m.extension_id, "x");
        assert.strictEqual(m.optional, false);
    });

    it("preserves unknown top-level keys in extras", () => {
        const m = parseManifest(`
extension_id = "x"
description = "custom"
`);
        assert.deepStrictEqual(m.extras, { description: '"custom"' });
    });

    it("rejects unknown ast_type", () => {
        assert.throws(
            () => parseManifest(`ast_type = "mdx-ext"`),
            (e: unknown) => e instanceof ManifestError && /unknown ast_type/.test((e as Error).message),
        );
    });
});

describe("renderManifest", () => {
    it("round-trips the canonical shape", () => {
        const source = {
            extension_id: "callouts",
            optional: true,
            ast_type: "zetl-ext" as const,
            ast_version: "1.0",
            ordering: { before: ["admonition"], after: [] },
            contract: { preserves: ["Wikilink", "Embed", "SplBlock"] },
        };
        const rendered = renderManifest(source);
        const parsed = parseManifest(rendered);
        assert.strictEqual(parsed.extension_id, source.extension_id);
        assert.strictEqual(parsed.optional, source.optional);
        assert.strictEqual(parsed.ast_type, source.ast_type);
        assert.strictEqual(parsed.ast_version, source.ast_version);
        assert.deepStrictEqual(resolvedBefore(parsed), ["admonition"]);
        assert.deepStrictEqual(parsed.contract?.preserves, source.contract.preserves);
    });

    it("emits top-level before/after when no [ordering] table is requested", () => {
        const rendered = renderManifest({
            extension_id: "x",
            before: ["a"],
            after: ["b"],
        });
        assert.match(rendered, /^before = \["a"\]$/m);
        assert.match(rendered, /^after = \["b"\]$/m);
        assert.doesNotMatch(rendered, /\[ordering\]/);
    });
});

describe("defaultExtensionId", () => {
    it("strips leading \\d+- prefix and trailing extension", () => {
        assert.strictEqual(defaultExtensionId("20-tasks.py"), "tasks");
        assert.strictEqual(defaultExtensionId("callouts"), "callouts");
        assert.strictEqual(defaultExtensionId("001-rewrite.js"), "rewrite");
        assert.strictEqual(defaultExtensionId("no-prefix.mjs"), "no-prefix");
    });
});
