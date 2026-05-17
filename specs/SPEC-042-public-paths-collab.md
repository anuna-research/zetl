---
id: SPEC-042
title: "Public Paths for `zetl --collab` — mixed unauthenticated + authenticated routing"
version: 0.2.0-strawman
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
revision_notes:
  - v0.1.0 (initial strawman): treated `public_paths` as a gate-level
    TOML matcher parallel to SPL; documented Threat Model E ("two
    policy surfaces") as residual.
  - v0.2.0 (this revision): re-architected `public_paths` as SUGAR
    over SPL — TOML config compiles to `(given (can-read "anonymous"
    PATTERN))` facts; the page-ACL pipeline gains `Option<user_id>`
    mirroring the existing asset-ACL pipeline. One policy surface;
    Threat Model E + Threat Model I resolved by construction. Adds
    REQ-4208 revised, REQ-4214, ADR-4208; supersedes ADR-4205.
    Adds Phase 0.7 (page-ACL refactor) to the implementation plan.
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
| Version        | 0.2.0-strawman (v0.1.0 had TOML+SPL two-surface design; v0.2.0 unifies as TOML-sugar-over-SPL — see `revision_notes` in YAML frontmatter) |
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

**Anonymous-readability is a form of access control, and zetl already
has an access-control system.** The first cut of this spec
(strawman v0.1.0) treated `public_paths` as a *gate-level TOML
allowlist* parallel to [[SPL]] — creating two policy surfaces ([[SPL]]
for authenticated subjects, TOML for anonymous), warning operators
about the confusion via Threat Model E, and shipping the smell. The
revised insight (v0.2.0): the [[SPL]] page-ACL pipeline can be
extended to handle anonymous subjects the same way the *asset*-ACL
pipeline (`check_can_read_assets`, `src/acl.rs:1351–1370`) already
does — by accepting `Option<user_id>`, mapping `None` →
`("anonymous", false)`, and evaluating built-in defaults that
consult an `(authenticated …)` predicate. The asset path has done
this since SPEC-020; we follow its pattern for pages.

So the architecture is:

* `[collab.auth] public_paths = ["/about/**"]` in TOML is **sugar**
  that compiles at startup to [[SPL]] facts of the shape
  `(given (can-read "anonymous" "/about/**"))`.
* `collab_gate` consults [[SPL]] for anonymous requests instead of
  pattern-matching a separate GlobSet. Same engine, same rule
  language, same fact store.
* Operators with `--features reason` *off* get a degenerate
  GlobSet matcher driven by the same TOML config — no SPL evaluation,
  but the operator-facing config shape is identical.
* Operators with `--features reason` *on* can express anything SPL
  can express: "anonymous can read `/about/**` BUT NOT
  `/about/draft/**`", per-time-of-day rules, per-IP via proxy-header
  facts, etc. The TOML knob is the easy path; SPL is the
  expressiveness escape hatch.

[[SPEC-041]] §1.2's invariant — "authorization is the seam that
doesn't move" — still holds. We are not *adding* a new policy
surface; we are *unifying* an apparent new one with the existing one.

### 1.3 Design Principles

1. **One policy surface.** Anonymous-readability is access control;
   access control lives in [[SPL]]. `public_paths` is operator-
   friendly TOML sugar that compiles to [[SPL]] facts. There is one
   evaluator, one fact store, one query language.
2. **Default is today's behaviour.** A vault with no `public_paths`
   in `[collab.auth]` AND no anonymous-allowing SPL rule
   authenticates exactly as the SPEC-041 release does.
3. **Graceful feature-flag degradation.** Under `--features reason`
   *off*, the SPL evaluator isn't compiled in — but operators still
   need anonymous read access. The TOML config drives a GlobSet
   matcher in that case (today's strawman design); the operator-
   facing config is identical, only the *evaluation* differs.
4. **Read-side widening only.** `public_paths` is sugar for
   `(can-read "anonymous" PATH)` facts. It does NOT mint
   `(can-edit "anonymous" PATH)` facts. Anonymous writes require an
   operator to explicitly author the SPL rule — making the
   attribution-story decision explicit, not implicit. The default
   built-in `(forbidden edit (subject anonymous) (any))` rule
   ensures anonymous-edit defaults to deny under all configurations.
5. **`collab_gate` becomes a thin SPL caller.** The gate's job is
   "query SPL for `(can-read PRINCIPAL PATH)`; pass on permit,
   401/403 on deny." Path-matching, role-checking, and capability-
   scope-checking move into SPL facts and built-in rules. The gate
   stays the single decision site (one call, one answer), but it
   doesn't pattern-match itself anymore.
6. **Fail closed on configuration ambiguity.** A glob that matches
   the admin surface, or one too broad to be plausibly intentional
   (`/**`, `/_*`), is a startup error before the TOML→SPL
   compilation runs — the SPL fact store never sees the dangerous
   shape.
7. **All input is recognised before it is acted on.** Per
   [[PROTO-001]] Constitutional Principle 14 ([[LangSec]]), the
   `public_paths` patterns parse against a declared grammar and
   reject anything outside it (REQ-4210) BEFORE compiling to SPL
   facts.
8. **Search and backlinks share the same predicate.** If `(can-read
   "anonymous" "/private/runbook")` returns deny, the search filter
   omits `/private/runbook` from anonymous responses — same query,
   same answer, same code path. This also closes [[#Threat Model I]]
   (per-role search filtering) for free: a Reader's search results
   come from the same `(can-read READER_USER_ID …)` query.

### 1.4 Scope

**In scope:**

- A `[collab.auth] public_paths` glob list (TOML) that **compiles at
  startup to [[SPL]] facts of the shape `(given (can-read "anonymous"
  PATTERN))`** under `--features reason`, OR drives a degenerate
  GlobSet matcher under the default build. The operator-facing
  config shape is identical either way.
- Extension of the page-ACL pipeline (`src/acl.rs::evaluate`) to
  accept `Option<user_id>` and map `None` → `("anonymous", false)`,
  mirroring the existing `check_can_read_assets` anonymous-aware
  pattern (`src/acl.rs:1351–1370`).
- A built-in default SPL rule `(forbidden edit (subject anonymous)
  (any))` to ensure anonymous-edit defaults to deny under all
  configurations (REQ-4203).
- A **`zetl collab public-paths preview` CLI** ([[#REQ-4211]],
  [[#CON-4205]]) that compiles the TOML config, runs the same
  evaluator the live gate uses against the vault scan, and prints
  (a) every page slug an anonymous visitor would be permitted to
  read, (b) the titles a `/search` request would surface for an
  anonymous visitor, (c) any glob that matches zero pages (likely
  typo), (d) any startup warnings (REQ-4206 dangerous shapes,
  REQ-4208 SPL rule conflicts). **Operators MUST be able to preview
  the public surface before bringing a `--collab` server up.**
- Safe-method restriction enforced at the gate via the built-in
  `(forbidden edit (subject anonymous) (any))` rule.
- Startup validation rejecting dangerous globs before TOML→SPL
  compilation runs.
- Interaction with the [[Capability URL]] authenticator: a
  capability principal's scope check is skipped for safe methods on
  paths where SPL says `(can-read "anonymous" PATH)` is permitted
  (the page is public for everyone; the capability adds nothing on
  reads). Writes still enforce capability scope (REQ-4204).
- Interaction with [[#REQ-4112|REQ-4112 CSRF exemption]] from
  [[SPEC-041]]: anonymous requests have no cookie session, so the
  CSRF guard is a no-op for them — anonymous-write defaults to
  forbidden via the built-in SPL rule, so CSRF never gets the chance
  to matter for them.
- Anonymous-aware search-result + backlink filtering (REQ-4207),
  using the same `(can-read PRINCIPAL SLUG)` query as per-page
  authorization — this also closes [[#Threat Model I]] (Reader-role
  per-result filtering) for free.
- Cache-Control / Vary headers on anonymous responses (REQ-4212).
- Audit + operator-log entries naming public-path requests
  (REQ-4209).

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

### 3.4 HP4: Anonymous POST Rejected; Authenticated Editor Passes Through

**Preconditions:** Same config; `/about` is public.

**Steps (anonymous):**

1. Anonymous visitor sends `POST /about/comment`.
2. `collab_gate` classifies the path as public; recognises that the
   request carries no [[Principal]] AND uses a non-safe method.
3. Response: `401 Unauthorized` with `WWW-Authenticate` per the
   configured [[SPEC-041]] chain. Writing requires auth even on a
   page anyone can read.

**Steps (authenticated Editor):**

1. Editor (logged in via passkey) sends `PUT /about` to update the
   public landing.
2. `auth_resolve` runs, attaches `Principal { id: User("alice"),
   method: "passkey", ... }`.
3. `collab_gate` sees public path + authenticated principal +
   non-safe method → passes through (public_paths is read-side
   bypass; it doesn't narrow authority).
4. The PUT handler runs; the role check inside the handler /
   [[SPEC-041]] [[#REQ-4204|capability_gate]] enforces Editor-or-
   higher; git commit attributes the edit to `alice`.
5. Response: `200 OK`. The audit log records `method=passkey
   user=alice path=/about result=ok` (REQ-4209).

**Postconditions:** No anonymous write side-effect; legitimate
operator workflow ("editor logged in, updating the public landing")
is unaffected.

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

### REQ-4203: Anonymous Writes Forbidden; Authenticated Writes Pass Through

`public_paths` is a **read-side** bypass: it widens *anonymous* access
for safe methods (GET / HEAD); it does NOT restrict what authenticated
principals can do.

Specifically:

* **Anonymous + safe method on public path** → pass through, page renders.
* **Anonymous + non-safe method (POST / PUT / DELETE / PATCH) on public
  path** → `401 Unauthorized` (REST-correct; writing requires
  authentication, even on an otherwise-public page).
* **Authenticated + any method on public path** → pass through; the
  downstream handler / SPL / [[#REQ-4204|capability_gate]] applies
  role-based authorization as for any other route. An Editor logged
  into a vault where `/about` is public can still `PUT /about` to
  edit the public landing; a Reader cannot. Attribution flows normally
  (the edit is git-committed as the authenticated user, not as
  anonymous).

This corrects an earlier strawman over-restriction that 405'd writes
even for authenticated editors. The earlier "public-path classification
takes precedence" rule was the wrong intuition: public_paths classifies
the *anonymity* of reads, not the *forbiddenness* of writes.

**Trace:** [[#TEST-4203]], [[#ADR-4202]]; [[#3.4 HP4]]; [[SPEC-020]]
"every edit is attributed" invariant preserved.

### REQ-4204: Capability-URL Principal + Public Path Interaction

WHEN a request whose path matches `public_paths` carries a
[[Capability URL]] principal, the capability-scope check (SPEC-041
[[#REQ-4117|REQ-4117]] / `capability_gate`) SHALL apply asymmetrically:

* **Safe methods (GET / HEAD):** scope check SKIPPED — the page is
  public for everyone, and re-asserting scope would inconsistently 403
  capability holders on pages anonymous visitors can freely read.
* **Non-safe methods (POST / PUT / DELETE / PATCH):** scope check
  ENFORCED exactly as for non-public paths. A capability scoped to
  `/shared/**` CANNOT write to `/about` even if `/about` is public.
  The capability's write authority is bounded by its declared scope.

The principal still flows to the handler in both cases; `admin_gate`
is unchanged (capability principals never satisfy it).

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

### REQ-4208: SPL Invariant — Anonymous DOES Reach SPL (revised)

[[SPL]] policy SHALL be evaluated for anonymous requests when
`--features reason` is compiled in (the standard zetl build that
includes the SPL engine — `cargo build --features collab,reason`).
The page-ACL pipeline accepts `Option<user_id>`; anonymous maps to
`("anonymous", false)` in the same shape as the existing
`check_can_read_assets` pipeline (`src/acl.rs:1351–1370`).

Built-in defaults the system ships AS SPL FACTS at startup:

```spl
;; anonymous-edit defaults to deny (REQ-4203, ADR-4202).
(forbidden edit (subject anonymous) (any))

;; compiled-from-TOML for each `[collab.auth] public_paths` entry.
(given (can-read "anonymous" "/about/**"))
(given (can-read "anonymous" "/blog/*"))
;; … one fact per public_paths entry.
```

Operator-authored SPL rules that reference an `"anonymous"` subject
or `(not (authenticated …))` predicate are now FIRST-CLASS — they
fire against anonymous requests and compose with the compiled-from-
TOML facts. The startup warning the v0.1.0 draft of this REQ
mandated ("SPL rule referencing anonymous is unreachable") is
INVERTED: such rules are now reachable. The startup warning that
DOES still fire is "an operator-authored SPL rule about anonymous
*directly conflicts* with a compiled-from-TOML fact" — e.g.,
operator writes `(forbidden read (subject anonymous) (page-glob
"/about/**"))` AND has `[collab.auth] public_paths = ["/about/**"]`.
The conflict is resolved by SPL's normal defeasibility (operator-
authored `(forbidden …)` outranks system-generated `(given (can-read
…))` because explicit forbidden defeats default permit); the warning
surfaces so the operator notices the implicit override.

Under `--features reason` *off*, [[#REQ-4214]] specifies the fast-
path matcher equivalent.

**Trace:** [[#TEST-4208]], [[#ADR-4205]] (revised), [[#ADR-4208]].
This REQ supersedes the v0.1.0 draft which said "SPL doesn't see
anonymous"; see [[#ADR-4205]] for the architectural reasoning.

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

### REQ-4212: Anonymous-Aware Cache-Control Headers

WHEN `collab_gate` admits an anonymous request to a public path,
the response SHALL carry, at minimum:

* `Cache-Control: public, max-age=0, must-revalidate` (default;
  operator may override at a reverse proxy if they understand the
  cache-poisoning implications);
* `Vary: Cookie, Authorization` (so any intermediary that DOES cache
  cannot conflate an anonymous cached entry with the response served
  to an authenticated principal carrying a cookie or bearer token).

WHEN `collab_gate` serves an *authenticated* response on a public
path (the editor-PUT case per [[#REQ-4203]]), the existing per-route
`Cache-Control` headers apply unchanged — most authenticated content
routes today set `Cache-Control: private, no-store` and that path is
preserved.

Operators who terminate TLS at a CDN / reverse proxy that adds its
own caching MUST be warned in [[docs/collab-public-paths.md]] about
the standard mixed-auth caching footgun: an intermediary that ignores
`Vary` can serve an anonymous cached entry to an authenticated user.
The recommendation is "don't enable shared-cache CDN caching of
zetl-served content unless you've audited the `Vary` story"; the
spec doesn't try to enforce CDN behaviour.

**Trace:** [[#TEST-4212]], [[#Threat Model G]]; [[SPEC-041]]
§Threat Model F — Cache Poisoning, which this REQ closes.

### REQ-4213: Wikilink Rendering on Public Pages Referencing Private Pages

WHEN a public page renders a `[[wikilink]]` that points to a private
(non-public-path) page, the rendered HTML SHALL NOT expose the
target page's title text to an anonymous viewer.

The implementation choice (strike-through, render as plain text with
no link, omit the link entirely, render as `[[unknown]]`) is left to
[[#ADR-4207]] (to be filed); the invariant is "no private page title
appears in HTML served anonymously, regardless of where the rendering
decision lives." This includes:

* inline wikilinks in the page body;
* the backlink panel rendered inline on the page;
* OpenGraph / Twitter Card metadata in `<head>` for links to private
  pages;
* recent-changes / sidebar widgets if rendered into a public page;
* the search result snippet view if it embeds linked-page titles.

Authenticated views of the same public page render the wikilinks
normally (so an editor previewing their public-landing draft sees
the real titles).

**Trace:** [[#TEST-4213]], [[#Threat Model B]], [[#ADR-4207]];
[[#15. Open Questions Surfaced by This Strawman]] Q8.

### REQ-4214: Feature-Flag Behaviour — `--features reason` On vs Off

The system SHALL produce equivalent operator-observable behaviour
under both feature configurations, differing only in *evaluation
shape*, not in *what is permitted*:

**Build A: `cargo build --features collab,reason` (SPL engine
present).**

- TOML `[collab.auth] public_paths` is parsed + validated, then
  compiled to [[SPL]] facts per [[#ADR-4208]] and inserted into the
  fact store alongside the built-in `(forbidden edit (subject
  anonymous) (any))`.
- `collab_gate` calls `evaluate(AclQuery{ user: Anonymous,
  action: Read, page: <slug> })`; permit → next, deny → 401.
- Anonymous-edit attempts trigger the built-in `(forbidden edit
  …)` → 401.
- Operator-authored SPL rules that reference anonymous subjects
  FIRE and compose with the compiled facts via SPL defeasibility.
- Search + backlinks share `(can-read PRINCIPAL SLUG)` query
  (closes [[#Threat Model I]] for free).
- Preview CLI runs SPL against the vault scan; output reflects
  EXACTLY what the live gate would permit.

**Build B: `cargo build --features collab` (default; SPL engine
absent).**

- TOML `[collab.auth] public_paths` is parsed + validated, then
  compiled to a `globset::GlobSet` (today's strawman-v0.1.0 design).
- `collab_gate` does:
  ```
  if anonymous && path matches GlobSet:
      if safe method → next, with REQ-4212 cache headers
      else            → 401 (hardcoded equivalent of the built-in
                             `(forbidden edit (subject anonymous) (any))`)
  if anonymous && path does NOT match GlobSet:
      → 401 / login redirect
  ```
- Operator-authored SPL rules don't exist (the engine isn't
  compiled in); the TOML config IS the policy.
- Search + backlinks consult the GlobSet directly (the
  [[#REQ-4207]] response-boundary filter degrades to GlobSet
  match instead of SPL query).
- Preview CLI runs the GlobSet matcher; output equivalent to
  Build A for any vault whose operator hasn't authored anonymous-
  referencing SPL rules.

**Equivalence invariant:** For any vault that uses *only* the TOML
`[collab.auth] public_paths` knob (no operator-authored SPL rules
about anonymous), Build A and Build B SHALL admit and deny the
same requests. A vault that adds operator-authored anonymous SPL
rules will diverge — Build A applies them; Build B silently
ignores them. The startup banner under Build B MUST warn
"`.zetl/collab/access.spl` exists but `--features reason` is off;
operator-authored SPL rules will not fire" so operators don't
mistake silent-ignore for evaluated-and-permitted.

**Trace:** [[#TEST-4214]], [[#ADR-4208]]; closes the build-
configuration-equivalence concern raised by the path-C
re-architecture.

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

### ADR-4202: `public_paths` Widens Anonymous *Reads* Only; Writes Unchanged

**Status:** Proposed (strawman default — revised from "forbid all
non-safe methods" after the read/edit-distinction review)

**Context:** Could anonymous visitors POST to a public page (comments,
sign a guestbook)? Could an *authenticated* Editor PUT to a public
page they need to maintain (e.g., update the public landing)? An
earlier draft of REQ-4203 said "no" to both — 405 across the board on
non-safe methods, regardless of principal. The reasoning was crispness.
The cost was breaking the legitimate "editor maintains the public
page" workflow.

**Decision:** Reframe `public_paths` as a *read-side widening only*:

* Anonymous + GET/HEAD on public path → pass through (the feature).
* Anonymous + non-safe method on public path → `401 Unauthorized`
  (REST-correct; you still need to authenticate to write).
* Authenticated principal + any method on public path → pass through;
  the downstream handler / SPL / [[#REQ-4204|capability_gate]]
  applies role-based authorization exactly as for any other route.
  Public-paths classification does NOT narrow authority.

Comment / form / submission flows that need *anonymous* writes are
still a clean follow-up spec — they require an attribution story
(the [[SPEC-020]] "every edit is attributed" invariant) that doesn't
exist today.

**Consequences:** (+) Legitimate workflow preserved — the operator
who set the public path can still edit it. (+) Attack surface stays
narrow (anonymous POST = 401, not 405; no anonymous CSRF concerns).
(+) Audit trail attribution unchanged — authenticated edits on public
paths log the user, exactly as today. (−) The gate has to inspect
both path AND principal before deciding the response code, where the
earlier "405 across the board" rule looked at path only. Trade-off
worth it for not breaking the obvious workflow.

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

### ADR-4204: Capability Scope Skipped for *Reads* on Public Paths; Enforced for Writes

**Status:** Proposed (strawman default — revised alongside
[[#ADR-4202]] to apply the same read/write distinction)

**Context:** If `/` is public AND a capability-URL holder scoped to
`/shared/**` requests it, should `capability_gate` evaluate the scope?
Per [[SPEC-041]]'s strict scope check, the request would 403 because
`/` isn't in scope — even though every anonymous visitor can read it.
But what if the capability holder tries to `PUT /about` (a public
page outside their scope)? Skipping scope outright would let a cap
URL with role=editor write to any public page anywhere.

**Decision:** Skip capability-scope check for **safe methods only**:

* Capability principal + GET/HEAD on public path → scope skipped;
  pass through (the page is public to everyone — the capability adds
  nothing on the read side, failing closed is operationally confusing).
* Capability principal + non-safe method on public path → scope check
  applies exactly as for non-public paths. A capability scoped to
  `/shared/**` CANNOT write to `/about` even if `/about` is public.
  The capability's write authority is bounded by its declared scope,
  full stop.

`admin_gate` is unchanged — capability principals still can't reach
`/_admin/*`, public or otherwise.

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

### ADR-4205: SPL *Does* See Anonymous (Unified Policy Surface)

**Status:** Revised — withdraws the strawman-v0.1.0 position ("SPL
doesn't see anonymous"). The earlier position created two policy
surfaces (TOML `public_paths` and [[SPL]]) and admitted the
resulting confusion as [[#Threat Model E]]. The revised position
unifies them — see [[#ADR-4208]] for the architectural shift.

**Context:** The strawman-v0.1.0 reasoning was: "extending SPL to
handle anonymous would be a big refactor; gate-level TOML config is
smaller; we can warn operators about the two-surface confusion."

The flaw: the *asset*-ACL pipeline (`check_can_read_assets`,
`src/acl.rs:1351–1370`) already extends SPL to handle anonymous —
it accepts `Option<&str>`, maps `None` → `("anonymous", false)`,
passes `is_authenticated: bool`, and ships built-in defaults like
`(normally r-public-read-assets (and (visibility-mode transparent)
(not (authenticated "user_id"))) (can-read-assets "anonymous" "*"))`.
The pattern is established and shipped; we're not inventing it.

**Decision:** The page-ACL pipeline gains the same `Option<user_id>`
treatment. SPL rules CAN reference anonymous subjects via the same
`(not (authenticated …))` predicate the asset rules already use.
TOML `public_paths` becomes sugar — it compiles to SPL facts
([[#ADR-4208]]); the SPL engine is the single evaluator.

**Consequences:** (+) One policy surface. (+) [[#Threat Model E]]
becomes "no longer applicable" — there is no second surface to
conflict with. (+) Layered policy ("public except this sub-folder")
becomes natural SPL ("`(can-read "anonymous" "/about/**")` AND
`(forbidden read (subject anonymous) (page-glob "/about/draft/**"))`").
(+) Per-role search filtering ([[#Threat Model I]]) becomes the same
machinery — the predicate widens from "is the request anonymous?" to
"what can this principal read?", same query. (−) The page-ACL
pipeline gains an `Option<user_id>` parameter, threading through
every call site — a medium-sized refactor. The asset path proves
it's feasible; the diff shape is established.

### ADR-4207: Wikilink-on-Public-Page Rendering Policy

**Status:** **Open** — decision deferred to a follow-up task before
Phase 2 implementation. [[#REQ-4213]] captures the invariant ("no
private title in HTML served anonymously"); this ADR captures the
choice of rendering shape.

**Context:** When `[[Internal Memo]]` appears on a public page and
the visitor is anonymous, the rendered HTML must not contain the
text "Internal Memo" (REQ-4213). Several shapes satisfy that:

| Shape | Rendered HTML (anonymous) | Pros | Cons |
|---|---|---|---|
| (a) Strike-through | `<s class="zetl-redacted">[[unknown]]</s>` | Explicit; visitor sees a gap; layout preserved | Visual noise; signals "there's something here" |
| (b) Plain text literal | `[[Internal Memo]]` rendered as plain text | Faithful to source; no extra UI | Title leaks via the literal — defeats REQ-4213 |
| (c) Omit entirely | Wikilink + surrounding whitespace removed | No leak; invisible | Layout shifts; reader doesn't know content was removed |
| (d) 404-link | `<a href="/internal-memo">[unknown]</a>` | Linked but title-less | Slug leaks via href |
| (e) Generic placeholder | `<span class="zetl-redacted">[redacted]</span>` | No leak; explicit | Same noise as (a) |

**Strawman lean:** (a) or (e). Both preserve layout, both signal
the redaction explicitly so the visitor doesn't think they're
seeing the whole picture, neither leaks title or slug. (b) is the
rejected default — it defeats the requirement. (c) is too quiet —
operators preview-rendering their public landing won't notice they've
silently removed paragraphs. (d) leaks the slug, which is often a
human-readable variant of the title.

**Decision:** Deferred. The right way to pick is to render the same
public page under each shape against a real vault and ask the
operator. The follow-up task ([[DESIGN-042-public-paths-collab]]
sub-task `wikilink-redaction-shape`) decides before Phase 2 wires
[[#REQ-4213]] into the renderer.

**Consequences (any choice):** Authenticated views of the same
public page render wikilinks normally — the redaction is anonymous-
view-only. This means an editor previewing their own draft sees the
real titles; an anonymous visitor in a private window sees the
redacted view. Operators verify-before-deploy by viewing the public
page in an incognito tab.

### ADR-4208: `public_paths` Is Sugar — Compiles to [[SPL]] Facts

**Status:** Proposed (strawman v0.2.0 — supersedes the v0.1.0
ADR-4205 "SPL doesn't see anonymous" position)

**Context:** The strawman-v0.1.0 design treated `public_paths` as a
**gate-level GlobSet** that pattern-matched the request path before
[[SPL]] was consulted. The result was two policy surfaces — TOML
`public_paths` AND [[SPL]] — with overlapping concerns and explicit
operator confusion documented as [[#Threat Model E]]. The
*asset*-ACL pipeline (`src/acl.rs::check_can_read_assets`,
`src/acl.rs:1351–1370`) already proves zetl's [[SPL]] engine can
handle anonymous subjects via `Option<&str>` + `is_authenticated:
bool` + built-in defaults referencing `(not (authenticated …))`. The
page-ACL pipeline can do the same.

**Decision:** `[collab.auth] public_paths` is a **sugar layer**, not
a separate policy. At startup:

1. Parse + validate the TOML `public_paths` list (REQ-4206 dangerous-
   glob rejection runs here, BEFORE any compilation).
2. Compile each entry to an [[SPL]] fact:
   ```spl
   (given (can-read "anonymous" "/about/**"))
   (given (can-read "anonymous" "/blog/*"))
   ```
   plus a single built-in default the system always ships:
   ```spl
   (forbidden edit (subject anonymous) (any))
   ```
   so anonymous-edit defaults to deny regardless of operator config
   (preserves REQ-4203 by construction, makes the anonymous-write
   decision an explicit operator opt-in via SPL rather than an
   implicit gate-side restriction).
3. Insert the compiled facts into the same fact store the existing
   page-ACL evaluator already consults. The evaluator's query
   `(can-read PRINCIPAL PATH)` works unchanged — `PRINCIPAL` is just
   `"anonymous"` instead of a user ID.

`collab_gate` rewrites to: "resolve principal (possibly None);
query SPL `(can-read PRINCIPAL_OR_ANONYMOUS PATH)`; permit → next;
deny → 401 if anonymous, 403 if authenticated-but-unauthorized."

Under `--features reason` **off** (default zetl build that doesn't
compile the SPL engine), the same TOML config drives a `globset`-
based fast path matcher — the operator-facing config is identical,
the in-process evaluation degrades to "match path against
GlobSet → permit; default deny." Built-in `(forbidden edit
(subject anonymous) (any))` becomes a hardcoded "anonymous + non-
safe method → 401" check in the fast path. See [[#REQ-4214]] for
the precise behavioural contract under each feature configuration.

**Consequences:** (+) One policy surface — TOML is just sugar.
[[#Threat Model E]] is no longer applicable. (+) Search +
backlinks share the same `(can-read PRINCIPAL SLUG)` query —
[[#Threat Model I]] (per-role result filtering) is solved by the
same machinery for free. (+) Operators who outgrow the TOML knob
can drop into raw SPL ("`(can-read "anonymous" "/about/**")` AND
`(forbidden read (subject anonymous) (page-glob "/about/draft/**"))`")
without needing a new config surface. (+) The asset-path pattern is
established; the diff shape for the page-ACL refactor is known.
(−) The page-ACL pipeline's `evaluate()` function signature gains
`Option<&str>` for `user_id` — every call site needs touching.
Mitigated by the asset-path precedent. (−) Anyone reading the SPL
fact store sees `(given (can-read "anonymous" …))` entries they
didn't author; the preview CLI ([[#REQ-4211]]) and SPL trace
viewer ([[hence query explain]]) both list provenance
(`from: public_paths`) so the operator can trace the fact back to
its TOML source.

**Trace:** [[#REQ-4214]], [[#REQ-4201]], [[#REQ-4202]]; supersedes
[[#ADR-4205]] v0.1.0; closes [[#Threat Model E]]; resolves
[[#Threat Model I]].

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

### CON-4202: `collab_gate` Becomes a Thin SPL Caller (with Fast-Path Fallback)

The gate becomes a thin caller into the shared ACL evaluator. The
*decision* (permit/deny) is owned by SPL (Build A) or a degenerate
GlobSet matcher (Build B); the gate's job is to translate that
decision into an HTTP response with the right headers.

```rust
pub async fn collab_gate(
    State(state): State<WebState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if !state.collab { return next.run(request).await; }

    let principal = request.extensions().get::<Principal>().cloned();
    let path = request.uri().path().to_string();
    let action = action_from_method(request.method()); // Read | Edit

    // `decide` is the unified entry point — Build A delegates to the
    // SPL evaluator with Option<user_id>; Build B uses the compiled
    // GlobSet + hardcoded anonymous-edit deny.
    let outcome = state.acl.decide(principal.as_ref(), action, &path);

    match (outcome, principal.is_some(), request.method()) {
        (Decision::Permit, false, m) if is_safe(m) => {
            // anonymous safe-method read on a permitted path.
            apply_anonymous_cache_headers(next.run(request).await)
        }
        (Decision::Permit, true, _) => {
            // authenticated principal — handler / capability_gate /
            // any further per-page SPL runs as today.
            next.run(request).await
        }
        (Decision::Deny, false, _) => unauthorized_response(&state),
        (Decision::Deny, true, _)  => forbidden_response(&state),
        // Permit + anonymous + non-safe is impossible — the built-in
        // `(forbidden edit (subject anonymous) (any))` rule (Build A)
        // or the hardcoded equivalent (Build B) returns Deny first.
        _ => unauthorized_response(&state),
    }
}
```

`AclEvaluator::decide`:

* **Build A (`--features reason`)**: maps `principal: None` →
  `("anonymous", false)`, constructs an `AclQuery` (the same
  shape `evaluate()` already uses for authenticated callers),
  and consults the SPL fact store (which contains the compiled-
  from-TOML `(given (can-read "anonymous" …))` facts +
  operator-authored `.zetl/collab/access.spl` + built-in
  defaults).
* **Build B (default)**: a degenerate matcher with two branches —
  `Action::Read` checks the compiled `globset::GlobSet`,
  `Action::Edit` always returns `Deny` for anonymous (the
  hardcoded equivalent of `(forbidden edit (subject anonymous)
  (any))`); authenticated principals always permit (the gate
  has no further role machinery without SPL — per-page handlers
  do their own role checks today).

This is the architectural shift from the strawman-v0.1.0 design,
where the gate pattern-matched the request path against a local
`GlobSet` and made its own decision. The v0.2.0 design pushes the
decision into a single evaluator, accessed via a common
`AclEvaluator::decide` interface that the page-handler ACL calls
also use.

**Pre-conditions:** `auth_resolve` has already run (the Principal
extension may be `Some` or `None`). `state.acl` is the shared
ACL evaluator (Build A → SPL; Build B → GlobSet matcher).

**Post-conditions:**
- (REQ-4202) anonymous + safe + decision Permit → pass through.
- (REQ-4203) anonymous + non-safe → Deny → 401 (via built-in
  `(forbidden edit (subject anonymous) (any))` Build A, or
  hardcoded Build B).
- (REQ-4203) authenticated + any method → pass through; downstream
  handlers / per-page SPL / capability_gate enforce role-based
  authorization unchanged.
- (REQ-4205) absent or empty `public_paths` AND no anonymous-
  permitting SPL rule → anonymous Permit returns false everywhere
  → pre-SPEC-042 behaviour.
- (REQ-4208) operator-authored `.zetl/collab/access.spl` rules
  about anonymous fire (Build A) and compose with compiled facts
  via SPL defeasibility.
- (REQ-4212) anonymous-permit responses carry cache headers;
  authenticated responses do not.
- (REQ-4214) Build A and Build B produce equivalent outcomes for
  any vault using only TOML config.

**Implements:** [[#REQ-4202]], [[#REQ-4203]], [[#REQ-4205]],
[[#REQ-4208]], [[#REQ-4212]], [[#REQ-4214]]. **Verified by:**
[[#TEST-4202]], [[#TEST-4203]], [[#TEST-4205]], [[#TEST-4208]],
[[#TEST-4212]], [[#TEST-4214]].

### CON-4203: Capability-Gate Asymmetric Public-Path Bypass

`capability_gate` gains a method-aware early-return (post-revision
of REQ-4204 / ADR-4204):

```rust
pub async fn capability_gate(req, next) -> Response {
    let Some(grant) = principal.capability else { return next.run(req).await; };

    // SPEC-042 — skip scope/role check ONLY for safe methods on
    // public paths. Writes still respect cap scope even when the
    // path is public.
    if PUBLIC_PATHS.is_match(req.uri().path())
        && matches!(req.method(), &Method::GET | &Method::HEAD)
    {
        return next.run(req).await;
    }

    // …existing scope + role checks apply unchanged for:
    //   - non-public paths (any method)
    //   - public paths + non-safe methods
}
```

**Pre-conditions:** Principal carries a [[CapabilityGrant]]; gate is
not a no-op.

**Post-conditions:** (REQ-4204) on public-path match + safe method,
scope + role checks are skipped — the request proceeds. On public-
path match + non-safe method, the SPEC-041 scope+role check applies
exactly as for non-public paths. Off public paths, SPEC-041
[[#REQ-4117|REQ-4117]] enforcement is unchanged.

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

### CON-4204: Principal-Aware Search / Backlinks Filtering via Shared ACL Query

The search and backlinks query paths gain a per-result filter that
consults the same `AclEvaluator::decide(principal, Action::Read,
slug)` interface `collab_gate` uses ([[#CON-4202]]). Pages for
which the evaluator returns `Decision::Deny` are omitted from the
response body BEFORE serialisation, so titles, slugs, and excerpts
never leave the process.

* **Anonymous request:** filter via `decide(None, Read, slug)`.
  Build A → SPL query `(can-read "anonymous" slug)`; Build B →
  compiled GlobSet membership.
* **Authenticated request (User principal):** filter via
  `decide(Some(user), Read, slug)`. Build A → SPL query
  `(can-read USER_ID slug)`; this CLOSES [[#Threat Model I]] for
  free — a Reader's results omit Editor-only pages exactly as
  per-page authorization would. Build B → no per-role filtering
  (the SPL engine is absent); current SPEC-020 behaviour preserved.
* **Authenticated request (Capability principal):** filter via
  `decide(Some(cap_principal), Read, slug)` — Build A SPL rules
  for capability principals decide; Build B uses the existing
  capability_gate scope check applied per-result.

The filter point is the response boundary. The search engine still
indexes the full corpus; only the SERIALISED set differs by
principal.

**Pre-conditions:** A search / backlinks request reached the
handler. The handler holds the request's Principal extension AND
the shared `AclEvaluator` from `WebState`.

**Post-conditions:** (REQ-4207) responses contain only slugs the
principal is permitted to read. The same query that decides
in-page authorization decides search-result emission — there is no
risk of the two views disagreeing.

**Implements:** [[#REQ-4207]]. **Verified by:** [[#TEST-4207]].
**Closes:** [[#Threat Model I]] under Build A.

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
| [[#TEST-4203]]        | example + neg-output      | Anonymous POST/PUT/DELETE on public path → 401; authenticated Editor PUT on public path → 200 + git-attributed; authenticated Reader PUT → 403 from handler | [[#REQ-4203]]                      |
| [[#TEST-4204]]        | example                   | Cap principal scoped `/shared/**` GET `/about` (public) → 200 (scope skipped); same cap PUT `/about` → 403 (scope enforced on writes); off-public-path behaviour → REQ-4117 unchanged | [[#REQ-4204]]   |
| [[#TEST-4205]]        | snapshot                  | No `public_paths` ⇒ SPEC-041 / SPEC-020 collab suites pass unchanged                  | [[#REQ-4205]]                      |
| [[#TEST-4206]]        | example + neg-input       | Each dangerous-glob shape (`/**`, `/_admin/**`, `/auth/**`, bad syntax) → startup error naming pattern | [[#REQ-4206]]    |
| [[#TEST-4207]]        | example                   | Anonymous search omits private slugs; authenticated search returns full set            | [[#REQ-4207]]                      |
| [[#TEST-4208]]        | example                   | Build A: operator-authored `.zetl/collab/access.spl` rule about anonymous fires + composes with compiled-from-TOML facts via SPL defeasibility; conflicting rules surface a startup warning. Build B: `.zetl/collab/access.spl` ignored, startup banner warns | [[#REQ-4208]]                      |
| [[#TEST-4209]]        | example                   | Anonymous request → operator log + audit line with `method=anonymous identity=-`     | [[#REQ-4209]]                      |
| [[#TEST-4210]]        | fuzz + property           | Random byte sequences against the public-path-glob recogniser: no panics, no acceptance of out-of-grammar input | [[#REQ-4210]] |
| [[#TEST-4211]]        | example + snapshot        | Preview CLI emits expected slug + title + summary + validation sections against a fixture vault; `--json` is structurally stable; `--strict` exits non-zero on WARN; no file writes; output agrees with the actual gate's runtime behaviour | [[#REQ-4211]]                      |
| [[#TEST-4212]]        | example                   | Anonymous GET on public path → response carries `Cache-Control: public, max-age=0, must-revalidate` AND `Vary: Cookie, Authorization`; authenticated GET on same path → existing per-route headers, NO anonymous-cache headers | [[#REQ-4212]]                      |
| [[#TEST-4213]]        | example + neg-output      | Public page containing `[[Private Page]]` rendered to anonymous viewer → HTML contains NEITHER "Private Page" NOR the private page's slug; rendered to authenticated Reader → contains the title and link | [[#REQ-4213]]                      |
| [[#TEST-4214]]        | parity                    | Same TOML config (no operator SPL rules) under Build A and Build B admits/denies the same anonymous requests; presence of operator SPL rules with `--features reason` off triggers startup banner | [[#REQ-4214]], [[#ADR-4208]]       |
| TEST-4214-spl-sugar   | example                   | TOML `public_paths = ["/about/**"]` compiles to `(given (can-read "anonymous" "/about/**"))` in the fact store under Build A; `hence query explain` traces back to `from: public_paths` provenance | [[#REQ-4214]], [[#ADR-4208]]       |
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

### Threat Model B — Title / Slug / Excerpt Leakage via Adjacent Endpoints

> Anonymous visitor hits `/search?q=*`, `/api/graph`, `/feed.xml`,
> `/sitemap.xml`, `/llms.txt`, or any other endpoint that emits a
> page-name list; if those endpoints aren't scoped to `public_paths`,
> private titles, slugs, excerpts, and graph-edge endpoints leak.

This is **the load-bearing risk class for SPEC-042** — broader than
"just /search". The full enumeration of leak surfaces a Phase-2 audit
MUST close before the spec status flips to `implemented`:

| Surface | Leak content | Filter point |
|---|---|---|
| `/search`, `/api/search` | titles, slugs, excerpts | response boundary, REQ-4207 |
| `/api/backlinks/<slug>` | titles of pages linking to slug | response boundary, REQ-4207 |
| `/api/graph` | every page slug + every edge | response boundary, needs Phase-2 wiring |
| `/llms.txt` | typically full vault index | response boundary, needs Phase-2 wiring |
| `/feed.xml`, `/feed.atom` (SPEC-038) | titles + excerpts of recent edits | response boundary |
| `/sitemap.xml` | every URL | response boundary |
| `robots.txt` | could leak via negative-space `Disallow:` | operator-authored, document the trap |
| Wikilink rendering on a public page | linked private page's title text | render-time, REQ-4213 |
| Backlink panel rendered inline on public page | linker titles | render-time, REQ-4213 |
| Recent-changes sidebar | titles of recently-edited private pages | render-time, REQ-4213 |
| `<title>` / OpenGraph / Twitter Card metadata | could embed linked-page titles | render-time, REQ-4213 |
| HTML error pages (`<title>404 — MyPrivateWiki</title>`) | vault name | minor; document |

**Primary mitigation:** [[#REQ-4207]] / [[#CON-4204]] for the
response-boundary endpoints + [[#REQ-4213]] for the render-time
cases. The filter applies at the response boundary, not at the index,
so the search engine still uses the full corpus; only the OUTPUT
changes.

**Defence-in-depth:** a single `is_public(slug)` predicate that every
list-emitting endpoint MUST thread through, plus a CI lint that
flags new endpoints which produce `Vec<PageName>`-shaped responses
without consulting it. The Phase-2 endpoint audit is the gating
deliverable — without a complete inventory we can't know we've
closed every leak.

**Residual risk:** new endpoints added in future SPECs forget the
filter. Mitigation is operational discipline (the CI lint) + the
endpoint inventory living next to the GlobSet so reviewers see the
list whenever they add a list-emitting handler.

### Threat Model C — Anonymous State Mutation

> Anonymous visitor crafts a POST to a public path expecting to mutate
> server state.

**Mitigation:** [[#REQ-4203]] (post-revision) — POST / PUT / DELETE
on public paths from an anonymous principal return `401 Unauthorized`.
The gate enforces this BEFORE any handler runs, so even a handler
that legitimately accepts unauthenticated writes (none exist today)
couldn't be exploited via this mechanism. Authenticated writes pass
through and are subject to normal role-based authorization — an
authenticated Editor can still PUT a public page they maintain,
which is the legitimate workflow [[#ADR-4202]] preserves.

**Residual risk:** none beyond the deferred "anonymous comment
forms" use case.

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

### Threat Model E — SPL / `public_paths` Policy Confusion *(resolved)*

> An operator writes both a `public_paths` glob and an SPL rule
> referencing "anonymous", expecting layered policy. SPL doesn't fire
> for public requests; the operator's expectation isn't met.

**Status: NO LONGER APPLICABLE under the v0.2.0 architecture.**

The v0.1.0 strawman shipped this confusion because it treated
`public_paths` as a gate-level TOML matcher parallel to (and
ignoring) [[SPL]]. The v0.2.0 architecture ([[#ADR-4208]],
[[#REQ-4208]] revised) compiles `public_paths` into [[SPL]] facts
that flow through the existing evaluator alongside operator-
authored rules. There is now ONE policy surface; operator
expectations of layered policy are satisfied by SPL's normal
defeasibility (operator's explicit `(forbidden …)` defeats the
compiled-from-TOML `(given (can-read …))`). A startup warning
still fires when an operator-authored rule *directly* conflicts
with a compiled-from-TOML fact — but to surface the implicit
override, not because the rule is unreachable.

**Historical note:** kept in the threat list (rather than deleted)
to preserve the trace from v0.1.0 reviews; the resolution itself
is what's load-bearing.

### Threat Model F — Capability-URL Operator Confusion (skipped scope)

> An operator mints a capability for `scope = "shared/**"` and shares
> the URL. The recipient visits `/` (which the operator listed as
> public). Per [[#ADR-4204]], the scope check is skipped and the page
> renders. The operator may be surprised that the cap URL "works on the
> home page."

**Mitigation:** The home page was already public for everyone; the
capability adds nothing on the read side. The revised [[#ADR-4204]]
preserves scope enforcement on the WRITE side, so a cap URL scoped
to `/shared/**` cannot write to `/about` even if `/about` is public.
Documentation explains the principle ("public is public on reads;
writes still respect cap scope").

**Residual risk:** operator UX confusion only on the read side; no
security weakening.

### Threat Model G — Cache Poisoning via Intermediary CDN

> A CDN or reverse proxy sits between visitors and zetl. The
> anonymous response to a public-path GET is cached. A logged-in user
> requests the same path; the CDN serves the cached anonymous response
> instead of the per-principal one. Worse: a config error briefly
> exposes `/private/secret`; the CDN caches it; even after the operator
> narrows the glob, the cached version stays accessible until TTL.

This is the classic mixed-auth caching footgun. zetl's current
collab-mode posture is "every response is authenticated, so caching
is bounded per-session" — public_paths breaks that posture by
introducing genuinely-cacheable responses on shared URLs.

**Mitigation:** [[#REQ-4212]] — every anonymous public-path response
carries `Cache-Control: public, max-age=0, must-revalidate` and
`Vary: Cookie, Authorization` by default. Authenticated responses on
public paths inherit the existing per-route `private, no-store`
headers. Operators are warned in [[docs/collab-public-paths.md]]
about the standard CDN footgun.

**Residual risk:** an operator overrides the cache headers at their
reverse proxy without understanding the implications. The spec
cannot enforce CDN behaviour; the mitigation is operator-facing
warnings + a worked example in the operator guide. Operators on
shared CDNs (Cloudflare, Fastly, CloudFront) MUST be told to either
disable shared caching or audit their `Vary` story.

### Threat Model H — Persistent Exposure via Web Crawlers / Archive.org

> An operator marks `/blog/**` public, runs the server for a week,
> then narrows the glob to remove a specific page. Search engines
> (Google, Bing, Kagi) and archive.org have already indexed and
> cached the page. Narrowing the glob doesn't reach back through
> those caches.

This is operational, not technical — the feature does what it says
on the tin (make these pages public). The risk is operator-expectation
mismatch: operators who treat `public_paths` as a soft "I'll
un-public this later" knob will be surprised that retraction is
hard.

**Mitigation:** Operator-doc warning in [[docs/collab-public-paths.md]]:
"Anything reachable via `public_paths` for any length of time should
be assumed cached externally. Retraction means narrowing the glob
AND requesting removal from major search-engine caches AND noting
that archive.org has its own takedown process. Don't use this
feature for content you might want to retract."

**Residual risk:** operator misunderstanding remains the dominant
risk. No technical mitigation possible.

### Threat Model I — Authenticated-Search Per-Result Role Gating Gap *(resolved by unification)*

> A vault uses SPL to restrict `/internal/runbook` to Editor-or-
> higher. A Reader-role authenticated user searches for "runbook" —
> they get a search hit on `/internal/runbook` even though clicking
> the result 403s. The hit reveals the page's title and excerpt.

This was **pre-existing in SPEC-020 / SPEC-041** under the
v0.1.0 strawman framing. It is **resolved by the v0.2.0
re-architecture** ([[#ADR-4208]]) as a side-effect of unifying
the policy surface.

**Mitigation:** The [[#REQ-4207]] response-boundary filter now
uses the same `(can-read PRINCIPAL SLUG)` SPL query as per-page
authorization — the predicate is the same, the answer is the
same. Under Build A (`--features reason` on), a Reader's search
results are filtered through `(can-read READER_USER_ID SLUG)`;
restricted pages are omitted exactly as they are at click-time.
Anonymous visitors' results are filtered through `(can-read
"anonymous" SLUG)`. Same machinery; the principal substitution
is all that varies. Under Build B, search consults the GlobSet
for anonymous filtering; per-role search filtering is N/A
because role-based SPL rules don't exist without the engine.

**Residual risk:** zero under Build A. Under Build B, the
pre-existing gap remains for role-restricted pages — but Build B
is the *no-SPL* configuration, so there are no per-role rules to
enforce; the residual is a non-issue.

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
  dangerous-shape rules), `compile` (→ `globset::GlobSet`).
- New `compile_to_spl_facts(&PublicPathsConfig) -> Vec<SplFact>`
  (Build A) — converts each entry to `(given (can-read "anonymous"
  PATTERN))` with `from: public_paths` provenance metadata.
- New `resolve(evaluator: &AclEvaluator, vault: &VaultData) ->
  PreviewReport` (pure): walks the vault page index, calls
  `decide(None, Read, slug)` per page, returns per-glob matches +
  zero-match-globs + the anonymous-search title list.
- Extend [[SPEC-041]] `CollabAuthConfig` with
  `public_paths: Option<Vec<String>>`.
- Unit tests: grammar accept/reject matrix, dangerous-shape rejection
  matrix, compile-to-SPL-facts round-trip, GlobSet fallback for
  Build B, resolver against a fixture vault.
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

### Phase 0.7 — Page-ACL `Option<user_id>` Refactor (Build A only)

**Goal:** unify the page-ACL pipeline with the asset-ACL pipeline so
SPL can evaluate anonymous subjects.

- `src/acl.rs::AclQuery` gains `user_id: Option<String>` (was
  `String`); construct sites that already had `Option<String>` from
  `extract_session_user_id` stop unwrapping.
- `evaluate()`: when `user_id.is_none()`, inject `("anonymous",
  false)` and skip the `(given (authenticated …))` fact — exactly
  mirroring `check_can_read_assets` at `src/acl.rs:1351–1370`.
- Built-in defaults: ship `(forbidden edit (subject anonymous)
  (any))` as a hardcoded SPL fact in `built_in_defaults()`.
- New `AclEvaluator::decide(principal: Option<&Principal>, action:
  Action, page: &str) -> Decision` — the unified entry point both
  `collab_gate` and per-page handlers consult. Wraps `evaluate()`.
- Build B (`--features reason` off): `AclEvaluator::decide` is a
  separate impl that consults the compiled GlobSet for reads,
  hardcoded deny for anonymous-edit, permit-all for authenticated
  (preserves current SPEC-020 behaviour where per-page role checks
  live in handlers).
- Re-run the entire SPEC-020 and SPEC-041 test suite — no behavioural
  regressions for authenticated callers.
- **Gate:** existing SPEC-020 / SPEC-041 suites green; new tests for
  `decide(None, …)` shape.

### Phase 1 — Gate Becomes a Thin SPL Caller

**Goal:** mixed public/auth pages work via the unified ACL evaluator.

- `WebState` gains `pub acl: Arc<AclEvaluator>` (replaces the
  Phase-0-only `public_paths: Arc<GlobSet>` if it landed earlier).
  Compiled once in `web::run` from the loaded config + the vault's
  `.zetl/collab/access.spl` (Build A) or the GlobSet (Build B).
- Modify `src/web/session.rs::collab_gate` per [[#CON-4202]] — call
  `state.acl.decide(principal, action, path)`, translate
  permit/deny to next/401/403.
- Modify `src/web/auth/capability_url.rs::capability_gate` per
  [[#CON-4203]] — asymmetric scope check (skip on safe methods,
  enforce on non-safe) per the revised [[#REQ-4204]].
- Add the [[#REQ-4212]] cache-header layer to anonymous-permit
  responses.
- Build A: load + compile `.zetl/collab/access.spl` at startup;
  startup-banner warning per [[#REQ-4208]] for operator rules that
  conflict with compiled-from-TOML facts.
- Build B: startup-banner warning per [[#REQ-4214]] when
  `.zetl/collab/access.spl` exists but the engine isn't compiled in.
- OBS-4203 / OBS-4204 / OBS-4205 wiring; OBS for compiled-fact
  provenance ("`from: public_paths`" annotation visible in
  `hence query explain` and the preview CLI).
- Integration tests against an axum mock router for the gate matrix
  (including the editor-PUT-on-public-page case from HP4 and the
  cap-URL write-rejected case from REQ-4204). Run the matrix under
  both Build A and Build B.
- **Gate:** [[#TEST-4202]], [[#TEST-4203]], [[#TEST-4204]],
  [[#TEST-4205]], [[#TEST-4208]], [[#TEST-4209]], [[#TEST-4212]],
  [[#TEST-4214]] green under both build configurations. Existing
  SPEC-041 + SPEC-020 suites unchanged.

### Phase 2 — Principal-Aware Search / Backlinks Filtering

**Goal:** no title / slug / excerpt leakage to anyone who shouldn't
see them — anonymous OR Reader-role.

- Thread `principal: Option<&Principal>` (was `is_anonymous: bool`
  in the v0.1.0 strawman) into the search + backlinks query
  pipelines.
- Add the per-result `decide(principal, Read, slug)` filter at the
  response boundary ([[#CON-4204]]). Same query the gate uses.
- Audit every endpoint that emits page-name lists (the [[#Threat
  Model B]] inventory table is the canonical checklist): `/api/search`,
  `/search`, `/api/backlinks/*`, `/api/graph`, `/llms.txt`, RSS
  feeds (SPEC-038), `/sitemap.xml` if present, `<title>` / OpenGraph /
  Twitter Card metadata, error-page titles. Each gets the
  response-boundary filter or a documented exemption recorded in
  `docs/collab-public-paths.md`.
- Implement [[#REQ-4213]] wikilink-on-public-page rendering — the
  render-time half of [[#Threat Model B]]. Author the missing
  [[#ADR-4207]] (strike-through vs plain-text vs omit) before
  implementing. Render-time predicate is also `decide(principal,
  Read, target_slug)`.
- Add a CI lint that flags new endpoints which return
  `Vec<PageName>`-shaped responses without going through the
  shared `decide()` filter (defence-in-depth against future SPECs
  forgetting the filter).
- **Gate:** [[#TEST-4207]], [[#TEST-4213]] green; targeted property
  test that no slug outside the principal's `(can-read)` set
  appears in any response across the full endpoint inventory, for
  both anonymous and Reader-role principals.

### Phase 3 — Docs + Review

- `docs/collab-auth.md` extended with a "Public + Private Pages"
  section: the TOML sugar, the SPL escape hatch, the threat-model
  summary, the dangerous-glob list, the Build A vs Build B
  distinction.
- `docs/collab-public-paths.md` (new) — dedicated operator guide
  covering the CDN/cache footgun (Threat Model G), persistent
  exposure (Threat Model H), and the endpoint-inventory exemption
  list from Phase 2.
- `user-guide/collaboration/Authentication Methods.md` extended
  with a `public_paths` subsection.
- CHANGELOG entry under `[Unreleased]`.
- TEST-adversarial-042 — cross-model adversarial review of the
  deliverable (PROTO-001 Principle 12, fresh context, different
  model).

### Sequencing Rationale

Phase 0 is pure data + grammar; trivially reversible. **Phase 0.5
ships the preview CLI before the gate is wired** — operators get a
"show me what this WOULD do" tool with no server change, so the
SPEC-042 design can be evaluated against real vaults before any code
path actually exposes a page anonymously. **Phase 0.7 is the
load-bearing architectural shift** — the page-ACL `Option<user_id>`
refactor unifies the policy surface (closes Threat Model E by
construction, sets up Threat Model I resolution). The asset-ACL
path proves the diff shape; risk is mostly about disciplined
audit of every call site. Phase 1 ships the operator-visible feature
on top of the unified evaluator — the gate becomes a thin caller,
not a parallel decision maker. Phase 2 closes the search/backlinks
leak surface using the same evaluator (so anonymous + Reader-role
filtering share machinery). Phase 3 is docs + review.

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
3. **Cache headers on public responses.** ~~Strawman deferred~~ —
   resolved by [[#REQ-4212]] after the [[#Threat Model G]] review.
   Default is `public, max-age=0, must-revalidate` + `Vary: Cookie,
   Authorization`; operators can override at their reverse proxy if
   they understand the caching implications.
4. **`robots.txt` and search-engine indexability.** Public pages
   probably want to be indexable; private pages should be
   `Disallow:`'d. Should the gate emit a generated `/robots.txt`
   reflecting `public_paths`? Likely yes; small adjacency, may belong
   in a follow-up. Beware: a `robots.txt` listing every private path
   under `Disallow:` is a negative-space leak — adversaries scrape
   `robots.txt` first to discover hidden URLs. Recommended pattern:
   generate `Allow: <public glob>` lines + a single `Disallow: /`
   below them, instead of enumerating private paths.
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
8. **Wikilink-on-public-page rendering policy** (filed as
   [[#REQ-4213]]; ADR-4207 still TBD). When a public page renders a
   `[[wikilink]]` to a private page, what does the anonymous viewer
   see? Candidates: (a) strike-through with the title text removed
   (`[[unknown]]`-style); (b) plain text of the link literal
   (`[[Internal Memo]]` rendered as plain text, no link); (c) the
   link omitted entirely (text reflows around it); (d) rendered as a
   404-link (title removed, target stays as the page slug). Each has
   different UX (strike is most explicit; omit is most invisible;
   plain-text leaks the wikilink target but not the title; 404-link
   leaks the slug). The strawman doesn't pick — operator feedback on
   the real corpus is the right way to decide; ADR-4207 captures the
   decision before Phase 2 implements.
9. **Per-result role-aware filtering for authenticated search**
   (Threat Model I follow-up). Search currently doesn't consult SPL
   per-result, so a Reader-role user can see search hits on
   Editor-restricted pages. SPEC-042's response-boundary filter is
   the natural place to layer this on top — same machinery, the
   predicate widens from "is the request anonymous?" to "what does
   the requesting principal's role permit?". Out of scope for
   SPEC-042 itself; flagged for a small follow-up SPEC.
10. **Authenticated-edit attribution on public paths after revised
    REQ-4203.** The revised REQ-4203 lets authenticated editors PUT
    public pages; the audit log records the user, git attributes the
    commit. Operators should verify their public-page edit workflow
    AFTER the revised semantics ship — pre-revision, this path was
    405'd, so no operator has exercised it.
