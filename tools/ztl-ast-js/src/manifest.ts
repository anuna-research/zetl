// Manifest helpers for authoring `<executable>.toml` sidecars
// (SPEC-032 REQ-3206 / REQ-3217 / REQ-3221 / REQ-3224).
//
// This module:
//   - exposes the manifest schema as TypeScript types;
//   - provides `renderManifest()` to emit a minimal canonical TOML
//     document so tooling (e.g. `ztl hook new`) can scaffold sidecars
//     without pulling in a full TOML serializer;
//   - provides `parseManifest()` that reads the slice ztl's
//     composition layer reads (extension_id / optional / ordering /
//     ast_type / ast_version / contract.preserves). Unknown top-level
//     keys are preserved under `extras` so callers can round-trip
//     foreign metadata when editing existing files.
//
// A full TOML parser is out of scope — we ship a tiny one that covers
// the shape the ztl composition layer accepts. For arbitrary TOML,
// hook authors should use `@iarna/toml` or similar in their own
// tooling; ztl only consumes what's documented here.

/** AST ecosystem declarations recognised at the composition boundary. */
export type AstType = "ztl-ext" | "mdast-ext" | "pandoc-ext";

export interface OrderingTable {
    before?: string[];
    after?: string[];
}

export interface ContractTable {
    /** Node types the hook declares it will not strip (REQ-3224). */
    preserves?: string[];
}

export interface Manifest {
    /** Stable id used for `before`/`after` cross-references and `page.ext.<id>.*`. */
    extension_id?: string;
    /** Skip silently if the hook cannot be probed instead of erroring out. */
    optional?: boolean;
    /** Canonical ordering constraints (REQ-3217). */
    ordering?: OrderingTable;
    /** Top-level aliases accepted by ztl; merged with `ordering.*` at parse time. */
    before?: string[];
    after?: string[];
    /** Required by transform-stage hooks targeting non-ztl ecosystems (REQ-3221). */
    ast_type?: AstType;
    /** Semver range against the declared ast_type's version scheme. */
    ast_version?: string;
    /** Behavioural contract block (REQ-3224). Only `preserves` is read at composition. */
    contract?: ContractTable;
    /** Unknown top-level keys preserved from parse, in source order. */
    extras?: Record<string, string>;
}

const VALID_AST_TYPES: ReadonlySet<string> = new Set(["ztl-ext", "mdast-ext", "pandoc-ext"]);

export class ManifestError extends Error {
    readonly key: string | undefined;
    constructor(message: string, key?: string) {
        super(key ? `${key}: ${message}` : message);
        this.name = "ManifestError";
        this.key = key;
    }
}

// ── Writer ──────────────────────────────────────────────────────────────────

/**
 * Emit a canonical TOML document for `manifest`. Keys are written in a
 * stable order so generated files diff-cleanly. Unknown keys in `extras`
 * are emitted verbatim after the known ones.
 */
export function renderManifest(manifest: Manifest): string {
    const out: string[] = [];

    if (manifest.extension_id !== undefined) {
        out.push(`extension_id = ${tomlString(manifest.extension_id)}`);
    }
    if (manifest.optional !== undefined) {
        out.push(`optional = ${manifest.optional ? "true" : "false"}`);
    }
    if (manifest.ast_type !== undefined) {
        out.push(`ast_type = ${tomlString(manifest.ast_type)}`);
    }
    if (manifest.ast_version !== undefined) {
        out.push(`ast_version = ${tomlString(manifest.ast_version)}`);
    }

    // Top-level before/after alias. Prefer the `[ordering]` table when
    // `manifest.ordering` is set; fall back to top-level when only the
    // alias form is given.
    const hasOrderingTable = manifest.ordering && (manifest.ordering.before || manifest.ordering.after);
    if (!hasOrderingTable) {
        if (manifest.before !== undefined) out.push(`before = ${tomlStringArray(manifest.before)}`);
        if (manifest.after !== undefined) out.push(`after = ${tomlStringArray(manifest.after)}`);
    }

    if (manifest.extras) {
        for (const [k, v] of Object.entries(manifest.extras)) {
            out.push(`${k} = ${v}`);
        }
    }

    if (hasOrderingTable) {
        out.push("");
        out.push("[ordering]");
        if (manifest.ordering?.before !== undefined) out.push(`before = ${tomlStringArray(manifest.ordering.before)}`);
        if (manifest.ordering?.after !== undefined) out.push(`after = ${tomlStringArray(manifest.ordering.after)}`);
    }

    if (manifest.contract?.preserves !== undefined) {
        out.push("");
        out.push("[contract]");
        out.push(`preserves = ${tomlStringArray(manifest.contract.preserves)}`);
    }

    return out.join("\n") + "\n";
}

