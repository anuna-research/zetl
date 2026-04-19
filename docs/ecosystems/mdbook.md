# mdBook ecosystem guide

The mdBook adapter lets zetl run
[mdBook](https://rust-lang.github.io/mdBook/) preprocessors
(`mdbook-<name>` binaries) over vault pages as part of the
pre-parse stage. Preprocessors are one-shot subprocesses that read a
JSON envelope on stdin and emit a transformed `Book` JSON on stdout.

The authoritative surface for the mdBook adapter is
[SPEC-033 REQ-3304 / CON-3304 / REQ-3309 / CON-3309](../../specs/SPEC-033.md);
this doc restates the material a user facing a failing run needs and is
kept in sync with the shipped `ecosystem-mdbook` feature flag.

<!-- toc -->
- [Install](#install)
- [Envelope shape (REQ-3309)](#envelope-shape-req-3309)
- [Invocation contract](#invocation-contract)
- [Scope (`page` vs `vault`)](#scope-page-vs-vault)

## Install

The mdBook adapter does not require the `mdbook` binary itself on
`$PATH` — preprocessors run independently. Install any preprocessor
with `cargo install`:

```sh
cargo install mdbook-mermaid
cargo install mdbook-toc
cargo install mdbook-admonish
```

`zetl ecosystem check` surfaces preprocessor availability per
[SPEC-033 REQ-3310](../../specs/SPEC-033.md#req-3310-zetl-ecosystem-check-subcommand).

## Envelope shape (REQ-3309)

On every invocation, zetl writes a two-element JSON array to the
preprocessor's stdin:

```json
[
  {
    "root": "/path/to/vault",
    "config": {
      "book": {"title": "<vault.name>", "authors": [], "src": "."},
      "preprocessor": {}
    },
    "renderer": "html",
    "mdbook_version": "0.4.40"
  },
  {
    "sections": [
      {"Chapter": {
        "name": "<page.name>",
        "content": "<raw markdown>",
        "number": null,
        "sub_items": [],
        "path": "<page.slug>.md",
        "source_path": "<page.slug>.md",
        "parent_names": []
      }}
    ],
    "__non_exhaustive": null
  }
]
```

The canonical schema lives at
[`tools/zetl-mdbook-envelope-schema-v1.json`](../../tools/zetl-mdbook-envelope-schema-v1.json)
and is asserted against every constructed envelope in the test suite
(`test_3309_every_fixture_page_envelope_passes_schema_validation`).

The preprocessor writes the transformed `Book` (the second element,
without the surrounding context) to stdout. zetl reads the first
chapter's `content` field back out and feeds it into the pipeline's
next stage as transformed Markdown.

**Fidelity guarantees:**

- Envelope construction is a pure function
  ([`build_envelope_for_page`](../../src/ecosystems/mdbook.rs)); the
  input body is copied verbatim into `Chapter.content`.
- Extraction (`extract_chapter_content`) on the zetl-built envelope
  returns the input body byte-for-byte; this round-trip property is
  enforced by
  `test_3309_envelope_content_round_trip_is_byte_identical`.
- Inbound preprocessor responses are validated structurally before
  the adapter trusts them; a preprocessor that drops `Chapter` or
  emits a mis-typed field surfaces as a `malformed_output` failure
  rather than a silent content loss.

**Known flexibility in inbound validation:**

- The `__non_exhaustive: null` marker is required on *outgoing*
  envelopes (mdBook's own serde emits it) but optional on *inbound*
  responses, because preprocessors in non-Rust languages (Node,
  Python) typically don't reproduce the marker.
- `PartTitle` and `Separator` items in `book.sections` are accepted
  when present — zetl never emits them but preprocessors that
  restructure the book may.

## Invocation contract

mdBook preprocessors follow a two-call protocol per
[CON-3304](../../specs/SPEC-033.md#con-3304):

1. `<exec> supports html` — probe. Exit 0 = the preprocessor accepts
   the html renderer; exit non-zero = it refuses and the real run
   never spawns.
2. `<exec>` (no argv) — real run. Stdin = envelope above; stdout =
   transformed `Book` JSON.

The adapter bounds the probe to 5 seconds and the real run to the
manifest's declared timeout. Binary-not-found, probe-failure,
non-zero exit, malformed JSON, and malformed envelope shape each map
to a typed `FailureReason` for the observability pipeline.

## Scope (`page` vs `vault`)

The manifest's `scope` field picks the invocation cardinality:

- `scope = "page"` (default): one preprocessor call per vault page;
  maximally parallelisable. The envelope always carries exactly one
  `Chapter`.
- `scope = "vault"`: accepted for forward compatibility, but v1
  still runs one call per page and surfaces a warning diagnostic.
  Whole-vault batching (for preprocessors like `mdbook-toc` that
  need to see sibling chapters) lands in a later phase per
  [CON-3309 "Vault-scope invocations — known semantic gap"](../../specs/SPEC-033.md#con-3309-mdbook-book-envelope-schema).
