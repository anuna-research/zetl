#!/usr/bin/env node
// ztl remark harness (SPEC-033 REQ-3305 / CON-3305 / ADR-3304).
//
// A long-lived Node.js subprocess that bridges ztl's Rust hook runtime
// and any installed remark/unified plugins. The harness speaks a tiny
// JSON-RPC-like protocol over line-delimited JSON on stdin/stdout:
//
//   ztl → harness:  {"id":N, "type":"load_plugin", "package":"...", "options":{...}}
//   harness → ztl:  {"id":N, "type":"load_result", "ok":true, "plugin_id":"rp_X"}
//
//   ztl → harness:  {"id":N, "type":"apply", "plugin_id":"rp_X", "ast":{...mdast...}}
//   harness → ztl:  {"id":N, "type":"apply_result", "ok":true, "ast":{...mdast...}}
//
//   ztl → harness:  {"id":N, "type":"shutdown"}
//   harness exits 0.
//
// A banner message is emitted on startup (type=ready, harness_version,
// node_version, unified_available) so the spawning adapter can confirm
// the pipe is live before sending requests.
//
// Plugin resolution: `import(package)` runs from the harness's current
// working directory, which ztl sets to the plugin-resolution root
// (`node_modules`' parent). This lets users override resolution without
// touching the harness.
//
// Errors are reported in-band via {"ok": false, "error": "..."} responses
// on the same message id. The harness never exits on a per-message
// failure; only the shutdown message terminates the process. Uncaught
// exceptions in a plugin are surfaced as an apply_result with ok=false.
//
// isolation = "shared"        → one harness per ztl process; all plugins
//                               share the module cache.
// isolation = "fresh-context" → ztl spawns a new harness subprocess per
//                               invocation and shuts it down afterwards.
//                               Enforced on the Rust side; this harness
//                               script is identical in both modes.

'use strict';

const HARNESS_VERSION = '1.0.0';

// Resolution cache: plugin_id → loaded plugin module + options. One
// slot per load_plugin call so re-applies skip the import() cost.
const plugins = new Map();
let nextPluginIndex = 1;

let unified = null;
let unifiedImportError = null;

// Probe for unified up-front so the ready banner carries an accurate
// `unified_available` flag. Plugin application can still fail later (a
// user asked for remark-foo but only remark-bar is installed) — that is
// reported per-apply rather than at startup.
try {
    const mod = await import('unified');
    unified = mod.unified ?? mod.default?.unified ?? null;
} catch (e) {
    unifiedImportError = String(e && e.message ? e.message : e);
}

// Line-buffered stdin reader. We deliberately re-implement the
// line-buffering here rather than relying on readline so the harness
// stays robust against partial reads / embedded newlines in mdast
// string values (JSON escapes them, so the outer record terminator is
// always a real newline).
let pending = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', (chunk) => {
    pending += chunk;
    let nl;
    while ((nl = pending.indexOf('\n')) !== -1) {
        const line = pending.slice(0, nl);
        pending = pending.slice(nl + 1);
        if (line.length === 0) continue;
        handleLine(line).catch((err) => {
            // Last-resort: a handler threw outside its own try/catch.
            // Emit a generic error response with id=0 so the adapter
            // can surface it as a pipeline-level diagnostic.
            emit({ id: 0, type: 'error', ok: false, error: String(err && err.message ? err.message : err) });
        });
    }
});

process.stdin.on('end', () => {
    // Peer closed the pipe without a shutdown message — exit cleanly
    // rather than hang. The Rust side does send shutdown under normal
    // teardown; this branch covers abrupt closes (e.g. SIGPIPE).
    process.exit(0);
});

// Banner. The adapter blocks on this message before sending any real
// requests, so it arrives promptly.
emit({
    type: 'ready',
    harness_version: HARNESS_VERSION,
    node_version: process.version,
    unified_available: unified !== null,
    unified_import_error: unifiedImportError,
});

