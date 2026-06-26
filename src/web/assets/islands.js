// ============================================================================
// islands.js — SPEC-050 Component-Islands browser runtime (v1)
// ----------------------------------------------------------------------------
// Plain ES module / IIFE. NO framework, NO build step, NO npm deps, NO imports.
// Security-critical: this file is the trusted host / reference monitor that
// stands between untrusted content-island code (Web Workers, untrusted
// localStorage/storage events, untrusted inbound postMessages) and the page.
//
// Style: mirrors src/web/assets/bm25.js — a single self-contained IIFE that
// attaches its public surface to `window`. ES5-ish var usage is intentional for
// readability/conservatism; a few modern platform APIs are used (Map/WeakMap,
// IntersectionObserver, requestIdleCallback) guarded where appropriate.
//
// Each major block is tied to its SPEC-050 REQ/CON in comments.
// ============================================================================
//
// ----------------------------------------------------------------------------
// DOM CONTRACT (READ THIS — the Rust emitter MUST match these exact names)
// ----------------------------------------------------------------------------
// The spec (REQ-5001 / §11) mandates the component hydration marker
// `data-z="<name>"` and otherwise leaves the precise island-descriptor
// attribute names to IMPL (Open Question Q1, "exact data-island encoding").
// This runtime defines them as follows. An island mount node is any element:
//
//   <div data-z="<component-name>"          REQ-5001 hydration marker (REQUIRED)
//        data-island="<component-name>"     island name (REQUIRED to hydrate; if
//                                           absent we fall back to data-z's value)
//        data-island-worker="<url>"         Worker script URL (default transport,
//                                           REQ-5025). Same-origin/relative only.
//        data-island-hydrate="<strategy>"   REQ-5024 timing: "load" (default) |
//                                           "idle" | "visible" | "visible(<m>)" |
//                                           "media(<query>)"
//        data-island-grants='<json>'        capability grant table, a JSON array
//                                           (REQ-5016/CON-5006). See GRANT SHAPE.
//        data-island-types='<json>'         per-topic declared value types, a JSON
//                                           object { "<topic>": "<type-expr>" }
//                                           (CON-5005). Used to recognise every
//                                           payload at the bridge.
//        data-island-iframe="<url>"         OPT-IN escape hatch (REQ-5015). If
//                                           present (and no -worker), the iframe
//                                           path is used. v1: STUBBED (see TODO).
//        data-island-allowlist='<json>'     CON-5007 element/attr allowlist for the
//                                           controlled-element renderer, a JSON
//                                           object (see ALLOWLIST SHAPE). If absent
//                                           we fall back to a built-in conservative
//                                           default allowlist; an *empty* allowlist
//                                           => render nothing (fail-closed).
//        data-island-paints="true|false">   CON-5002 `paints` grant: may the worker
//                                           emit `render`? Default false (headless).
//   ... static, no-JS component HTML lives here (REQ-5002 progressive enhancement) ...
//     <script type="application/json" data-island-props>{ ...author props... }</script>
//   </div>
//
//   data-island-props (optional): a single child <script type="application/json"
//     data-island-props> whose JSON is forwarded to the worker as its boot config.
//     It is author data, NOT executed; only JSON.parse is applied.
//
//   Once a node is hydrated the runtime sets `data-island-hydrated="1"` on it so a
//   second hydrateAll() pass (e.g. after a SPA subtree swap) never double-binds it
//   (REQ-5003 idempotency).
//
// GRANT SHAPE (data-island-grants): JSON array of objects:
//     [{ "topic": "content:filter", "direction": "publish" },
//      { "topic": "content:filter", "direction": "emit" },
//      { "topic": "theme",          "direction": "subscribe" },   // read-only trusted
//      { "direction": "render" }]                                 // paint capability
//   `direction` is one of: "publish" | "emit" | "subscribe" | "render".
//   A trusted (non-`content:`) topic may ONLY be granted "subscribe" (REQ-5016).
//   The grant table is the SOLE authority; the island cannot widen/forge/enumerate.
//
// ALLOWLIST SHAPE (data-island-allowlist): JSON object:
//     { "div": ["class","id"], "a": ["href","class"], "span": ["class"], ... }
//   Keys = allowed tag names (lowercase); values = extra attribute names allowed
//   on that tag (on top of the always-allowed aria-*/role and the URL attrs which
//   are themselves scheme-checked). Hard-forbidden tags/attrs (below) are rejected
//   even if listed. A missing/empty/malformed allowlist => render nothing.
//
// ============================================================================

