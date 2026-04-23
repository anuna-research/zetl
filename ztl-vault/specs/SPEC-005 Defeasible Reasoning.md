# SPEC-005: Defeasible Reasoning

Adds the logical reasoning layer using [[concepts/Spindle Lisp]] embedded in Markdown. See [[Reason Commands]] for the CLI interface and [[Reasoning Engine]] for the architecture.

```spl
(given spec-005-documented)
```

## Core insight

Zettelkasten notes make claims that can be formalized. SPL code blocks in Markdown express defeasible rules that ztl extracts, combines into a unified theory, and reasons over using `spindle-core`.

## Key requirements

| ID | Requirement |
|----|-------------|
| REQ-026 | SPL extraction from Markdown |
| REQ-027 | Theory construction with provenance |
| REQ-028 | Vault reasoning (+D/-D/+d/-d conclusions) |
| REQ-029 | Explanation with document provenance |
| REQ-030 | Hypothetical reasoning (what-if) |
| REQ-031 | Failure explanation (why-not) |
| REQ-032 | Knowledge gap detection (require) |
| REQ-033 | Conflict detection |
| REQ-034 | SPL validation in check |
| REQ-035 | Theory export |
| REQ-036 | Cross-reference graph and theory |

## Eight commands

1. `reason status` — current conclusions
2. `reason explain` — proof tree with provenance
3. `reason why-not` — why something can't be proved
4. `reason require` — abductive reasoning (missing facts)
5. `reason what-if` — hypothetical reasoning
6. `reason conflicts` — unresolved contradictions
7. `reason export` — theory export (JSON or SPL)
8. `reason provenance` — cross-referenced source tracing

## Architecture decisions

- **ADR-005:** Embed `spindle-core` as Rust library behind [[Feature Gates]]
- **ADR-006:** Theory caching in `.ztl/theory.json` — see [[architecture/Cache]]
- **ADR-007:** Provenance via spindle-core Meta API

## Performance targets

- Reasoning: <= 500ms for 10,000 rules
- Memory: <= 50 MB for reasoning
- Incremental extraction from cache

## SPL-in-Markdown as literate programming

Embedding logic in prose follows the literate programming tradition (Knuth). The SPL blocks are grounded in their surrounding prose — see [[SPEC-006 Merkle Tree]] for content-addressed grounding.

See also: [[Spec Index]], [[Reason Commands]], [[Reasoning Engine]], [[concepts/Spindle Lisp]]
