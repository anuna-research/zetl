---
id: SPEC-042
title: "Public Paths for `zetl --collab` — mixed unauthenticated + authenticated routing"
version: 0.1.0-strawman
status: draft
date: 2026-05-17
audience: agent, human
parent: SPEC-041
related:
  - SPEC-020  # Multi-user collaborative editing (the `--collab` base)
  - SPEC-041  # Pluggable authentication (the chain this extends)
  - SPEC-034  # Capability mode (parallel public-distribution story)
  - SPEC-005  # SPL / defeasible reasoning (authorization layer)
plan: DESIGN-042-public-paths-collab
---

# SPEC-042: Public Paths for `zetl --collab`

> **Strawman notice.** This document is a first-pass design drafted from a
> single design-conversation exchange, *before* the Phase 1 surveys,
> synthetic-user runs, and cross-model adversarial review called for by
> [[DESIGN-042-public-paths-collab]]. Per [[PROTO-001]] Constitutional
> Principle 11 ([[Anti-Slop Bias]]), treat every clause as carrying hidden
> debt until adversarial review proves otherwise. Sections marked
> **`[Provisional]`** are placeholders for grounded findings.
> [[Authentication]] / [[Authorization]] is a [[PROTO-001]] §AI Trust
> Boundaries **Tier 1 no-go area**: **no implementation begins** until the
> human-expert review package is approved. The document reaches `0.1.0`
> only after the Phase 1 + Phase 2 quality gates pass.

## Information Table

| Field          | Value                                                                  |
| -------------- | ---------------------------------------------------------------------- |
| Document ID    | [[SPEC-042-public-paths-collab\|SPEC-042]]                              |
| Title          | Public Paths for `zetl --collab`                                       |
| Version        | 0.1.0-strawman                                                         |
| Status         | Draft (strawman; pending [[DESIGN-042-public-paths-collab]] execution) |
| Author         | Agent (Claude Opus 4.7, [[PROTO-001\|USDD Agent Protocol]] v1.7.0)      |
| Date           | 2026-05-17                                                             |
| Audience       | Agent, Human                                                           |
| Trace          | [[PROTO-001]] §Phase 1, §Phase 2, §AI Trust Boundaries, §LangSec        |
| Parent         | [[SPEC-041]] Pluggable Authentication                                  |
| Related        | [[SPEC-020]] Collab base, [[SPEC-034]] Capability mode, [[SPEC-005]] SPL |
| Plan           | [[DESIGN-042-public-paths-collab]]                                     |
| Feature Gate   | `--features collab` (no new cargo features)                            |
| Review tier    | Tier 1 (security-sensitive; extends the [[Authentication]] core)       |

---

## 1. Overview

### 1.1 Problem

[[SPEC-041]] makes `zetl serve --collab` authentication pluggable, but
keeps the gate **binary**: when `--collab` is on, `collab_gate`
middleware requires an authenticated [[Principal]] on **every** content
route. There is no way to expose `/about` to anonymous visitors while
keeping `/private/**` behind auth in the same server.

Real deployments routinely want this mix:

- a public landing page + private notes;
- a blog with public posts + private drafts;
- a documentation portal with public reference + private internal pages;
- a portfolio with public work-samples + a private CMS surface.

The current workarounds — running a parallel bare `zetl serve` against a
subset of the vault, or relying on the [[SPEC-034]] capability-URL
static-site path — all force operators to duplicate state or choose an
architecture that doesn't match their mental model.

### 1.2 Core Insight

**The path-vs-principal decision is already made before any
authentication logic runs.** A request's URL tells us — without
consulting the [[AuthChain]] or [[SPL]] — whether the page is something
the operator marked public. The [[Authenticator]] chain and the
authorization layer don't need to learn anything new; we just teach
`collab_gate` to peek at the request path against an operator-declared
allowlist and bypass itself on match.

This is the same invariant SPEC-041 §1.2 names — authorization is the
seam that doesn't move. Anonymous-public requests skip the gate; the
[[Principal]] extension stays `None`; the chain didn't authenticate, and
SPL doesn't need to evaluate. The path glob *is* the policy for public
pages.

### 1.3 Design Principles

1. **Authorization is invariant.** The [[AuthChain]] doesn't change. SPL
   doesn't see anonymous requests. The path glob is a gate, not a
   policy. Anyone implementing per-page rules in [[SPL]] can ignore
   public_paths entirely.
2. **Default is today's behaviour.** A vault with no `public_paths` in
   `[collab.auth]` authenticates exactly as the SPEC-041 release does.
3. **One gate, one decision.** `collab_gate` makes the public-vs-gated
   decision in one place, before delegating to the principal check.
   `csrf_guard`, `admin_gate`, and the extractors are untouched — they
   only run when the gate let the request through.
4. **Safe methods only.** Anonymous `POST`/`PUT`/`DELETE` is forbidden
   in v1. Public paths are read-only ports — write attribution
   ([[SPEC-020]] §every-edit-is-attributed) cannot be satisfied for
   anonymous principals. State-changing requests against a public path
   are 405 / 403, not 200.
5. **Fail closed on configuration ambiguity.** A glob that matches the
   admin surface, or one too broad to be plausibly intentional (`/**`,
   `/_*`), is a startup error, not a silent surprise.
6. **All input is recognised before it is acted on.** Per [[PROTO-001]]
   Constitutional Principle 14 ([[LangSec]]), the `public_paths`
   patterns parse against a declared grammar and reject anything outside
   it (REQ-4210).
7. **Search and backlinks must learn.** If `/private/**` is gated,
   anonymous-visible search results and backlink lists must not leak
   private titles. The default for anonymous visitors is "scoped to
   public_paths."

### 1.4 Scope

**In scope:**

- A `[collab.auth] public_paths` glob list (TOML) that bypasses
  `collab_gate` for matching request paths.
