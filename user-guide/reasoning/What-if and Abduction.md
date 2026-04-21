---
title: What-if and Abduction
tags: [reasoning, hypothetical, abduction]
---

# What-if and Abduction

Two commands — `zetl reason what-if` and `zetl reason require` — let you play with the theory without actually editing it. One asks "what changes if I add this fact?". The other asks "what facts would I need to prove this goal?". Both are indispensable when planning.

> **Requires `--features reason` at install.** See [[Installation]].

## `reason what-if` — forward hypotheticals

`what-if` takes a snippet of SPL, temporarily adds it to the theory, recomputes conclusions, and reports the diff. Your vault files are not touched.

```bash
zetl reason what-if "(given acme-legal-cleared)"
```

Output: every conclusion that changed — literals that newly hold, literals that stopped holding, tags that flipped between `+d` and `-d`. The rest is filtered out.

### Focus on a specific goal

`--goal` narrows the diff to a single literal you care about. Answers the question "if I add this, does *this* become provable?":

```bash
zetl reason what-if "(given acme-legal-cleared)" --goal "acme-ready-to-start"
```

### From a file

If you want to try a larger experiment — a whole new rule, several facts — put it in a file and pass it with `--file`:

```bash
cat > /tmp/experiment.spl <<'SPL'
(given acme-legal-cleared)
(given acme-tests-green)
(prefer r-acme-ready d-acme-legal)
SPL

zetl reason what-if --file /tmp/experiment.spl --goal "acme-ready-to-start"
```

Useful when you are exploring "what if we relaxed this constraint?" without committing to the change.

### Use cases

- **Planning.** "If the client approves the proposal on Monday, are we clear to start?" — add `(given client-approved)` and look at the diff.
- **Gating a decision.** "The build is red. Would merging this feature make any currently-`+d` conclusion flip to `-d`?" — add the facts your feature introduces and check.
- **Defensive checks.** Before committing a new SPL rule, run `what-if` with it pasted in. If the conclusions diff is bigger than you expected, your rule is too broad.

## `reason require` — backward abduction

`require` is the inverse question: you have a goal in mind, and you want to know **what facts you'd need to add** to make it provable. This is abductive reasoning — inference from goal to premises.

```bash
zetl reason require "acme-ready-to-start"
```

zetl returns the minimal sets of missing facts that would complete the derivation. Each set is a separate solution — there may be several ways to prove the goal, via different rules.

### Example output

```text
Literal: acme-ready-to-start
Status: requirements_found

Solution 1 (via r-acme-ready):
  - acme-approved          (not derivable; must be asserted)
  - acme-has-owner         (not derivable; must be asserted)

Solution 2 (via r-acme-ready-alt):
  - acme-exec-override     (not derivable; must be asserted)
```

Read this as a to-do list. Each required fact is something you (or the project) must actually produce; it can't be derived from the rules currently in the vault.

### Chaining assumptions with `--assume`

The `--assume` flag lets you say "treat these as already given" and ask what else is still needed:

```bash
zetl reason require "acme-ready-to-start" \
  --assume "(given acme-approved)" \
  --assume "(given acme-has-owner)"
```

This is the practical planning form: you know some things are already true, you want to know what remains. Chain multiple `--assume` flags for cumulative hypotheticals.

### Capping results

`--max-solutions N` (default 5) caps how many distinct requirement sets are returned. For tightly constrained goals you usually get one; for goals reachable through many alternative rules you may want a higher cap to see all the options.

## Use cases side by side

| Question                                                | Command |
|--------------------------------------------------------|---------|
| "What's blocking the release?"                         | `reason require "release-candidate"` |
| "If the client approves, are we good?"                 | `reason what-if "(given client-approved)" --goal "ready"` |
| "Would adding this rule change any current conclusion?"| `reason what-if --file new-rule.spl` |
| "What single fact is closest to getting us unblocked?" | `reason require "ready-to-ship"` |

`require` is the command to run at the start of a planning session — it turns a vague goal into a concrete list of facts to produce. `what-if` is the command to run in the middle, as you weigh one decision at a time.

## Time travel

Both accept `--at` when `--features history` is installed. `require --at "last monday"` tells you what the required facts *were* at a past point, which is useful for decision retrospectives: "what did we think it would take to ship, before we knew about the legal issue?". See [[Time Travel]].

## Related

- [[Running Queries]]
- [[Proof Trees]]
- [[Writing SPL]]
- [[What is Defeasible Reasoning]]
