import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { Readable } from "node:stream";

import { AST_MAJOR, AST_VERSION } from "../src/ast.ts";
import { handshakeLine, parseHostMessage, serializeHookMessage, ProtocolError } from "../src/protocol.ts";
import { run } from "../src/run.ts";
import { sampleAst } from "./fixtures/sample.ts";

describe("protocol", () => {
    it("handshakeLine embeds the pinned AST major", () => {
        const line = handshakeLine("my-hook", "0.1.0");
        const parsed = JSON.parse(line) as { zetl_ast: number; hook: string; version: string; ready: boolean };
        assert.strictEqual(parsed.zetl_ast, AST_MAJOR);
        assert.strictEqual(parsed.hook, "my-hook");
        assert.strictEqual(parsed.ready, true);
    });

    it("parseHostMessage handles each host message type", () => {
        const init = parseHostMessage(
            JSON.stringify({ type: "init", stage: "transform", zetl_version: "0", ast_schema_version: "1.0", ctx: {} }),
        );
        assert.strictEqual(init.type, "init");

        const runMsg = parseHostMessage(
            JSON.stringify({ type: "run", page_slug: "p", frontmatter: {}, payload: {}, deadline_ms: 100 }),
        );
        assert.strictEqual(runMsg.type, "run");

        assert.strictEqual(parseHostMessage('{"type":"finalise"}').type, "finalise");
        assert.strictEqual(parseHostMessage('{"type":"shutdown"}').type, "shutdown");
    });

    it("rejects malformed JSON and unknown types", () => {
        assert.throws(() => parseHostMessage("not-json"), ProtocolError);
        assert.throws(() => parseHostMessage('{"type":"garbage"}'), ProtocolError);
    });

    it("serializeHookMessage emits a single newline-terminated line", () => {
        const line = serializeHookMessage({ type: "result", payload: {} });
        assert.ok(line.endsWith("\n"));
        assert.strictEqual(line.split("\n").length, 2);
    });
});

// Spin up run() against in-memory streams and assert each phase.
describe("run()", () => {
    it("handshake → init → run → finalise → shutdown round-trip", async () => {
        const ast = sampleAst();
        const hostMessages = [
            JSON.stringify({ type: "init", stage: "transform", zetl_version: "0", ast_schema_version: AST_VERSION, ctx: { theme: "default" } }),
            JSON.stringify({ type: "run", page_slug: "p", frontmatter: { title: "Sample" }, payload: ast, deadline_ms: 1000 }),
            JSON.stringify({ type: "finalise" }),
            JSON.stringify({ type: "shutdown" }),
        ].join("\n") + "\n";

        const stdin = Readable.from([hostMessages]);
        const { sink, output } = mkSink();

        await run(
            (input, ctx) => {
                assert.strictEqual(ctx.pageSlug, "p");
                assert.strictEqual(ctx.env.theme, "default");
                return input;
            },
            { stdin, stdout: sink, hookId: "echo", version: "0.1.0" },
        );

        const lines = output().trim().split("\n");
        assert.strictEqual(lines.length, 4);

        const handshake = JSON.parse(lines[0]!);
        assert.strictEqual(handshake.hook, "echo");
        assert.strictEqual(handshake.ready, true);

        const initResp = JSON.parse(lines[1]!);
        assert.strictEqual(initResp.type, "result");

        const runResp = JSON.parse(lines[2]!);
        assert.strictEqual(runResp.type, "result");
        assert.strictEqual(runResp.payload.type, "Document");
        assert.strictEqual(runResp.payload.ast_version, AST_VERSION);

        const finaliseResp = JSON.parse(lines[3]!);
        assert.strictEqual(finaliseResp.type, "result");
    });

    it("emits {type:error} on ast_version major mismatch during init", async () => {
        const hostMessages =
            JSON.stringify({ type: "init", stage: "transform", zetl_version: "0", ast_schema_version: "2.0", ctx: {} }) +
            "\n";

        const stdin = Readable.from([hostMessages]);
        const { sink, output } = mkSink();

        await run((x) => x, { stdin, stdout: sink, hookId: "h", version: "0" });

        const lines = output().trim().split("\n");
        // handshake + error response
        assert.strictEqual(lines.length, 2);
        const err = JSON.parse(lines[1]!);
        assert.strictEqual(err.type, "error");
        assert.strictEqual(err.reason, "ast_version_mismatch");
    });

    it("emits {type:error} when the transform throws", async () => {
        const ast = sampleAst();
        const hostMessages =
            JSON.stringify({ type: "init", stage: "transform", zetl_version: "0", ast_schema_version: AST_VERSION, ctx: {} }) +
            "\n" +
            JSON.stringify({ type: "run", page_slug: "p", frontmatter: {}, payload: ast, deadline_ms: 100 }) +
            "\n";

        const stdin = Readable.from([hostMessages]);
        const { sink, output } = mkSink();

        await run(
            () => {
                throw new Error("boom");
            },
            { stdin, stdout: sink, hookId: "h", version: "0" },
        );

        const lines = output().trim().split("\n");
        const runResp = JSON.parse(lines[2]!);
        assert.strictEqual(runResp.type, "error");
        assert.strictEqual(runResp.reason, "hook_exception");
        assert.match(runResp.detail, /boom/);
    });

    it("emits diagnostics and build_data from ctx.emit_*", async () => {
        const ast = sampleAst();
        const hostMessages =
            JSON.stringify({ type: "init", stage: "transform", zetl_version: "0", ast_schema_version: AST_VERSION, ctx: {} }) +
            "\n" +
            JSON.stringify({ type: "run", page_slug: "p", frontmatter: {}, payload: ast, deadline_ms: 100 }) +
            "\n";

        const stdin = Readable.from([hostMessages]);
        const { sink, output } = mkSink();

        await run(
            (input, ctx) => {
                ctx.warn("deprecated syntax");
                ctx.emitBuildData({ count: 7 });
                ctx.emitTemplateVars({ summary: "ok" });
                return input;
            },
            { stdin, stdout: sink, hookId: "h", version: "0" },
        );

        const lines = output().trim().split("\n");
        const runResp = JSON.parse(lines[2]!);
        assert.deepStrictEqual(runResp.diagnostics, [
            { severity: "warn", message: "deprecated syntax", page_slug: "p" },
        ]);
        assert.deepStrictEqual(runResp.build_data, { count: 7 });
        assert.strictEqual(runResp.template_vars.summary, "ok");
    });
});

// ── helpers ────────────────────────────────────────────────────────────────

import { Writable } from "node:stream";

function mkSink(): { sink: Writable; output: () => string } {
    const chunks: Buffer[] = [];
    const sink = new Writable({
        write(chunk, _enc, cb) {
            chunks.push(Buffer.from(chunk));
            cb();
        },
    });
    return {
        sink,
        output: () => Buffer.concat(chunks).toString("utf8"),
    };
}
