// #73 regression test for themes/default/static/spa.js, run under node with a
// minimal DOM stub. On a static host with SPA-style 404 fallback (e.g.
// Cloudflare Pages with no 404.html in the build), fetch() of ANY unknown
// path answers 200 + the homepage document, so navigate()'s `r.ok` check
// passes and — before the fix — swap() rendered the homepage under the
// phantom URL. The fix verifies the fetched document's <body data-slug>
// self-identification against the URL before swapping, and hard-navigates
// (location.href) on mismatch so the host's real behaviour stays visible.
//
// Run via `tests/spa_js.rs` (cargo) or `node tests/js/spa_fallback.test.mjs`.

import { readFileSync } from "node:fs";

let failures = 0;
function ok(cond, msg) {
  if (!cond) { failures++; console.error("FAIL:", msg); }
  else { console.log("ok  -", msg); }
}

const VOLATILE_SEL = "[data-zetl-volatile]";
const BOOT_URL = "https://site.example/guide/intro/index.html";

// ---- stub documents -------------------------------------------------------
// The live document boots on a real page whose data-slug matches its URL —
// that pair is what spa.js uses to locate the site root for verifying
// documents that claim the empty (vault-index) slug.
let hardNavigatedTo = null;
let swapped = false;

function stubVolatileRoot() {
  return {
    replaceWith() { swapped = true; },
    querySelectorAll() { return []; },
    getAttribute() { return null; },
  };
}

function makeDoc(slug, title) {
  return {
    title: title || "stub",
    body: { getAttribute(k) { return k === "data-slug" ? slug : null; } },
    querySelector(sel) {
      return sel === VOLATILE_SEL || sel === "main" ? stubVolatileRoot() : null;
    },
  };
}

// ---- minimal browser globals ----------------------------------------------
globalThis.location = {
  get href() { return BOOT_URL; },
  set href(v) { hardNavigatedTo = v; },
  origin: "https://site.example",
};
globalThis.history = { pushState() {} };
globalThis.CustomEvent = class { constructor(type, init) { this.type = type; this.detail = init && init.detail; } };
globalThis.window = {
  __zetlSpaMounted: false,
  addEventListener() {},
  dispatchEvent() { return true; },
  scrollTo() {},
};
globalThis.document = {
  title: "boot",
  body: { getAttribute(k) { return k === "data-slug" ? "guide/intro" : null; } },
  querySelectorAll() { return []; },
  querySelector(sel) {
    return sel === VOLATILE_SEL || sel === "main" ? stubVolatileRoot() : null;
  },
  addEventListener() {},
  getElementById() { return null; },
};

// navigate() parses the fetched HTML with DOMParser; the HTML string here is
// just a routing key into the stubbed parse results below.
const PARSE_RESULTS = {};
globalThis.DOMParser = class {
  parseFromString(html) { return PARSE_RESULTS[html]; }
};

let nextResponseHtml = null;
globalThis.fetch = function () {
  return Promise.resolve({ ok: true, text: () => Promise.resolve(nextResponseHtml) });
};

// ---- load spa.js -----------------------------------------------------------
const code = readFileSync(new URL("../../themes/default/static/spa.js", import.meta.url), "utf8");
(0, eval)(code);

// spa.js keeps navigate() private — drive it through the popstate/click paths?
// Neither is reachable without a full event stub, so re-invoke fetch the way
// navigate does: through the captured click listener. document.addEventListener
// above discarded it, so instead we re-eval a tiny driver that mirrors the
// integration point: we simulate by calling the code path via a synthetic
// click. Simpler and just as faithful: capture the click listener.
let clickListener = null;
globalThis.document.addEventListener = (type, fn) => { if (type === "click") clickListener = fn; };
globalThis.window.__zetlSpaMounted = false; // allow a second mount to capture the listener
(0, eval)(code);
ok(typeof clickListener === "function", "click listener captured on mount");

function clickTo(url) {
  hardNavigatedTo = null;
  swapped = false;
  clickListener({
    defaultPrevented: false,
    button: 0,
    preventDefault() {},
    target: {
      closest() {
        return {
          getAttribute(k) { return k === "href" ? url : null; },
          hasAttribute() { return false; },
          dataset: {},
          target: "",
          closest() { return null; },
          href: url,
        };
      },
    },
  });
  // navigate() resolves fetch asynchronously — flush microtasks.
  return new Promise((r) => setTimeout(r, 0));
}

// ---- case 1: host 404-fallback masks a broken link -------------------------
// The homepage document (data-slug="") comes back with HTTP 200 for a phantom
// deep URL. Before the fix this swapped silently; now it must hard-navigate.
const PHANTOM = "https://site.example/papers/concepts/adaptive-protocols/index.html";
nextResponseHtml = "HOMEPAGE_FALLBACK";
PARSE_RESULTS["HOMEPAGE_FALLBACK"] = makeDoc("", "index — Site");
await clickTo(PHANTOM);
ok(!swapped, "fallback homepage document is NOT swapped in at a phantom URL");
ok(hardNavigatedTo === PHANTOM,
   "mismatching document triggers a hard navigation to the requested URL");

// ---- case 2: legitimate navigation still swaps -----------------------------
const REAL = "https://site.example/papers/security/langsec/index.html";
nextResponseHtml = "REAL_PAGE";
PARSE_RESULTS["REAL_PAGE"] = makeDoc("papers/security/langsec", "LangSec — Site");
await clickTo(REAL);
ok(swapped, "document whose data-slug matches the URL is swapped normally");
ok(hardNavigatedTo === null, "matching navigation does not hard-navigate");

// ---- case 3: homepage document at the real site root is accepted -----------
const HOME = "https://site.example/index.html";
nextResponseHtml = "REAL_HOME";
PARSE_RESULTS["REAL_HOME"] = makeDoc("", "index — Site");
await clickTo(HOME);
ok(swapped, "vault index (empty slug) is accepted at the site root");
ok(hardNavigatedTo === null, "site-root navigation does not hard-navigate");

// ---- case 4: docs without the marker keep the old behaviour ----------------
const LEGACY = "https://site.example/custom/thing/index.html";
nextResponseHtml = "LEGACY_DOC";
PARSE_RESULTS["LEGACY_DOC"] = {
  title: "legacy",
  body: { getAttribute() { return null; } },
  querySelector(sel) {
    return sel === VOLATILE_SEL || sel === "main" ? stubVolatileRoot() : null;
  },
};
await clickTo(LEGACY);
ok(swapped, "document without a data-slug claim is swapped (no verdict, no regression)");

// ---- case 5: page-history documents claim their page's slug ----------------
const HIST = "https://site.example/guide/intro/_history.html";
nextResponseHtml = "HIST_DOC";
PARSE_RESULTS["HIST_DOC"] = makeDoc("guide/intro", "History — Site");
await clickTo(HIST);
ok(swapped, "page-history document (slug + /_history URL) is accepted");

if (failures > 0) {
  console.error(`\n${failures} spa.js fallback check(s) failed`);
  process.exit(1);
}
console.log("\nall spa.js fallback checks passed");
