---
title: Writing SPL
tags: [reasoning, spl, syntax]
---

# Writing SPL

Spindle Lisp (SPL) is the small language ztl reads when you ask it to reason over your vault. This page walks through the syntax by example — no formal grammar, just enough to start writing useful theories.

> **Requires `--features reason` at install.** See [[Installation]].

## Where SPL goes

SPL lives in two places:

1. **Fenced code blocks** inside any Markdown file, tagged `spl`.
2. **Standalone `.spl` files** anywhere in the vault.

````markdown
# Q2 Release

We're tracking the release of the next minor version here.

```spl
(given core-features-done)
(given docs-drafted)
```
````

ztl merges every `spl` block and every `.spl` file across the vault into **one combined theory** before reasoning. A fact stated on one page can feed a rule defined on another. This is the point — rules and facts can live next to the prose that motivated them.

## Facts

A fact is something you assert to be true. Use `(given ...)`:

```spl
(given project-approved)
(given has-owner)
(given budget-signed-off)
```

One fact per line is conventional. The thing you are asserting (`project-approved`) is called a **literal**. Literals are lower-case identifiers with dashes — ztl does not care about the particular word you pick, only that you spell it the same way everywhere.

To state the absence of something, prefix with `~` or write `(not ...)`:

```spl
(given (not blocked))
```

## Rules: normally and always

Rules say *if the body holds, the head holds*. Two flavours:

- `normally` — a defeasible rule. Holds by default, but can be defeated by a stronger rule or a defeater. Produces `+d` / `-d` conclusions.
- `always` — a strict rule. Cannot be defeated. Produces `+D` / `-D` conclusions.

```spl
(given project-approved)
(given has-owner)
(given code-merged)
(given docs-updated)
(given tests-green)
(given qa-signed-off)

; Defeasible: ordinarily true
(normally r-ready-to-start
  (and project-approved has-owner)
  ready-to-start)

; Strict: no exceptions
(always r-complete
  (and code-merged docs-updated tests-green qa-signed-off)
  complete)
```

The first token after `normally`/`always` is the rule's **label** — a short name you can refer to later. Labels are optional but helpful: proof trees and conflict reports use them to point back at the exact rule.

The body can be a single literal or an `(and ...)` of several. An implicit "all of these must hold" binds the conjunction.

## Defeaters

A defeater blocks a conclusion without asserting the opposite. Useful for exceptions:

```spl
(given open-critical-bug)

(except d-critical-bug
  open-critical-bug
  (not ready-to-ship))
```

If `open-critical-bug` is true, any defeasible rule concluding `ready-to-ship` is blocked. This is how you write "normally yes, **except** when …" without tangling up your main rules.

## Preferences

When two rules compete — one says `ready`, another says `not ready` — ztl flags the conflict. To resolve it, declare which rule wins:

```spl
(prefer d-critical-bug r-ready-to-start)
```

Reads: "prefer `d-critical-bug` over `r-ready-to-start`". The critical-bug defeater trumps the normal readiness rule.

## Comments

Lines starting with `;` are ignored:

```spl
; Superiority rules for Q2
(prefer d-critical-bug r-ready-to-start)
```

## Modal operators

SPL has three modal operators that wrap a literal to express permission, obligation, or prohibition:

| Operator | Meaning | Example |
|----------|---------|---------|
| `(may X)` | X is permitted | `(may (edit ?user ?page))` |
| `(must X)` | X is obligatory | `(must (review ?document))` |
| `(forbidden X)` | X is forbidden | `(forbidden (publish ?draft))` |

Use them as the head of a rule:

```spl
(given signed-contract)

(normally contract-requires-payment
  signed-contract
  (must pay))
```

They compose with defeasible reasoning — a narrower `forbidden` rule can defeat a broader `may` rule via `(prefer ...)`. This is how [[Access Control]] expresses policy like *"editors can edit research, **except** in `research/private/`."*

## Learning more

SPL is implemented by **[spindle-rust](https://codeberg.org/anuna/spindle-rust)**, which ships a formal reference: grammar, every keyword, temporal operators, arithmetic constraints, trust-weighted claims. If you're writing anything non-trivial, open its docs alongside this page.

## A realistic vault example

Imagine a project-tracking vault. You have a page per project, plus a `theories/` folder with the shared rules.

In `projects/Acme Website.md`:

````markdown
# Acme Website

Client: Acme Corp. Owner: Priya. Budget: approved Q1.

```spl
(given acme-approved)
(given acme-has-owner)
(given acme-budget-signed-off)
```
````

In `theories/project-readiness.spl`:

```spl
; A project is ready to start when approval, ownership, and budget are all in.
(normally r-acme-ready
  (and acme-approved acme-has-owner acme-budget-signed-off)
  acme-ready-to-start)

; Blocker: if the legal review is outstanding, we are not ready regardless.
(except d-acme-legal
  acme-legal-pending
  (not acme-ready-to-start))
```

Later, in `meetings/2026-03-15 Acme Kickoff.md`:

````markdown
Legal flagged a concern about the privacy policy wording.

```spl
(given acme-legal-pending)
```
````

Run `ztl reason status --literal "acme*"` and you'll see `acme-ready-to-start` go from `+d` to `-d` the moment the meeting note is saved. Remove the `given` once legal clears it, and the conclusion flips back. The rules never moved — only the facts.

## A few habits that pay off

- **Prefix literals by project or theme.** `acme-approved` reads better than `approved`, and it prevents collisions as the vault grows.
- **Keep `always` rules rare.** Strict rules are brittle. Save them for genuine invariants.
- **Name every rule you might want to defeat or prefer.** Bare defeasible rules work fine, but labels make conflict reports readable.
- **Put evergreen rules in `.spl` files** and point-in-time facts inside the page they belong to. Rules are stable; facts come and go.

## Related

- [[What is Defeasible Reasoning]]
- [[Running Queries]]
- [[Proof Trees]]
- [[What is SPL]]
