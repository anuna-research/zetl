---
title: Provenance
---

# Provenance

Provenance is the ability to trace a conclusion back to its origins. In ztl, every fact, rule, and conclusion carries metadata recording the source file, line number, and page name where it was defined.

## Why it matters

A reasoning system that says "X is true" is only useful if you can ask "why?" and get a concrete answer. Provenance connects the [[Reasoning Engine]]'s abstract logic back to the human-readable documents in your vault.

## What is tracked

| Element | Provenance |
|---------|------------|
| Fact | File path, line number, page name |
| Rule | File path, line number, page name, rule label |
| Conclusion | Full proof tree of contributing rules and facts |

## Commands

- `ztl reason explain <literal>` — proof tree with source locations
- `ztl reason provenance <literal>` — cross-referenced with the [[Link Graph]]
- `ztl reason export --format spl` — reconstructed [[Spindle Lisp]] with provenance comments

## Example

Running `ztl reason provenance "release-candidate"` on this vault traces the conclusion through rules in `theories/release-readiness.spl` back to facts scattered across the architecture and feature pages.

See also: [[Reasoning Engine]], [[Reason Commands]], [[Defeasible Reasoning]]
