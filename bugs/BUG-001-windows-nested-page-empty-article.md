---
id: BUG-001
title: `zetl serve` on Windows: nested pages render with empty `<article>` body
status: fixed
severity: S2
priority: P1
detection-method: reproduction on Windows + triage of HTML output
date: 2026-05-06
binary: zetl 0.6.1 (Windows)
vault: vault containing root pages (`home.md`, `index.spl`) and nested pages (`<dir-a>/<page-a>.md`, `<dir-b>/<page-b>.md`, …)
affects:
  - `[[zetl serve]]` (live dev server)
  - `[[zetl watch]]` (same handler chain)
  - any HTTP consumer of the dev server's HTML output on Windows
not-affected:
  - `[[zetl build]]` static output (`dist/<slug>/index.html` writes the body correctly)
  - macOS / Linux dev server (forward-slash native, no separator divergence)
---

## Summary

On Windows, `[[zetl serve]]` (and `[[zetl watch]]`) at version 0.6.1 returns
HTML with an **empty `<article>` element** for every page that lives in a
subdirectory of the vault. The chrome — sidebar, backlinks, linked-page
excerpts, graph data — populates correctly, so `zetl` knows the page exists
and knows what it links to, but the markdown body is never injected.

Pages at the **vault root** (e.g. `home.md`, `index.spl`) render
correctly. Pages inside **any subdirectory** (e.g. `<dir-a>/<page-a>.md`,
`<dir-b>/<page-b>.md`) render with an empty body. The bug is independent of
content, frontmatter, or the `--public` overlay (a separate concept that
covers the asset folder; this bug affects markdown body injection, not
asset serving).

## Specification Reference

- **Violates:** specification gap — no current `REQ-###` or `CON-###`
  covers cross-platform path-separator handling in the
  [[scanner|page_slug_from_path]] → URL-slug comparison pipeline.
  This bug should produce a new `REQ-###` codifying that page slugs are
  the URL-canonical key (forward-slash separated, lowercase, hyphenated)
  on every platform `zetl` runs on.
- **Related:** `[[SPEC-009]]` (build/serve handler chain), the `[[zetl build]]`
  static-output path which already handles this correctly via
  filesystem-aware writes.

## Environment

- **OS:** Windows (any 10/11 build)
- **Binary:** zetl 0.6.1
- **Vault layout:** vault root contains a mix of root-level files and
  subdirectory pages (`<dir-a>/<page-a>.md`, `<dir-b>/<page-b>.md`, etc.)
- **Detection:** `curl http://localhost:3000/<slug>/<slug>/`

## Steps to Reproduce

```cmd
:: from a Windows shell
mkdir vault\about
echo # About > vault\home.md
echo # Nested About > vault\about\about.md
zetl --no-cache index --vault vault
zetl serve --vault vault --port 3000
```

In another shell:

```cmd
curl http://localhost:3000/home/
curl http://localhost:3000/about/about/
```

## Expected Behaviour

Both responses include the page body inside the `<article>` element:

```html
<article class="prose prose-lg max-w-none">
  <h1>About</h1>            <!-- root page renders -->
  …
</article>
```

```html
<article class="prose prose-lg max-w-none">
  <h1>Nested About</h1>     <!-- nested page should render -->
  …
</article>
```

This matches the behaviour of `zetl build` on the same vault: the static
output at `dist/about/about/index.html` is ~123 KB and contains the full
body. The dev server is expected to serve the same HTML the static
builder writes.

## Actual Behaviour

The root page renders normally. **Nested pages** render with an empty
`<article>`:

```html
<article class="prose prose-lg max-w-none"></article>
```

The sidebar, backlinks, "linked from" excerpts, graph data, search index,
and frontmatter all populate correctly — only the markdown body is
missing. The page-list HTML in the sidebar emits anchor hrefs with
**backslashes inside the path segment**:

```html
<a href="/about\about/">about</a>
<a href="/notes\foo/">foo</a>
<a href="/ideas\bar/">bar</a>
```

## Evidence

