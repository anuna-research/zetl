---
title: "SPEC-038: RSS / Atom feed support for zetl"
version: 0.1.0-strawman
status: strawman
date: 2026-05-06
audience: agent, human
parent: null
related:
  - SPEC-005  # Defeasible reasoning (SPL — candidate selection mechanism)
  - SPEC-012  # Named themes for serve/build (theme override conventions)
  - SPEC-016  # Lifecycle hooks (current user-space RSS workaround; build timing)
  - SPEC-020  # Multi-user collaborative editing (visibility / ACL)
  - SPEC-030  # Theme data contract (template context compatibility)
  - SPEC-034  # Capability-mode HTML sanitiser (feed-content sanitisation alignment)
  - SPEC-035  # Static asset uploads (file emission idioms; precedent for build-time outputs)
plan: DESIGN-038-rss-support
---

# SPEC-038: RSS / Atom feed support for zetl

> **Strawman notice.** This document is a first-pass produced *before* the
> Phase 0 surveys, prior-art research, and synthetic-user simulations called
> for by [[DESIGN-038-rss-support]] (`plans/DESIGN-038-rss-support.spl`).
> Sections labelled **`[Provisional — refined by DESIGN-038 task X]`** are
> deliberate placeholders that the plan tasks will replace with grounded
> findings. Do not implement against this version. The version reaches
> `0.2.0` (status `draft`) only after Phase 1 + Phase 2 quality gates pass,
> and `1.0.0` (status `approved`) only after the Tier 2 cross-model review
> and human reviewer sign-off (per [[PROTO-001]] §AI Trust Boundaries
> §Multi-Model Cognitive Diversity).

## Information Table

| Field          | Value                                                                       |
| -------------- | --------------------------------------------------------------------------- |
| Document ID    | SPEC-038                                                                    |
| Title          | RSS / Atom feed support for zetl                                            |
| Version        | 0.1.0-strawman                                                              |
| Status         | Strawman (not implementable; pending [[DESIGN-038-rss-support]] execution)  |
| Author         | Agent (Claude Opus 4.7, [[PROTO-001]] v1.6.0)                               |
| Date           | 2026-05-06                                                                  |
| Audience       | Agent, Human                                                                |
| Trace          | [[PROTO-001]] §Phase 1, §Phase 2, §AI Trust Boundaries                      |
| Parent         | (none — independent feature, not a child specification)                     |
| Related        | [[SPEC-005]], [[SPEC-012]], [[SPEC-016]], [[SPEC-020]], [[SPEC-030]], [[SPEC-034]], [[SPEC-035]] |
| Plan           | `plans/DESIGN-038-rss-support.spl` ([[DESIGN-038-rss-support]])             |
| Review tier    | Tier 2 (core feature; inbound-feed direction is at a [[Trust Boundary]])    |

---

## 1. Overview

`zetl` parses `[[wikilinks]]` from a vault of Markdown pages, builds a
bi-directional link graph, and exposes the graph through CLI queries, a
[[Page Viewer]] TUI, a [[zetl serve]] web UI, and a [[zetl build]] static
export. The build output already includes machine-readable artefacts:
`/sitemap.xml`, `/pages.json`, `/graph-index.json`, and `/llms.txt`. Vaults
published this way are reachable, indexable by search engines, navigable by
graph clients, and consumable by language-model agents — but they are
**not subscribable**. A reader who wants notifications when the publisher
adds or updates a page has no standards-conformant feed to subscribe to.