async function handleLine(line) {
    let msg;
    try {
        msg = JSON.parse(line);
    } catch (e) {
        emit({ id: 0, type: 'error', ok: false, error: `malformed JSON: ${e.message}` });
        return;
    }
    const id = typeof msg.id === 'number' ? msg.id : 0;
    const type = msg.type;
    try {
        switch (type) {
            case 'load_plugin':
                await handleLoadPlugin(id, msg);
                break;
            case 'apply':
                await handleApply(id, msg);
                break;
            case 'unload_plugin':
                handleUnload(id, msg);
                break;
            case 'shutdown':
                // Reply so the adapter can block on shutdown if it wants
                // to. Then exit. stdout is flushed synchronously via
                // process.stdout.write in emit().
                emit({ id, type: 'shutdown_result', ok: true });
                process.exit(0);
                break;
            case 'ping':
                emit({ id, type: 'pong', ok: true });
                break;
            default:
                emit({ id, type: 'error', ok: false, error: `unknown message type: ${String(type)}` });
        }
    } catch (err) {
        emit({
            id,
            type: `${type || 'error'}_result`,
            ok: false,
            error: String(err && err.message ? err.message : err),
        });
    }
}

async function handleLoadPlugin(id, msg) {
    if (!unified) {
        emit({
            id,
            type: 'load_result',
            ok: false,
            error: `unified is not importable from this harness: ${unifiedImportError || 'no unified package resolved from the current working directory'}`,
        });
        return;
    }
    const pkg = msg.package;
    if (typeof pkg !== 'string' || pkg.length === 0) {
        emit({ id, type: 'load_result', ok: false, error: 'load_plugin requires a non-empty `package` string' });
        return;
    }
    const options = msg.options ?? {};
    let mod;
    try {
        mod = await import(pkg);
    } catch (e) {
        emit({
            id,
            type: 'load_result',
            ok: false,
            error: `could not import '${pkg}': ${e && e.message ? e.message : String(e)}`,
        });
        return;
    }
    // remark plugin convention: the package's default export is the
    // plugin factory.
    const plugin = mod.default ?? mod;
    if (typeof plugin !== 'function') {
        emit({
            id,
            type: 'load_result',
            ok: false,
            error: `package '${pkg}' did not expose a callable default export (got ${typeof plugin})`,
        });
        return;
    }
    const pluginId = `rp_${nextPluginIndex++}`;
    plugins.set(pluginId, { package: pkg, plugin, options });
    emit({ id, type: 'load_result', ok: true, plugin_id: pluginId, package: pkg });
}

async function handleApply(id, msg) {
    const pluginId = msg.plugin_id;
    const ast = msg.ast;
    const entry = plugins.get(pluginId);
    if (!entry) {
        emit({
            id,
            type: 'apply_result',
            ok: false,
            error: `unknown plugin_id '${pluginId}' — call load_plugin first`,
        });
        return;
    }
    if (!ast || typeof ast !== 'object') {
        emit({ id, type: 'apply_result', ok: false, error: 'apply requires an `ast` object' });
        return;
    }
    let processor;
    try {
        processor = unified().use(entry.plugin, entry.options);
    } catch (e) {
        emit({
            id,
            type: 'apply_result',
            ok: false,
            error: `plugin '${entry.package}' failed to initialise: ${e && e.message ? e.message : String(e)}`,
        });
        return;
    }
    let out;
    try {
        out = await processor.run(ast);
    } catch (e) {
        emit({
            id,
            type: 'apply_result',
            ok: false,
            error: `plugin '${entry.package}' threw during run: ${e && e.message ? e.message : String(e)}`,
        });
        return;
    }
    emit({ id, type: 'apply_result', ok: true, ast: out });
}

function handleUnload(id, msg) {
    const pluginId = msg.plugin_id;
    const existed = plugins.delete(pluginId);
    emit({ id, type: 'unload_result', ok: existed, error: existed ? undefined : `unknown plugin_id '${pluginId}'` });
}

function emit(obj) {
    // One message per line; JSON stringification is safe for arbitrary
    // mdast values because strings in the tree are escaped.
    const line = JSON.stringify(obj);
    process.stdout.write(line + '\n');
}
