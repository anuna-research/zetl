# Provenance

Provenance is the ability to trace a conclusion back to its origins. In ztl, every fact, rule, and conclusion carries metadata recording the source file, line number, and page name where it was defined.

```spl
(given provenance-documented)
```

## Why it matters

A reasoning system that says "X is true" is only useful if you can ask "why?" and get a concrete answer. Provenance connects the [[Reasoning Engine]]'s abstract logic back to the human-readable documents in your vault.

## What is tracked

| Element | Provenance |
|---------|------------|
| Fact | File path, line number, page name |
| Rule | File path, line number, page name, rule label |
| Conclusion | Full proof tree of contributing rules and facts |
| Grounding | Section hash or explicit source reference (see [[Drift Detection]]) |

## Commands

- `ztl reason explain <literal>` — proof tree with source locations
- `ztl reason provenance <literal>` — cross-referenced with the [[Link Graph]]
- `ztl reason export --format spl` — reconstructed [[concepts/Spindle Lisp]] with provenance comments

## Grounding freshness

Since [[SPEC-006 Merkle Tree]], provenance includes grounding freshness. Each source in a provenance trace reports whether its grounding (section or explicit) is still fresh — i.e., whether the prose that justifies the fact has changed since the theory was built.

## Example

Running `ztl reason provenance "vault-ready-to-publish"` on this vault traces the conclusion through rules in `theories/vault-readiness.spl` back to facts scattered across the architecture, reference, and concept pages.

See also: [[Reasoning Engine]], [[Reason Commands]], [[concepts/Defeasible Reasoning]], [[Drift Detection]]
