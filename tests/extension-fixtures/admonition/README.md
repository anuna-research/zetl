# Admonition extension fixture

Golden-HTML gate for the canonical `admonition` extension (SPEC-032 REQ-3212,
TEST-3212c). The fixture exercises both recognised syntaxes:

- **Obsidian `ad-*` fenced blocks** (e.g. `` ```ad-note `` / `` ```ad-danger ``),
  optionally preceded by a `title:` header line inside the block.
- **Python-Markdown MkDocs admonitions** (`!!! type "Title"`, body indented by
  four spaces).

The committed runner (`tools/xtask/src/runners.rs::admonition_runner`) is the
thin in-process stub the task description specifies. Real vaults run the
transformation through an ecosystem plugin (e.g. `pandoc-admonition`,
`mdbook-admonish`); the stub exists to gate the theme CSS + template contract
without pulling an external binary into CI.

## Regenerate after edits

```
cargo xtask update-golden admonition
```

Review the diff carefully before committing — a surprising change often
indicates a regression in the stub, not a fixture refresh.
