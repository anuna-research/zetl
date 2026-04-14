# SPEC-003: Agent Ergonomics

Documents findings from running zetl as an LLM agent would — systematically exercising every command and identifying friction points. See [[Agent Ergonomics]] for the decision record.

```spl
(given spec-003-documented)
(given agent-ergonomics-audited)
```

## Findings

### P0 — Bugs

| Finding | Issue | Fix |
|---------|-------|-----|
| FINDING-001 | Empty search query panic | Return `{"results": []}` |
| FINDING-002 | Plain-text errors in JSON mode | [[ADR-003 JSON Errors]] |

### P1 — Quality

| Finding | Issue | Fix |
|---------|-------|-----|
| FINDING-004 | Duplicate link results | [[ADR-004 Link Dedup]] |
| FINDING-005 | Duplicate backlink results | [[ADR-004 Link Dedup]] |
| FINDING-006 | UTF-8 context splitting | Character-boundary-aware slicing |

### P2 — New capabilities

| Requirement | Command | Purpose |
|-------------|---------|---------|
| REQ-019 | `zetl list` | Page enumeration for agent discovery |
| REQ-020 | `search --path` | Restrict search to subdirectory |
| REQ-021 | `zetl export` | Full graph adjacency list |

## Design principles established

- Agents consume stdout as JSON — errors must be structured
- Non-zero exit codes signal failure
- Page enumeration (`list`) is essential for agent bootstrapping
- Deduplication at the output layer keeps the internal graph faithful

## Impact

These fixes made zetl significantly more reliable as an agent building block. The [[JSON by Default]] philosophy, structured errors, and non-zero exit codes all trace back to this spec.

See also: [[Spec Index]], [[Agent Ergonomics]], [[JSON by Default]], [[CLI Reference]]
