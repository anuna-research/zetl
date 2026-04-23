// run() — hook entry point.
//
// Handles both persistent-mode (handshake + line-delimited JSON loop —
// the path ztl takes) and one-shot mode (stdin→stdout→exit, for
// filter-style integrations). The same transform_fn works in both;
// the helper library chooses the mode from environment and stdin
// shape.

import type { Writable } from "node:stream";
import { AST_MAJOR, AST_VERSION, validateDocument, type DocumentNode } from "./ast.ts";
import { buildContext, type BuildEnv, type Context, type ContextWrites } from "./context.ts";
import {
    type HookMessage,
    type HostMessage,
    type InitContext,
    type ResultMessage,
    ProtocolError,
    handshakeLine,
    parseHostMessage,
    readLines,
    serializeHookMessage,
} from "./protocol.ts";

/**
 * A hook's transform function. Receives the parsed payload (typed
 * `DocumentNode` on the `transform` stage; raw `string` on pre-parse /
 * post-render stages as appropriate) plus a [`Context`]. Return a
 * transformed payload, or mutate in place and return it.
 *
 * May be async — the host awaits the returned promise.
 */
export type TransformFn<In = unknown, Out = In> = (input: In, ctx: Context) => Out | Promise<Out>;

export interface RunOptions {
    /** Hook id for the handshake — defaults to `$ztl_EXTENSION_ID` or `"ztl-hook"`. */
    hookId?: string;
    /** Free-form version emitted in the handshake. */
    version?: string;
    /** When set, payloads passed to `transform_fn` are first validated against the v1 AST schema. Defaults to `true`. */
    validateAst?: boolean;
    /** Override stdio for testing. Defaults to `process.stdin`/`process.stdout`. */
    stdin?: NodeJS.ReadableStream;
    stdout?: Writable;
    stderr?: Writable;
    /** Called if the main loop crashes before handshake. Useful for tests. */
    onFatal?: (error: Error) => void;
    /** Exit the process on fatal error. Defaults to `true` (skipped when `stdin` is overridden — tests don't want to die). */
    exitOnFatal?: boolean;
}

/**
 * Persistent-mode run loop. Writes the handshake to stdout, then reads
 * line-delimited JSON from stdin and replies with a single JSON line per
 * message. Handles `init` / `run` / `finalise` / `shutdown` and exits
 * cleanly on stdin close.
 *
 * Errors inside `transform_fn` become `{"type":"error"}` responses —
 * ztl's REQ-3207 policy decides whether to continue.
 */
export async function run<In = DocumentNode, Out = In>(
    transform: TransformFn<In, Out>,
    options: RunOptions = {},
): Promise<void> {
    const stdin = options.stdin ?? process.stdin;
    const stdout = options.stdout ?? process.stdout;
    const validate = options.validateAst !== false;
    const hookId = options.hookId ?? process.env["ztl_EXTENSION_ID"] ?? "ztl-hook";
    const version = options.version ?? "0.0.0";
    const exitOnFatal = options.exitOnFatal ?? (options.stdin === undefined);

    try {
        stdout.write(handshakeLine(hookId, version));

        // Latched build env — updated on `init`, carried forward into
        // every subsequent `run` / `finalise`.
        let env: BuildEnv = defaultEnvFromProcess(hookId);
        let stage = process.env["ztl_STAGE"] ?? "transform";

        for await (const line of readLines(stdin)) {
            let msg: HostMessage;
            try {
                msg = parseHostMessage(line);
            } catch (e) {
                stdout.write(serializeHookMessage({ type: "error", reason: "protocol_error", detail: (e as Error).message }));
                continue;
            }

            if (msg.type === "shutdown") {
                break;
            }

            if (msg.type === "init") {
                try {
                    checkAstMajor(msg.ast_schema_version);
                } catch (e) {
                    stdout.write(serializeHookMessage({ type: "error", reason: "ast_version_mismatch", detail: (e as Error).message }));
                    break;
                }
                stage = msg.stage || stage;
                env = envFromInit(msg.ctx, hookId, env);
                stdout.write(serializeHookMessage({ type: "result", payload: null, diagnostics: [], template_vars: {}, build_data: {} }));
                continue;
            }

            if (msg.type === "finalise") {
                stdout.write(serializeHookMessage({ type: "result", payload: null, diagnostics: [], template_vars: {}, build_data: {} }));
                continue;
            }

            // msg.type === "run"
            const { ctx, writes } = buildContext({
                pageSlug: msg.page_slug,
                frontmatter: msg.frontmatter,
                stage,
                env: { ...env, extension_id: hookId },
                buildData: undefined,
            });

            let out: unknown;
            try {
                const input = validate && stage === "transform" ? validateDocument(msg.payload) : (msg.payload as In);
                out = await transform(input as In, ctx);
            } catch (e) {
                stdout.write(
                    serializeHookMessage({
                        type: "error",
                        reason: "hook_exception",
                        detail: errorDetail(e),
                    }),
                );
                continue;
            }

            stdout.write(serializeHookMessage(resultFromWrites(out, writes)));
        }
    } catch (e) {
        options.onFatal?.(e as Error);
        (options.stderr ?? process.stderr).write(`ztl-ast-js fatal: ${(e as Error).message}\n`);
        if (exitOnFatal) process.exit(1);
    }
}

