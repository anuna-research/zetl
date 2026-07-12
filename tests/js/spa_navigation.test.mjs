// #70 regression test for themes/default/static/spa.js, run under node with a
// minimal DOM stub. The SPA shell keeps the sidebar persistent across pushState
// navigations, but the sidebar's links are rendered once with page-relative
// `root_path` hrefs. Before the fix, a pushState to a deeper page left those
// stale relative hrefs to resolve against an ever-deeper base, compounding the
// URL path on the next click. The fix freezes shell links to absolute URLs on
// mount. This harness proves: (1) shell links resolve identically after a
// deeper pushState, and (2) links inside [data-zetl-volatile] are left relative.
//
// Run via `tests/spa_js.rs` (cargo) or `node tests/js/spa_navigation.test.mjs`.

import { readFileSync } from "node:fs";

let failures = 0;
function ok(cond, msg) {
  if (!cond) { failures++; console.error("FAIL:", msg); }
  else { console.log("ok  -", msg); }
}

// ---- mutable document base (changes when we simulate pushState) ----
const VOLATILE_SEL = "[data-zetl-volatile]";
let CURRENT_URL = "https://site.example/guide/intro/";

// A stub anchor whose `.href` getter resolves its literal href attribute
// against the *current* document base — exactly like a real <a> element. This
// is what makes the compounding bug observable: a relative href re-resolves
// every time the base changes.
function makeAnchor(href, inVolatile) {
  return {
    attrs: { href: String(href) },
    inVolatile: !!inVolatile,
    getAttribute(k) { return k === "href" ? this.attrs.href : undefined; },
    setAttribute(k, v) { if (k === "href") this.attrs.href = String(v); },
    closest(sel) {
      return (sel === VOLATILE_SEL && this.inVolatile) ? {} : null;
    },
    get href() { return new URL(this.attrs.href, CURRENT_URL).href; },
  };
}

// Sidebar / shell links as rendered for a depth-2 build page (root_path = "../../").
const sidebarNotes = makeAnchor("../../notes/index.html", false);
const wordmark = makeAnchor("../../index.html", false);
const hashOnly = makeAnchor("#section", false);            // must be left alone
// A link inside the swapped content region — must NOT be frozen. Same target as
// the sidebar link so it doubles as the "un-frozen control" after navigation.
const volatileLink = makeAnchor("../../notes/index.html", true);

const ALL_ANCHORS = [sidebarNotes, wordmark, hashOnly, volatileLink];

// ---- minimal document / window / location stubs ----
globalThis.location = { get href() { return CURRENT_URL; }, origin: "https://site.example" };
globalThis.history = { pushState() {} };
globalThis.window = {
  __zetlSpaMounted: false,
  addEventListener() {},
  dispatchEvent() { return true; },
};
globalThis.document = {
  querySelectorAll(sel) { return sel === "a[href]" ? ALL_ANCHORS.slice() : []; },
  querySelector() { return null; },
  addEventListener() {},
};

// ---- load spa.js (IIFE runs freezeShellLinks() on mount) ----
const code = readFileSync(new URL("../../themes/default/static/spa.js", import.meta.url), "utf8");
(0, eval)(code); // indirect eval: runs in global scope

// After mount, shell links should be pinned to absolute, hash + volatile left as-is.
ok(sidebarNotes.getAttribute("href") === "https://site.example/notes/index.html",
   "sidebar link frozen to absolute URL");
ok(wordmark.getAttribute("href") === "https://site.example/index.html",
   "wordmark link frozen to absolute URL");
ok(hashOnly.getAttribute("href") === "#section",
   "pure-hash link left untouched");
ok(volatileLink.getAttribute("href") === "../../notes/index.html",
   "volatile-region link NOT frozen (re-rendered per navigation)");

// Both links resolve to the same target on the original (depth-2) page.
const sidebarBefore = sidebarNotes.href;
ok(sidebarBefore === "https://site.example/notes/index.html",
   "sidebar link resolves to /notes/ on the page it was rendered for");
ok(volatileLink.href === "https://site.example/notes/index.html",
   "un-frozen link also resolves to /notes/ before navigation");

// Simulate SPA navigation to a page DEEPER than the shell's render depth — the
// shell (rendered for depth 2) is not re-rendered, but the document base moves
// to depth 4. This is the condition that makes stale relative hrefs compound.
CURRENT_URL = "https://site.example/a/b/c/d/";

// The frozen shell link must still resolve to the SAME absolute URL — no
// compounding — even though the base is now deeper.
ok(sidebarNotes.href === sidebarBefore,
   "frozen sidebar link resolves identically after a deeper pushState (no compounding)");
ok(sidebarNotes.href === "https://site.example/notes/index.html",
   "frozen sidebar link still points at /notes/ after navigation");

// Sanity / control: the identical but un-frozen relative href DOES compound
// against the deeper base — this is exactly the #70 bug the freeze prevents.
ok(volatileLink.href === "https://site.example/a/b/notes/index.html",
   "control: an un-frozen relative link compounds against the deeper base");

if (failures > 0) {
  console.error(`\n${failures} spa.js navigation check(s) failed`);
  process.exit(1);
}
console.log("\nall spa.js navigation checks passed");
