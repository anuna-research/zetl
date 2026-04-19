// Wire-protocol types for persistent-mode hooks
// (SPEC-032 CON-3201 / NFR-3207 — mirror of src/hooks/persistent.rs).
//
// zetl speaks line-delimited JSON: one message per line on stdin; the
// hook replies with one line on stdout for each host message (except
// `shutdown`, which has no response). The helper keeps these types
// structural — no zod/ajv dependency — so the package stays dep-free.

import type { Diagnostic } from "./context.ts";
import { AST_MAJOR } from "./ast.ts";

/** First line written by the hook on process start (CON-3201). */
export interface HookHandshake {
    /** Must equal the major the helper is pinned to. */
    zetl_ast: number;
    /** Hook id (usually matches the manifest `extension_id`). */
    hook: string;
    /** Free-form version string for `OBS-3201` logs. */
    version: string;
    /** Must be `true` for zetl to proceed past the handshake. */
    ready: true;
}

export function handshakeLine(hook: string, version: string): string {
    const h: HookHandshake = { zetl_ast: AST_MAJOR, hook, version, ready: true };
    return JSON.stringify(h) + "\n";
}

/** REQ-3220 context block embedded in the init payload. */
export interface InitContext {
    mode?: "build" | "serve";
    theme?: string | null;
    vault_root?: string;
    out_dir?: string | null;
    verbose?: boolean;
    at?: string | null;
    hook_path?: string | null;
    extension_id?: string;
    [key: string]: unknown;
}

export type HostMessage =
    | { type: "init"; stage: string; zetl_version: string; ast_schema_version: string; ctx: InitContext }
    | { type: "run"; page_slug: string; frontmatter: Record<string, unknown>; payload: unknown; deadline_ms: number }
    | { type: "finalise" }
    | { type: "shutdown" };

export interface ResultMessage {
    type: "result";
    payload?: unknown;
    diagnostics?: Diagnostic[];
    template_vars?: Record<string, unknown>;
    build_data?: Record<string, unknown>;
}

export interface ErrorMessage {
    type: "error";
    reason: string;
    detail?: string;
}

export type HookMessage = ResultMessage | ErrorMessage;

export type ProtocolErrorKind = "parse" | "unexpected" | "ast_major_mismatch";

export class ProtocolError extends Error {
    readonly kind: ProtocolErrorKind;
    constructor(message: string, kind: ProtocolErrorKind) {
        super(message);
        this.name = "ProtocolError";
        this.kind = kind;
    }
}

export function parseHostMessage(line: string): HostMessage {
    let value: unknown;
    try {
        value = JSON.parse(line);
    } catch (e) {
        throw new ProtocolError(`malformed JSON line: ${(e as Error).message}`, "parse");
    }
    if (value === null || typeof value !== "object") {
        throw new ProtocolError(`host message must be an object, got ${typeof value}`, "parse");
    }
    const msg = value as Record<string, unknown>;
    switch (msg.type) {
        case "init": {
            const ctx = msg.ctx;
            return {
                type: "init",
                stage: String(msg.stage ?? ""),
                zetl_version: String(msg.zetl_version ?? ""),
                ast_schema_version: String(msg.ast_schema_version ?? ""),
                ctx: (ctx !== null && typeof ctx === "object" ? ctx : {}) as InitContext,
            };
        }
        case "run": {
            const fm = msg.frontmatter;
            return {
                type: "run",
                page_slug: String(msg.page_slug ?? ""),
                frontmatter:
                    fm !== null && typeof fm === "object" && !Array.isArray(fm)
                        ? (fm as Record<string, unknown>)
                        : {},
                payload: msg.payload,
                deadline_ms: typeof msg.deadline_ms === "number" ? msg.deadline_ms : 0,
            };
        }
        case "finalise":
            return { type: "finalise" };
        case "shutdown":
            return { type: "shutdown" };
        default:
            throw new ProtocolError(`unknown host message type ${JSON.stringify(msg.type)}`, "unexpected");
    }
}

export function serializeHookMessage(msg: HookMessage): string {
    return JSON.stringify(msg) + "\n";
}

/**
 * Line-delimited stream reader for `NodeJS.ReadableStream` (process.stdin).
 * Non-blocking — buffers partial lines between `data` events and yields
 * complete lines via the async iterator. A lone carriage return before
 * `\n` is tolerated.
 */
export async function* readLines(stream: NodeJS.ReadableStream): AsyncGenerator<string> {
    stream.setEncoding?.("utf8");
    let buffer = "";
    for await (const chunk of stream as AsyncIterable<string>) {
        buffer += chunk;
        let nl: number;
        while ((nl = buffer.indexOf("\n")) >= 0) {
            let line = buffer.slice(0, nl);
            buffer = buffer.slice(nl + 1);
            if (line.endsWith("\r")) line = line.slice(0, -1);
            if (line.length > 0) yield line;
        }
    }
    if (buffer.trim().length > 0) {
        const line = buffer.endsWith("\r") ? buffer.slice(0, -1) : buffer;
        if (line.length > 0) yield line;
    }
}