- A **`zetl collab public-paths preview` CLI** ([[#REQ-4211]],
  [[#CON-4205]]) that resolves the configured globs against the vault
  scan and prints (a) every page slug that would be served
  unauthenticated, (b) the titles a `/search` request would surface
  for an anonymous visitor, (c) any glob that matches zero pages
  (likely a typo), and (d) any startup warnings (REQ-4206 dangerous
  shapes, REQ-4208 unreachable-SPL). **Operators MUST be able to
  preview the public surface before bringing a `--collab` server up.**
- Safe-method restriction at the gate.
- Startup validation rejecting dangerous globs.
- Interaction with the [[Capability URL]] authenticator: a capability
  principal's scope check is skipped on public paths (the page is
  public for everyone; the capability adds nothing).
- Interaction with [[#REQ-4112|REQ-4112 CSRF exemption]] from
  [[SPEC-041]]: anonymous requests have no cookie session, so the
  CSRF guard is a no-op for them — but the safe-method restriction
  means it never gets the chance to matter.
- Anonymous-aware search-result + backlink filtering (REQ-4207).
- Audit + operator-log entries naming public-path requests (REQ-4209).

**Out of scope:**

- Per-glob role overrides ("everyone with a capability-url can write
  /shared/**") — that's already expressible via [[SPEC-041]]
  [[Capability URL]] scopes; layering it here would conflate gates.
- Anonymous writes / form submissions. (Documented as a clean
  deferral; would require synthesising an "anonymous" attribution
  identity for git, plus a privacy story.)
- Per-host or per-vhost mixed serving (one process serving two
  vaults). Future spec.
- IP-based public/private (different policy by client IP). Adjacent
  to the [[SPEC-041]] proxy-header trust gate; would belong there.

---

## 2. User Profiles

> **`[Provisional — refined by [[DESIGN-042-public-paths-collab]] task
> user-profiles]`** Sketched from the design conversation; the plan task
> produces the grounded version after surveying real `--collab` adopters
> who've asked for mixed access.

### 2.1 Public-Landing Operator (carries from [[SPEC-020]] / [[SPEC-041]])

Runs `zetl serve --collab` on their own infrastructure. Wants `/` to
show a public landing page (the project's home, an "about us"
description, the latest blog post) so casual visitors can read it
without seeing a login wall. Everything else stays gated.

### 2.2 Documentation-Portal Operator

Has a knowledge base of mixed public/private docs. Public reference
material (`/docs/**`) goes to customers; internal runbooks
(`/internal/**`) stay private. Wants the same vault, the same git
history, the same author attribution — just two access surfaces.

### 2.3 Anonymous Reader *(new — motivates `public_paths`)*

Visits the site with no account, no cookie, no capability URL. Expects
to land on a page that renders normally — no login redirect, no 401.
Cannot write, cannot search private titles, cannot enumerate the vault
graph. Closest existing user profile is the [[SPEC-041]] §2.5
[[Link-Holder Collaborator]], but the anonymous reader doesn't even
have a link — they came from a search engine, a social-media
share, or a direct URL.

### 2.4 Authenticated User Crossing a Public Path

A logged-in operator who hits `/` (which is in `public_paths`). The
gate lets them through as a public request; the [[AuthChain]] still
runs first and may populate a [[Principal]] from their cookie, so the
page can show their name or an "edit" link if it wants. **The principal
exists; the gate just didn't require it.**

---

## 3. Happy Paths

> **`[Provisional — refined by [[DESIGN-042-public-paths-collab]] task
> happy-paths]`**

### 3.1 HP1: Default — No `public_paths`, Nothing Changes

**Preconditions:** Operator upgrades zetl; `.zetl/config.toml` has no
`public_paths` in `[collab.auth]`.

**Steps:** none. `zetl serve --collab` runs exactly as it does after
[[SPEC-041]]. Every content route requires authentication; anonymous
GETs receive the standard redirect/401.

**Postconditions:** Behaviour-identical to pre-SPEC-042. Verified by
the unchanged [[SPEC-041]] test suite ([[#TEST-4205]]).

### 3.2 HP2: Public Landing + Private Vault

**Preconditions:** Operator wants the root index + everything under
`/about/` public; everything else gated.

**Steps:**

1. Operator sets, in `.zetl/config.toml`:
   ```toml
   [collab.auth]
   methods       = ["passkey", "agent-token"]
   public_paths  = ["/", "/about", "/about/**"]
   ```
   (Note: `/` alone matches only the root path `/`. `/about` matches
   the bare `/about` page; `/about/**` matches everything under it.
   See [[#CON-4201|the common-patterns table]] for what each shape
   captures — getting this right is the most common operator
   stumble.)
2. Operator runs **`zetl collab public-paths preview`** ([[#REQ-4211]])
   to confirm the configured globs match the intended page set BEFORE
   starting the server. Output names every page slug that will be
   served unauthenticated and every page title that would surface in
   anonymous search.
3. Operator restarts `zetl serve --collab`. Startup line gains a
   `public=…` segment (OBS-4205); a startup WARN line per glob that
   matches zero pages (OBS-4207) catches typos.
4. Anonymous visitor opens `https://wiki.example.com/`. `collab_gate`
   matches the path against the glob → public → request proceeds. The
   landing page renders.
5. The landing page contains a `[[Welcome]]` wikilink, which renders as
   `<a href="/Welcome">`. The visitor clicks. `/Welcome` does NOT match
   any of `["/", "/about", "/about/**"]` → gate runs normally → 307
   redirect to `/auth/login`. **This is the cliff most operators
   stumble on:** wikilinks off a public page that target slugs outside
   the public_paths gate the visitor. The preview output catches the
   inverse (private pages the operator forgot to gate) but not this
   side directly — fix is to broaden `public_paths` (e.g. add `/Welcome`
   or use `/*` for "every top-level page"), OR redesign the landing
   not to link to gated pages.
6. Anonymous visitor follows a link to `/about/team/alice`. Glob
   `/about/**` matches → public → renders.
7. Anonymous visitor follows a link to `/private/notes`. No glob
   matches → 307 redirect to the login surface.

**Postconditions:** Pages within the declared `public_paths` reachable
without auth; everything else gated. No principal in public requests'
extensions. The preview run from step 2 gave the operator the full
picture before they exposed anything to the network.

### 3.3 HP3: Authenticated User on a Public Path

**Preconditions:** Same config as HP2; user has a valid `zetl_session`
cookie.

**Steps:**

1. User hits `/`. `auth_resolve` runs the chain; passkey adapter sees
   the cookie and yields a [[Principal]]. `collab_gate` sees the path
   matches `public_paths` AND the principal exists — both gates trivially
   pass.
2. The page renders, with the principal available to the handler (it
   can show "Logged in as Alice" etc.).

**Postconditions:** Anonymous and authenticated users get the same page
body; the handler may differentiate (login chrome, edit links).

### 3.4 HP4: Anonymous POST Rejected

**Preconditions:** Same config; anonymous visitor attempts to `POST` to
`/` or `/about/comment`.

**Steps:**

1. `collab_gate` sees method is `POST` and request path is in
   `public_paths`. REQ-4203 forbids state-changing methods on public
   paths.
2. Response: `405 Method Not Allowed` with a documented `Allow: GET,
   HEAD` header.

**Postconditions:** No write side-effect; the audit log records the
attempt (REQ-4209).

### 3.5 HP5: Search Doesn't Leak Private Titles

**Preconditions:** Same config; vault has `/about/intro` (public) and
`/internal/runbook` (private).

**Steps:**

1. Anonymous visitor (a) hits `/search?q=runbook`. The search endpoint
   recognises the request as anonymous (no principal) and constrains
   the index to `public_paths`-matching pages. No private titles
   appear.
2. Authenticated user hits the same URL. The search endpoint sees the
   principal and returns the full result set (subject to SPL).

**Postconditions:** Private title strings do not appear in any response
served to an anonymous principal.

### 3.6 HP6: Dangerous Glob Refused at Startup

**Preconditions:** Operator types `public_paths = ["/**"]` (which would
make everything public).

**Steps:**

1. Startup runs `validate(cfg)`. The pattern is recognised as the
   "match everything" shape and rejected with a named error (REQ-4206).
2. Server refuses to start.

**Postconditions:** Misconfiguration is a startup error, not a silent
catastrophe.

### 3.7 HP7: Preview Before Deploy

**Preconditions:** Operator has configured `public_paths` and wants to
audit the public surface before exposing it. The vault has 47 pages,
some sensitive.

**Steps:**

1. Operator runs `zetl collab public-paths preview`. The CLI scans the
   vault, compiles the globs, matches each page slug, and prints:

   ```
   [zetl] public_paths preview — .zetl/config.toml

   Configured globs (in [collab.auth]):
     "/"           → 1 page
       /

     "/about"      → 1 page
       /about

     "/about/**"   → 5 pages
       /about/contact
       /about/team
       /about/team/alice
       /about/team/bob
       /about/values

   Summary: 7 pages public, 40 pages gated, 0 globs match zero pages.

   Anonymous search (/search, /api/search) would surface these page
   titles to unauthenticated visitors (REQ-4207 boundary):
     About                  /about
     Contact us             /about/contact
     The team               /about/team
     Alice's bio            /about/team/alice
     Bob's bio              /about/team/bob
     Our values             /about/values
     [vault root]           /

   Validation:
     ✓ No dangerous globs (REQ-4206)
     ✓ No glob matches zero pages
     ✓ No SPL rule references an "anonymous" subject (REQ-4208)
   ```
2. Operator notices that `/about/values` was meant to be private,
   adjusts the glob to `/about/team/**` only, re-runs `preview`,
   confirms `/about/values` no longer appears in the output.
3. Operator restarts the server with the corrected config.

**Postconditions:** The operator deployed exactly the public surface
they intended; surprises were caught before any anonymous visitor saw
a private page. `--json` (`-f json` / piped) emits the same data
machine-readable for CI gating ("fail the deploy if the public-page
count exceeds N" or "fail if any page tagged `private` appears in the
public set").

**Failure modes (enumerated by [[DESIGN-042-public-paths-collab]]
task `happy-paths`):** glob that matches zero pages → WARN line + non-
zero exit when `--strict`; dangerous glob → same hard error as the
server's startup-time check, so preview and startup agree.

---

## 4. Functional Requirements

> Numbering: SPEC-042 → REQ-42xx. Each REQ is decomposed into positive /
> negative-input / negative-output tests per [[PROTO-001]]
> §Requirement-Targeted Test Decomposition.

### REQ-4201: `public_paths` Glob List

The system SHALL accept an optional `public_paths` field in the
`[collab.auth]` block of `.zetl/config.toml`, an array of
[[globset]]-compatible path-glob strings. Absent or empty ⇒ no public
paths (the SPEC-041 default behaviour holds).

**Trace:** [[#TEST-4201]], [[#CON-4201]]; [[#3.2 HP2]].

### REQ-4202: Gate Bypass Semantics

WHEN the request path (`request.uri().path()`, percent-decoded for the
match) matches any glob in `public_paths`, `collab_gate` SHALL allow
the request through regardless of whether `auth_resolve` produced a
[[Principal]]. WHEN the path does not match, the gate SHALL behave
exactly as the SPEC-041 implementation does today.

**Trace:** [[#TEST-4202]], [[#CON-4202]], [[#ADR-4201]]; [[#3.2 HP2]].

### REQ-4203: Safe-Method Restriction on Public Paths

WHEN a request whose path matches `public_paths` uses a method other
than `GET` or `HEAD`, `collab_gate` SHALL respond `405 Method Not
Allowed` with an `Allow: GET, HEAD` header AND SHALL NOT invoke the
downstream handler. This restriction applies even when the request
carries an authenticated [[Principal]] — the public-path classification
takes precedence to keep semantics unambiguous.

**Trace:** [[#TEST-4203]], [[#ADR-4202]]; [[#3.4 HP4]].

### REQ-4204: Capability-URL Principal + Public Path Interaction

WHEN a request whose path matches `public_paths` carries a
[[Capability URL]] principal, the capability-scope check (SPEC-041
[[#REQ-4117|REQ-4117]] / `capability_gate`) SHALL be skipped — the
page is public for everyone, and re-asserting scope would inconsistently
403 capability holders on pages anonymous visitors can read. The
principal still flows to the handler; `admin_gate` is unchanged
(capability principals never satisfy it).

**Trace:** [[#TEST-4204]], [[#ADR-4204]].

### REQ-4205: Backwards-Compatible Default

WHEN `.zetl/config.toml` does not declare `public_paths` (the field is
absent or the list is empty), the system SHALL behave exactly as the
SPEC-041 implementation: every content route requires authentication.

**Trace:** [[#TEST-4205]]; [[#3.1 HP1]].

### REQ-4206: Startup Validation of `public_paths`

The system SHALL validate `public_paths` at startup and SHALL refuse
to start on any of:

* a glob that fails to parse against the [[globset]] grammar (REQ-4210);
* a glob that matches the literal path `/` AND also `**` (the
  "match everything" shape, including `"/**"`, `"**"`, `"/*?**"`, etc.);
* a glob that matches any admin path (`/_admin/*`, `/_admin/**`);
* a glob that matches any `auth/*` path (`/auth/login`,
  `/auth/oidc/*`, etc.) — those routes are not part of `content_routes`
  and overriding them via this knob would have no useful effect;
* `[collab.auth.public_paths]` declared anywhere other than as a list
  (typed as `Option<Vec<String>>` with `deny_unknown_fields` on the
  parent table).

Each refusal SHALL name the offending pattern AND the corrective
action.

**Trace:** [[#TEST-4206]], [[#REQ-4210]]; [[#3.6 HP6]].

### REQ-4207: Search and Backlink Scoping for Anonymous Visitors

For requests resolved to no [[Principal]] AND whose path matches
`public_paths`, the search endpoint (`/api/search`, `/search`) AND the
backlinks endpoint AND any other endpoint that surfaces page-name lists
SHALL constrain their result set to pages whose canonical slug also
matches `public_paths`. Private titles, slugs, and excerpts SHALL NOT
appear in any response served to an anonymous request.

**Trace:** [[#TEST-4207]], [[#ADR-4203]]; [[#3.5 HP5]]; [[#Threat Model B]].

### REQ-4208: SPL Invariant (Anonymous Requests Don't Reach SPL)

[[SPL]] policy SHALL NOT be evaluated for an anonymous request that
the public-path gate admitted. The path glob is the policy for these
requests. Existing SPL rules that reference an "anonymous" subject (if
any) MUST surface as a startup warning so the operator notices that the
policy is unreachable.

**Trace:** [[#TEST-4208]], [[#ADR-4205]].

### REQ-4209: Audit and Operator-Log of Anonymous Accesses

The system SHALL emit an operator-log line and an audit-log line per
anonymous public-path request, with `method=anonymous` and an `identity`
field of `-` (the literal dash). The `cause` field SHALL be the matched
glob — operator-channel only, never user-visible. Anonymous denials
(safe-method violations per REQ-4203) SHALL also be logged with the
specific cause.

**Trace:** [[#TEST-4209]], [[#OBS-4202]], [[#OBS-4203]]; [[#3.4 HP4]];
[[SPEC-041]] [[#REQ-4115|REQ-4115]] redaction contract still applies.

### REQ-4211: `zetl collab public-paths preview` CLI

The system SHALL provide `zetl collab public-paths preview` that, given
the current `.zetl/config.toml`, resolves the configured globs against
the vault scan and prints:

* every page slug that matches any glob (sorted within each glob,
  globs presented in declared order);
* the page titles that would surface in an anonymous `/search` or
  `/api/search` response (the [[#REQ-4207]] / [[#CON-4204]] filter
  output);
* per-glob page-count summary AND a vault-wide `public / gated / total`
  summary;
* validation results: dangerous globs ([[#REQ-4206]]), globs matching
  zero pages (typo signal), SPL rules referencing an unreachable
  "anonymous" subject ([[#REQ-4208]]).

The command SHALL NOT start a server, open any network socket, or
modify any file. It is read-only with respect to the vault and idempotent.

Output respects the existing `-f json` / `-f table` (default `auto`)
convention. `--strict` SHALL exit non-zero if ANY validation entry is
a WARN or higher, so the command can gate a CI deploy.

**Trace:** [[#TEST-4211]], [[#CON-4205]], [[#ADR-4206]]; [[#3.7 HP7]].

### REQ-4210: Input Grammar for `public_paths`

Each entry in `public_paths` SHALL satisfy the following grammar
(REQ-4210 / [[PROTO-001]] §LangSec / Constitutional Principle 14):

```abnf
public-path-glob = "/" path-segment *( "/" path-segment )
path-segment     = 1*( segment-char / glob-meta / pct-encoded )
segment-char     = ALPHA / DIGIT / "-" / "_" / "."
glob-meta        = "*" / "?" / "[" 1*char "]"
pct-encoded      = "%" HEXDIG HEXDIG
```

Patterns failing this grammar are rejected at parse time, before
[[globset]] is consulted. `**` (multi-segment wildcard) appears as
two `*` segment-meta characters joined by `/` — the
canonical [[globset]] form. Patterns are case-sensitive on the
filesystem-canonical path representation.

**Trace:** [[#TEST-4210]], [[#CON-4201]]; [[PROTO-001]] §LangSec.

---

## 5. Non-Functional Requirements

### NFR-4201: Path-Match Latency

`collab_gate`'s public-path check SHALL add no more than **`[Provisional:
50 µs]`** at the 95th percentile to the request hot path per
`public_paths` glob compiled, with the upper bound holding for lists of
up to 256 patterns. Implementations SHOULD use a compiled
[[globset::GlobSet]] (single match call regardless of list size) rather
than a per-glob loop.

**Trace:** [[#TEST-NFR-4201]], [[#OBS-4201]].

### NFR-4202: Startup Glob Compilation

`public_paths` compilation at startup SHALL complete in ≤
**`[Provisional: 100 ms]`** for lists of up to 256 patterns. A failure
SHALL be a startup error, not a runtime fallback.

**Trace:** [[#TEST-NFR-4202]]; [[#REQ-4206]].

---

## 6. Architecture Decision Records

> ADRs sketched as positions. [[DESIGN-042-public-paths-collab]] plan
> tasks finalise each.

### ADR-4201: Gate-Level vs Route-Level Public Marking

**Status:** Proposed (strawman default)

**Context:** Two ways to mark a path public: (a) at the *gate* (glob
match in `collab_gate`), or (b) at the *route* (per-route
`#[public]`-style annotation on each handler). (b) gives finer per-
route control and is type-checked by axum, but every new route needs
to remember to mark itself, and operators can't customise the set
without recompiling.

**Decision:** (a). `public_paths` is operator config; the gate is the
single decision site; new routes inherit gating behaviour by default.

**Consequences:** (+) Operators control the set without touching code.
(+) One place to audit. (−) A route handler that does something special
for "public requests" still has to read the [[Principal]] extension to
notice it's anonymous — no compile-time hint.

### ADR-4202: Forbid State-Changing Methods on Public Paths (v1)

**Status:** Proposed (strawman default)

**Context:** Could anonymous visitors POST to a public page (e.g.,
submit a comment, sign a guestbook)? In principle yes; in practice
[[SPEC-020]]'s "every edit is attributed" invariant means a write
without a [[Principal]] has no author. Synthesising an "anonymous"
git-author identity raises its own privacy + spam questions.

**Decision:** v1 forbids non-safe methods (anything other than `GET` /
`HEAD`) on public paths, with a `405 Method Not Allowed` response.
Comment / form / submission flows that need anonymous writes are a
clean follow-up spec — they require an attribution story that doesn't
exist today.

**Consequences:** (+) Crisp semantics; no surprise writes. (+) Attack
surface stays narrow (no anonymous POST = no anonymous CSRF concerns
on public paths). (−) Operators who want public comment forms must use
a separate service or wait for the follow-up.

### ADR-4203: Default Anonymous-Search Behaviour — Scoped, Not Refused

**Status:** Proposed (strawman default)

**Context:** When an anonymous visitor hits `/search`, three options:
(i) return 401 / redirect to login (no search), (ii) return scoped
results (only `public_paths` matches), (iii) return the full search
(leaks private titles).

**Decision:** (ii). Returning empty + helpful is the natural
extension of "I serve public pages without auth" — a public landing
should let visitors find other public pages. (iii) is a leak. (i)
makes the public site unusable for discovery.

**Consequences:** (+) Public-site UX is complete. (−) Search-result
ranking may degrade against the smaller corpus; document as expected.
(−) Operators who want anonymous search disabled outright can set
`[access.search] mode = "off"` ([[SPEC-034]]) as today.

### ADR-4204: Capability Principals Get Public Access (No Scope Check)

**Status:** Proposed (strawman default)

**Context:** If `/` is public AND a capability-URL holder requests it,
should `capability_gate` evaluate the scope? Per SPEC-041's strict
scope check, a capability bound to `/shared/**` would 403 on `/`
because `/` isn't in scope — even though every anonymous visitor can
read `/`.

**Decision:** Skip the capability-scope check when the path matches
`public_paths`. Reasoning: the capability adds no authority above
"anyone with the URL can read it" for public pages; failing closed
would be operationally confusing ("the link I sent doesn't work on the
home page"). `admin_gate` is unchanged — capability principals still
can't reach `/_admin/*`.

**Consequences:** (+) Capability URLs work intuitively on mixed
sites. (−) A capability holder cannot be *more restricted than
anonymous* via this mechanism — but that's already the case (anonymous
sees public pages; a capability holder also sees public pages plus
their scope).

### ADR-4206: Preview CLI Over Startup-Warning-Only

**Status:** Proposed (strawman default — operator-experience-first)

**Context:** Two ways to help the operator understand what they're
about to expose: (a) print warnings at server-startup time (cheap, no
new surface), (b) ship a separate `zetl collab public-paths preview`
CLI that resolves globs against the vault scan and lists pages +
anonymous-search titles before the server is up.

**Decision:** Both, but the preview is the load-bearing tool. (a) is
necessary (typos + dangerous globs MUST stop the server cold) but
insufficient — at startup the operator is already "going live" and may
miss a WARN line in the noise. (b) lets the operator iterate locally
(`vim config.toml && zetl collab public-paths preview` until it
matches the intended page set), gate CI deploys (`--strict` exits
non-zero on any WARN), and audit the public surface as the vault
grows over time (re-run weekly).

**Consequences:** (+) Operators see exactly which page slugs and
which search titles go public BEFORE any anonymous visitor does — the
[[#Threat Model B]] leakage surface becomes visible in dry-run, not
post-incident. (+) Mirrors the existing `zetl cap check` /
`zetl hook dry-run` pattern; consistent operator muscle memory. (−)
One more surface to maintain. (−) Preview output drifts from runtime
behaviour if the vault scan and the gate use different path
canonicalisation — the implementation must share the canonicaliser
between the two (deferred to [[DESIGN-042-public-paths-collab]] task
`preview-cli` to validate against a fixture vault).

### ADR-4205: SPL Doesn't See Anonymous (Path Glob *Is* the Policy)

**Status:** Proposed (strawman default — minimal coupling)

**Context:** Could [[SPL]] rules reference an "anonymous" subject to
e.g. forbid certain pages even for anonymous visitors? Technically yes;
but it would create two policy surfaces (`public_paths` and SPL
`(forbidden read (subject anonymous) …)`) that could conflict.

**Decision:** v1 routes anonymous requests around SPL entirely. The
`public_paths` glob is the policy for these requests. SPL rules that
reference an "anonymous" subject (if any exist in a vault) surface as
a startup warning so the operator knows their rule is unreachable.

**Consequences:** (+) One source of truth for "what's public." (+)
Existing SPL libraries don't have to learn about anonymous. (−)
Operators who want layered policy (e.g., "public except this private
sub-folder") express it via the glob (`"!/about/secret/**"` if
[[globset]] supports negation; otherwise narrow the positive glob).
Documented limitation.

---

## 7. Contracts

### CON-4201: `[collab.auth] public_paths` Schema

**Grammar:** The input is a [[TOML]] document — same recogniser as
[[SPEC-041]] [[#CON-4102|CON-4102]]. The added field:

```toml
[collab.auth]
public_paths = ["/", "/about", "/about/**", "/blog/*", "/docs/**"]
```

`Option<Vec<String>>`, default `None`. The string-element grammar is
[[#REQ-4210]]. The parent `[collab.auth]` table's `deny_unknown_fields`
remains; a misspelt `publicpaths` is a startup error per [[SPEC-041]].

**Common patterns** (a table operators should consult before deploying;
mirrored in `docs/collab-auth.md` and surfaced by `zetl collab
public-paths preview` per [[#REQ-4211]]):

| Glob              | Matches                                   | Use case                                              |
| ----------------- | ----------------------------------------- | ----------------------------------------------------- |
| `/`               | ONLY the root path `/`                    | Bare landing page; visitors must log in for anything else |
| `/file`           | The literal path `/file`                  | A single named page                                   |
| `/*`              | Single-segment paths (`/file`, `/About`)  | Root index + every top-level page (no descent)        |
| `/about`          | The literal path `/about`                 | The bare `about` page (no children)                   |
| `/about/*`        | Direct children of `/about` only          | `/about/contact` matches; `/about/team/alice` does not |
| `/about/**`       | `/about` AND everything under it          | `about` section, recursive                            |
| `/blog/*.html`    | One-deep `*.html` files under `/blog`     | Published `.html` only, no draft sub-folders          |
| `/**`             | (REJECTED at startup)                     | Use `zetl serve` without `--collab` instead           |

Semantics inherit from [[globset]]'s `gitignore`-style globs: each
pattern is anchored to the full request path; `*` matches one path
segment (no descent into `/`); `**` matches zero-or-more components.
**Most operator stumbles are about not anchoring the path the way they
expected** — run [[#REQ-4211|preview]] first and the surprises surface
before the server is up.

**Pre-conditions:** Read once at startup.

**Post-conditions:** (REQ-4201) field is parsed into a `Vec<String>`;
absent ⇒ `None`. (REQ-4206) each pattern passes the REQ-4210 grammar
and the dangerous-shape rejects, OR startup fails naming the pattern.

**Implements:** [[#REQ-4201]], [[#REQ-4206]], [[#REQ-4210]].
**Verified by:** [[#TEST-4201]], [[#TEST-4206]], [[#TEST-4210]].

### CON-4202: `collab_gate` Public-Path Bypass

The gate's decision tree gains one branch at the top:

```rust
pub async fn collab_gate(
    State(state): State<WebState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if !state.collab { return next.run(request).await; }

    // SPEC-042 — public-path bypass. Cheap GlobSet match on the
    // request path; on hit, safe-method check then pass-through.
    if state.public_paths.is_match(request.uri().path()) {
        let method = request.method();
        if method != Method::GET && method != Method::HEAD {
            return (StatusCode::METHOD_NOT_ALLOWED,
                    [("allow", "GET, HEAD")]).into_response();
        }
        return next.run(request).await;
    }

    // … pre-SPEC-042 logic unchanged: check Principal extension,
    // redirect / 401 on miss.
}
```

**Pre-conditions:** `auth_resolve` has already run (the Principal
extension may be `Some` or `None`). `state.public_paths` is a
pre-compiled [[globset::GlobSet]].

**Post-conditions:** (REQ-4202) public-path matches with a safe method
proceed; non-safe methods return 405. (REQ-4205) absent or empty
`public_paths` reproduces pre-SPEC-042 behaviour.

**Implements:** [[#REQ-4202]], [[#REQ-4203]], [[#REQ-4205]].
**Verified by:** [[#TEST-4202]], [[#TEST-4203]], [[#TEST-4205]].

### CON-4203: Capability-Gate Public-Path Bypass

`capability_gate` gains a symmetric early-return:

```rust
pub async fn capability_gate(req, next) -> Response {
    // unchanged — read Principal, peek at capability grant
    let Some(grant) = principal.capability else { return next.run(req).await; };

    // SPEC-042 — skip scope/role check on public paths.
    if PUBLIC_PATHS.is_match(req.uri().path()) {
        return next.run(req).await;
    }

    // …existing scope + role checks unchanged.
}
```

**Pre-conditions:** Principal carries a [[CapabilityGrant]]; gate is
not a no-op.

**Post-conditions:** (REQ-4204) on public-path match, scope + role
checks are skipped — the request proceeds. Off public paths,
SPEC-041 [[#REQ-4117|REQ-4117]] enforcement is unchanged.

**Implements:** [[#REQ-4204]]. **Verified by:** [[#TEST-4204]].

### CON-4205: CLI — `zetl collab public-paths preview`

**Endpoint:** `zetl collab public-paths preview [--strict] [--json]`.
Reads `.zetl/config.toml`, scans the vault, and emits the structured
output described in [[#3.7 HP7]]. The command is read-only (no file
writes, no network).

**Output schema** (JSON form, also reflected in the table form):

```json
{
  "globs": [
    { "pattern": "/about/**", "pages": ["/about/contact", "/about/team", ...] }
  ],
  "summary": { "public": 7, "gated": 40, "total": 47, "zero_match_globs": 0 },
  "anonymous_search_titles": [
    { "title": "About", "slug": "/about" }
  ],
  "validation": {
    "dangerous_globs":          [],
    "zero_match_globs":         [],
    "unreachable_anonymous_spl": []
  },
  "exit_code": 0
}
```

**Pre-conditions:** vault is scannable; `[collab.auth]` parses.

**Post-conditions:** (REQ-4211) every matching slug + every anonymous-
search title is listed; per-glob counts and a vault total are emitted;
validation results name any dangerous glob, zero-match glob, or
unreachable-anonymous-SPL rule. `--strict` causes exit code 2 if any
WARN-level entry appears; without it, exit code is 0 on a successful
run regardless of warnings (operator can run for visibility without
gating).

**Error model:** non-zero exit + stderr for: vault not scannable,
`.zetl/config.toml` parse error (same error message as the server
startup would emit for byte-for-byte consistency), feature mismatch
(unimplemented method named in `methods`).

**Implements:** [[#REQ-4211]]. **Verified by:** [[#TEST-4211]].

### CON-4204: Anonymous-Aware Search / Backlinks Filtering

The search and backlinks query paths gain an `is_anonymous: bool` flag
derived from `request_principal(extensions).is_none()`. When `true`,
the result set is filtered through the same [[globset::GlobSet]] used
by the gate; only pages whose slug matches `public_paths` survive into
the response body. The filter is applied *after* the search engine
returns results but *before* serialisation, so titles, slugs, and
excerpts of private pages never leave the process.

**Pre-conditions:** A search / backlinks request has been authorised
by `collab_gate` (either via Principal or via public-path bypass).

**Post-conditions:** (REQ-4207) anonymous responses contain only pages
matching `public_paths`. Authenticated responses are unchanged.

**Implements:** [[#REQ-4207]]. **Verified by:** [[#TEST-4207]].

---

## 8. Test Specifications

> Per [[PROTO-001]] §Selecting a Verification Strategy: this is an
> AI-synthesised, Tier-1, security-critical extension — requirement-
> targeted test decomposition (positive / negative-input / negative-
> output), mutation testing on the gate, fuzzing on the glob recogniser,
> and adversarial testing are **mandatory**.

| ID                    | Technique                 | Target                                                                                | Validates                          |
| --------------------- | ------------------------- | ------------------------------------------------------------------------------------- | ---------------------------------- |
| [[#TEST-4201]]        | example + neg-input       | Valid `public_paths` parses to a non-empty list; invalid TOML shapes fail              | [[#REQ-4201]]                      |
| [[#TEST-4202]]        | example                   | Path in glob → 200 / pass-through; path not in glob → 307 / 401                       | [[#REQ-4202]]                      |
| [[#TEST-4203]]        | example + neg-output      | POST/PUT/DELETE on public path → 405 with `Allow: GET, HEAD`                          | [[#REQ-4203]]                      |
| [[#TEST-4204]]        | example                   | Capability principal hits public path → 200 (scope check skipped); off public path → REQ-4117 unchanged | [[#REQ-4204]]   |
| [[#TEST-4205]]        | snapshot                  | No `public_paths` ⇒ SPEC-041 / SPEC-020 collab suites pass unchanged                  | [[#REQ-4205]]                      |
| [[#TEST-4206]]        | example + neg-input       | Each dangerous-glob shape (`/**`, `/_admin/**`, `/auth/**`, bad syntax) → startup error naming pattern | [[#REQ-4206]]    |
| [[#TEST-4207]]        | example                   | Anonymous search omits private slugs; authenticated search returns full set            | [[#REQ-4207]]                      |
| [[#TEST-4208]]        | example                   | SPL `(forbidden read (subject anonymous) …)` rule surfaces startup warning           | [[#REQ-4208]]                      |
| [[#TEST-4209]]        | example                   | Anonymous request → operator log + audit line with `method=anonymous identity=-`     | [[#REQ-4209]]                      |
| [[#TEST-4210]]        | fuzz + property           | Random byte sequences against the public-path-glob recogniser: no panics, no acceptance of out-of-grammar input | [[#REQ-4210]] |
| [[#TEST-4211]]        | example + snapshot        | Preview CLI emits expected slug + title + summary + validation sections against a fixture vault; `--json` is structurally stable; `--strict` exits non-zero on WARN; no file writes; output agrees with the actual gate's runtime behaviour | [[#REQ-4211]]                      |
| [[#TEST-NFR-4201]]    | benchmark                 | Hot-path match ≤ 50 µs 95p for 256-pattern GlobSet                                   | [[#NFR-4201]]                      |
| [[#TEST-NFR-4202]]    | benchmark                 | Startup glob compilation ≤ 100 ms for 256 patterns                                    | [[#NFR-4202]]                      |
| TEST-mutation-gate    | mutation                  | Mutation kill rate ≥ 90% on the public-path branch of `collab_gate`                  | [[#REQ-4202]] robustness           |
| TEST-mutation-validate | mutation                | Mutation kill rate ≥ 90% on `validate_public_paths` (dangerous-shape rejection)       | [[#REQ-4206]] robustness           |
| TEST-fuzz-glob        | fuzz                      | Glob recogniser against arbitrary bytes: never panics, never bypasses                 | [[#REQ-4210]]                      |
| TEST-adversarial-042  | adversarial (cross-model) | Fresh-context adversary attacks the SPEC-042 REQ set for admitted-but-unintended behaviour | all REQ-42xx                  |

---

## 9. Observability Signals

| ID             | Type   | Signal                                                                                          | Trace                              |
| -------------- | ------ | ----------------------------------------------------------------------------------------------- | ---------------------------------- |
| [[#OBS-4201]]  | metric | `zetl_collab_public_path_match_duration_seconds` histogram, label `outcome` (match/miss)         | [[#NFR-4201]]                      |
| [[#OBS-4202]]  | metric | `zetl_collab_public_path_requests_total{glob, method, outcome}` counter (outcome ∈ admit/method-rejected) | [[#REQ-4209]]              |
| [[#OBS-4203]]  | log    | Per-decision operator log: `[zetl] auth: method=anonymous outcome=admitted glob=<g>` (admit) or `cause=method-not-allowed` (reject) | [[#REQ-4209]] |
| [[#OBS-4204]]  | log    | Audit log: same shape as SPEC-041 [[#OBS-4104|OBS-4104]] but with `method=anonymous identity=-` | [[#REQ-4209]]                      |
| [[#OBS-4205]]  | log    | Startup line gains a `public=[<glob>, <glob>, …]` segment listing the configured patterns        | [[#REQ-4201]]                      |
| [[#OBS-4206]]  | log    | Startup WARN line per SPL rule referencing an unreachable "anonymous" subject (REQ-4208)         | [[#REQ-4208]]                      |
| [[#OBS-4207]]  | log    | Startup WARN line per glob that matches zero pages in the current vault (typo signal) — same data the preview surfaces, surfaced again at server-up time for operators who skipped the dry-run | [[#REQ-4206]], [[#REQ-4211]] |

> The `glob` label on OBS-4202 is **operator-channel only**; it MUST
> NOT be exposed on any unauthenticated HTTP-readable metrics endpoint,
> to avoid letting anonymous visitors enumerate which paths the operator
> classified as public via metric scraping.

---

## 10. Purity Boundary Map

> **`[Provisional]`**

### Pure Core (no I/O, no shared state, deterministic)

- `web::auth::public_paths::parse(value: &toml::Value) -> Result<PublicPathsConfig, ConfigError>` — TOML lens + REQ-4210 grammar check.
- `web::auth::public_paths::validate(cfg: &PublicPathsConfig) -> Result<(), ConfigError>` — REQ-4206 dangerous-shape rules.
- `web::auth::public_paths::compile(cfg: &PublicPathsConfig) -> Result<GlobSet, GlobError>` — eager compile.
- `web::auth::public_paths::is_public(set: &GlobSet, path: &str) -> bool` — the hot-path predicate.

### Effectful Shell (orchestrates I/O, calls pure core)

- `collab_gate` middleware — reads `state.public_paths`, calls `is_public`, returns 405 / pass-through / falls through to the existing logic.
- `capability_gate` middleware — calls `is_public` to decide whether to skip the scope check.
- Search / backlinks endpoints — call `is_public` per result before serialising.
- Audit + operator-log emission for OBS-4203/OBS-4204.

### Dependency Rule

Shell modules MAY import pure-core modules; the reverse MUST NOT
hold. Enforced via `clippy::disallowed_methods` on `std::fs::*`,
`SystemTime::now`, `tokio::*`, and HTTP-client crates inside the pure
modules — the same mechanism [[SPEC-041]] [[#10. Purity Boundary Map]]
uses.

---

## 11. Threat Model (Summary)

> Detailed model lives in `research/SPEC-042-threat-model.md`, produced
> by [[DESIGN-042-public-paths-collab]] task `threat-model`. This
> section summarises adversaries.

### Threat Model A — Glob Overreach

> An operator types `public_paths = ["/**"]` (or any pattern that
> matches everything) intending "all docs under /docs" but spelled
> wrong. The entire vault becomes anonymous-readable.

**Mitigation:** [[#REQ-4206]] startup validation rejects the literal
"match everything" shapes. Documentation cautions about positive-list-
plus-narrowing as the safer pattern. **Residual risk:** an operator who
genuinely intends `"/**"` would have to remove the safeguard manually;
not a default-state vulnerability.

### Threat Model B — Title / Slug / Excerpt Leakage via Search

> Anonymous visitor hits `/search?q=*`; if search isn't scoped to
> `public_paths`, private titles and excerpts leak.

**Mitigation:** [[#REQ-4207]] / [[#CON-4204]] — search engine results
are filtered through the same GlobSet before serialisation. The
filter applies at the response boundary, not at the index, so the
search engine still uses the full corpus; only the OUTPUT changes.

**Residual risk:** if a future feature adds another endpoint that
emits page-name lists (e.g. a sitemap, an RSS feed, an autocomplete
endpoint) and forgets to filter, it leaks. Mitigation: documentation +
a `is_public` predicate that's easy to thread through every new
list-emitting endpoint. A linter / static-analysis check could catch
this in CI.

### Threat Model C — Anonymous State Mutation

> Anonymous visitor crafts a POST to a public path expecting to mutate
> server state.

**Mitigation:** [[#REQ-4203]] — POST / PUT / DELETE on public paths
return 405. The gate enforces this BEFORE any handler runs, so even a
handler that legitimately accepts unauthenticated writes (none exist
today) couldn't be exploited via this mechanism. **Residual risk:**
none beyond the deferred "anonymous comment forms" use case.

### Threat Model D — Glob-Match Performance DoS

> Adversary discovers that the path-glob match is slow; floods the
> server with long, pathological paths to exhaust CPU.

**Mitigation:** [[#NFR-4201]] caps hot-path latency at 50 µs 95p; the
compiled GlobSet is single-call regardless of list size. The existing
[[SPEC-041]] [[AuthRateLimiters]] per-IP layer applies before the gate
in `auth_routes`, but content_routes don't have a per-IP limiter today —
that's a SPEC-041 follow-up, not new for SPEC-042. **Per the
[[PROTO-001]] §Security-Review exclusions**, DoS is out of scope for
this spec.

### Threat Model E — SPL / `public_paths` Policy Confusion

> An operator writes both a `public_paths` glob and an SPL rule
> referencing "anonymous", expecting layered policy. SPL doesn't fire
> for public requests; the operator's expectation isn't met.

**Mitigation:** [[#REQ-4208]] surfaces a startup warning for any SPL
rule referencing the "anonymous" subject when `public_paths` is set.
Documentation explicitly states "public_paths is the policy for
anonymous requests; SPL does not evaluate."

### Threat Model F — Capability-URL Operator Confusion (skipped scope)

> An operator mints a capability for `scope = "shared/**"` and shares
> the URL. The recipient visits `/` (which the operator listed as
> public). Per [[#ADR-4204]], the scope check is skipped and the page
> renders. The operator may be surprised that the cap URL "works on the
> home page."

**Mitigation:** The home page was already public for everyone; the
capability adds nothing. Documentation explains the principle ("public
is public; capability adds authority on TOP of that, not BELOW").
**Residual risk:** operator UX confusion only; no security weakening.

---

## 12. Quality Attribute Checklist

> **`[Provisional]`** Applied to each REQ in [[#4. Functional
> Requirements]].

| REQ            | Unambiguous | Verifiable | Atomic | Consistent | Quantified         | Traceable | Error-aware |
| -------------- | :---------: | :--------: | :----: | :--------: | :----------------: | :-------: | :---------: |
| [[#REQ-4201]]  | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |
| [[#REQ-4202]]  | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |
| [[#REQ-4203]]  | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |
| [[#REQ-4204]]  | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |
| [[#REQ-4205]]  | ✓ | ✓ | ✓ | ✓ | n/a (binary) | ✓ | n/a |
| [[#REQ-4206]]  | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |
| [[#REQ-4207]]  | ⚠ "any other endpoint that surfaces page-name lists" needs enumeration | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |
| [[#REQ-4208]]  | ⚠ "if any exist" — needs survey | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |
| [[#REQ-4209]]  | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |
| [[#REQ-4210]]  | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |

Provisional ⚠ entries close once the named [[DESIGN-042-public-paths-collab]]
task completes.

---

## 13. Implementation Plan

> Phased so the change ships in two coherent slices: the gate-level
> bypass (Phase 0–1) is independently useful even without anonymous-
> aware search (Phase 2). Each phase is gated by its own tests. The
> detailed task DAG lives in [[DESIGN-042-public-paths-collab]]; this
> section is the human-readable summary. **No phase begins** before the
> Tier-1 human-expert review of this specification.

### Phase 0 — Pure-Core + Config Lens + Preview Resolver

**Goal:** the pure pieces, no server wiring; preview can already be
useful against a vault even before the gate is wired.

- New `src/web/auth/public_paths.rs`: `PublicPathsConfig` (typed
  `Vec<String>`), `parse` (REQ-4210 grammar), `validate` (REQ-4206
  dangerous-shape rules), `compile` (→ `globset::GlobSet`), `is_public`
  (hot-path predicate).
- New `resolve(set: &GlobSet, vault: &VaultData) -> PreviewReport`
  (pure): walks the vault page index, returns per-glob matches +
  zero-match-globs + the title list for anonymous search.
- Extend [[SPEC-041]] `CollabAuthConfig` with
  `public_paths: Option<Vec<String>>`.
- Unit tests: grammar accept/reject matrix, dangerous-shape rejection
  matrix, compile+match round-trip, resolver against a fixture vault.
- **Gate:** [[#TEST-4201]], [[#TEST-4206]], [[#TEST-4210]] green.

### Phase 0.5 — Preview CLI

**Goal:** operators can audit the public surface before any server is
up. Independently useful even before Phase 1 ships the gate.

- `src/cli.rs`: extend the [[SPEC-041]] `CollabCommand` with
  `PublicPaths { command: PublicPathsCommand }` and
  `PublicPathsCommand::Preview { strict, /* output flags inherited */ }`.
  Variant doc-comments scrubbed of SPEC-IDs per the existing
  `test_help_no_spec_references` constraint.
- `src/main.rs`: handler that loads config, scans the vault, calls the
  Phase-0 `resolve`, renders the `[[#3.7 HP7|preview output]]` (table
  or JSON per the existing `-f` flag).
- Integration tests via `assert_cmd` against fixture vaults: happy
  path, glob-with-zero-matches, dangerous-glob (same error byte-for-
  byte as the server's startup-time error per [[#CON-4205]]),
  `--strict` exit behaviour.
- **Gate:** [[#TEST-4211]] green.

### Phase 1 — Gate Bypass

**Goal:** mixed public/auth pages work; SPEC-041 chains untouched.

- Add `pub public_paths: Arc<globset::GlobSet>` to `WebState` (Arc so
  it's cheap to clone). Compiled once in `web::run` from the
  Phase-0 config.
- Modify `src/web/session.rs::collab_gate` per [[#CON-4202]].
- Modify `src/web/auth/capability_url.rs::capability_gate` per
  [[#CON-4203]].
- OBS-4203 / OBS-4204 / OBS-4205 wiring.
- Integration tests against an axum mock router for the gate matrix.
- **Gate:** [[#TEST-4202]], [[#TEST-4203]], [[#TEST-4204]],
  [[#TEST-4205]], [[#TEST-4209]] green. Existing SPEC-041 suite
  unchanged.

### Phase 2 — Anonymous-Aware Search / Backlinks

**Goal:** no title / slug / excerpt leakage to anonymous visitors.

- Thread `is_anonymous: bool` into the search + backlinks query
  pipelines.
- Add the per-result `is_public` filter at the response boundary
  ([[#CON-4204]]).
- Audit every endpoint that emits page-name lists (`/api/search`,
  `/search`, `/api/backlinks/*`, `/api/graph`, `/llms.txt`, RSS
  feeds, sitemap if present). Each gets the filter or a documented
  exemption.
- **Gate:** [[#TEST-4207]] green; targeted property test that no
  private slug appears in any anonymous response across the full
  endpoint inventory.

### Phase 3 — SPL Coherence + Docs + Review

- SPL startup-warning for unreachable-anonymous rules ([[#REQ-4208]]).
- `docs/collab-auth.md` extended with a "Public + Private Pages"
  section, the threat-model summary, and the dangerous-glob list.
- `user-guide/collaboration/Authentication Methods.md` extended with
  a `public_paths` subsection.
- CHANGELOG entry under `[Unreleased]`.
- TEST-adversarial-042 — cross-model adversarial review of the
  deliverable (PROTO-001 Principle 12, fresh context, different
  model).

### Sequencing Rationale

Phase 0 is pure data + grammar; trivially reversible. **Phase 0.5
ships the preview CLI before the gate is wired** — operators get a
"show me what this WOULD do" tool with no server change, so the
SPEC-042 design can be evaluated against real vaults before any code
path actually exposes a page anonymously. Phase 1 ships the operator-
visible feature with the leak-safe property (anonymous requests can't
write, can't search private titles via the gate mechanism). Phase 2
closes the search/backlinks leak surface and makes the feature
complete. Phase 3 is hardening + docs + review.

---

## 14. Status & Next Actions

- This strawman is an **input** to
  [[DESIGN-042-public-paths-collab]], not an output. The plan's tasks
  refine every `[Provisional]` section and finalise the REQ / CON / ADR
  IDs against the highest existing IDs at draft time.
- **No implementation begins** until: (a) the Phase 1 + Phase 2
  quality gates pass; (b) cross-model adversarial review completes
  ([[PROTO-001]] Constitutional Principle 12); (c) the human-expert
  review package is approved — extending the [[Authentication]] /
  [[Authorization]] gate is a [[PROTO-001]] §AI Trust Boundaries
  Tier-1 area.
- The anonymous-aware search / backlinks scoping ([[#REQ-4207]])
  carries the highest leakage risk in this specification ([[#Threat
  Model B]]); the [[DESIGN-042-public-paths-collab]] `threat-model`
  task and the human-expert review MUST explicitly sign off on the
  full list-emitting-endpoint inventory before Phase 2 ships.
- After review and refinement, this document is re-issued at version
  `0.1.0`, status `approved`, with the provisional markers removed.

---

## 15. Open Questions Surfaced by This Strawman

1. **Negative globs.** [[globset]] doesn't natively support exclusion
   patterns. Should `public_paths` accept a leading `!` for "match this
   but explicitly exclude this", or do operators just narrow the
   positive patterns? Strawman defers; the `validate` task decides.
2. **Anonymous comment forms / sign-this-page** — explicit deferral
   from [[#ADR-4202]]. Worth a follow-up spec when the use case has a
   concrete adopter.
3. **Cache headers on public responses.** Should the gate add
   `Cache-Control: public, max-age=…` on admitted public requests so
   intermediary CDNs / reverse proxies can cache? Probably operator
   choice (some operators set their own at the proxy layer); strawman
   defers.
4. **`robots.txt` and search-engine indexability.** Public pages
   probably want to be indexable; private pages should be
   `Disallow:`'d. Should the gate emit a generated `/robots.txt`
   reflecting `public_paths`? Likely yes; small adjacency, may belong
   in a follow-up.
5. **Multi-IdP / multi-realm** (out-of-scope reminder). [[SPEC-041]]
   §15.4 already flags this; SPEC-042 doesn't change that picture.
6. **Per-glob role override** (out-of-scope reminder). The
   "everyone-with-this-link can edit" use case is already covered by
   capability-URL `role = "editor"`; mixing role overrides into
   `public_paths` would conflate gates.
7. **Negative interaction with the Phase-0 [[SPEC-041]] gate refactor's
   extractor deferral.** The extractors (`SessionUser`,
   `SessionRole`, `BearerUser`, `AuthUser`) still parse cookies
   internally; they don't currently know about public paths. Routes
   that use these extractors expect the gate to have authenticated the
   request — for public paths the gate let it through unauthenticated,
   so an extractor like `SessionUser` on a public route would 401.
   Strawman position: public-path routes MUST NOT use authentication-
   requiring extractors; a Phase-1 audit lists which routes need to be
   reviewed.
