# Links Command

`zetl links` and `zetl backlinks` query the [[Link Graph]] for forward and backward connections.

## Usage

```bash
# Forward links — pages that <page> links to
zetl -d ./my-vault links "Scanner"

# Backlinks — pages that link to <page>
zetl -d ./my-vault backlinks "Cache" --depth 2
```

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--depth N` | 1 | Traverse N hops (1 = direct only) |
| `--fuzzy` | off | Enable fuzzy page name matching via [[concepts/SimHash]] |
| `--context N` | 0 | Include N characters of surrounding text |
| `--with-conclusions` | off | Show SPL conclusions each linked page contributes |

## Multi-hop traversal

With `--depth 2`, zetl follows links from the target page AND from every page the target links to (or is linked by). This reveals indirect relationships in the vault's knowledge structure.

## Deduplication

By default, zetl deduplicates results — a page appears once even if multiple links point to it. This is the output-layer deduplication described in [[ADR-004 Link Dedup]].

## Cross-referencing with reasoning

Add `--with-conclusions` to see what [[concepts/Spindle Lisp]] conclusions each linked page contributes. This bridges the structural graph with the logical theory from the [[Reasoning Engine]].

```bash
zetl -d . links "Cache" --with-conclusions
```

See also: [[CLI Reference]], [[Path Command]], [[Link Graph]], [[Search Command]]
