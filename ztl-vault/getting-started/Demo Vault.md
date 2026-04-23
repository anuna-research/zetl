# Demo Vault

ztl ships with a `demo-vault/` directory — a self-referential knowledge base that documents ztl itself using both [[concepts/Wikilinks]] and [[concepts/Spindle Lisp]].

## What's inside

The demo vault contains pages across several categories:

- **architecture/** — [[architecture/Scanner]], [[Link Graph]], [[architecture/Cache]], [[Reasoning Engine]], [[architecture/Performance]]
- **concepts/** — [[concepts/Wikilinks]], [[concepts/Defeasible Reasoning]], [[concepts/Spindle Lisp]], [[concepts/Provenance]]
- **decisions/** — [[ADR-001 Rust]], [[Feature Gates]], [[JSON by Default]], [[decisions/Local-first Design]]
- **features/** — command documentation for graph queries, search, diagnostics, and reasoning
- **theories/** — standalone `.spl` files with cross-cutting rules

## SPL showcase

The demo vault embeds SPL facts in prose pages (e.g., `(given wikilink-extraction)` in the Scanner page) and combines them with rules in `theories/release-readiness.spl`. Running `ztl reason status` derives conclusions like `release-candidate` from facts scattered across the vault.

The `theories/caching.spl` file includes a deliberate conflict that `ztl reason conflicts` surfaces — two competing rules about cache reasoning results with no superiority relation.

## Try it

```bash
ztl -d ./demo-vault index
ztl -d ./demo-vault stats --format table
ztl -d ./demo-vault reason explain "release-candidate" --format natural
ztl -d ./demo-vault reason conflicts --format table
ztl -d ./demo-vault serve
```

## This vault vs the demo vault

This `ztl-vault/` is a more comprehensive documentation vault. The demo vault is smaller and focused on showcasing features. Both are self-documenting — they use ztl's own features to describe ztl.

See also: [[Quick Start]], [[Index]]