/**
 * One-shot mode — read the whole stdin as a single JSON document, apply
 * `transform`, write the result to stdout, and exit. For filter-style
 * integrations where a wrapper invokes the hook per page. No handshake
 * is emitted.
 */
export async function runOneShot<In = DocumentNode, Out = In>(
    transform: TransformFn<In, Out>,
    options: RunOptions = {},
): Promise<void> {
    const stdin = options.stdin ?? process.stdin;
    const stdout = options.stdout ?? process.stdout;
    const validate = options.validateAst !== false;
    const hookId = options.hookId ?? process.env["ztl_EXTENSION_ID"] ?? "ztl-hook";

    const raw = await readAll(stdin);
    const parsed = JSON.parse(raw);
    const env = defaultEnvFromProcess(hookId);
    const { ctx, writes } = buildContext({
        pageSlug: process.env["ztl_PAGE_SLUG"] ?? "",
        frontmatter: {},
        stage: process.env["ztl_STAGE"] ?? "transform",
        env: { ...env, extension_id: hookId },
        buildData: undefined,
    });
    const input = validate ? validateDocument(parsed) : parsed;
    const out = await transform(input as In, ctx);
    void writes; // diagnostics/template_vars are dropped in one-shot mode
    stdout.write(JSON.stringify(out));
}

// ── Internals ───────────────────────────────────────────────────────────────

function checkAstMajor(schemaVersion: string): void {
    if (!schemaVersion) return;
    const parts = schemaVersion.split(".");
    const major = Number.parseInt(parts[0] ?? "", 10);
    if (!Number.isFinite(major)) {
        throw new ProtocolError(`malformed ast_schema_version ${JSON.stringify(schemaVersion)}`, "parse");
    }
    if (major !== AST_MAJOR) {
        throw new ProtocolError(
            `ztl advertised ast_schema_version=${schemaVersion}; this hook speaks major ${AST_MAJOR} (pinned to ${AST_VERSION}).`,
            "ast_major_mismatch",
        );
    }
}

function defaultEnvFromProcess(hookId: string): BuildEnv {
    return {
        mode: (process.env["ztl_MODE"] as "build" | "serve" | undefined) ?? "build",
        theme: process.env["ztl_THEME"] ?? null,
        vault_root: process.env["ztl_VAULT_ROOT"] ?? "",
        out_dir: process.env["ztl_OUT_DIR"] ?? null,
        verbose: process.env["ztl_VERBOSE"] === "true",
        at: process.env["ztl_AT"] ?? null,
        hook_path: process.env["ztl_HOOK_PATH"] ?? null,
        extension_id: process.env["ztl_EXTENSION_ID"] ?? hookId,
    };
}

function envFromInit(ctx: InitContext, hookId: string, fallback: BuildEnv): BuildEnv {
    return {
        mode: (ctx.mode as "build" | "serve") ?? fallback.mode,
        theme: ctx.theme ?? fallback.theme,
        vault_root: ctx.vault_root ?? fallback.vault_root,
        out_dir: ctx.out_dir ?? fallback.out_dir,
        verbose: ctx.verbose ?? fallback.verbose,
        at: ctx.at ?? fallback.at,
        hook_path: ctx.hook_path ?? fallback.hook_path,
        extension_id: ctx.extension_id ?? hookId,
    };
}

function resultFromWrites(payload: unknown, writes: ContextWrites): ResultMessage {
    const msg: ResultMessage = { type: "result", payload };
    if (writes.diagnostics.length > 0) msg.diagnostics = writes.diagnostics;
    if (Object.keys(writes.templateVars).length > 0 || Object.keys(writes.vaultTemplateVars).length > 0) {
        msg.template_vars = { ...writes.templateVars };
        if (Object.keys(writes.vaultTemplateVars).length > 0) {
            (msg.template_vars as Record<string, unknown>)["__vault__"] = writes.vaultTemplateVars;
        }
    }
    if (Object.keys(writes.buildData).length > 0) msg.build_data = writes.buildData;
    return msg;
}

function errorDetail(e: unknown): string {
    if (e instanceof Error) {
        return e.stack ?? e.message;
    }
    try {
        return JSON.stringify(e);
    } catch {
        return String(e);
    }
}

async function readAll(stream: NodeJS.ReadableStream): Promise<string> {
    stream.setEncoding?.("utf8");
    let out = "";
    for await (const chunk of stream as AsyncIterable<string>) {
        out += chunk;
    }
    return out;
}
