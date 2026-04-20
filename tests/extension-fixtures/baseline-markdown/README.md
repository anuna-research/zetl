# baseline-markdown

Canary fixture for the CON-3212 golden-HTML harness.

This fixture does **not** exercise a canonical extension — it exists only
to prove the harness wiring works end-to-end before the actual extensions
(`callouts`, `tasks`, `admonition`) ship.

The runner is a plain pulldown-cmark pass (the same one the core
renderer uses for vault pages, minus wikilink rewriting — no slug map is
supplied). Any drift between pulldown-cmark's output and `expected.html`
trips the `ext-golden-html` gate.

Regenerate with:

```shell
cargo xtask update-golden baseline-markdown
```
