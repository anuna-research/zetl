// Reader troubleshooting deep-link coverage (task-cap-docs-reader).
//
// SPEC-034 §12 documentation plan calls for a reader-facing
// `docs/reader.html` (served from dist/) with per-error-kind anchors,
// and `docs/reader-troubleshooting.md` as the markdown mirror. The
// shim's `renderError` deep-links to those anchors via a sibling
// `[data-zetl-error-help]` element. This test pins:
//
//   1. Every `ErrorKind` yields an `#err-<kind>` href.
//   2. `renderError` emits the help link alongside the byte-stable
//      summary (REQ-3427), without mutating the summary.
//   3. Both `docs/reader.html` and `docs/reader-troubleshooting.md`
//      contain a section anchor matching every kind, so the link
//      actually resolves.

import { strict as assert } from "node:assert";
import { test, before } from "node:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { Window } from "happy-dom";

import {
  READER_DOC_BASE,
  helpHrefFor,
  renderError,
} from "../errors.ts";
import type { ErrorKind } from "../errors.ts";

// Keep in sync with the `ErrorKind` union in ../errors.ts — exhaustive
// by construction at the type level, so adding a new kind requires
// extending this list and both docs.
const ALL_KINDS: ErrorKind[] = [
  "signature-failed",
  "need-invite",
  "identity-unavailable",
  "tofu-failed",
  "decrypt-failed",
  "envelope-malformed",
  "sw-purge-failed",
  "lock-unavailable",
  "host-missing",
  "internal",
];

before(() => {
  const win = new Window({ url: "https://cap.example/c/onboarding.html" });
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const g = globalThis as any;
  g.window = win;
  g.document = win.document;
});

test("helpHrefFor returns `${READER_DOC_BASE}#err-<kind>` for every kind", () => {
  for (const kind of ALL_KINDS) {
    assert.equal(helpHrefFor(kind), `${READER_DOC_BASE}#err-${kind}`);
  }
});

test("renderError emits a data-zetl-error-help link alongside the byte-stable summary", () => {
  const kind: ErrorKind = "signature-failed";
  renderError(kind);

  const host = document.querySelector("main[data-zetl-capability]");
  assert.ok(host, "capability host present");

  // The summary copy is pinned byte-stable by REQ-3427 — a help link
  // must not be appended into this paragraph.
  const summary = host!.querySelector("[data-zetl-error-summary]");
  assert.ok(summary, "summary element present");
  assert.equal(
    summary!.textContent,
    "This page's signature did not verify — possible tampering; contact your wiki operator",
    "help link wiring must not change the summary copy",
  );

  const help = host!.querySelector("[data-zetl-error-help] a");
  assert.ok(help, "help link element present");
  assert.equal(help!.getAttribute("href"), helpHrefFor(kind));
  assert.equal(help!.getAttribute("rel"), "noopener noreferrer");
  assert.equal(help!.textContent, "Troubleshooting");
});

test("renderError attaches a help link for every error kind", () => {
  for (const kind of ALL_KINDS) {
    renderError(kind);
    const host = document.querySelector("main[data-zetl-capability]");
    const help = host!.querySelector("[data-zetl-error-help] a");
    assert.ok(help, `help link present for ${kind}`);
    assert.equal(help!.getAttribute("href"), helpHrefFor(kind));
  }
});

// Walk up the tree from this test file until we find the repo root —
// identified by the presence of both `docs/reader.html` and
// `docs/reader-troubleshooting.md`. Avoids baking a relative depth
// into the test.
function repoRoot(): string {
  let dir = new URL("..", import.meta.url).pathname;
  for (let i = 0; i < 10; i++) {
    try {
      readFileSync(join(dir, "docs", "reader.html"));
      readFileSync(join(dir, "docs", "reader-troubleshooting.md"));
      return dir;
    } catch {
      dir = join(dir, "..");
    }
  }
  throw new Error("could not locate docs/reader.html from test context");
}

test("docs/reader.html contains an #err-<kind> anchor for every error kind", () => {
  const html = readFileSync(join(repoRoot(), "docs", "reader.html"), "utf8");
  for (const kind of ALL_KINDS) {
    const anchor = `id="err-${kind}"`;
    assert.ok(
      html.includes(anchor),
      `docs/reader.html missing section ${anchor} — deep-link would 404`,
    );
  }
});

test("docs/reader-troubleshooting.md contains a section heading for every error kind", () => {
  const md = readFileSync(
    join(repoRoot(), "docs", "reader-troubleshooting.md"),
    "utf8",
  );
  for (const kind of ALL_KINDS) {
    const heading = `## err-${kind}`;
    assert.ok(
      md.includes(heading),
      `docs/reader-troubleshooting.md missing '${heading}' — markdown mirror would drift`,
    );
  }
});
