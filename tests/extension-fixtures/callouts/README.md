# Callouts extension fixture

Golden-HTML gate for the canonical `callouts` extension (SPEC-032
REQ-3212, stub model). The fixture exercises Obsidian callout
blockquote syntax:

- `> [!type]` — opens a callout block. The rest of the blockquote
  (lines prefixed with `>`) is the body.
- `> [!type] Custom title` — inline title replaces the default.
- `> [!type]+` / `> [!type]-` — optional fold marker (default-expanded
  vs default-collapsed). Rendered as `data-callout-fold`.

The committed runner (`tools/xtask/src/runners.rs::callouts_runner`)
is the thin in-process stub the task description specifies. Real
vaults run the transformation through an ecosystem plugin
(`mdbook-admonish` under the mdBook ecosystem, or Pandoc native div
syntax); the stub exists to gate the theme CSS + template contract
without pulling an external binary into CI.

## Regenerate after edits

```
cargo xtask update-golden callouts
```

Review the diff carefully before committing — a surprising change
often indicates a regression in the stub, not a fixture refresh.
