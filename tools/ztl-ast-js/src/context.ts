// Per-invocation context passed to the hook's transform function.
//
// Shapes mirror REQ-3219 (build_data) and REQ-3220 (build context).
// Emit helpers accumulate writes into a response payload that the
// protocol layer flushes back to ztl.

export type Severity = "info" | "warn" | "error";

export interface Diagnostic {
    severity: Severity;
    message: string;
    page_slug?: string;
}

/** Immutable snapshot view of the current `build_data` store (REQ-3219). */
export interface BuildDataView {
    /** Full map keyed by writing-hook's `extension_id`. */
    readonly byExtension: Readonly<Record<string, Readonly<Record<string, unknown>>>>;
    /** Convenience: look up a value written by another hook. */
    get(extensionId: string, key: string): unknown;
}

function freezeView(raw: unknown): BuildDataView {
    const map =
        raw !== null && typeof raw === "object" && !Array.isArray(raw)
            ? (raw as Record<string, Record<string, unknown>>)
            : {};
    return {
        byExtension: map,
        get(extensionId, key) {
            const bag = map[extensionId];
            if (!bag) return undefined;
            return bag[key];
        },
    };
}

/** Shape of the REQ-3220 context block ztl sends in `init`. */
export interface BuildEnv {
    mode: "build" | "serve";
    theme: string | null;
    vault_root: string;
    out_dir: string | null;
    verbose: boolean;
    at: string | null;
    hook_path: string | null;
    extension_id: string;
    ztl_version?: string;
    ast_schema_version?: string;
}

export interface Context {
    /** Slug of the page currently being transformed (empty on `init` / `finalise`). */
    readonly pageSlug: string;
    /** Raw frontmatter object for the current page. */
    readonly frontmatter: Readonly<Record<string, unknown>>;
    /** Stage this hook was wired to (`"pre-parse"` | `"transform"` | `"post-render"`). */
    readonly stage: string;
    /** REQ-3220 build context (env vars + init ctx payload). */
    readonly env: Readonly<BuildEnv>;
    /** Read-only snapshot of the shared build-data store (REQ-3219). */
    readonly buildData: BuildDataView;

    // ── Emit helpers (flushed in the response payload) ─────────────────────
    /** Write into the `build_data` response under this hook's extension_id. */
    emitBuildData(writes: Record<string, unknown>): void;
    /** Write `page.ext.<extension_id>.*` template variables (REQ-3214). */
    emitTemplateVars(writes: Record<string, unknown>): void;
    /** Write `vault.ext.<extension_id>.*` template variables (REQ-3214). */
    emitVaultTemplateVars(writes: Record<string, unknown>): void;
    /** Attach a structured diagnostic to the response. */
    diag(severity: Severity, message: string): void;
    warn(message: string): void;
    info(message: string): void;
    error(message: string): void;
}

/** Payload accumulated on a `Context` and drained into the protocol response. */
export interface ContextWrites {
    buildData: Record<string, unknown>;
    templateVars: Record<string, unknown>;
    vaultTemplateVars: Record<string, unknown>;
    diagnostics: Diagnostic[];
}

export function emptyContextWrites(): ContextWrites {
    return { buildData: {}, templateVars: {}, vaultTemplateVars: {}, diagnostics: [] };
}

export interface BuildContextInput {
    pageSlug: string;
    frontmatter: unknown;
    stage: string;
    env: BuildEnv;
    buildData: unknown;
}

export function buildContext(input: BuildContextInput): { ctx: Context; writes: ContextWrites } {
    const writes = emptyContextWrites();
    const slug = input.pageSlug;
    const ctx: Context = {
        pageSlug: slug,
        frontmatter:
            input.frontmatter !== null && typeof input.frontmatter === "object" && !Array.isArray(input.frontmatter)
                ? (input.frontmatter as Record<string, unknown>)
                : {},
        stage: input.stage,
        env: Object.freeze({ ...input.env }),
        buildData: freezeView(input.buildData),
        emitBuildData(bag) {
            Object.assign(writes.buildData, bag);
        },
        emitTemplateVars(bag) {
            Object.assign(writes.templateVars, bag);
        },
        emitVaultTemplateVars(bag) {
            Object.assign(writes.vaultTemplateVars, bag);
        },
        diag(severity, message) {
            writes.diagnostics.push(slug ? { severity, message, page_slug: slug } : { severity, message });
        },
        warn(message) {
            ctx.diag("warn", message);
        },
        info(message) {
            ctx.diag("info", message);
        },
        error(message) {
            ctx.diag("error", message);
        },
    };
    return { ctx, writes };
}
