# Agent Ergonomics

**Status:** Accepted

## Context

[[SPEC-003 Agent Ergonomics]] documents findings from running ztl as an LLM agent would — systematically exercising every command and checking for friction points.

## Findings and fixes

### P0 — Bugs

- **Empty search query panic** — `ztl search ""` crashed instead of returning an empty result. Fixed to return `{"results": []}`.
- **Plain-text errors** — errors were emitted as plain text even in JSON mode. Fixed with [[ADR-003 JSON Errors]].

### P1 — Quality

- **Duplicate results** — `links` and `backlinks` returned the same page multiple times. Fixed with [[ADR-004 Link Dedup]].
- **UTF-8 safety** — context extraction could split multi-byte characters. Fixed with character-boundary-aware slicing.

### P2 — New capabilities

- **`ztl list`** — page enumeration for agent discovery workflows. See [[List and Export Commands]].
- **`search --path`** — restrict search to a subdirectory. See [[Search Command]].
- **`ztl export`** — full graph adjacency list for external consumption. See [[List and Export Commands]].

## Design principle

```spl
(given agent-ergonomics-audited)
```

Agent-friendliness is not an afterthought — it's tested by simulating real agent sessions. The [[JSON by Default]] philosophy, structured errors, and non-zero exit codes all serve this goal.

See also: [[SPEC-003 Agent Ergonomics]], [[JSON by Default]], [[ADR-003 JSON Errors]], [[CLI Reference]]
