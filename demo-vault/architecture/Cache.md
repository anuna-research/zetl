---
title: Cache
---

# Cache

ztl uses mtiame-based incremental caching for both the [[Link Graph]] and the [[Reasoning Engine]]'s theory. On subsequent runs, only files modified since the last scan are re-parsed.

## How it works

The cache lives in `.ztl/` at the vault root:

- `index.json` — serialised link graph
- `theory.json` — serialised reasoning theory and conclusions

Each entry records the file's last-modified timestamp. When ztl starts, it compares mtimes and only re-scans changed files. Use `--no-cache` to force a full rebuild.

```spl
(given mtime-based-cache)
(given incremental-rebuild)
```

## Design tension

There is an unresolved design tension in how aggressively to cache reasoning results:

```spl
; Cache the theory for fast startup
(normally r-cache-theory
  mtime-based-cache
  cache-reasoning-results)

; But recomputing ensures results are always fresh
(normally r-recompute-theory
  incremental-rebuild
  (not cache-reasoning-results))
```

Both arguments have merit. Try `ztl reason conflicts` on this vault to see how ztl surfaces this kind of tension. A future [[Plugin System]] could let users configure the trade-off.

## Safety

The cache is disposable — deleting `.ztl/` and re-running `ztl index` regenerates it. See [[Local-first Design]].

See also: [[Scanner]], [[Link Graph]], [[Reasoning Engine]] [[Search]] [[TUI]] [[good-idea-2]]