function tomlString(s: string): string {
    return `"${s.replace(/\\/g, "\\\\").replace(/"/g, "\\\"")}"`;
}

function tomlStringArray(xs: readonly string[]): string {
    if (xs.length === 0) return "[]";
    return `[${xs.map(tomlString).join(", ")}]`;
}

// ── Parser (subset of TOML used by ztl composition) ────────────────────────

/**
 * Parse a sidecar manifest. Supports:
 *   - top-level scalar keys (string / boolean / string-array)
 *   - `[ordering]` and `[contract]` tables
 *   - `#` line comments, blank lines
 *
 * Returns the manifest with ordering aliases resolved: `ordering.before`
 * and top-level `before` are both exposed; `resolvedBefore()` / `resolvedAfter()`
 * match the Rust composition layer's `CompositionManifest::resolved_*`
 * helpers.
 */
export function parseManifest(text: string): Manifest {
    const lines = text.split(/\r?\n/);
    const manifest: Manifest = { extras: {} };
    let table: "root" | "ordering" | "contract" | "unknown" = "root";

    for (let lineno = 1; lineno <= lines.length; lineno++) {
        const rawLine = lines[lineno - 1] ?? "";
        const line = stripComment(rawLine).trim();
        if (!line) continue;

        if (line.startsWith("[") && line.endsWith("]")) {
            const name = line.slice(1, -1).trim();
            if (name === "ordering") {
                manifest.ordering ??= {};
                table = "ordering";
            } else if (name === "contract") {
                manifest.contract ??= {};
                table = "contract";
            } else {
                // Unknown table — skip its contents (forward-compat per
                // NFR-3206 additive-only rule).
                table = "unknown";
            }
            continue;
        }

        const eq = line.indexOf("=");
        if (eq < 0) {
            throw new ManifestError(`unexpected line at ${lineno}: ${rawLine}`);
        }
        const key = line.slice(0, eq).trim();
        const valueStr = line.slice(eq + 1).trim();
        const value = parseValue(valueStr, key, lineno);

        if (table === "unknown") continue;
        if (table === "ordering") {
            if (key === "before") manifest.ordering!.before = asStringArray(value, "ordering.before");
            else if (key === "after") manifest.ordering!.after = asStringArray(value, "ordering.after");
            continue;
        }
        if (table === "contract") {
            if (key === "preserves") manifest.contract!.preserves = asStringArray(value, "contract.preserves");
            continue;
        }

        // Root table.
        switch (key) {
            case "extension_id":
                manifest.extension_id = asString(value, key);
                break;
            case "optional":
                manifest.optional = asBoolean(value, key);
                break;
            case "before":
                manifest.before = asStringArray(value, key);
                break;
            case "after":
                manifest.after = asStringArray(value, key);
                break;
            case "ast_type": {
                const s = asString(value, key);
                if (!VALID_AST_TYPES.has(s)) {
                    throw new ManifestError(
                        `unknown ast_type ${JSON.stringify(s)}; expected one of ${[...VALID_AST_TYPES].join(", ")}`,
                        key,
                    );
                }
                manifest.ast_type = s as AstType;
                break;
            }
            case "ast_version":
                manifest.ast_version = asString(value, key);
                break;
            default:
                // Unknown top-level key — preserve source form so editors
                // can round-trip the file without losing author metadata.
                manifest.extras![key] = valueStr;
        }
    }

    return manifest;
}