(function () {
  "use strict";

  // If a non-browser context loads this file (SSR sanity, tooling), `window`
  // may be undefined. Bail out of the IIFE but stay syntactically valid.
  var glob = (typeof window !== "undefined") ? window : (typeof globalThis !== "undefined" ? globalThis : null);
  if (!glob) { return; }

  // --------------------------------------------------------------------------
  // SINGLE-INSTANCE GUARD (REQ-5007 / REQ-5023)
  // The bus is a single instance living on the nav-surviving SPA shell. If an
  // earlier island page in this session already installed `window.zetl`, we MUST
  // NOT recreate it (a second instance would reset retained values + subs).
  // We still (re-)run hydration below for freshly-swapped subtrees.
  // --------------------------------------------------------------------------
  var alreadyInstalled = !!glob.zetl;

  // ==========================================================================
  // NFR-5002 — fail-closed bounds. Provisional constants; breach => drop + warn,
  // NEVER throw to the caller and NEVER unbounded allocation (Threat D).
  // ==========================================================================
  var LIMITS = {
    MAX_TOPICS: 256,            // distinct store topics
    MAX_SUBSCRIBERS: 1024,      // total live subscribers (incl. bridge-relayed)
    MAX_VALUE_BYTES: 64 * 1024, // per retained value, JSON.stringify length proxy
    // CON-5007 render-tree bounds (provisional, IMPL-050):
    RENDER_MAX_DEPTH: 32,
    RENDER_MAX_CHILDREN: 256,
    RENDER_MAX_NODES: 4096,
    RENDER_MAX_TEXT_BYTES: 256 * 1024,
    MAX_WORKER_SPAWNS: 64       // per-page content-island runtime (re)creations
  };

  // Single console diagnostic helper. Diagnostics NEVER throw to callers.
  function diag(code, msg, extra) {
    try {
      // eslint-disable-next-line no-console
      console.warn("[zetl/islands] " + code + (msg ? ": " + msg : ""), extra === undefined ? "" : extra);
    } catch (_e) { /* console may be absent; swallow */ }
  }

  // Rough UTF-8 byte length without allocating a TextEncoder per call when
  // avoidable. Used for value-size + text-byte bounds.
  function byteLength(str) {
    if (typeof str !== "string") { return 0; }
    if (typeof TextEncoder !== "undefined") {
      try { return new TextEncoder().encode(str).length; } catch (_e) { /* fall through */ }
    }
    // Fallback: count code units' UTF-8 contribution.
    var n = 0;
    for (var i = 0; i < str.length; i++) {
      var c = str.charCodeAt(i);
      if (c < 0x80) { n += 1; }
      else if (c < 0x800) { n += 2; }
      else if (c >= 0xD800 && c <= 0xDBFF) { n += 4; i++; } // surrogate pair
      else { n += 3; }
    }
    return n;
  }

  // ==========================================================================
  // §2. TYPE RECOGNISER (CON-5005)
  // One shared recogniser per the type-expr grammar:
  //   string | bool | int | number | enum(a,b,c) | {field:scalar,...}
  // Returns { ok:true, value } (normalised) or { ok:false }.
  // Records are FLAT only (no nested records). Rebuilds records onto a
  // null-proto object and rejects __proto__/constructor/prototype keys (Threat H).
  // ==========================================================================

  // ASCII control char check (string type rejects control chars, CON-5005).
  function hasControlChar(s) {
    for (var i = 0; i < s.length; i++) {
      var c = s.charCodeAt(i);
      // C0 controls (0x00-0x1F), DEL (0x7F), and C1 controls (0x80-0x9F).
      if (c <= 0x1F || c === 0x7F || (c >= 0x80 && c <= 0x9F)) { return true; }
    }
    return false;
  }

  var MAX_SAFE = 9007199254740991; // 2^53 - 1

  // Parse a type-expr STRING into a small descriptor object, or null if invalid.
  // Cached per expr string so we don't re-parse on every payload.
  var typeExprCache = Object.create(null);
  function parseTypeExpr(expr) {
    if (typeof expr !== "string") { return null; }
    if (Object.prototype.hasOwnProperty.call(typeExprCache, expr)) {
      return typeExprCache[expr];
    }
    var parsed = parseTypeExprUncached(expr);
    typeExprCache[expr] = parsed;
    return parsed;
  }

  function parseScalar(s) {
    s = s.trim();
    if (s === "string" || s === "bool" || s === "int" || s === "number") {
      return { kind: s };
    }
    if (s.indexOf("enum(") === 0 && s.charAt(s.length - 1) === ")") {
      var inner = s.slice(5, -1);
      var lits = parseEnumLiterals(inner);
      if (!lits) { return null; }
      return { kind: "enum", literals: lits };
    }
    return null;
  }

  // enum literals are JSON strings, comma-separated. Non-empty, duplicate-free.
  function parseEnumLiterals(inner) {
    // Wrap as a JSON array to leverage JSON.parse for correct string escaping.
    var arr;
    try { arr = JSON.parse("[" + inner + "]"); } catch (_e) { return null; }
    if (!Array.isArray(arr) || arr.length === 0) { return null; }
    var seen = Object.create(null);
    for (var i = 0; i < arr.length; i++) {
      var lit = arr[i];
      if (typeof lit !== "string" || hasControlChar(lit)) { return null; }
      if (seen[lit]) { return null; } // duplicate => invalid
      seen[lit] = true;
    }
    return arr;
  }

  function parseTypeExprUncached(expr) {
    var e = expr.trim();
    // record: {field:scalar, ...}
    if (e.charAt(0) === "{" && e.charAt(e.length - 1) === "}") {
      var body = e.slice(1, -1).trim();
      if (body === "") { return null; } // record MUST be non-empty
      var fields = splitTopLevel(body, ",");
      if (!fields) { return null; }
      var rec = { kind: "record", fields: [] };
      var fieldNames = Object.create(null);
      for (var i = 0; i < fields.length; i++) {
        var pair = fields[i].trim();
        var colon = pair.indexOf(":");
        if (colon < 0) { return null; }
        var name = pair.slice(0, colon).trim();
        // field idents: simple identifier, NOT a dangerous proto key.
        if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) { return null; }
        if (name === "__proto__" || name === "constructor" || name === "prototype") { return null; }
        if (fieldNames[name]) { return null; } // unique idents required
        fieldNames[name] = true;
        var sc = parseScalar(pair.slice(colon + 1));
        if (!sc) { return null; } // no nested records: scalar-type only
        rec.fields.push({ name: name, type: sc });
      }
      if (rec.fields.length === 0) { return null; }
      return rec;
    }
    // scalar / enum
    return parseScalar(e);
  }

  // Split on a delimiter at top level (not inside quotes / parens / braces).
  function splitTopLevel(s, delim) {
    var out = [];
    var depth = 0, inStr = false, esc = false, cur = "";
    for (var i = 0; i < s.length; i++) {
      var ch = s.charAt(i);
      if (inStr) {
        cur += ch;
        if (esc) { esc = false; }
        else if (ch === "\\") { esc = true; }
        else if (ch === '"') { inStr = false; }
        continue;
      }
      if (ch === '"') { inStr = true; cur += ch; continue; }
      if (ch === "(" || ch === "{" || ch === "[") { depth++; cur += ch; continue; }
      if (ch === ")" || ch === "}" || ch === "]") { depth--; cur += ch; continue; }
      if (ch === delim && depth === 0) { out.push(cur); cur = ""; continue; }
      cur += ch;
    }
    out.push(cur);
    return out;
  }

  // Recognise a scalar value against a scalar descriptor; returns {ok, value}.
  function recogniseScalar(value, sc) {
    switch (sc.kind) {
      case "string":
        if (typeof value === "string" && !hasControlChar(value)) { return { ok: true, value: value }; }
        return { ok: false };
      case "bool":
        if (value === true || value === false) { return { ok: true, value: value }; }
        return { ok: false };
      case "int":
        if (typeof value !== "number" || !isFinite(value)) { return { ok: false }; }
        if (Math.floor(value) !== value) { return { ok: false }; } // no fractional part
        if (value < -MAX_SAFE || value > MAX_SAFE) { return { ok: false }; }
        if (value === 0) { value = 0; } // -0 -> 0 normalisation
        return { ok: true, value: value };
      case "number":
        if (typeof value === "number" && isFinite(value)) {
          if (value === 0) { value = 0; }
          return { ok: true, value: value };
        }
        return { ok: false };
      case "enum":
        if (typeof value === "string") {
          for (var i = 0; i < sc.literals.length; i++) {
            if (sc.literals[i] === value) { return { ok: true, value: value }; } // case-sensitive
          }
        }
        return { ok: false };
      default:
        return { ok: false };
    }
  }

  // The single shared recogniser. `value` is an ALREADY-NORMALISED live value
  // (callers normalise structured-clone graphs / JSON.parse output first).
  function recognise(value, typeExpr) {
    var desc = (typeof typeExpr === "object" && typeExpr) ? typeExpr : parseTypeExpr(typeExpr);
    if (!desc) { return { ok: false }; } // invalid/unknown type-expr => fail closed
    if (desc.kind === "record") {
      // Must be a plain object (not array, not null).
      if (value === null || typeof value !== "object" || Array.isArray(value)) { return { ok: false }; }
      // Strict key-set equality: declared fields exactly == own enumerable keys.
      var ownKeys = Object.keys(value);
      if (ownKeys.length !== desc.fields.length) { return { ok: false }; } // missing/extra
      // Reject dangerous keys defensively (should already be excluded by parse).
      for (var k = 0; k < ownKeys.length; k++) {
        var key = ownKeys[k];
        if (key === "__proto__" || key === "constructor" || key === "prototype") { return { ok: false }; }
      }
      // Rebuild onto a null-proto object so no inherited junk rides along.
      var rebuilt = Object.create(null);
      for (var f = 0; f < desc.fields.length; f++) {
        var field = desc.fields[f];
        if (!Object.prototype.hasOwnProperty.call(value, field.name)) { return { ok: false }; }
        var r = recogniseScalar(value[field.name], field.type);
        if (!r.ok) { return { ok: false }; }
        rebuilt[field.name] = r.value;
      }
      return { ok: true, value: rebuilt };
    }
    // scalar / enum
    return recogniseScalar(value, desc);
  }

  // Structural equality for normalised values (REQ-5005 coalescing + REQ-5021
  // value-change relay). Records compared by field values; -0===0; values finite.
  function structEqual(a, b) {
    if (a === b) { return true; }
    var ta = typeof a, tb = typeof b;
    if (ta !== tb) { return false; }
    if (ta === "object") {
      if (a === null || b === null) { return a === b; }
      if (Array.isArray(a) || Array.isArray(b)) { return false; } // records only
      var ka = Object.keys(a), kb = Object.keys(b);
      if (ka.length !== kb.length) { return false; }
      for (var i = 0; i < ka.length; i++) {
        var key = ka[i];
        if (!Object.prototype.hasOwnProperty.call(b, key)) { return false; }
        if (!structEqual(a[key], b[key])) { return false; }
      }
      return true;
    }
    return false; // primitives already handled by === (NaN N/A; values finite)
  }

  // Normalise a structured-clone object graph: prototype-check + null-proto
  // rebuild copying only own-enumerable DATA properties (CON-5006). Returns the
  // normalised value or a sentinel REJECT on a poisoned/exotic prototype.
  var REJECT = { __reject: true };
  function normaliseClone(value, depth) {
    depth = depth || 0;
    if (depth > 4) { return REJECT; } // flat records => shallow; guard anyway
    if (value === null) { return null; }
    var t = typeof value;
    if (t === "string" || t === "boolean" || t === "number") { return value; }
    if (t !== "object") { return REJECT; } // functions/symbols/undefined => reject
    if (Array.isArray(value)) {
      // arrays are not valid topic values (records only), but normalise defensively
      return REJECT;
    }
    var proto = Object.getPrototypeOf(value);
    if (proto !== Object.prototype && proto !== null) { return REJECT; } // exotic/poisoned
    var out = Object.create(null);
    var keys = Object.keys(value); // own enumerable only
    for (var i = 0; i < keys.length; i++) {
      var key = keys[i];
      if (key === "__proto__" || key === "constructor" || key === "prototype") { return REJECT; }
      // Reject accessor properties (getters) — only plain data allowed.
      var d = Object.getOwnPropertyDescriptor(value, key);
      if (!d || ("get" in d && d.get) || ("set" in d && d.set)) { return REJECT; }
      var nv = normaliseClone(d.value, depth + 1);
      if (nv === REJECT) { return REJECT; }
      out[key] = nv;
    }
    return out;
  }

  // ==========================================================================
  // §1. THE BUS — retained store + ephemeral bus (REQ-5004/5005/5007/5030, CON-5003)
  // Internal state lives ENTIRELY in these closure variables, never on the
  // frozen `window.zetl` object (REQ-5019 — an island can't reach/mutate it).
  // ==========================================================================

  // topic -> { value, hasValue, seq, subs:[fn,...] }
  var stores = new Map();
  // ephemeral topic -> [fn,...]
  var ephemeral = new Map();
  // declared types: topic -> type-expr string (registered by islands at hydrate).
  var declaredTypes = new Map();
  // declared defaults: topic -> normalised default value (for persisted reset).
  var declaredDefaults = new Map();
  // which topics are persisted (localStorage-backed, REQ-5006).
  var persistedTopics = new Set();

  // Host-assigned monotonic sequence (REQ-5030). Per session, from 1.
  var seqCounter = 0;
  function nextSeq() { seqCounter += 1; return seqCounter; }

  // Total live subscriber count for the NFR-5002 cap.
  var totalSubscribers = 0;

  // Register a topic's declared type so recognise() can run at set()/bridge.
  function registerType(topic, typeExpr) {
    if (typeof topic !== "string" || typeof typeExpr !== "string") { return; }
    if (parseTypeExpr(typeExpr) === null) { diag("island-topic-type-invalid", topic); return; }
    declaredTypes.set(topic, typeExpr);
  }

  function getStore(topic) {
    var s = stores.get(topic);
    if (!s) {
      if (stores.size >= LIMITS.MAX_TOPICS) {
        diag("bus-topic-cap", "topic cap reached, dropping " + topic);
        return null; // fail closed
      }
      s = { value: undefined, hasValue: false, seq: 0, subs: [] };
      stores.set(topic, s);
    }
    return s;
  }

  // Normalise + recognise a value about to be stored. `untrusted` selects whether
  // we treat a non-conforming value as a hard drop (bridge/persisted) — here we
  // simply drop+diag in both cases; callers decide whether to apply a default.
  function recogniseForTopic(topic, rawNormalised) {
    var typeExpr = declaredTypes.get(topic);
    if (!typeExpr) {
      // No declared type: accept only JSON-safe primitives/flat records defensively.
      // (A trusted island setting an undeclared topic is a wiring gap, not a crash.)
      return { ok: true, value: rawNormalised };
    }
    return recognise(rawNormalised, typeExpr);
  }

  // Core mutation. Returns true if a change fanned out, false if coalesced/dropped.
  function storeSet(topic, value, opts) {
    opts = opts || {};
    var s = getStore(topic);
    if (!s) { return false; }

    // Size bound (NFR-5002): JSON.stringify length as a cheap proxy (Threat D).
    var serialized;
    try { serialized = JSON.stringify(value); } catch (_e) {
      diag("bus-value-unserialisable", topic); return false;
    }
    if (serialized && serialized.length > LIMITS.MAX_VALUE_BYTES) {
      diag("bus-value-too-large", topic + " (" + serialized.length + " > " + LIMITS.MAX_VALUE_BYTES + ")");
      return false;
    }

    // Coalesce: structural equality against current value (REQ-5005).
    if (s.hasValue && structEqual(s.value, value)) { return false; }

    s.value = value;
    s.hasValue = true;
    s.seq = (opts.seq != null) ? opts.seq : nextSeq(); // host total-order stamp (REQ-5030)

    // Synchronous fan-out, deterministic subscription order, per-subscriber
    // try/catch so one throwing subscriber cannot wedge the others (REQ-5005,
    // CON-5003 fault isolation, Threat L).
    var snapshot = s.subs.slice(); // copy: a subscriber may (un)subscribe during fan-out
    for (var i = 0; i < snapshot.length; i++) {
      var fn = snapshot[i];
      try { fn(s.value, s.seq); } catch (e) { diag("subscriber-threw", topic, e); }
    }
    return true;
  }

  // Public store(topic) handle. All methods are closures over private state.
  function makeStoreHandle(topic) {
    return Object.freeze({
      get: function () {
        var s = stores.get(topic);
        return s && s.hasValue ? s.value : undefined;
      },
      // set: recognise the value (robustness check for in-realm callers,
      // security control at the bridge — same recogniser), then storeSet.
      set: function (value) {
        // In-realm trusted callers pass a live value; recognise directly.
        var r = recogniseForTopic(topic, value);
        if (!r.ok) { diag("island-payload-type", "store.set rejected for " + topic); return; }
        storeSet(topic, r.value);
      },
      // subscribe: replay current value immediately (or fire with nothing if
      // unset), then on every change. Returns an unsubscribe() (REQ-5005).
      subscribe: function (fn) {
        if (typeof fn !== "function") { diag("subscribe-not-fn", topic); return function () {}; }
        if (totalSubscribers >= LIMITS.MAX_SUBSCRIBERS) {
          diag("bus-subscriber-cap", "subscriber cap reached, dropping subscribe to " + topic);
          return function () {}; // fail closed: a no-op unsubscribe
        }
        var s = getStore(topic);
        if (!s) { return function () {}; }
        s.subs.push(fn);
        totalSubscribers += 1;
        // Immediate replay-on-subscribe.
        try { fn(s.hasValue ? s.value : undefined, s.hasValue ? s.seq : 0); }
        catch (e) { diag("subscriber-threw", topic, e); }
        var active = true;
        return function unsubscribe() {
          if (!active) { return; }
          active = false;
          var idx = s.subs.indexOf(fn);
          if (idx >= 0) { s.subs.splice(idx, 1); totalSubscribers -= 1; }
        };
      }
    });
  }

  // Ephemeral bus: fire-and-forget, NO retain, NO replay (REQ-5004 / CON-5003).
  var busApi = Object.freeze({
    emit: function (topic, detail) {
      if (typeof topic !== "string") { return; }
      var typeExpr = declaredTypes.get(topic);
      if (typeExpr) {
        var r = recognise(detail, typeExpr);
        if (!r.ok) { diag("island-payload-type", "bus.emit rejected for " + topic); return; }
        detail = r.value;
      }
      var list = ephemeral.get(topic);
      if (!list) { return; }
      var snapshot = list.slice();
      for (var i = 0; i < snapshot.length; i++) {
        try { snapshot[i](detail); } catch (e) { diag("listener-threw", topic, e); }
      }
    },
    on: function (topic, fn) {
      if (typeof topic !== "string" || typeof fn !== "function") { return function () {}; }
      if (totalSubscribers >= LIMITS.MAX_SUBSCRIBERS) {
        diag("bus-subscriber-cap", "subscriber cap reached, dropping on() for " + topic);
        return function () {};
      }
      var list = ephemeral.get(topic);
      if (!list) { list = []; ephemeral.set(topic, list); }
      list.push(fn);
      totalSubscribers += 1;
      var active = true;
      return function off() {
        if (!active) { return; }
        active = false;
        var idx = list.indexOf(fn);
        if (idx >= 0) { list.splice(idx, 1); totalSubscribers -= 1; }
      };
    }
  });

  // memoised store handles so repeated store(topic) returns a stable frozen object.
  var storeHandles = new Map();
  function storeFn(topic) {
    if (typeof topic !== "string") { diag("store-bad-topic", String(topic)); return makeStoreHandle("__invalid__"); }
    var h = storeHandles.get(topic);
    if (!h) { h = makeStoreHandle(topic); storeHandles.set(topic, h); }
    return h;
  }

  // --------------------------------------------------------------------------
  // §3. PERSISTED TOPICS (REQ-5006, CON-5004)
  // localStorage key: zetl:topic:<topic>. Values are UNTRUSTED -> recognise
  // before applying; on any failure apply the declared default. Cross-tab live
  // reflection via the `storage` event uses event.newValue directly (no
  // follow-up getItem — TOCTOU). null/empty newValue => default, no JSON parse.
  // --------------------------------------------------------------------------
  function storageKey(topic) { return "zetl:topic:" + topic; }

  // Apply the declared default (or leave unset if none) for a persisted topic.
  function applyDefault(topic) {
    if (declaredDefaults.has(topic)) {
      storeSet(topic, declaredDefaults.get(topic));
    }
  }

  // Recognise a raw JSON-text value for a persisted topic; apply or default.
  function applyPersistedText(topic, rawText) {
    var typeExpr = declaredTypes.get(topic);
    if (rawText === null || rawText === undefined || rawText === "") {
      applyDefault(topic); return; // "absent" => default, NO parse (CON-5004)
    }
    if (byteLength(rawText) > LIMITS.MAX_VALUE_BYTES) {
      diag("persisted-too-large", topic); applyDefault(topic); overwriteWithCurrent(topic); return;
    }
    var parsed;
    try { parsed = JSON.parse(rawText); } catch (_e) {
      diag("persisted-parse-fail", topic); applyDefault(topic); overwriteWithCurrent(topic); return;
    }
    // parsed is a JSON value (plain) — normalise records onto null-proto then recognise.
    var normalised = parsed;
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      normalised = normaliseClone(parsed, 0);
      if (normalised === REJECT) { diag("persisted-proto-fail", topic); applyDefault(topic); overwriteWithCurrent(topic); return; }
    }
    var r = typeExpr ? recognise(normalised, typeExpr) : { ok: true, value: normalised };
    if (!r.ok) { diag("persisted-type-fail", topic); applyDefault(topic); overwriteWithCurrent(topic); return; }
    storeSet(topic, r.value);
  }

  // Overwrite the bad localStorage entry with the now-current (default) value
  // so the next read is clean (CON-5004 "bad entry is overwritten").
  function overwriteWithCurrent(topic) {
    try {
      var s = stores.get(topic);
      if (s && s.hasValue && glob.localStorage) {
        glob.localStorage.setItem(storageKey(topic), JSON.stringify(s.value));
      }
    } catch (_e) { /* storage may be unavailable/full; ignore */ }
  }

  // Seed a persisted topic from localStorage at registration time.
  function seedPersisted(topic) {
    persistedTopics.add(topic);
    var raw = null;
    try { if (glob.localStorage) { raw = glob.localStorage.getItem(storageKey(topic)); } }
    catch (_e) { raw = null; }
    applyPersistedText(topic, raw);
  }

  // Persist a topic's value to localStorage when it changes (so other tabs see it).
  function persistOnChange(topic) {
    storeFn(topic).subscribe(function (value) {
      if (value === undefined) { return; }
      try { if (glob.localStorage) { glob.localStorage.setItem(storageKey(topic), JSON.stringify(value)); } }
      catch (_e) { /* ignore */ }
    });
  }

  // Live cross-tab reflection via the storage event (only installed once).
  var storageListenerInstalled = false;
  function installStorageListener() {
    if (storageListenerInstalled || typeof glob.addEventListener !== "function") { return; }
    storageListenerInstalled = true;
    glob.addEventListener("storage", function (event) {
      if (!event || typeof event.key !== "string") { return; }
      if (event.key.indexOf("zetl:topic:") !== 0) { return; }
      var topic = event.key.slice("zetl:topic:".length);
      if (!persistedTopics.has(topic)) { return; }
      // Use event.newValue DIRECTLY — never a follow-up getItem (TOCTOU, CON-5004).
      applyPersistedText(topic, event.newValue);
    });
  }

  // ==========================================================================
  // §4 + §5. CAPABILITY BRIDGE + CONTROLLED-ELEMENT RENDERER
  // (REQ-5016/5025/5029, CON-5006/CON-5007)
  // ==========================================================================

  // Hard-forbidden tag names (CON-5007) — excluded even if an allowlist lists them.
  var FORBIDDEN_TAGS = {
    "script": 1, "style": 1, "iframe": 1, "object": 1, "embed": 1, "applet": 1,
    "form": 1, "input": 1, "template": 1, "noscript": 1, "svg": 1, "math": 1,
    "foreignobject": 1, "base": 1, "meta": 1, "link": 1
  };

  // A conservative built-in default allowlist used when the mount node declares
  // none. Keys = allowed tags; values = extra (non-URL, non-aria) attrs.
  var DEFAULT_ALLOWLIST = {
    "div": ["class", "id", "tabindex", "title", "lang", "dir", "hidden"],
    "span": ["class", "id", "title", "lang", "dir"],
    "p": ["class", "id"],
    // `target`/`rel`/`download` are deliberately NOT allowed: an untrusted island could
    // emit `target="_blank"` (reverse tabnabbing, no enforced rel="noopener") or
    // `download` to force a drive-by save — and the server-side body sanitiser (CON-4902)
    // forbids them too. Keep the runtime renderer no more permissive than the static
    // fallback (BUG-9).
    "a": ["class", "id", "href", "title", "hreflang"],
    "button": ["class", "id", "type", "disabled", "title", "value"], // submit guarded below
    "ul": ["class", "id"], "ol": ["class", "id", "start", "reversed"], "li": ["class", "id", "value"],
    "h1": ["class"], "h2": ["class"], "h3": ["class"], "h4": ["class"], "h5": ["class"], "h6": ["class"],
    "strong": ["class"], "em": ["class"], "small": ["class"], "code": ["class"], "pre": ["class"],
    "img": ["class", "id", "src", "srcset", "alt", "width", "height", "loading", "decoding", "title"],
    "figure": ["class"], "figcaption": ["class"],
    "section": ["class", "id"], "article": ["class", "id"], "header": ["class"], "footer": ["class"],
    "nav": ["class"], "label": ["class", "for"], "table": ["class"], "thead": [], "tbody": [],
    "tr": ["class"], "th": ["class", "scope", "colspan", "rowspan"], "td": ["class", "colspan", "rowspan"],
    "br": [], "hr": ["class"], "blockquote": ["class", "cite"], "time": ["class", "datetime"]
  };

  // Always hard-forbidden attribute predicates (CON-5007).
  function isForbiddenAttr(name) {
    var n = name.toLowerCase();
    if (n.indexOf("on") === 0) { return true; }       // on* event handlers
    if (n === "is") { return true; }                   // customised built-in => script
    if (n === "srcdoc") { return true; }
    if (n === "name") { return true; }                 // form/target name shadowing
    if (n === "style") { return true; }                // inline CSS => exfil/clickjack
    if (n.indexOf("xlink:") === 0) { return true; }    // foreign-content URL vector
    if (n.indexOf("form") === 0) { return true; }      // formaction/formmethod/... guarded explicitly too
    return false;
  }

  // URL attributes whose VALUE must be scheme-allowlisted (CON-5007).
  var URL_ATTRS = { "href": 1, "src": 1, "srcset": 1, "poster": 1, "action": 1, "formaction": 1, "cite": 1 };

  // Scheme allowlist: https/http/mailto/relative. Reject javascript:/data:/blob:/
  // vbscript:/file: and protocol-relative //.
  // CANONICALISE a URL the way browsers do before scheme/origin checks: strip ALL ASCII
  // control chars AND whitespace and lowercase. `Java\tscript:` ≡ `javascript:`. Used by
  // BOTH isSafeUrl and isRemoteUrl so the scheme-allowlist and the egress-taint check see
  // the same value the browser will resolve.
  function canonUrl(url) {
    var canon = "";
    for (var i = 0; i < url.length; i++) {
      var ch = url.charCodeAt(i);
      if (ch <= 0x20 || ch === 0x7f) { continue; } // drop C0 controls, space, DEL
      canon += url[i];
    }
    return canon.toLowerCase();
  }

  function isSafeUrl(url) {
    if (typeof url !== "string") { return false; }
    if (url.length > 4096) { return false; }
    var canon = canonUrl(url);
    if (canon === "") { return true; } // empty / whitespace-only is a harmless reference
    // Reject scheme-relative //host and \\host (resolve off-origin).
    if (canon.indexOf("//") === 0 || canon.indexOf("\\\\") === 0) { return false; }
    // A scheme is [a-z][a-z0-9+.-]* ':' appearing before any '/', '?', '#'.
    var m = /^([a-z][a-z0-9+.\-]*):/.exec(canon);
    if (m) {
      var before = canon.slice(0, m[1].length);
      var hasPathSep = before.indexOf("/") >= 0 || before.indexOf("?") >= 0 || before.indexOf("#") >= 0;
      if (!hasPathSep) {
        var scheme = m[1];
        return scheme === "https" || scheme === "http" || scheme === "mailto";
      }
    }
    // No scheme => relative URL (path, query, fragment, or bare). Allowed.
    return true;
  }

  // srcset is a comma-separated list of "url descriptor" — validate each URL.
  function isSafeSrcset(value) {
    if (typeof value !== "string") { return false; }
    var parts = value.split(",");
    for (var i = 0; i < parts.length; i++) {
      var token = parts[i].trim();
      if (token === "") { continue; }
      var url = token.split(/\s+/)[0];
      if (!isSafeUrl(url)) { return false; }
    }
    return true;
  }

  // Build a usable allowlist object from a node-declared one, falling back to the
  // built-in default. Returns null if explicitly empty/malformed (=> render nothing).
  function resolveAllowlist(declared) {
    if (declared === undefined || declared === null) { return DEFAULT_ALLOWLIST; }
    if (typeof declared !== "object" || Array.isArray(declared)) { return null; } // malformed
    var keys = Object.keys(declared);
    if (keys.length === 0) { return null; } // empty => fail-closed: render nothing
    return declared;
  }

  // Render one tree node into a DOM node, or null if dropped (denied:render).
  // `ctx` tracks bounds + the readGranted egress-taint flag (CON-5007).
  function buildNode(tree, allowlist, ctx, depth) {
    // Bounds (fail-closed):
    if (depth > LIMITS.RENDER_MAX_DEPTH) { ctx.denied.push({ reason: "depth" }); return null; }
    if (ctx.nodeCount >= LIMITS.RENDER_MAX_NODES) { ctx.denied.push({ reason: "nodes" }); return null; }

    // Text node.
    if (typeof tree === "string") {
      ctx.textBytes += byteLength(tree);
      if (ctx.textBytes > LIMITS.RENDER_MAX_TEXT_BYTES) { ctx.denied.push({ reason: "text-bytes" }); return null; }
      ctx.nodeCount += 1;
      return ctx.doc.createTextNode(tree); // textContent path only
    }
    if (tree === null || typeof tree !== "object") { ctx.denied.push({ reason: "malformed" }); return null; }

    // Cycle detection across the structured-clone graph (visited set).
    if (ctx.visited.has(tree)) { ctx.denied.push({ reason: "cycle" }); return null; }
    ctx.visited.add(tree);

    var tag = (typeof tree.tag === "string") ? tree.tag.toLowerCase() : "";
    if (!tag) { ctx.visited.delete(tree); ctx.denied.push({ reason: "render" }); return null; }
    if (FORBIDDEN_TAGS[tag] === 1) { ctx.visited.delete(tree); ctx.denied.push({ reason: "render", tag: tag }); return null; }
    if (!Object.prototype.hasOwnProperty.call(allowlist, tag)) {
      ctx.visited.delete(tree); ctx.denied.push({ reason: "render", tag: tag }); return null;
    }

    var el = ctx.doc.createElement(tag);
    ctx.nodeCount += 1;
    // Stash the key for the reconciler (NOT a rendered attribute).
    if (typeof tree.key === "string") { el.__zKey = tree.key; }

    // Attributes.
    var props = tree.props;
    if (props && typeof props === "object" && !Array.isArray(props)) {
      var extraAllowed = allowlist[tag] || [];
      var pkeys = Object.keys(props);
      for (var i = 0; i < pkeys.length; i++) {
        var name = pkeys[i];
        var ln = name.toLowerCase();
        var val = props[name];
        if (isForbiddenAttr(ln)) { ctx.denied.push({ reason: "render", attr: ln, tag: tag }); continue; }
        // submit buttons are form vectors: drop type="submit".
        if (tag === "button" && ln === "type" && String(val).toLowerCase() === "submit") {
          ctx.denied.push({ reason: "render", attr: "type=submit" }); continue;
        }
        // aria-* and role always allowed (a11y, REQ-5020) — value is plain string.
        var isAria = (ln.indexOf("aria-") === 0) || ln === "role";
        var isUrl = URL_ATTRS[ln] === 1;
        var inExtra = extraAllowed.indexOf(ln) >= 0 || extraAllowed.indexOf(name) >= 0;
        if (!isAria && !isUrl && !inExtra) { ctx.denied.push({ reason: "render", attr: ln, tag: tag }); continue; }
        // URL value validation + egress-taint rule (CON-5007).
        if (isUrl) {
          var ok = (ln === "srcset") ? isSafeSrcset(val) : isSafeUrl(val);
          if (!ok) { ctx.denied.push({ reason: "render", attr: ln }); continue; }
          // Egress-taint: a read-granted island may emit ONLY same-origin/relative
          // URLs (no remote http(s)) so it can't beacon a subscribed value out.
          if (ctx.readGranted && isRemoteUrl(val, ln)) { ctx.denied.push({ reason: "render", attr: ln + ":egress" }); continue; }
        }
        // Coerce non-string scalar attribute values to string; reject objects.
        var sval;
        if (typeof val === "string") { sval = val; }
        else if (typeof val === "number" || typeof val === "boolean") { sval = String(val); }
        else { ctx.denied.push({ reason: "render", attr: ln }); continue; }
        // setAttribute ONLY — never DOM-property, never innerHTML.
        try { el.setAttribute(name, sval); } catch (_e) { ctx.denied.push({ reason: "render", attr: ln }); }
      }
    }

    // Children (recursive, breadth-bounded).
    var children = tree.children;
    if (Array.isArray(children)) {
      var limit = Math.min(children.length, LIMITS.RENDER_MAX_CHILDREN);
      if (children.length > LIMITS.RENDER_MAX_CHILDREN) { ctx.denied.push({ reason: "children" }); }
      for (var c = 0; c < limit; c++) {
        var childNode = buildNode(children[c], allowlist, ctx, depth + 1);
        if (childNode) { el.appendChild(childNode); }
      }
    }

    ctx.visited.delete(tree); // allow the same node object in a sibling position
    return el;
  }

  // A URL is "remote" if it has an http(s) scheme with an authority (host) that
  // is not the current origin. Relative/same-origin => not remote.
  function isRemoteUrl(url, attr) {
    if (attr === "srcset") {
      var parts = String(url).split(",");
      for (var i = 0; i < parts.length; i++) {
        var u = parts[i].trim().split(/\s+/)[0];
        if (u && isRemoteUrl(u, "src")) { return true; }
      }
      return false;
    }
    // CANONICALISE first (Codex P1): the browser strips control/whitespace before
    // resolving, so `h\tttps://evil` is a cross-origin request — the egress check MUST see
    // the canonical form, not the raw trim()med string, or it under-detects remote.
    var s = canonUrl(String(url));
    // Any non-http(s) scheme (mailto:, etc.) is an egress channel for a read-granted
    // island (it can encode a subscribed value); the egress-taint rule (CON-5007)
    // restricts such an island to same-origin/relative URLs only.
    if (/^[a-z][a-z0-9+.\-]*:/.test(s) && !/^https?:/.test(s)) { return true; }
    if (!/^https?:/.test(s)) { return false; } // relative => not remote
    try {
      var resolved = new URL(s, glob.location ? glob.location.href : "https://example.invalid/");
      return !!(glob.location && resolved.origin !== glob.location.origin);
    } catch (_e) { return true; } // unparseable absolute => treat as remote (reject)
  }

  // paintTree: build DOM ONLY from a declarative tree and replace hostEl's painted
  // content. Returns {ok, denied:[...]}. Missing/empty allowlist => render nothing.
  function paintTree(hostEl, tree, options) {
    options = options || {};
    if (!hostEl || !hostEl.ownerDocument) { return { ok: false, denied: [{ reason: "malformed" }] }; }
    var allowlist = resolveAllowlist(options.allowlist);
    if (!allowlist) { diag("denied:render", "missing/empty allowlist — rendering nothing"); return { ok: false, denied: [{ reason: "render" }] }; }
    var ctx = {
      doc: hostEl.ownerDocument,
      nodeCount: 0, textBytes: 0,
      visited: new Set(),
      denied: [],
      readGranted: !!options.readGranted
    };
    // Build into a detached fragment first, then reconcile.
    var newRoot = ctx.doc.createElement("div");
    newRoot.setAttribute("data-island-root", "1");
    if (Array.isArray(tree)) {
      var lim = Math.min(tree.length, LIMITS.RENDER_MAX_CHILDREN);
      for (var i = 0; i < lim; i++) {
        var n = buildNode(tree[i], allowlist, ctx, 1);
        if (n) { newRoot.appendChild(n); }
      }
    } else {
      var node = buildNode(tree, allowlist, ctx, 1);
      if (node) { newRoot.appendChild(node); }
    }
    // §5 reconciler: diff against existing painted root if present.
    var existing = hostEl.querySelector(':scope > [data-island-root="1"]');
    if (existing) { reconcileChildren(existing, newRoot); }
    else { hostEl.appendChild(newRoot); }
    return { ok: ctx.denied.length === 0, denied: ctx.denied };
  }

  // --------------------------------------------------------------------------
  // §5 KEYED RECONCILER (REQ-5029). Minimal DOM mutations; preserves focus/scroll/
  // uncontrolled input state on keyed nodes. Keyed match by __zKey; keyless by
  // position. Not a framework — a single tiny diff, after Shopify Remote DOM.
  // --------------------------------------------------------------------------
  function reconcileChildren(oldParent, newParent) {
    var oldKids = Array.prototype.slice.call(oldParent.childNodes);
    var newKids = Array.prototype.slice.call(newParent.childNodes);

    // Index old keyed children for move/update matching.
    var oldByKey = Object.create(null);
    for (var i = 0; i < oldKids.length; i++) {
      var ok = oldKids[i].nodeType === 1 ? oldKids[i].__zKey : null;
      if (ok) { oldByKey[ok] = oldKids[i]; }
    }

    var usedOld = new Set();
    var resultNodes = []; // the desired final ordered list of old/new DOM nodes
    for (var j = 0; j < newKids.length; j++) {
      var nk = newKids[j];
      var matched = null;
      if (nk.nodeType === 1 && nk.__zKey && oldByKey[nk.__zKey] !== undefined) {
        var cand = oldByKey[nk.__zKey];
        if (!usedOld.has(cand) && cand.nodeName === nk.nodeName) { matched = cand; }
      }
      if (!matched && !(nk.nodeType === 1 && nk.__zKey)) {
        // keyless: try positional match against same-position old keyless node.
        var pos = oldKids[j];
        if (pos && !usedOld.has(pos) && pos.nodeType === nk.nodeType &&
            (pos.nodeType !== 1 || (pos.nodeName === nk.nodeName && !pos.__zKey))) {
          matched = pos;
        }
      }
      if (matched) {
        usedOld.add(matched);
        patchNode(matched, nk);
        resultNodes.push(matched);
      } else {
        resultNodes.push(nk); // fresh node, will be inserted
      }
    }

    // Remove old nodes that were not reused.
    for (var r = 0; r < oldKids.length; r++) {
      if (!usedOld.has(oldKids[r])) {
        if (oldKids[r].parentNode === oldParent) { oldParent.removeChild(oldKids[r]); }
      }
    }
    // Re-order / insert to match resultNodes order with minimal moves.
    for (var k = 0; k < resultNodes.length; k++) {
      var want = resultNodes[k];
      var atPos = oldParent.childNodes[k];
      if (atPos !== want) { oldParent.insertBefore(want, atPos || null); }
    }
  }

  // Patch a matched element in place: sync attributes + recurse into children.
  // Preserves the live DOM node (and thus focus/scroll/uncontrolled input).
  function patchNode(oldNode, newNode) {
    if (oldNode.nodeType === 3) { // text
      if (oldNode.nodeValue !== newNode.nodeValue) { oldNode.nodeValue = newNode.nodeValue; }
      return;
    }
    if (oldNode.nodeType !== 1) { return; }
    // Carry the key forward.
    oldNode.__zKey = newNode.__zKey;
    // Remove attributes no longer present.
    var oldAttrs = Array.prototype.slice.call(oldNode.attributes);
    for (var i = 0; i < oldAttrs.length; i++) {
      var an = oldAttrs[i].name;
      if (an === "data-island-root") { continue; }
      if (!newNode.hasAttribute(an)) { oldNode.removeAttribute(an); }
    }
    // Set/update new attributes (already validated when newNode was built).
    var newAttrs = Array.prototype.slice.call(newNode.attributes);
    for (var j = 0; j < newAttrs.length; j++) {
      var na = newAttrs[j];
      if (oldNode.getAttribute(na.name) !== na.value) { oldNode.setAttribute(na.name, na.value); }
    }
    // Recurse.
    reconcileChildren(oldNode, newNode);
  }

  // ==========================================================================
  // §4 BRIDGE — Worker transport (default, REQ-5025/5016, CON-5006). The PARENT
  // is the SOLE reference monitor. Grant table keyed on the Worker object.
  // On EVERY inbound message validate (a) known worker handle, (b) (topic,
  // direction) granted, (c) payload conforms (recognise). Failure => post
  // {op:"denied", reason}, never reaches the bus.
  // ==========================================================================

  // Per-page worker spawn counter (NFR-5002 re-creation cap).
  var workerSpawns = 0;

  // grant table: Map<Worker, IslandRecord>
  //   IslandRecord = { grants:Set<"dir:topic"|"render">, types:Map, paints:bool,
  //                    readGranted:bool, hostEl, subs:[unsub], relayState:Map,
  //                    debounce:Map, allowlist }
  var islandsByWorker = new Map();

  function grantKey(direction, topic) { return direction + ":" + topic; }

  // Build the grant set + types from the descriptor (REQ-5016/CON-5002).
  function buildGrants(grantsArr, typesObj) {
    var grants = new Set();
    var types = new Map();
    var paints = false;
    var readGranted = false; // any trusted (non-content:) subscribe => egress-taint
    if (Array.isArray(grantsArr)) {
      for (var i = 0; i < grantsArr.length; i++) {
        var g = grantsArr[i];
        if (!g || typeof g !== "object") { continue; }
        var dir = g.direction;
        if (dir === "render") { paints = true; grants.add("render"); continue; }
        if (typeof g.topic !== "string") { continue; }
        if (dir !== "publish" && dir !== "emit" && dir !== "subscribe") { continue; }
        var isContent = g.topic.indexOf("content:") === 0;
        // Trusted (non-content:) topics: subscribe-only (REQ-5016). Drop a forged
        // publish/emit grant for a trusted topic.
        if (!isContent && dir !== "subscribe") { diag("island-capability-ungranted", "trusted publish refused: " + g.topic); continue; }
        if (!isContent && dir === "subscribe") { readGranted = true; }
        grants.add(grantKey(dir, g.topic));
      }
    }
    if (typesObj && typeof typesObj === "object") {
      var keys = Object.keys(typesObj);
      for (var k = 0; k < keys.length; k++) {
        var t = typesObj[keys[k]];
        if (typeof t === "string" && parseTypeExpr(t)) { types.set(keys[k], t); }
      }
    }
    return { grants: grants, types: types, paints: paints, readGranted: readGranted };
  }

  // Mount a content island in a Worker (REQ-5025). `grants`/`types`/etc come from
  // the descriptor. Returns the IslandRecord (or null on failure).
  function mountWorkerIsland(hostEl, workerUrl, descriptor) {
    descriptor = descriptor || {};
    if (workerSpawns >= LIMITS.MAX_WORKER_SPAWNS) { diag("worker-spawn-cap", "refusing new worker"); return null; }
    if (typeof Worker === "undefined") { diag("no-worker-support"); return null; }
    // Worker URL must be same-origin/relative (REQ-5025/5026 — no remote code pull).
    if (!isSafeUrl(workerUrl) || isRemoteUrl(workerUrl, "src")) { diag("worker-url-rejected", workerUrl); return null; }

    var built = buildGrants(descriptor.grants, descriptor.types);
    var worker;
    try { worker = new Worker(workerUrl, { type: "module" }); }
    catch (e) { diag("worker-create-failed", workerUrl, e); return null; }
    workerSpawns += 1;

    var rec = {
      worker: worker,
      grants: built.grants,
      types: built.types,
      paints: built.paints,
      readGranted: built.readGranted,
      hostEl: hostEl,
      allowlist: descriptor.allowlist,
      subs: [],                 // unsubscribe handles to release on teardown (REQ-5017)
      relayLast: new Map(),     // topic -> last delivered value (value-change-only, REQ-5021)
      relayTimers: new Map(),   // topic -> debounce timer id (cancelled on teardown)
      torn: false
    };
    islandsByWorker.set(worker, rec);

    // Register declared types on the global bus so in-realm + bridge agree.
    built.types.forEach(function (typeExpr, topic) { registerType(topic, typeExpr); });

    worker.onmessage = function (event) { handleWorkerMessage(worker, event); };
    worker.onerror = function (e) { diag("worker-error", workerUrl, (e && e.message) || ""); };

    // Send a benign boot config (author props). NOT a capability — just data.
    try {
      worker.postMessage({ op: "boot", props: descriptor.props !== undefined ? descriptor.props : null });
    } catch (_e) { /* ignore */ }

    return rec;
  }

  // The reference monitor for inbound Worker messages (CON-5006).
  function handleWorkerMessage(worker, event) {
    // (a) Known handle? Handle-miss => silent drop (covers torn-down workers,
    // REQ-5017 — the load-bearing in-flight guard).
    var rec = islandsByWorker.get(worker);
    if (!rec || rec.torn) { return; }

    var msg = event && event.data;
    if (!msg || typeof msg !== "object") { return; }
    // Read ONLY own-enumerable data; reject poisoned-prototype envelopes.
    var proto = Object.getPrototypeOf(msg);
    if (proto !== Object.prototype && proto !== null) { return; }
    var op = msg.op;
    if (typeof op !== "string") { return; }

    switch (op) {
      case "publish": return bridgePublish(rec, msg, false);
      case "emit":    return bridgePublish(rec, msg, true);
      case "subscribe":   return bridgeSubscribe(rec, msg);
      case "unsubscribe": return bridgeUnsubscribe(rec, msg);
      case "render":  return bridgeRender(rec, msg);
      default:
        deny(rec, { reason: "malformed" });
    }
  }

  function postToIsland(rec, message) {
    if (rec.torn) { return; }
    // Outbound closure re-checks handle membership before posting (REQ-5017).
    if (!islandsByWorker.has(rec.worker)) { return; }
    try { rec.worker.postMessage(message); } catch (_e) { /* worker gone */ }
  }

  function deny(rec, info) {
    postToIsland(rec, { op: "denied", topic: info.topic, node: info.node, reason: info.reason });
  }

  // publish/emit: validate topic, grant, payload type, then forward to the bus.
  function bridgePublish(rec, msg, isEphemeral) {
    var topic = msg.topic;
    if (typeof topic !== "string") { return deny(rec, { reason: "malformed" }); }
    var dir = isEphemeral ? "emit" : "publish";
    // (b) (topic, direction) granted? Content islands publish only content: topics
    // (the grant table only contains content: publish/emit keys by construction).
    if (!rec.grants.has(grantKey(dir, topic))) {
      diag("island-capability-denied", dir + " " + topic);
      return deny(rec, { topic: topic, reason: "ungranted" });
    }
    // (c) payload conforms? Normalise the structured-clone graph first (CON-5006).
    var normalised = normaliseClone(msg.value, 0);
    if (normalised === REJECT) { diag("island-payload-type", topic); return deny(rec, { topic: topic, reason: "type" }); }
    // (c) payload conforms to the declared type? At the bridge (untrusted Worker)
    // recognition is a SECURITY control, not best-effort (REQ-5013): a granted topic with
    // NO declared type is fail-closed, never accept-raw (BUG-10 — the previous
    // "no type → forward" branch was fail-open at a trust boundary).
    var typeExpr = rec.types.get(topic) || declaredTypes.get(topic);
    if (!typeExpr) { diag("island-payload-type", topic + " (no declared type)"); return deny(rec, { topic: topic, reason: "type" }); }
    var r = recognise(normalised, typeExpr);
    if (!r.ok) { diag("island-payload-type", topic); return deny(rec, { topic: topic, reason: "type" }); }
    normalised = r.value;
    if (isEphemeral) { busApi.emit(topic, normalised); }
    else { storeSet(topic, normalised); }
  }

  // subscribe: answer ack, then value-change-only updates (REQ-5021 + seq REQ-5030).
  function bridgeSubscribe(rec, msg) {
    var topic = msg.topic;
    if (typeof topic !== "string") { return deny(rec, { reason: "malformed" }); }
    if (!rec.grants.has(grantKey("subscribe", topic))) {
      diag("island-capability-denied", "subscribe " + topic);
      return deny(rec, { topic: topic, reason: "ungranted" });
    }
    if (totalSubscribers >= LIMITS.MAX_SUBSCRIBERS) { return deny(rec, { topic: topic, reason: "cap-exceeded" }); }
    // ack first.
    postToIsland(rec, { op: "ack", topic: topic, cseq: msg.cseq });
    // Subscribe with a value-change-only relay (dedupe against last delivered).
    var unsub = storeFn(topic).subscribe(function (value, seq) {
      if (rec.torn) { return; }
      if (rec.relayLast.has(topic) && structEqual(rec.relayLast.get(topic), value)) { return; }
      rec.relayLast.set(topic, value);
      // Debounce to a security-relevant interval (REQ-5021); coalesce bursts.
      var prev = rec.relayTimers.get(topic);
      if (prev) { clearTimeout(prev); }
      var tid = setTimeout(function () {
        rec.relayTimers.delete(topic);
        // Re-check handle membership immediately before posting (REQ-5017).
        if (rec.torn || !islandsByWorker.has(rec.worker)) { return; }
        postToIsland(rec, { op: "update", topic: topic, value: value, seq: seq });
      }, 50); // provisional debounce interval (IMPL-050)
      rec.relayTimers.set(topic, tid);
    });
    rec.subs.push(unsub);
  }

  function bridgeUnsubscribe(rec, msg) {
    // v1: release ALL relays for the topic (we don't track per-topic unsub handles
    // separately; teardown releases all — this is a best-effort early release).
    var topic = msg.topic;
    if (typeof topic === "string") {
      var t = rec.relayTimers.get(topic);
      if (t) { clearTimeout(t); rec.relayTimers.delete(topic); }
      rec.relayLast.delete(topic);
    }
  }

  // render: paint a controlled element tree (CON-5007). Requires the render grant.
  function bridgeRender(rec, msg) {
    if (!rec.paints) {
      diag("island-capability-denied", "render (no paints grant)");
      return deny(rec, { reason: "render" });
    }
    if (!rec.hostEl) { return; }
    // Normalise the tree's structured-clone graph defensively at the top: the
    // CON-5007 recogniser inside paintTree handles per-node checks + cycles.
    var result = paintTree(rec.hostEl, msg.tree, {
      allowlist: rec.allowlist,
      readGranted: rec.readGranted
    });
    if (result.denied && result.denied.length) {
      for (var i = 0; i < result.denied.length; i++) {
        deny(rec, { reason: "render", node: result.denied[i] });
      }
    }
  }

  // Teardown an island (REQ-5017): revoke grant, cancel timers, terminate worker,
  // release bus subscriptions. In-flight messages drop via the handle-miss guard.
  function teardownIsland(rec) {
    if (!rec || rec.torn) { return; }
    rec.torn = true;
    // (a) revoke grant — delete the Map entry so future messages handle-miss.
    islandsByWorker.delete(rec.worker);
    // cancel pending debounced relay timers.
    rec.relayTimers.forEach(function (tid) { try { clearTimeout(tid); } catch (_e) {} });
    rec.relayTimers.clear();
    // (b) release bus subscriptions.
    for (var i = 0; i < rec.subs.length; i++) { try { rec.subs[i](); } catch (_e) {} }
    rec.subs.length = 0;
    // dispose realm.
    try { if (rec.worker) { rec.worker.terminate(); } } catch (_e) {}
  }

  // --------------------------------------------------------------------------
  // IFRAME ESCAPE HATCH (REQ-5015/5016 iframe transport) — STUBBED for v1.
  // The Worker transport above is the v1 default and is complete. The iframe
  // full-DOM path requires the child-ready-first MessageChannel bootstrap:
  // Map<WindowProxy, Island> keyed by iframe.contentWindow BEFORE listening; the
  // child posts zetl:ready over window; the parent routes SOLELY by event.source
  // WindowProxy identity (hit => transfer port2, miss => no-op, payload ignored);
  // thereafter handle = port1 object identity (WeakMap<port1, Island>); never
  // origin/source for per-message auth. Same grant table + same CON-5006/CON-5007
  // recognisers apply. See Threat I/J.
  // TODO(IMPL-050): implement mountIframeIsland() with the above bootstrap.
  // --------------------------------------------------------------------------
  function mountIframeIsland(hostEl, iframeUrl, descriptor) {
    diag("iframe-transport-stubbed", "iframe escape hatch not implemented in v1; use the Worker transport");
    return null;
  }

  // ==========================================================================
  // §6 HYDRATION (REQ-5024/5003). Scan for data-z mount nodes + descriptor,
  // apply the hydrate strategy, defer Worker creation for lazy strategies,
  // idempotent (mark hydrated). Re-hydration binds only fresh nodes.
  // ==========================================================================

  var HYDRATED_ATTR = "data-island-hydrated";

  // Read + parse a node's island descriptor from data-* attributes.
  function readDescriptor(el) {
    var name = el.getAttribute("data-island") || el.getAttribute("data-z");
    var workerUrl = el.getAttribute("data-island-worker");
    var iframeUrl = el.getAttribute("data-island-iframe");
    var strategy = el.getAttribute("data-island-hydrate") || "load";
    var grants = safeJson(el.getAttribute("data-island-grants"), []);
    var types = safeJson(el.getAttribute("data-island-types"), {});
    var allowlist = safeJson(el.getAttribute("data-island-allowlist"), undefined);
    var paints = el.getAttribute("data-island-paints") === "true";
    // Optional <script type="application/json" data-island-props> child.
    var props = null;
    var propScript = el.querySelector('script[type="application/json"][data-island-props]');
    if (propScript) { props = safeJson(propScript.textContent, null); }
    return {
      name: name, workerUrl: workerUrl, iframeUrl: iframeUrl, strategy: strategy,
      grants: grants, types: types, allowlist: allowlist, paints: paints, props: props
    };
  }

  function safeJson(text, fallback) {
    if (typeof text !== "string" || text === "") { return fallback; }
    try { return JSON.parse(text); } catch (_e) { return fallback; }
  }

  // Map of hostEl -> IslandRecord for lifecycle teardown on subtree removal.
  var mountedByEl = new WeakMap();

  // Hydrate a single node now (create the realm, mount the bridge).
  function hydrateNode(el) {
    if (el.getAttribute(HYDRATED_ATTR) === "1") { return; } // idempotent (REQ-5003)
    el.setAttribute(HYDRATED_ATTR, "1");
    var d = readDescriptor(el);
    if (!d.name) { return; } // not an island mount node

    var rec = null;
    if (d.workerUrl) {
      rec = mountWorkerIsland(el, d.workerUrl, d);
    } else if (d.iframeUrl) {
      rec = mountIframeIsland(el, d.iframeUrl, d); // STUB (v1)
    } else {
      // Trusted in-realm island with no separate realm: it loads its own
      // <script type="module"> and talks to window.zetl directly. Nothing for the
      // host to mount here beyond marking it hydrated. (The script self-binds.)
      return;
    }
    if (rec) { mountedByEl.set(el, rec); }
  }

  // Apply the hydrate strategy: defer realm creation for lazy strategies (REQ-5024).
  function scheduleHydration(el) {
    if (el.getAttribute(HYDRATED_ATTR) === "1") { return; }
    var strategy = el.getAttribute("data-island-hydrate") || "load";

    if (strategy === "load") { hydrateNode(el); return; }

    if (strategy === "idle") {
      if (typeof glob.requestIdleCallback === "function") { glob.requestIdleCallback(function () { hydrateNode(el); }); }
      else { setTimeout(function () { hydrateNode(el); }, 200); } // fallback timeout
      return;
    }

    var visMatch = /^visible(?:\(([^)]*)\))?$/.exec(strategy);
    if (visMatch) {
      var rootMargin = visMatch[1] ? visMatch[1].trim() : "0px";
      if (typeof IntersectionObserver === "function") {
        var io = new IntersectionObserver(function (entries) {
          for (var i = 0; i < entries.length; i++) {
            if (entries[i].isIntersecting) { io.disconnect(); hydrateNode(el); break; }
          }
        }, { rootMargin: rootMargin });
        io.observe(el);
      } else { hydrateNode(el); } // no IO support => hydrate immediately (still PE-safe)
      return;
    }

    var mediaMatch = /^media\((.*)\)$/.exec(strategy);
    if (mediaMatch) {
      var query = mediaMatch[1];
      if (typeof glob.matchMedia === "function") {
        var mql = glob.matchMedia(query);
        if (mql.matches) { hydrateNode(el); }
        else {
          var onChange = function () { if (mql.matches) { remove(); hydrateNode(el); } };
          var remove = function () {
            if (mql.removeEventListener) { mql.removeEventListener("change", onChange); }
            else if (mql.removeListener) { mql.removeListener(onChange); }
          };
          if (mql.addEventListener) { mql.addEventListener("change", onChange); }
          else if (mql.addListener) { mql.addListener(onChange); }
        }
      } else { hydrateNode(el); }
      return;
    }

    diag("island-hydrate-invalid", strategy);
    hydrateNode(el); // unknown strategy => fail safe to immediate (still PE-safe)
  }

  // hydrateAll: scan the (optionally scoped) DOM for island mount nodes and
  // schedule each. Idempotent; on SPA re-hydration only fresh nodes bind.
  function hydrateAll(root) {
    if (typeof document === "undefined") { return; }
    var scope = root || document;
    // First register persisted-topic types/defaults declared on mount nodes so
    // the bus + storage listener are wired before any island runs.
    registerPersistedFromDom(scope);
    installStorageListener();

    var nodes;
    try { nodes = scope.querySelectorAll("[data-z]"); } catch (_e) { return; }
    for (var i = 0; i < nodes.length; i++) {
      var el = nodes[i];
      if (el.getAttribute(HYDRATED_ATTR) === "1") { continue; } // never double-bind
      scheduleHydration(el);
    }
    // Observe subtree removals to drive lifecycle teardown (REQ-5017).
    installMutationObserver(scope);
  }

  // Persisted-topic registration from mount-node descriptors. A persisted topic
  // is declared via data-island-types + a default carried in data-island-persist.
  //   data-island-persist='{"<topic>":<defaultJsonValue>}'
  function registerPersistedFromDom(scope) {
    var nodes;
    try { nodes = scope.querySelectorAll("[data-island-types],[data-island-persist]"); } catch (_e) { return; }
    for (var i = 0; i < nodes.length; i++) {
      var el = nodes[i];
      var types = safeJson(el.getAttribute("data-island-types"), {});
      var tkeys = Object.keys(types);
      for (var t = 0; t < tkeys.length; t++) {
        if (typeof types[tkeys[t]] === "string") { registerType(tkeys[t], types[tkeys[t]]); }
      }
      var persist = safeJson(el.getAttribute("data-island-persist"), {});
      var pkeys = Object.keys(persist);
      for (var p = 0; p < pkeys.length; p++) {
        var topic = pkeys[p];
        if (persistedTopics.has(topic)) { continue; }
        // Recognise the declared default against the topic's type before storing.
        var typeExpr = declaredTypes.get(topic);
        var def = persist[topic];
        var norm = def;
        if (def && typeof def === "object" && !Array.isArray(def)) {
          norm = normaliseClone(def, 0);
          if (norm === REJECT) { diag("island-persisted-no-default", topic); continue; }
        }
        var dr = typeExpr ? recognise(norm, typeExpr) : { ok: true, value: norm };
        if (!dr.ok) { diag("island-persisted-no-default", topic); continue; }
        declaredDefaults.set(topic, dr.value);
        seedPersisted(topic);     // read localStorage (untrusted) -> recognise/default
        persistOnChange(topic);   // write-through so other tabs reflect changes
      }
    }
  }

  // MutationObserver: when an island's host subtree is removed, tear it down.
  var moInstalled = false;
  function installMutationObserver(scope) {
    if (moInstalled || typeof MutationObserver === "undefined" || typeof document === "undefined") { return; }
    moInstalled = true;
    var mo = new MutationObserver(function (records) {
      for (var i = 0; i < records.length; i++) {
        var removed = records[i].removedNodes;
        for (var j = 0; j < removed.length; j++) {
          var node = removed[j];
          if (node.nodeType !== 1) { continue; }
          teardownRemovedSubtree(node);
        }
      }
    });
    try { mo.observe(document.documentElement || document.body || scope, { childList: true, subtree: true }); }
    catch (_e) { /* ignore */ }
  }

  function teardownRemovedSubtree(node) {
    // The removed node itself, or any island host within it.
    var hosts = [];
    if (node.hasAttribute && node.hasAttribute(HYDRATED_ATTR)) { hosts.push(node); }
    if (node.querySelectorAll) {
      var inner = node.querySelectorAll("[" + HYDRATED_ATTR + "]");
      for (var i = 0; i < inner.length; i++) { hosts.push(inner[i]); }
    }
    for (var h = 0; h < hosts.length; h++) {
      var rec = mountedByEl.get(hosts[h]);
      if (rec) { teardownIsland(rec); mountedByEl.delete(hosts[h]); }
    }
  }

  // ==========================================================================
  // INSTALL window.zetl (single instance) + DEEP-FREEZE (REQ-5019).
  // Internal state lives in the closures above, NOT on this object, so freezing
  // the surface cannot be circumvented to mutate retained state (Threat L).
  // ==========================================================================
  function deepFreeze(obj) {
    if (obj && (typeof obj === "object" || typeof obj === "function")) {
      Object.getOwnPropertyNames(obj).forEach(function (name) {
        try { deepFreeze(obj[name]); } catch (_e) {}
      });
      try { Object.freeze(obj); } catch (_e) {}
    }
    return obj;
  }

  if (!alreadyInstalled) {
    var zetl = {
      store: storeFn,
      bus: busApi,
      // Host-only helpers exposed for trusted islands / tests. (Frozen.)
      recognise: recognise,
      paintTree: paintTree,
      mountWorkerIsland: mountWorkerIsland,
      hydrate: hydrateAll,
      version: "1.0.0-spec050"
    };
    deepFreeze(zetl);
    try {
      Object.defineProperty(glob, "zetl", { value: zetl, writable: false, configurable: false, enumerable: true });
    } catch (_e) {
      // Some environments disallow defineProperty on window; fall back.
      glob.zetl = zetl;
    }
  }

  // --------------------------------------------------------------------------
  // Auto-hydrate on DOMContentLoaded if a document exists (guard non-browser).
  // --------------------------------------------------------------------------
  if (typeof document !== "undefined") {
    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", function () { hydrateAll(); });
    } else {
      hydrateAll(); // document already parsed
    }
  }
})();