| Page         | Source path                  | Body rendered? |
|--------------|------------------------------|----------------|
| `home`       | `home.md` (vault root)       | ✓ yes          |
| `index`      | `index.spl` (vault root)     | ✓ yes          |
| `about/about`| `about/about.md`             | ✗ empty        |
| `notes/foo`  | `notes/foo.md`               | ✗ empty        |

Static-build comparison on the same vault (control):

| Output                              | Body present? |
|-------------------------------------|---------------|
| `dist/home/index.html`              | ✓             |
| `dist/about/about/index.html`       | ✓ (~123 KB)   |
| `dist/notes/foo/index.html`         | ✓             |

## Root Cause

- **Category:** implementation-error (with adjacent test-gap and spec-gap
  components).
- **Analysis:**

The slug-key generator in `[[scanner|page_slug_from_path]]`
(`src/scanner.rs:716`) computes the canonical page identity from the
relative file path:

```rust
pub fn page_slug_from_path(path: &Path) -> String {
    let s = path.to_string_lossy();          // ← native separators
    let stripped = if let Some(s) = s.strip_suffix(".md") {
        s
    } else if let Some(s) = s.strip_suffix(".spl") {
        s
    } else if let Some(s) = s.strip_suffix(".fountain") {
        s
    } else {
        &s
    };
    stripped.to_lowercase().replace(' ', "-")
}
```

On Windows, `Path::to_string_lossy()` returns the path with the
**platform-native separator**, which is `\`. So `about\about.md`
becomes the slug `about\about`. On macOS/Linux it would be
`about/about`. Spaces are normalised; backslashes are not.

The HTTP handler `[[page_handler]]` at
`src/web/routes.rs:430` receives a slug from axum's
`Path<String>` extractor that is always forward-slash-separated
(URLs cannot legally encode backslashes as path separators) and
performs the file lookup at line 503:

```rust
let file = data
    .files
    .iter()
    .find(|f| page_slug_from_path(&f.path).eq_ignore_ascii_case(slug));
```

On Windows this comparison is `"about\\about".eq_ignore_ascii_case("about/about")`,
which is **false**. The lookup returns `None`, so no markdown body is
loaded. The handler then renders the page chrome with whatever data it
*can* derive from the slug (sidebar, backlinks, graph) and emits an
empty `<article>`. There is no error, no log line, no 404 — the missing
file lookup is silently absorbed into a successful 200 response with a
blank body.

The same root cause produces the second visible symptom — the page-list
HTML emitting `<a href="/about\about/">`. Wherever the link generator
uses the slug verbatim as a URL path, the backslash leaks into the
emitted href. Browsers and `curl` tolerate the backslash (it gets
URL-encoded as `%5C` or normalised in some clients), but the
canonicalisation never reaches the comparator inside `page_handler`,
so even the link-driven request still misses.

The static builder is unaffected because it writes to disk via
`std::fs::create_dir_all(...)` + `Path::join`, which the OS happily
interprets in either separator style — it never round-trips a
slug-as-string back into a comparator. So `dist/about/about/index.html`
is written correctly and any plain HTTP server reading the file system
serves the body verbatim.

## Proposed Fix

Single point of repair, with cascading correctness:

```rust
// src/scanner.rs
pub fn page_slug_from_path(path: &Path) -> String {
    // Build slug from path components so the separator is forced to '/' on
    // every platform, then apply the lowercasing / space-normalisation that
    // makes slugs URL-canonical. This is the canonical key used by every
    // downstream consumer (URL routing, link emission, search index, graph).
    let mut parts: Vec<String> = path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();

    if let Some(last) = parts.last_mut() {
        for ext in [".md", ".spl", ".fountain"] {
            if let Some(stripped) = last.strip_suffix(ext) {
                *last = stripped.to_string();
                break;
            }
        }
    }

    parts.join("/").to_lowercase().replace(' ', "-")
}
```

Equivalent shorter spelling (one-line patch retaining the original
shape):

```rust
let s = path.to_string_lossy().replace('\\', "/");
```

…inserted at the top of the existing function. Both fixes are
behavioural no-ops on macOS/Linux (no `\` characters in their
`to_string_lossy()` output for paths under the vault root) and
correct on Windows. The `Path::components()` form is preferable
because it is resilient to mixed-separator paths (e.g. forward
slashes that snuck into a Windows path manually) and to leading
`./` or `../` components.

The fix MUST land in `[[scanner|page_slug_from_path]]` rather than
in any individual call site. There are at least four downstream
consumers (page handler, raw-source handler, ACL evaluator, link
emitter — all in `src/web/routes.rs`) and at least one offline
consumer (search index emit). Every one of them takes
`page_slug_from_path` as the canonical key; fixing them
independently would leave the same defect latent under a different
shape.

## Verification (regression tests required)

A regression test SHOULD live in `tests/` and exercise the slug
generator directly. The current cross-platform CI only runs Linux, so
the slug must be tested with a `PathBuf` constructed with backslashes
to simulate Windows behaviour deterministically:

```rust
#[test]
fn page_slug_normalises_backslashes_to_forward_slash() {
    use std::path::PathBuf;
    // Simulate a Windows path component as it would arrive from the
    // Windows directory walker.
    let p = PathBuf::from(r"about\about.md");
    assert_eq!(page_slug_from_path(&p), "about/about");
}

