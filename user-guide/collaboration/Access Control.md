---
title: Access Control
tags: [collaboration, permissions, roles]
---

# Access Control

zetl's access model has two layers. The built-in layer — **three roles plus glob-based page scoping** — covers the 90% case: readers, editors, admins, optionally confined to a folder. The optional layer — **SPL-based deontic rules** — lets you express richer policies ("editor everywhere except `billing/**`", "read access only between 9:00 and 17:00") using the same reasoning engine that powers [[What is defeasible reasoning]].

## The three roles

| Role | View | Edit | Invite | Admin UI |
|------|:----:|:----:|:------:|:--------:|
| `reader` | yes | no | no | no |
| `editor` | yes | yes | no | no |
| `admin` | yes | yes | yes | yes |

A user's role is set on the invitation that brought them in (see [[Invitations]]) and can be changed later from `/_admin/users`. There's no implicit hierarchy beyond this table — an editor isn't a superset of admin, they're just different.

## Page scoping with globs

When you issue an invitation you can restrict which pages the recipient reaches at all:

```bash
zetl invite --as Alice --role editor --pages "research/**"
```

The scope is a glob against vault-relative paths. `projects/*` matches direct children of `projects/`. `projects/**` matches everything recursively. You can pass only one `--pages` at invite time; for more intricate patterns, use deontic rules below.

Pages outside the scope are invisible — not just uneditable. An out-of-scope page won't appear in search, won't show up in the graph, and returns 404 if requested by URL. This is a deliberate minimum-surface choice: users don't learn about documents they can't touch.

## Deontic rules with SPL

> **Requires `--features reason` at install time.** See [[Installation]].

Glob-based scoping can't express subtraction ("editor on `research/**` *except* `research/private/**`") or time windows. For those, zetl reads deontic rules from SPL blocks in your vault. SPL expresses permission and prohibition with **modal operators** — `(may X)`, `(must X)`, `(forbidden X)` — used as the *head* of a `normally` rule:

```spl
;; In a page like 'Vault Policies.md'
(normally editor-may-edit-research
  (and (has-role ?user editor)
       (page-matches ?page "research/**")
       (not (page-matches ?page "research/private/**")))
  (may (edit ?user ?page)))

(normally editors-forbidden-billing
  (and (has-role ?user editor)
       (page-matches ?page "billing/**"))
  (forbidden (edit ?user ?page)))
```

Deontic rules compose with defeasible reasoning: a narrower forbidding rule can **defeat** a broader permitting one via `(prefer editors-forbidden-billing editor-may-edit-research)`. You write policy as overlapping layers rather than as a flat table. Conflicts show up in `zetl reason conflicts` just like any other defeasible conflict. See [[Writing SPL]] for the full syntax, [[Running Queries]] for how to test rules, and the upstream [Spindle SPL reference](https://codeberg.org/anuna/spindle-rust) for the complete modal-operator specification.

Time-bounded access is typically encoded via temporal literals or a fact that an external hook asserts at the appropriate time:

```spl
;; A separate process (cron / on-save hook) asserts `(given during-business-hours)`
;; between 09:00 and 17:00 and retracts it otherwise.
(normally readers-may-view-internal-hours
  (and (has-role ?user reader)
       (page-matches ?page "internal/**")
       during-business-hours)
  (may (view ?user ?page)))
```

## The `on-access-request` hook

When a user tries to do something zetl's built-in rules forbid — view a scoped-out page, edit a read-only one — zetl can run an `on-access-request` hook instead of silently denying. The hook receives the user, the requested action, the target page, and the justification (a free-text field the user filled in) on stdin as JSON. It prints a decision — `allow`, `deny`, or `defer` — and zetl acts on that.

This is how you wire in a custom approval flow without forking zetl:

- **Slack approval bot**: the hook posts to a channel and waits for a thumbs-up.
- **Auto-approve inside working hours, page an admin otherwise**.
- **Deny but email the admin** for an audit trail.

The hook is a normal zetl lifecycle hook — any executable, any language. See [[Lifecycle Hooks]] for the hook runtime and payload conventions.

## Keeping it grounded

For a small team vault, three roles plus `--pages` glob scoping is almost always enough. Reach for SPL deontic rules when you have a real policy — HR-sensitive folders, time-bounded contractors, a reviewer who should see A and B but not C — not as a matter of architectural tidiness. Plain roles leave less room to get wrong.

## Related

- [[Invitations]]
- [[Running a Team Server]]
- [[Lifecycle Hooks]]
- [[Writing SPL]]
- [[What is SPL]]
