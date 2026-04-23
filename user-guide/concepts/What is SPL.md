---
title: What is SPL
tags: [concepts, spl, reasoning]
---

# What is SPL

> **Requires `--features reason` at install time.** See [[Installation]].

**SPL** is **Spindle Lisp** — a small Lisp-family language for writing rules. ztl extracts SPL from your vault and draws conclusions: "if a project is approved *and* has an owner, it's ready to start". This page is a gentle orientation; see [[Writing SPL]] for the syntax reference.

## Why a second language at all

Most of your vault is prose — the thing humans are good at reading. But some knowledge is best expressed as a rule, not a paragraph. "Any release candidate must have a green test run and up-to-date docs" is easier to write (and check) as a rule than as a checklist you hope to follow.

SPL lets you write those rules *inside* your notes, alongside the prose they explain. ztl then reasons over all the SPL in the vault at once — a single combined theory — and tells you what follows.

## The shape of SPL

SPL is parenthesised. Facts and rules are S-expressions:

```spl
(given docs-updated)
(given tests-pass)

(normally r-ready
  (and docs-updated tests-pass)
  release-candidate)
```

Reading top to bottom: two facts, then a rule named `r-ready` that says *normally*, if both facts hold, conclude `release-candidate`. Run reasoning and ztl will report `+d release-candidate` — defeasibly proved.

"Normally" is load-bearing. SPL is a **defeasible** logic: a conclusion can be drawn by default, and then withdrawn when stronger contrary evidence appears. See [[What is Defeasible Reasoning]] for what this means and why it matters for knowledge that isn't strictly mathematical.

## Where SPL lives in your vault

Two places. ztl extracts from both and merges them into one theory:

1. **Fenced code blocks** in any Markdown file, tagged `spl`:

   ````markdown
   # Rust for CLI

   ztl is written in Rust because it gives us:

   ```spl
   (given type-safe)
   (given single-binary)
   (given fast-startup)
   ```

   These feed the vault-wide theory that concludes `good-cli-tool`.
   ````

2. **Standalone `.spl` files** anywhere in the vault — useful for rules that are bigger than a code block:

   ```spl
   ; release-readiness.spl
   (given fast-startup)
   (given single-binary)
   (given type-safe)

   (normally r-good-cli
     (and fast-startup single-binary type-safe)
     good-cli-tool)
   ```

Comments start with `;`. Both forms are first-class; mix them however suits your writing. Facts stated in a markdown `spl` block and rules defined in a `.spl` file merge into one theory, so you can keep facts close to the prose that motivated them while the rules live in a shared theory file.

## What you can ask

With SPL in place, a handful of commands become interesting:

```bash
ztl reason status                          # what does the vault currently conclude?
ztl reason explain release-candidate       # why does that conclusion hold?
ztl reason why-not release-candidate       # if it doesn't, what's missing?
ztl reason conflicts                       # find unresolved logical tensions
ztl reason what-if "(given docs-updated)" --goal release-candidate
```

Each command traces its reasoning back to the file and line where a fact or rule was written — so "why do you think this release is ready?" has an answer that points to real notes. See [[Proof Trees]] and [[Running Queries]].

## When to reach for SPL

SPL is a good fit when:

- The knowledge is genuinely rule-shaped, not prose-shaped.
- You want a second brain to *check* a conclusion, not just store the notes leading to it.
- You care about provenance — you want a conclusion to carry a trail back to its sources.

It is **not** a good fit for things Markdown already does well. Don't encode your grocery list in SPL. Don't turn a tag taxonomy into thirty rules. A rule is worth writing when checking it by hand would be tedious or error-prone.

## Optional, always

SPL is feature-gated. If you built ztl without `--features reason`, none of the `ztl reason` commands are available, but the rest of ztl — indexing, linking, viewing, building — works exactly the same. Your `.md` files with ` ```spl ` blocks remain valid Markdown regardless. Nothing locks you in.

## Related

- [[What is Defeasible Reasoning]]
- [[Writing SPL]]
- [[Running Queries]]
- [[Proof Trees]]
- [[Installation]]
