// Final pipeline steps: replaceState scrub, wikilink href rewrite, and
// DOM injection. The host page carries `<main data-ztl-capability>` as
// the single injection sink; anything outside that element is untouched.
//
// The capability-mode CSP (`require-trusted-types-for 'script'`;
// `trusted-types ztl-cap`) makes `innerHTML` a TrustedHTML sink. The
// shared policy is registered in `sanitise.ts::trustedHtml`; we pull
// it from there rather than re-registering so the whole shim emits a
// single TT policy (per the one-sink-per-document TT recommendation).

import { trustedHtml } from "./sanitise.ts";

const HOST_SELECTOR = "main[data-ztl-capability]";

/// Strip any `#...` fragment from the current URL (CON-3408 STEP 5).
/// Best-effort per §11.2 — browser sync, extensions, and link previewers
/// may have already captured the pre-scrub URL before this runs.
export function scrubFragment(): void {
  // `replaceState` silently no-ops in contexts without a history API
  // (Playwright harness sandbox stubs, for example). Guard so test
  // harnesses that stub only part of the History API do not trip.
  if (typeof history === "undefined" || typeof history.replaceState !== "function") {
    return;
  }
  history.replaceState(null, "", location.pathname + location.search);
}

/// Rewrite any `<a href="…#k=…">` to omit the fragment, so a click on an
/// intra-wiki link does NOT carry the invite-bound priv_A onward to a
/// sibling page whose path-cap already resolves.
export function rewriteWikiHrefs(root: Document | Element = document): void {
  const anchors = root.querySelectorAll("a[href]");
  for (const a of Array.from(anchors)) {
    const href = a.getAttribute("href");
    if (!href) continue;
    const hashIdx = href.indexOf("#");
    if (hashIdx < 0) continue;
    // Only strip fragments from same-origin (relative or origin-matched)
    // links. An absolute off-site URL with a fragment may legitimately
    // target an anchor the reader wants to reach.
    if (isInWikiHref(href)) {
      a.setAttribute("href", href.slice(0, hashIdx));
    }
  }
}

function isInWikiHref(href: string): boolean {
  if (href.startsWith("/") || href.startsWith("./") || href.startsWith("../")) {
    return true;
  }
  if (href.startsWith("#")) return true;
  try {
    const u = new URL(href, location.href);
    return u.origin === location.origin;
  } catch {
    return false;
  }
}

/// Inject the sanitised HTML into the capability-mode host. Returns the
/// host element so downstream tasks (collision UI, diagnostics) can
/// append sibling chrome without re-querying.
export function renderInto(sanitisedHtml: string): Element {
  const host = document.querySelector(HOST_SELECTOR);
  if (host === null) {
    throw new Error(
      `capability-mode host element ${HOST_SELECTOR} not found in the document — the HTML shell is missing its mount point`,
    );
  }
  // `innerHTML` accepts TrustedHTML or string; the assignment site is
  // one line so the union type stays local to this sink.
  (host as unknown as { innerHTML: unknown }).innerHTML = trustedHtml(sanitisedHtml);
  return host;
}
