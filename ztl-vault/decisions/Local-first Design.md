# Local-first Design (Decision Record)

**Status:** Accepted

## Context

ztl operates on personal knowledge bases that may contain years of accumulated notes. Users need confidence that the tool won't damage their data.

## Decision

ztl is strictly read-only against vault files. All derived data is stored in a disposable `.ztl/` directory. No network calls are made.

```spl
(given read-only-vault-access)
(given disposable-cache)
```

## Rationale

- **Trust** — users adopt CLI tools more readily when they can't cause data loss
- **Composability** — read-only access means ztl works alongside Obsidian, Logseq, Foam, Dendron, or any editor without conflict
- **Simplicity** — no write path means no write bugs, no file locking, no corruption recovery
- **Disposability** — the cache is derived data; losing it costs only a re-index

## Implications

- The [[architecture/Cache]] in `.ztl/` can be gitignored and deleted freely
- The [[Serve Command]]'s inline edit feature is the single exception (writes only on explicit user save)
- Future distributed sync (see [[Distributed Sync Future]]) would be handled by a separate sidecar process, not by ztl itself

## Relationship to concepts page

The [[Local-first Design]] concepts page describes the five principles for end users. This decision record captures the architectural rationale.

See also: [[Local-first Design]], [[architecture/Cache]], [[Compatibility]]