This specification introduces **first-class [[RSS 2.0]] and [[Atom 1.0]]
feed emission** as a [[zetl build]] / [[zetl serve]] output, derived
deterministically from vault pages and their frontmatter. The strawman
also surfaces a second, more contentious direction: **inbound feed
ingestion**, where `zetl` periodically pulls external feeds and
materialises items as vault pages, turning the PKM into a feed
aggregator. The inbound direction sits at a [[Trust Boundary]] — every
fetched feed is untrusted XML — so its inclusion in v1 is an explicit
[[ADR]] decision (see [[#ADR-3803]]) gated on threat-model and
fetch-stack survey findings.

### 1.1 Motivation

- **Subscription is the missing public-vault primitive.** A vault that
  publishes to the web and exposes a search index, sitemap, and graph
  but no feed is unfindable in the [[Open Web]] subscription layer. RSS
  and Atom remain the canonical way for a human or aggregator to follow
  a publisher's incremental output without polling the site by hand.
- **Vault frontmatter already carries the post-shape signal.** zetl
  pages routinely declare `title`, `date`, `status`, `tags`. A feed
  emitter is a thin projection from that data into a standards-conformant
  XML document; the load-bearing logic is selection (which pages?) and
  rewriting (wikilinks → absolute URLs).
- **Static and live builds align cleanly.** The same selection +
  serialisation pipeline produces a file at `dist/feed.xml` for
  [[zetl build]] and registers a route at `/feed.xml` for
  [[zetl serve]]. There is no architectural split.
- **PKM aggregation is a common ask, not a free lunch.** Inbound feed
  ingestion is the second most-requested PKM feature in the Reddit /
  Discord channels reviewed during research-prior-art (citation deferred
  to [[#research/SPEC-038-prior-art-inbound]]). It is also the largest
  threat surface this spec introduces. The plan separates the two
  directions so that v1 can ship outbound while the inbound design
  matures.

### 1.2 Design Principles

1. **Standards conformance is a hard gate.** Every emitted feed MUST
   validate against a pinned local feed validator / strict parser in CI
   with zero errors and zero warnings (see [[#NFR-3805]]). A manual
   external validator check MAY be part of release evidence, but CI MUST
   NOT depend on a mutable network service. Reader compatibility hinges
   on this.
2. **Deterministic and pure where possible.** Item selection, item
   ordering, item id derivation, and serialisation are pure functions
   of (vault snapshot, configuration). Re-running the same build on the
   same vault produces a byte-identical feed. See [[#Purity Boundary Map]].
3. **Stable item ids.** Each feed item has a permanent identifier that
   does not change when the page is edited, when the vault is rebuilt,
   or when the publication date is corrected. The id is derived from
   the page's stable slug, not from the title or content. See
   [[#REQ-3803]].
4. **Aggregate feeds must not widen visibility.** A feed is a projection
   of pages the reader could already access through the web UI. In
   `zetl serve --collab`, a feed route MUST evaluate the current
   session's per-page read ACL before including each item. In static
   public builds, a feed MUST include only pages selected for public
   publication. Capability-mode / encrypted static builds MUST suppress
   aggregate feeds unless a later contract defines per-cohort feed
   emission.
5. **Inbound is opt-in, off by default, and gated.** If [[#ADR-3803]]
   admits inbound to v1, every fetched URL goes through an explicit
   [[Allowlist]] / [[Denylist]] for [[SSRF]] defence; XML parsing uses
   a parser that disables [[XXE]] and bounds entity expansion; the
   fetch loop is bounded in size, time, and concurrency.
6. **Composition with existing zetl idioms.** Feed configuration lives
   in `.zetl/config.toml` under a `[feed]` table, matching the existing
   vault-configuration pattern. The publication base URL is resolved from
   `[feed].base_url` or the existing `zetl build --site-url` flag.
   Feed XML serialisation is core-owned in v1; themes may add discovery
   links in HTML templates, but MUST NOT override raw feed XML unless the
   post-template output is validated and the build fails on any
   validation error. Feeds emit to the same `dist/` tree as other build
   outputs.
7. **No mandatory new dependencies for outbound.** Outbound emission
   is implementable without a third-party RSS / Atom crate — a small
   pure-Rust serialiser with explicit XML escaping is preferred to a
   dependency that may have its own escaping bugs. The decision is
   recorded in [[#ADR-3801]] / [[#ADR-3806]] (the latter to be drafted
   if the survey-fetch-stack task surfaces a strong reason to depend
   on `atom_syndication` or `rss`).

### 1.3 Scope

**In scope (v1, outbound):**

- Emit `feed.xml` (or `atom.xml`, or both — pending [[#ADR-3801]]) at the
  vault root from [[zetl build]].
- Register the equivalent route(s) under [[zetl serve]] without breaking
  any existing route in `src/web/mod.rs` (see survey-publishing-surface
  task).
- Item selection by frontmatter opt-in (`feed: true`), folder
  membership, tag membership, or [[SPL]] query (precedence pending
  [[#ADR-3802]]).
- Per-tag and per-folder feeds at deterministic URL patterns (e.g.
  `/tags/<tag>/feed.xml`, `/<folder>/feed.xml`) — pending [[#ADR-3802]].
- Date resolution from frontmatter with a documented fallback chain
  (`published` → `date` → `created` → git-derived first/last commit
  date → structured missing-date error — pending survey-frontmatter-dates
  findings). Filesystem mtime is explicitly not part of the default
  deterministic chain.
- Wikilink rewriting in feed item content: every `[[target]]` resolves
  to its absolute URL against the effective publication base URL;
  unresolved wikilinks render per a documented policy (likely: drop the
  link syntax, preserve the literal text — pending [[#REQ-3807]]).
- ACL / visibility filtering for aggregate feeds: `serve --collab`
  filters every item by the current session's read ACL; static public
  feeds contain only publicly published pages; capability-mode aggregate
  feeds are out of scope unless explicitly admitted by a later contract.
- HTML sanitisation of item content matching the [[SPEC-034]]
  capability-mode sanitiser allowlist (or a tighter allowlist for the
  feed surface — pending [[#ADR-3807]] if drafted).
- Configurable per-feed limits: `max-items` (default 50), feed-document
  byte cap (default 1 MiB before pagination is required).
- Observability: counters for feed builds and feed bytes emitted; log
  lines on emission and on each per-page selection-rejection reason
  (with a sampling cap, since vaults can have thousands of pages — see
  [[#OBS-3801]]).
- Documentation: README section, feed-discovery theme authoring note,
  CHANGELOG entry.

**In scope (v1, inbound) — *only if [[#ADR-3803]] admits it; otherwise
deferred to v2 and the section below moves to "Out of scope":***

- An inbound feed registry in `.zetl/config.toml` (or a dedicated
  `.zetl/feeds.toml` — pending [[#CON-3807]]).
- A `zetl feed pull` CLI subcommand (or whatever surface
  [[#ADR-3805]] selects) that fetches each registered feed once.
- XML parsing with [[XXE]] disabled, entity-expansion bounded, and an
  explicit no-go list for crates lacking these controls.
- Persistence of fetched items as Markdown pages under a configured
  folder (default `inbox/<feed-slug>/`), with first-seen identity-record
  deduplication over [[GUID]], canonical link, and content fingerprint.
- Network-policy controls: redirect allowlist (no [[RFC 1918]],
  link-local, loopback, `file://`), per-fetch byte cap, per-fetch
  timeout, decompression bomb defence, connection-pool concurrency cap.
- Observability: counter for fetch attempts, counter for fetch
  outcomes (success, parse-error, network-error, policy-rejection),
  per-feed last-success timestamp gauge.

**Out of scope (v1):**

- [[WebSub]] / [[PubSubHubbub]] hub support (push-mode subscription).
- [[JSON Feed]] emission (RSS 2.0 + Atom 1.0 cover the field; JSON Feed
  is recommendable but not load-bearing).
- [[OPML]] import / export for inbound subscriptions (deferred even if
  inbound is in v1).
- [[ActivityPub]] or [[Fediverse]] interop (entirely separate spec).
- Per-feed authentication for inbound (HTTP Basic, bearer tokens,
  cookies). Feeds requiring auth fall outside v1 inbound scope.
- Inline image transclusion in feed item content (e.g., embedded base64
  images). v1 emits links to images at their absolute URLs.
- Real-time push notifications when a vault page is added (the static
  build is rebuilt on the publisher's cadence; readers poll on theirs).
- Per-tag `<atom:category>` enrichment beyond the simplest case (i.e.,
  tags are emitted as flat strings, not [[Taxonomy]] entries with
  schemes).
- Server-side feed-reader UI (this would duplicate miniflux / FreshRSS
  poorly; explicit non-goal).

### 1.4 Risks and open questions (the plan resolves these)

- **Inbound v1 yes/no.** The dominant scope decision. Defers to
  [[#ADR-3803]]. A "no" outcome shrinks this spec by roughly half.
- **Format choice.** RSS 2.0, Atom 1.0, both. Defers to [[#ADR-3801]].
- **Selection mechanism precedence.** Frontmatter / folder / tag / SPL.
  Defers to [[#ADR-3802]].
- **Wikilink unresolved-target policy.** Drop link, preserve text;
  preserve literal `[[target]]` syntax; emit a documented placeholder
  URL. Defers to [[#REQ-3807]] (provisional).
- **Date fallback chain in detail.** Defers to survey-frontmatter-dates.

---

## 2. User Profiles

> User profiles live in `/users/feed-publisher/user.md` (and
> `/users/feed-subscriber/user.md` if [[#ADR-3803]] admits inbound).
> The strawman summarises them inline; the full profiles are produced
> by [[DESIGN-038-rss-support#task-user-profiles]].

### 2.1 UP-3801 Vault Publisher

**Role:** Self-hoster running `zetl build` (or `zetl serve`) on a
public vault — a personal blog, a project changelog, a research
notebook, or a team-facing documentation site.

**Goals:** publish a feed at a stable URL; have it Just Work in
NetNewsWire / Feedbin / Reeder / FreshRSS / miniflux without manual
intervention; control which pages ship in the feed.

**Constraints (provisional):** intermediate-CLI fluency; comfortable
editing `.zetl/config.toml` and frontmatter; *not* assumed to know the
RSS or Atom XML schema.

**Daily workflow:** edits Markdown files, runs `zetl build`,
`rsync`/CI deploys `dist/` to the host, expects the feed at
`https://vault.example.com/feed.xml` to update automatically.

### 2.2 UP-3802 Feed Reader (downstream consumer)

**Role:** end-user of any standards-conformant feed reader; never
interacts with `zetl` directly.

**Goals:** subscribe with a single URL paste; see new items appear with
correct titles, dates, links, and content; have item links resolve to
the published page.

**Constraints (provisional):** behaviour is governed by the feed
reader; the reader expects RFC-conformant XML, valid date formats per
[[RFC 822]] (RSS) or [[RFC 3339]] (Atom), unique persistent ids, and
links that resolve at the publisher's host.

### 2.3 UP-3803 PKM Subscriber `[Provisional — refined by DESIGN-038 task user-profiles, contingent on ADR-3803]`

**Role:** vault owner who wants to ingest external feeds into their
vault for note-taking, citation, or aggregation.

(Full profile authored only if inbound is admitted to v1.)

### 2.4 UP-3804 Operator / Administrator `[Provisional — refined by DESIGN-038 task user-profiles, contingent on ADR-3803]`

**Role:** managing scheduled fetches, network-policy lists, dedup
state for one or more vaults.

(Full profile authored only if inbound is admitted to v1.)

---

## 3. Happy Paths

> Full happy paths live in `/users/feed-publisher/happy-paths.md`
> (and `/users/feed-subscriber/happy-paths.md` if applicable). The
> strawman lists titles only.

| ID | Title | Profile | Status |
|---|---|---|---|
| HP-3801 | Publisher emits a feed via `zetl build` | UP-3801 | Provisional; refined by task-happy-paths |
| HP-3802 | Publisher adds a per-tag feed | UP-3801 | Provisional |
| HP-3803 | A page is removed from the vault between builds | UP-3801 | Provisional; surfaces retention policy |
| HP-3804 | Publisher publishes a page with no date frontmatter | UP-3801 | Provisional; surfaces fallback chain |
| HP-3805 | Reader subscribes in NetNewsWire | UP-3802 | Provisional; standards-conformance probe |
| HP-3806 | PKM Subscriber adds a feed URL and ingests items | UP-3803 | Provisional; contingent on ADR-3803 |
| HP-3807 | An external feed serves malformed XML | UP-3803 / UP-3804 | Provisional; threat-model probe |
| HP-3808 | An external feed serves a 10 MB billion-laughs payload | UP-3803 / UP-3804 | Provisional; threat-model probe |

---

## 4. Synthetic User Simulation

`[Provisional — produced by DESIGN-038 task-synthetic-user-run.
research/SPEC-038-synthetic-user.md will hold the full transcript and
findings.]`

---

## 5. Functional Requirements

> Numbering convention: REQ-3801, REQ-3802, ... — the prefix `38`
> binds artefact ids to SPEC-038, matching the [[SPEC-035]] convention
> and avoiding collisions with cross-spec REQ-### references. The
> requirements below are **provisional placeholders**; the
> draft-requirements task graduates this section to grounded text.

### REQ-3801: Outbound feed emission

The system SHALL emit a feed document at `dist/feed.xml` (and/or
`dist/atom.xml`, per [[#ADR-3801]]) on every successful run of
`zetl build` for vaults that declare a feed configuration in
`.zetl/config.toml` (or via an explicit CLI flag admitted by
[[#CON-3804]]), WITHIN the same build pass that emits other static
artefacts (no second invocation), FOR every Vault Publisher
([[#UP-3801]]) WITH the emitted file validating against the pinned local
validator selected by [[#NFR-3805]].

Trace:
- [[#TEST-3801]] (positive)
- [[#TEST-3802]] (negative-input — empty vault)
- [[#TEST-3803]] (negative-output — invalid XML rejected)
- [[#CON-3802]] (build-output contract)
- [[#OBS-3801]] (feed-build counter)

### REQ-3802: Item selection

The system SHALL include a vault page in the canonical feed if and only
if the page satisfies BOTH the configured selection rule (frontmatter
opt-in, folder membership, tag membership, or [[SPL]] query —
precedence per [[#ADR-3802]]) AND the publication-visibility predicate
for the current output surface. For `zetl serve --collab`, the
visibility predicate is the current authenticated user's per-page read
ACL. For static public builds, it is membership in the publicly
published page set. For capability-mode / encrypted builds, the default
predicate is false until a per-cohort feed contract exists. The selection
function remains a deterministic pure projection of (vault snapshot,
configuration, authenticated user/cohort identity when applicable).

Trace: [[#TEST-3804]], [[#TEST-3805]], [[#CON-3804]], [[#CON-3805]]

### REQ-3803: Item identifier stability

Each feed item SHALL carry a permanent identifier (`<guid
isPermaLink="false">` for RSS, `<id>` for Atom) that is a deterministic
function of the page's resolved slug, INDEPENDENT of the page's title,
content, frontmatter values other than the slug-determining inputs,
build timestamp, or build host, FOR all pages emitted in any feed,
WITH the identifier byte-identical across rebuilds and machines for the
same input.

Trace: [[#TEST-3806]] (property test — id stability under content
edit), [[#TEST-3807]] (property test — id stability across machines),
[[#CON-3802]]

### REQ-3804: Date resolution

For every emitted feed item, the system SHALL determine the item's
publication date (and updated date for Atom) by walking a documented
fallback chain (provisional: `frontmatter.published` →
`frontmatter.date` → `frontmatter.created` → git committer date →
structured missing-date error), stopping at the first present value, FOR
all selected pages, WITH the resulting date being timezone-aware and
serialised in [[RFC 822]] (RSS) or [[RFC 3339]] (Atom) format.
Filesystem mtime MUST NOT be used in the default build because it is not
stable across checkout / copy / CI hosts; if a later ADR admits an
`allow_unstable_mtime` preview mode, that mode MUST be opt-in and MUST
mark the feed non-reproducible in diagnostics.

Trace: [[#TEST-3808]] .. [[#TEST-3812]] (one per fallback step plus
two negative cases — malformed and future-dated)

### REQ-3805: Wikilink rewriting

In feed item content, every `[[wikilink]]` SHALL be rewritten to an
absolute URL against the effective publication base URL if and only if
the target resolves to a vault page that is itself published and
readable on the current feed surface; unresolved, unpublished, or
ACL-denied targets SHALL be handled by a documented policy (per
[[#REQ-3807]]) without leaking denied-page URLs in hidden visibility
mode.

Trace: [[#TEST-3813]] .. [[#TEST-3815]]

### REQ-3806: Content sanitisation

Feed item content SHALL be HTML-escaped per the feed format's escaping
rules (CDATA for RSS content, character entities for Atom content
type=html or XHTML for content type=xhtml) AND subject to the
[[SPEC-034]] capability-mode sanitiser allowlist (or a tighter
allowlist per [[#ADR-3807]] if drafted) BEFORE emission, FOR every
item, WITH no item content bypassing sanitisation. If a theme or hook is
ever allowed to affect feed XML, sanitisation and standards validation
MUST run after that effect and MUST fail the build on error.

Trace: [[#TEST-3816]] (XSS-attempt fixture), [[#TEST-3817]] (allowlist
boundary)

### REQ-3807: Unresolved-wikilink policy `[Provisional — refined by DESIGN-038 task-draft-requirements]`

The system SHALL handle wikilinks whose target does not resolve to a
published page by [strategy: drop the link syntax and preserve literal
display text | preserve literal `[[target]]` syntax | emit a documented
placeholder URL with rel="nofollow"], FOR every emitted item.

Trace: [[#TEST-3818]] (provisional)

### REQ-3808: Atom self-link correctness

If Atom is emitted (per [[#ADR-3801]]), every emitted feed document
SHALL contain an `<atom:link rel="self" href="..."/>` whose `href`
matches the URL at which the feed is published, FOR every feed
document.

Trace: [[#TEST-3819]]

### REQ-3809: Feed self-publication safety

The system SHALL refuse to emit a feed for a vault whose
effective publication base URL (resolved from `.zetl/config.toml`
`[feed].base_url` and/or `zetl build --site-url`, per [[#CON-3804]]) is
unset, relative, non-HTTP(S), or contains a URL fragment, FAILING the
build with a structured error referencing this REQ, FOR every Vault
Publisher attempting to build without valid publication configuration.

Trace: [[#TEST-3820]] (negative-input)

### REQ-3810: Inbound fetch admission `[Provisional — contingent on ADR-3803]`

If [[#ADR-3803]] = Y: the system SHALL, on invocation of the
configured inbound trigger (per [[#ADR-3805]]), fetch each registered
feed exactly once per invocation, subject to the network policy
defined in [[#REQ-3811]], FOR every PKM Subscriber.

Trace: [[#TEST-3821]] (provisional)

### REQ-3811: Inbound network policy `[Provisional — contingent on ADR-3803]`

If [[#ADR-3803]] = Y: every inbound HTTP request SHALL respect a
default-deny network policy that REJECTS targets resolving to
[[RFC 1918]], link-local, loopback, multicast, or `file://`/`data://`
URI schemes, AND enforces a redirect-target check at every hop, AND
caps body size, response time, and decompression ratio, FOR every
fetch attempt.

Trace: [[#TEST-3822]] .. [[#TEST-3826]] (provisional)

### REQ-3812: Inbound dedup convergence `[Provisional — contingent on ADR-3803]`

If [[#ADR-3803]] = Y: the system SHALL deduplicate inbound items by a
persisted first-seen identity record containing every stable signal
observed at ingest time: feed URL, item [[GUID]] when present,
canonicalised item link when present, and a normalised content hash. A
new inbound item is a duplicate if ANY persisted signal for the same
feed matches. A feed-side [[GUID]] mutation MUST update the persisted
alias set for the existing item when link or content fingerprint matches;
it MUST NOT create a second vault page. [[GUID]] is therefore an input
signal, not the sole authority.

Trace: [[#TEST-3827]] (state-machine property test)

---

## 6. Non-Functional Requirements

### NFR-3801: Feed-build latency

Feed-build duration SHALL be ≤ 300 ms at the 95th percentile for
vaults of ≤ 5,000 pages on commodity x86_64 hardware (target: 4-core
2.5 GHz baseline) UNDER cold-cache conditions WITH 95th-percentile
confidence.

Trace: [[#OBS-3802]], [[#TEST-3828]] (benchmark)

### NFR-3802: Feed file size cap

Each emitted feed document SHALL be ≤ 1 MiB before pagination is
required, UNDER any vault size. If Atom is emitted and pagination is
enabled, continuation feeds SHALL follow [[Atom Paged Feeds]] [[RFC
5005]]. If RSS-only output is selected, excess items SHALL be truncated
at `max-items` by default; an RSS pagination extension MUST NOT be
invented without a separate ADR. If a configured item/content selection
would exceed 1 MiB even after truncation, the build SHALL fail with a
structured remediation hint.

Trace: [[#TEST-3829]]

### NFR-3803: Item count cap per feed

A single feed document SHALL contain ≤ `max-items` (default 50)
entries, with `max-items` configurable in `.zetl/config.toml`, WITH
excess items truncated newest-first for RSS-only output, and either
truncated or paginated per [[Atom Paged Feeds]] for Atom output
depending on configuration.

Trace: [[#TEST-3830]]

### NFR-3804: Item id stability NFR

The function `item-id : (page) → bytes` SHALL be a [[Pure Function]]
that produces byte-identical output for the same input across machines,
zetl versions in the same major-version line, and rebuilds within the
same major-version line, WITH a documented migration path if the
function is ever revised in a breaking way.

Trace: [[#TEST-3806]], [[#TEST-3807]]

### NFR-3805: Standards conformance

Every emitted feed document SHALL validate against a pinned local
validator / strict parser chosen per [[#ADR-3801]] with zero errors and
zero warnings, UNDER the test fixture corpus described in
[[#TEST-3801]] / [[#TEST-3802]], WITH verification automated in CI. A
manual [[W3C Feed Validator]] check MAY be recorded as release evidence,
but the public service MUST NOT be the CI gate.

Trace: [[#TEST-3831]] (CI validation step)

### NFR-3806: Inbound fetch concurrency `[Provisional — contingent on ADR-3803]`

If [[#ADR-3803]] = Y: concurrent inbound fetches SHALL be capped at
8 by default and configurable, UNDER any registered-feed count, WITH
fetches beyond the cap queued (not dropped).

Trace: [[#TEST-3832]] (provisional)

### NFR-3807: Failure-mode observability

For every distinct REQ failure mode, the system SHALL emit a log line
at `warn` or `error` level with a structured (JSON) payload naming the
violated REQ, the offending input identity (page slug or feed URL),
and a remediation hint, FOR every failure occurrence.

Trace: [[#OBS-3803]]

---

## 7. Architecture Decision Records

> ADR placeholders are listed by id and topic. The plan tasks
> [[DESIGN-038-rss-support#task-adr-rss-vs-atom]],
> [[DESIGN-038-rss-support#task-adr-feed-selection]],
> [[DESIGN-038-rss-support#task-adr-inbound-scope]],
> [[DESIGN-038-rss-support#task-adr-inbound-storage]], and
> [[DESIGN-038-rss-support#task-adr-scheduling]] populate them.

### ADR-3801: Feed format — RSS 2.0, Atom 1.0, or both

`[Provisional — refined by DESIGN-038 task-adr-rss-vs-atom]`

**Status:** proposed.
**Context:** RSS 2.0 has the widest feed-reader support but is
under-specified in places (date formats, namespacing, content
escaping); Atom 1.0 ([[RFC 4287]]) is precise and extensible but
slightly less universal in legacy readers.
**Decision:** *(deferred to plan task)*.
**Consequences:** *(deferred)*.

### ADR-3802: Item selection mechanism precedence

`[Provisional — refined by DESIGN-038 task-adr-feed-selection]`

**Status:** proposed.
**Context:** Five candidate mechanisms (frontmatter opt-in, folder,
tag, [[SPL]] query, hybrid). Each has discoverability / footgun /
implementation cost trade-offs.
**Decision:** *(deferred)*.
**Consequences:** *(deferred)*.

### ADR-3803: Inbound feed support in v1

`[Provisional — refined by DESIGN-038 task-adr-inbound-scope]`

**Status:** proposed (default-leaning toward N or D — see plan).
**Context:** Inbound brings substantial threat surface ([[XXE]],
[[SSRF]], decompression bombs, redirect attacks, [[GUID]]-mutation
dedup attacks) and operational surface (scheduling, network policy,
persistence). The plan evaluates Y / N / D-with-recipe.
**Decision:** *(deferred)*. The decision determines whether
[[#REQ-3810]], [[#REQ-3811]], [[#REQ-3812]], [[#NFR-3806]],
[[#ADR-3804]], [[#ADR-3805]], [[#CON-3806]], [[#CON-3807]] survive
to v1 or move to a "Deferred" appendix.

### ADR-3804: Inbound storage model `[Provisional — contingent on ADR-3803]`

`[Provisional — refined by DESIGN-038 task-adr-inbound-storage]`

### ADR-3805: Inbound scheduling model `[Provisional — contingent on ADR-3803]`

`[Provisional — refined by DESIGN-038 task-adr-scheduling]`

---

## 8. Contracts

> Contracts are defined per-clause so each implemented [[#REQ-3801]]
> .. [[#REQ-3812]] maps to a distinct, named clause of pre/post/error
> conditions ([[PROTO-001]] §Contract Specification).

### CON-3801: CLI surface — `zetl build` (feed emission) `[Provisional — refined by DESIGN-038 task-contracts]`

### CON-3802: Build output paths `[Provisional]`

### CON-3803: Serve route patterns `[Provisional]`

### CON-3804: Configuration surface (`.zetl/config.toml` `[feed]` table + `--site-url` precedence) `[Provisional]`

### CON-3805: Frontmatter contract for feed inclusion `[Provisional]`

### CON-3806: Inbound CLI surface `[Provisional — contingent on ADR-3803]`

### CON-3807: Inbound configuration surface `[Provisional — contingent on ADR-3803]`

---

## 9. Purity Boundary Map

> Authored in detail by [[DESIGN-038-rss-support#task-purity-boundary-map]].
> The strawman names the modules and the dependency rule.

### Pure Core (no I/O, no shared state, deterministic)
- `feed::serialise` — given an ordered list of `FeedItem` records and
  `FeedConfig`, produce the XML byte string for RSS 2.0 and/or Atom 1.0.
- `feed::select` — given a vault snapshot and a `SelectionRule`, produce
  the ordered list of pages that ship in a feed.
- `feed::rewrite_links` — given Markdown content, an effective
  publication base URL, an ACL / publication-visibility predicate, and a
  slug-resolver function, produce content with every `[[wikilink]]`
  rewritten to an absolute URL (or handled per the unresolved/denied
  target policy).
- `feed::resolve_date` — given a page's frontmatter and deterministic
  history metadata, produce the item's date per the documented fallback
  chain.
- `feed::item_id` — given a page's resolved slug, produce the stable
  feed-item identifier ([[#REQ-3803]]).
- *(inbound, contingent)* `feed::parse_inbound` — given fetched XML
  bytes, produce a `Vec<InboundItem>` or a structured `ParseError`,
  with [[XXE]] and entity-expansion controls applied at parser
  construction.
- *(inbound, contingent)* `feed::dedup` — given a new `Vec<InboundItem>`
  and the persisted state, produce the diff to apply.

### Effectful Shell (orchestrates I/O, calls pure core)
- `feed::build` — read vault, call pure core, write `dist/feed.xml`.
- `feed::serve` — register the route on the [[Axum]] router, call pure
  core on each request.
- *(inbound)* `feed::fetch` — issue HTTP requests bounded by the
  network policy from [[#REQ-3811]], stream into a bounded buffer,
  hand bytes to the pure parser.
- *(inbound)* `feed::persist` — write items to the vault folder
  configured by [[#ADR-3804]], update the dedup state.

### Boundary Contracts
- `FeedConfig` (shell → core): the parsed, validated feed-configuration
  record.
- `FeedItem` (core → shell): the validated, ready-to-serialise item.
- `InboundItem` (core → shell, contingent): the parsed inbound item
  prior to dedup-driven persistence.
- `SelectionRule`, `ParseError`, `NetworkPolicyDecision`: explicit
  boundary types so the shell cannot smuggle effects through the core.

### Dependency Rule
Dependencies point inward: shell → core. Core MUST NOT import from
shell. Specifically, the pure modules MUST NOT import `tokio`, `axum`,
`ureq`, `reqwest`, `git2`, or `std::fs::File` write-side APIs.

### Enforcement
Module visibility (`pub(crate)` on shell-side functions only),
arch-lint rule in CI prohibiting `use` from `feed::shell` inside
`feed::core`, and code review.

---

## 10. Verification Strategy

> Authored in detail by [[DESIGN-038-rss-support#task-test-strategy]].

| Technique | Target |
|---|---|
| Example-based testing | every [[#REQ-3801]] .. [[#REQ-3812]], decomposed into positive / negative-input / negative-output per [[PROTO-001]] §Requirement-Targeted Test Decomposition |
| Property-based testing | XML roundtrip (parse(serialise(items)) ≡ items modulo whitespace), wikilink-rewriting idempotence, item-id stability under content edit, dedup convergence (state-machine model) |
| Standards-conformance testing | every emitted-feed test fixture validates against the pinned local validator / strict parser chosen by [[#ADR-3801]] with zero errors / zero warnings — automated in CI per [[#NFR-3805]]; external W3C validation is optional release evidence |
| Fuzzing (inbound, contingent) | XML parser at the trust boundary; corpus seeded from prior-art-inbound research and the threat-model |
| Mutation testing | the pure core (`feed::serialise`, `feed::select`, `feed::rewrite_links`, `feed::resolve_date`, `feed::item_id`); kill-rate threshold ≥ 80 % |
| Adversarial testing | per [[DESIGN-038-rss-support#task-adversarial-tests]] — boundary cases at preconditions, joint-input combinations, intent contradictions |

---

## 11. Threat Model

> Authored in detail by [[DESIGN-038-rss-support#task-threat-model]].
> The strawman enumerates the threat list; severity / likelihood /
> mitigation columns are filled by the plan task.

| ID | Threat | Surface | Status |
|---|---|---|---|
| T1 | Malicious vault content injected into outbound feed item content | outbound | Provisional; mitigation references [[SPEC-034]] sanitiser |
| T2 | Vault-size DoS on feed build (millions of pages) | outbound | Provisional; mitigation [[#NFR-3803]] |
| T9 | Aggregate feed leaks title, content, or URL for a page denied by ACL or hidden visibility mode | outbound | Provisional; mitigation [[#REQ-3802]] and [[#REQ-3805]] |
| T3 | [[XXE]] in fetched feed XML | inbound (contingent) | Provisional; mitigation parser choice per task-survey-fetch-stack |
| T4 | Billion-laughs / quadratic blowup in fetched XML | inbound | Provisional; mitigation parser entity-expansion bound + size cap |
| T5 | [[SSRF]] via redirected feed URLs | inbound | Provisional; mitigation [[#REQ-3811]] |
| T6 | Decompression bomb (gzip) | inbound | Provisional; mitigation bounded decompression |
| T7 | Credential / metadata leak via UA / Referer | inbound | Provisional; mitigation minimal UA, no Referer |
| T8 | Dedup-state poisoning via [[GUID]] mutation | inbound | Provisional; mitigation [[#REQ-3812]] (first-seen pinning) |

---

## 12. Observability

### OBS-3801: Feed-build counter

Counter `zetl_feed_build_total{result}` incremented per build attempt,
with `result` ∈ {`success`, `error`}.

### OBS-3802: Feed-build duration histogram

Histogram `zetl_feed_build_duration_seconds` per build, bucketed for
the [[#NFR-3801]] target.

### OBS-3803: Failure-mode log lines

Per [[#NFR-3807]]: structured warn/error log lines naming the violated
REQ, offending identity, and remediation hint.

### OBS-3804 .. OBS-380N: Inbound observability `[Provisional — contingent on ADR-3803]`

Per-feed last-success timestamp, fetch-outcome counter, parse-error
counter — populated by [[DESIGN-038-rss-support#task-test-strategy]]
and the inbound test-strategy section.

---

## 13. Traceability

| Artefact | Implements | Verified by |
|---|---|---|
| [[#REQ-3801]] | UP-3801 daily workflow | TEST-3801, TEST-3802, TEST-3803 |
| [[#REQ-3802]] | ADR-3802 outcome + [[SPEC-020]] visibility preservation | TEST-3804, TEST-3805 |
| [[#REQ-3803]] | NFR-3804 | TEST-3806, TEST-3807 |
| [[#REQ-3804]] | survey-frontmatter-dates resolution chain | TEST-3808..TEST-3812 |
| [[#REQ-3805]] | UP-3801 expectation that links resolve | TEST-3813..TEST-3815 |
| [[#REQ-3806]] | [[SPEC-034]] alignment | TEST-3816, TEST-3817 |
| [[#REQ-3807]] | UP-3801 expectation about unresolved links | TEST-3818 |
| [[#REQ-3808]] | NFR-3805 (Atom standards conformance) | TEST-3819 |
| [[#REQ-3809]] | UP-3801 footgun resistance | TEST-3820 |
| [[#REQ-3810]] | UP-3803 (contingent) | TEST-3821 |
| [[#REQ-3811]] | T5 mitigation | TEST-3822..TEST-3826 |
| [[#REQ-3812]] | T8 mitigation | TEST-3827 |

The full traceability table — including [[CON-3801]] .. [[CON-3807]],
[[OBS-3801]] .. [[OBS-380N]], and bug-back-links once any are filed —
is produced and validated by [[DESIGN-038-rss-support#task-phase2-quality-gates]].
This strawman version exists to make orphans visible: any REQ /
CON / OBS without a row here at the time of `task-phase2-quality-gates`
fails the gate.

---

## 14. Quality Gates

### 14.1 USDD Phase 1 quality gates

- [ ] All requirements unambiguous — `[Provisional — produced by DESIGN-038 task-phase1-quality-gates]`
- [ ] All requirements verifiable
- [ ] All requirements atomic
- [ ] No internal conflicts
- [ ] Ambiguities resolved with measurable criteria (prohibited-terms
      table applied)

### 14.2 USDD Phase 2 quality gates

- [ ] Adheres to constitutional principles (1–13) — `[Provisional —
      produced by DESIGN-038 task-phase2-quality-gates]`
- [ ] All components have single responsibility (per [[#Purity Boundary Map]])
- [ ] All functionality exposed via well-defined interfaces (per
      [[#Contracts]])
- [ ] Tests derived from requirements (REQ → TEST traceability complete)
- [ ] Security controls specified with verifiable criteria (per
      [[#Threat Model]])
- [ ] Observability requirements captured

---

## 15. Convergence and Status Lifecycle

This document is `0.1.0-strawman` until [[DESIGN-038-rss-support]]
runs. The plan promotes it through:

| Version | Trigger |
|---|---|
| `0.1.0-strawman` | this initial pass |
| `0.2.0` (status `draft`) | after Phase 1 + Phase 2 quality gates pass (plan tasks `task-phase1-quality-gates` + `task-phase2-quality-gates` complete) |
| `0.3.0-rc` | after cross-model review completes ([[DESIGN-038-rss-support#task-cross-model-review]]) and adversary exhaustion is reached ([[DESIGN-038-rss-support#task-adversarial-tests]]) |
| `1.0.0` (status `approved`) | after a human reviewer signs off (final task `task-mark-spec-approved`) |
| `implementing` | when [[IMPL-038-rss-support]] begins (Phase 3, out of scope for [[DESIGN-038-rss-support]]) |
| `implemented` | when [[IMPL-038-rss-support]]'s Phase 3 quality gates pass — all tests green, documentation generated, traceability links verified |

---

## 16. Open questions for human review

The plan ([[DESIGN-038-rss-support]]) is designed to resolve every
open question before requesting human sign-off. The strawman lists
them here so reviewers reading this document directly know what is
deliberately unsettled:

1. **Inbound v1 yes / no / defer-by-recipe.** The single largest scope
   decision; resolved by [[#ADR-3803]].
2. **Format choice.** RSS, Atom, both? Resolved by [[#ADR-3801]].
3. **Selection-mechanism precedence.** Resolved by [[#ADR-3802]].
4. **Date fallback chain order.** Resolved by survey-frontmatter-dates
   findings consumed by [[#REQ-3804]].
5. **Unresolved-wikilink policy.** Resolved by [[#REQ-3807]] in the
   draft-requirements task.
6. **Whether to depend on a third-party feed crate vs roll a small
   pure-Rust serialiser.** Resolved by survey-fetch-stack and
   draft-requirements.
7. **Per-tag / per-folder feed URL pattern.** Resolved by [[#CON-3802]]
   pending [[#ADR-3802]].

---

**END OF STRAWMAN.** The next document state (`0.2.0` draft) is
produced by executing [[DESIGN-038-rss-support]].