/** Merge `[ordering].before` with top-level `before` (canonical first). */
export function resolvedBefore(manifest: Manifest): string[] {
    return [...(manifest.ordering?.before ?? []), ...(manifest.before ?? [])];
}

export function resolvedAfter(manifest: Manifest): string[] {
    return [...(manifest.ordering?.after ?? []), ...(manifest.after ?? [])];
}

/**
 * Default extension_id from a hook filename — matches the Rust composition
 * layer's `default_extension_id`: strip the leading `\d+-` ordering prefix
 * and the final extension. `20-tasks.py` → `tasks`, `callouts` → `callouts`.
 */
export function defaultExtensionId(filename: string): string {
    const withoutExt = filename.replace(/\.[^./]+$/, "");
    return withoutExt.replace(/^\d+-/, "");
}

// ── TOML value helpers ──────────────────────────────────────────────────────

type TomlValue = string | boolean | number | TomlValue[];

function parseValue(text: string, key: string, lineno: number): TomlValue {
    if (text === "true") return true;
    if (text === "false") return false;
    if (text.startsWith('"')) return parseString(text, key, lineno);
    if (text.startsWith("[")) return parseArray(text, key, lineno);
    if (/^-?\d+(\.\d+)?$/.test(text)) return Number.parseFloat(text);
    throw new ManifestError(`line ${lineno}: unsupported value ${JSON.stringify(text)}`, key);
}

function parseString(text: string, key: string, lineno: number): string {
    if (!text.startsWith('"') || !text.endsWith('"') || text.length < 2) {
        throw new ManifestError(`line ${lineno}: malformed string ${JSON.stringify(text)}`, key);
    }
    const body = text.slice(1, -1);
    let out = "";
    for (let i = 0; i < body.length; i++) {
        const c = body[i];
        if (c === "\\" && i + 1 < body.length) {
            const next = body[++i];
            if (next === "n") out += "\n";
            else if (next === "t") out += "\t";
            else if (next === "r") out += "\r";
            else if (next === "\\") out += "\\";
            else if (next === "\"") out += '"';
            else {
                throw new ManifestError(`line ${lineno}: unknown escape \\${next}`, key);
            }
            continue;
        }
        out += c;
    }
    return out;
}

function parseArray(text: string, key: string, lineno: number): TomlValue[] {
    if (!text.startsWith("[") || !text.endsWith("]")) {
        throw new ManifestError(`line ${lineno}: malformed array ${JSON.stringify(text)}`, key);
    }
    const body = text.slice(1, -1).trim();
    if (!body) return [];
    // Top-level comma split (no nesting in our subset).
    const parts: string[] = [];
    let depth = 0;
    let inString = false;
    let start = 0;
    for (let i = 0; i < body.length; i++) {
        const c = body[i];
        if (inString) {
            if (c === "\\") {
                i++;
                continue;
            }
            if (c === '"') inString = false;
            continue;
        }
        if (c === '"') {
            inString = true;
            continue;
        }
        if (c === "[") depth++;
        else if (c === "]") depth--;
        else if (c === "," && depth === 0) {
            parts.push(body.slice(start, i).trim());
            start = i + 1;
        }
    }
    parts.push(body.slice(start).trim());
    return parts.filter((p) => p.length > 0).map((p) => parseValue(p, key, lineno));
}

function stripComment(line: string): string {
    let inString = false;
    for (let i = 0; i < line.length; i++) {
        const c = line[i];
        if (c === '"' && line[i - 1] !== "\\") inString = !inString;
        else if (c === "#" && !inString) return line.slice(0, i);
    }
    return line;
}

function asString(v: TomlValue, key: string): string {
    if (typeof v !== "string") throw new ManifestError("expected a string", key);
    return v;
}

function asBoolean(v: TomlValue, key: string): boolean {
    if (typeof v !== "boolean") throw new ManifestError("expected a boolean", key);
    return v;
}

function asStringArray(v: TomlValue, key: string): string[] {
    if (!Array.isArray(v)) throw new ManifestError("expected an array", key);
    return v.map((x, i) => {
        if (typeof x !== "string") throw new ManifestError(`element ${i} is not a string`, key);
        return x;
    });
}
