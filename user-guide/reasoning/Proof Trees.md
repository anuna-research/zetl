---
title: Proof Trees
tags: [reasoning, explain, provenance]
---

# Proof Trees

`zetl reason explain <literal>` shows **why** a conclusion holds. The output is a proof tree: the rules that fired, the facts they rested on, and any defeaters that were considered. This page is about when and how to read one.

> **Requires `--features reason` at install.** See [[Installation]].

## What a proof tree is

When zetl concludes that some literal holds (or doesn't), it found — or failed to find — a chain of rules and facts that lead to that literal. A proof tree is that chain laid out as text:

- The **root** is the literal you asked about.
- Each **node** is a rule that fired, labelled with the rule's name.
- Each **leaf** is a fact (`given`) or a dead end (a body literal that couldn't be proved).
- **Defeaters** that were considered — and either won or were overridden — appear alongside the main chain.

Every node carries **provenance**: the source file, line number, and page name where that rule or fact was written. You can walk from any conclusion back to the exact prose that originated it.

## The command

```bash
zetl reason explain "acme-ready-to-start"
```

By default the output is JSON. Three other formats are available via `--as`:

| `--as`        | What you get |
|---------------|--------------|
| `json`        | Structured proof tree (default) |
| `table`       | A tabular view of nodes, rules, and sources |
| `natural`     | English prose: "The literal X is defeasibly provable because …" |
| `dot`         | Graphviz DOT source — pipe to `dot -Tpng` for a diagram |

There is also `--depth N` (default 10) which caps how deep the tree goes. Unbounded trees in large vaults can be overwhelming; 4 or 5 is usually enough for day-to-day use.

### The natural-language form

`--as natural` is the one to use when you want a sentence you can paste into Slack:

```bash
zetl reason explain "acme-ready-to-start" --as natural
```

Output looks like:

```text
The literal 'acme-ready-to-start' is defeasibly provable.
It follows from rule r-acme-ready (theories/project-readiness.spl:4)
whose body (acme-approved, acme-has-owner, acme-budget-signed-off)
is supported by facts given in projects/Acme Website.md lines 6–8.
No active defeater blocks this conclusion.
```

No formal notation, no symbols — just the chain of reasoning written out.

### The diagram form

`--as dot` emits Graphviz source. Good for architectural reviews and posters:

```bash
zetl reason explain "release-candidate" --as dot \
  | dot -Tpng -o release-proof.png
```

The resulting diagram has one node per rule and fact, with arrows showing how conclusions flow. Defeaters appear as dotted edges.

## When to read one

The usual trigger is surprise. You ran `zetl reason status`, you saw a conclusion that doesn't match what you expected, and you need to know why. A few concrete cases:

- **"Why does zetl say the release candidate is ready?"** You expected one more review. `explain` tells you which facts made the readiness rule fire — including, possibly, an outdated `given` still sitting in an old meeting note.
- **"Who signed off on docs-updated?"** The rule fired, which means somewhere a `given docs-updated` was asserted. The proof tree names the file and line. Go look; it might be three months old.
- **"Why *isn't* the build-blocked literal holding?"** When you expected a negative conclusion but got silence. The tree shows which body literals failed to be proved, pointing you at the missing premise.

When the conclusion is negative and you want the **failure** analysis — what's missing, what defeated what — reach for `zetl reason why-not <literal>` instead. It is the explicit counterpart to `explain`, and it lists candidate rules with their blockers:

```bash
zetl reason why-not "acme-ready-to-start"
```

For each rule that could have produced the conclusion, `why-not` shows which body literals were unprovable or which defeater fired.

## Cross-referencing with provenance

`reason explain` is the deep-dive view of a single conclusion. [[Running Queries|`reason provenance`]] is the shallower, graph-oriented view — "what pages contributed?" rather than "what tree of rules fired?". Use them together:

```bash
# Drill down into the logical chain
zetl reason explain "acme-ready-to-start" --as natural

# Zoom out to the pages involved
zetl reason provenance "acme-ready-to-start"
```

## A worked pattern

A habit worth adopting: whenever `zetl reason status` tells you something you didn't expect, pipe the literal straight into `explain --as natural`:

```bash
zetl reason status --literal "acme*" --positive
# Oh — acme-ready-to-start is +d, but legal hasn't cleared yet?
zetl reason explain "acme-ready-to-start" --as natural
```

Nine times out of ten, the explanation points at a stale `given` or a missing `(prefer ...)` — the kind of thing you fix in thirty seconds once you can see it.

## Related

- [[Running Queries]]
- [[Writing SPL]]
- [[What-if and Abduction]]
- [[What is Defeasible Reasoning]]