#[test]
fn page_slug_handles_mixed_separators() {
    use std::path::PathBuf;
    let p = PathBuf::from(r"a\b/c.md");
    assert_eq!(page_slug_from_path(&p), "a/b/c");
}

#[test]
fn page_slug_lowercases_and_dehyphens_after_normalising() {
    use std::path::PathBuf;
    let p = PathBuf::from(r"About Us\Contact Page.md");
    assert_eq!(page_slug_from_path(&p), "about-us/contact-page");
}
```

Additionally, a Windows job SHOULD be added to CI (`.github/workflows`)
so this class of bug is caught at PR time. The current test surface
catches no Windows-specific separator bugs because there is no
Windows runner.

## Workaround

For anyone blocked today: build statically and serve the result.

```cmd
zetl build --public public
python -m http.server 3000 --directory dist
```

The static build writes the body once per page and any plain file
server will serve it correctly because no per-request slug lookup
takes place. The trade-off is loss of live edit/reload — for
read-only browsing or sharing a snapshot this is generally
acceptable.

An earlier zetl release (approximately 0.6.0) is reported to serve
the same vault correctly, which is consistent with the slug
generator having changed at some point between 0.6.0 and 0.6.1.
A `git log -p src/scanner.rs` on `page_slug_from_path` will
confirm the regression point.

## Triage Notes

- **Severity S2 (Major):** core feature (`zetl serve` body rendering)
  broken on a supported platform; no in-tool workaround other than
  switching to the static builder.
- **Priority P1:** affects every Windows user with any subdirectory
  structure in their vault — i.e. effectively every non-trivial
  vault.
- **Detection method:** dev server opened on Windows; nested pages
  observed empty; triage of the response HTML surfaced the
  backslash-URL smoking gun in the page-list anchor hrefs.

## Related Hardening

While fixing this, two adjacent gaps SHOULD be closed:

1. **Silent miss in `page_handler`.** When the file lookup at
   `src/web/routes.rs:503` returns `None` for a slug that is not
   reserved (`/_*`), the handler currently falls through to the
   "page doesn't exist yet" path and renders chrome with empty
   body. It SHOULD log a warning (or emit a debug-mode banner) so
   the next class of slug-mismatch bug is loud rather than silent.
2. **CI matrix.** Add a Windows job to the test workflow so
   path-separator regressions surface at PR time. The cost is
   modest; the benefit is direct evidence that future refactors
   of the slug pipeline preserve cross-platform behaviour.

## Detection Context

- **Detection method:** live reproduction (`curl` against a 0.6.1
  dev server on Windows) + static analysis identifying
  `page_slug_from_path` as the single point of repair.
- **Confidence:** high — the failure mode (empty `<article>`) is
  directly observable, the URL emission with backslashes is direct
  evidence of the slug containing native separators, and the
  implicated function uses `to_string_lossy()` against a `Path`
  which is documented to use native separators on Windows.
