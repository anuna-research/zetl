---
title: What is Defeasible Reasoning
tags: [reasoning, concepts, primer]
---

# What is Defeasible Reasoning

A short primer on the kind of logic ztl uses when you ask it to reason over your notes. No symbols, no formal notation — just an explanation of why this flavour of logic suits knowledge work.

> **Requires `--features reason` at install.** See [[Installation]].

## Classical logic: everything is final

In classical logic, facts combine and the conclusion is permanent. If you write down that every bird flies, and that Tweety is a bird, then Tweety flies. Forever. Adding new information can only produce new conclusions — it can never retract one you already made.

That rule works beautifully inside a maths textbook. It breaks almost immediately when you try to write down the actual state of a research project, a release plan, or a set of beliefs you formed over six months of reading. Real knowledge is provisional. You revise it. You hedge. You say "normally X, but not when Y".

## Defeasible logic: conclusions have strength

Defeasible reasoning lets you write rules that are **normally** true but **can be defeated** by stronger evidence. In a project-tracking vault you might write:

- *Normally, a release candidate is ready if core features are done, docs are written, and tests pass.*
- *Except when there is an outstanding critical bug.*

Both rules live in the theory at the same time. When ztl reasons over them, the defeater wins if its conditions hold — the release is blocked. If the bug is later fixed, the block lifts and the conclusion flips. You never rewrote the rules; you just added and removed facts.

This is exactly how thinking about ongoing work feels. "We're on track — unless the client pushes back on the design." "The proposal is done — except we still need a legal review." You already reason this way. ztl just gives you a notation for it.

## The four conclusion tags

Every literal (a thing that might or might not hold) ends up with one of four labels:

| Tag  | Meaning                   | Plain English |
|------|---------------------------|---------------|
| `+D` | Definitely provable       | The strict rules force this — there is no way around it. |
| `-D` | Definitely not provable   | Strict rules actively rule it out. |
| `+d` | Defeasibly provable       | Supported by the evidence you have; no stronger defeater is active. |
| `-d` | Defeasibly not provable   | Either nothing supports it, or a defeater blocks the supporting rule. |

Think of `+D`/`-D` as the **hard** layer — things you are certain about, or things that are mathematically excluded. Think of `+d`/`-d` as the **soft** layer — your current best belief, open to revision as new facts arrive.

A well-formed reasoning vault uses `always` rules (which produce `+D`/`-D`) sparingly — for genuine invariants — and `normally` rules (which produce `+d`/`-d`) for almost everything else. Knowledge work is defeasible by default.

## Why this fits notes better than classical logic

Three reasons:

1. **Beliefs revise.** Your notes are a record of thinking that changes over time. A logic that can downgrade yesterday's conclusion without you rewriting it is a better match for that record.
2. **Rules have strengths.** When two pages disagree — a design doc says "use Redis", a later benchmark says "use SQLite" — you want the newer, stronger claim to win without silencing the older one. Defeasible logic gives you preference relations for exactly this.
3. **Tentative answers are legitimate.** Saying "on current evidence, probably yes, but I'd want to check" is how humans actually reason. The `+d` tag is ztl's way of saying the same thing back to you.

## What this enables

Once rules and facts live inside your vault, ztl can do things that static notes cannot:

- Tell you *why* a conclusion holds, tracing it back to the page and line that contributed (see [[Proof Trees]]).
- Flag contradictions before you notice them (see [[Running Queries]]).
- Answer hypothetical questions — "what would I need to add to make this project ready?" (see [[What-if and Abduction]]).

A vault with reasoning rules doubles as a living checklist, a set of architectural commitments, or a decision journal — queryable from the command line.

## Related

- [[Writing SPL]]
- [[Running Queries]]
- [[What is SPL]]
- [[Proof Trees]]
