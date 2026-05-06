---
title: "SPEC-039: Webmention support for zetl"
version: 0.1.0-strawman
status: strawman
date: 2026-05-06
audience: agent, human
parent: null
related:
  - SPEC-005  # Defeasible reasoning (SPL — moderation rule substrate)
  - SPEC-020  # Multi-user collaborative editing (visibility / ACL)
  - SPEC-028  # Interactive 2D Graph View (external backlinks render)
  - SPEC-034  # Capability-mode HTML sanitiser (source-content sanitisation)
  - SPEC-038  # RSS / Atom feed support (sibling federation primitive)
plan: DESIGN-039-webmention
---

# SPEC-039: Webmention support for zetl

> **Strawman notice.** This document is a first-pass produced *before* the
> Phase 0 surveys, prior-art research, and synthetic-user simulations called
> for by [[DESIGN-039-webmention]] (`plans/DESIGN-039-webmention.spl`).
> Sections labelled **`[Provisional — refined by DESIGN-039 task X]`** are
> deliberate placeholders that the plan tasks will replace with grounded
> findings. Do not implement against this version. The version reaches
> `0.2.0` (status `draft`) only after Phase 1 + Phase 2 quality gates pass,
> and `1.0.0` (status `approved`) only after the Tier 2 cross-model review
> and human reviewer sign-off (per [[PROTO-001]] §AI Trust Boundaries
> §Multi-Model Cognitive Diversity).

## Information Table

| Field          | Value                                                                       |
| -------------- | --------------------------------------------------------------------------- |
| Document ID    | SPEC-039                                                                    |
| Title          | Webmention support for zetl                                                 |
| Version        | 0.1.0-strawman                                                              |
| Status         | Strawman (not implementable; pending [[DESIGN-039-webmention]] execution)   |
| Author         | Agent (Claude Opus 4.7, [[PROTO-001]] v1.6.0)                               |
| Date           | 2026-05-06                                                                  |
| Audience       | Agent, Human                                                                |
| Trace          | [[PROTO-001]] §Phase 1, §Phase 2, §AI Trust Boundaries                      |
| Parent         | (none — sibling to [[SPEC-038-rss-support]] under the federation theme)     |
| Related        | [[SPEC-005]], [[SPEC-020]], [[SPEC-028]], [[SPEC-034]], [[SPEC-038-rss-support]] |
| Plan           | `plans/DESIGN-039-webmention.spl` ([[DESIGN-039-webmention]])               |
| Review tier    | Tier 2 (core feature; receiver endpoint sits at a [[Trust Boundary]] and parses untrusted HTML / [[Microformats2]]) |

---

## 1. Overview

[[SPEC-038-rss-support]] introduces feed-shaped read-side federation: a
zetl vault can publish an [[RSS 2.0]] / [[Atom 1.0]] feed and pull
external feeds as inbound items. RSS gives a publisher a way to be
*subscribed to*, but it does not let either side know who is *linking
to* whom. The link graph that makes a wiki valuable — bi-directional
backlinks revealing how concepts connect — stops at the vault boundary.

This specification introduces **[[Webmention]]** ([[W3C]] REC,
2017-01-12) as the link-shaped federation primitive. Webmention is a
small protocol: when one site publishes a page that links to another
site, the linking site POSTs a `(source, target)` pair to the linked
site's [[Webmention Endpoint]]. The receiver fetches `source`,
verifies it actually contains a link to `target`, and records an
external backlink. Combined with [[SPEC-038-rss-support]], two zetl
vaults exchanging RSS *and* webmentions get a fully bidirectional
federation graph with no central server.

zetl is unusually well-positioned for webmention because the local
backlink graph (`src/graph.rs::backlinks`) is already a first-class
concept. An external backlink is structurally identical to an internal
one — the only differences are the source URL (off-vault) and the
trust posture (untrusted, requires verification + moderation). The
moderation surface aligns naturally with the existing [[SPL]]
defeasible-logic infrastructure ([[SPEC-005]]): moderation rules are
SPL rules; user accept/reject decisions become facts that update the
rule store; "why was this mention auto-accepted?" is answered by
`zetl reason explain`.

### 1.1 Motivation

- **The link graph already wants to extend across the federation
  boundary.** Every wiki author who publishes an externally-reachable
  page eventually wants to know who's linking to them. Webmention is
  the standardised, lightweight way to surface that.
- **Bidirectional federation with zero protocol weight.** Compared to
  [[ActivityPub]] — which is what the [[Fediverse]] uses for full
  federation — Webmention is two HTTP endpoints (POST receive, POST
  send) and a discovery convention (`rel=webmention`). For a wiki, this
  is the right tool: the wiki *is* the federated artefact, and the
  graph between wikis is what matters; we don't need profile streams,
  follows, or boosts.
- **The defeasible logic finally gets a use-case beyond static
  reasoning.** SPL's strength has been local reasoning over vault
  facts. Moderation is the natural place where rules express user
  judgement that evolves with use: "normally accept from already-linked
  domains" is exactly the kind of defeasible rule SPL was built for.
- **Static publishers can participate too.** Build-time webmention
  *send* (post-publish hook that POSTs to every external link's
  endpoint) works in `zetl build` natively. Webmention *receive* in
  static mode requires a hosted relay or a sidecar service — that's
  the primary trade-off [[#ADR-3902]] resolves.

### 1.2 Design Principles

1. **Receive is gated; send is automatic.** Every received webmention
   passes through verification, then through the defeasible-rule
   moderation gate, before any edge enters the link graph. Sending,
   in contrast, is an unattended post-publish action driven by the
   diff of external links between builds.
2. **Idempotency on both sides.** Receiver-side idempotency is
   spec-mandated: duplicate `(source, target)` tuples are treated as
   updates. Sender-side idempotency is built on top: a persisted log
   keyed on `(source, target, content-hash)` ensures rebuilds without
   changes produce zero outbound POSTs. See [[#REQ-3906]] and
   [[#ADR-3906]].
3. **Verification before recording.** A receiver MUST fetch the source
   page and confirm it contains a link to the target before accepting
   the mention. This is the critical anti-spam control and it is
   non-negotiable. See [[#REQ-3903]].
4. **Federation MUST NOT widen visibility.** A webmention to a private
   page MUST NOT reveal whether the page exists. The endpoint
   responses are oracle-resistant — an attacker probing for valid
   private URLs gets the same response shape as for public URLs. See
   [[#REQ-3909]] and [[#T8]] / [[#T9]] in [[#Threat Model]].
5. **Moderation is a defeasible-rule decision, not a hardcoded
   policy.** The bundled rule set is a starting point; vault owners
   override it by editing SPL in their vault. User accept/reject
   actions feed back into the rule store as facts. See [[#REQ-3905]]
   and [[#ADR-3903]].
6. **Static-build participation is a first-class scope decision, not
   an afterthought.** [[zetl build]] users SHOULD be able to send and
   receive webmentions; the receive path is the asymmetric one that
   [[#ADR-3902]] resolves.
7. **Threat surface inherits SPEC-038's shared infrastructure.** The
   SSRF-safe HTTP fetcher (introduced in [[SPEC-038-rss-support]] for
   inbound feed pulls) is reused for the source-page verification
   fetch. The HTML sanitiser ([[SPEC-034]]) is reused for any e-content
   surfaced in the moderation UI. SPEC-039 does not introduce new
   trust-boundary primitives; it composes existing ones.

### 1.3 Scope

**In scope (v1):**

- **Discovery** — every published page emits `rel=webmention` in both
  the HTTP `Link` header (when served live) and an HTML `<link>` tag
  (always, including static builds). See [[#REQ-3901]] and [[#CON-3902]].
- **Receive** — a `POST /webmention` endpoint in `zetl serve` (and
  optionally `zetl serve --collab`) that accepts `(source, target)`
  pairs, queues them for verification, fetches the source via the
  SSRF-safe shared fetcher, confirms the link, runs the moderation
  gate, and either records or queues. See [[#REQ-3902]]–[[#REQ-3905]].
- **Verification** — fetch source, parse HTML, search for an `<a
  href="…">`, `<link href="…">`, or other link to the target.
  Verification is a pure function of (HTML bytes, target URL). See
  [[#REQ-3903]].
- **Moderation** — defeasible-rule gate with the bundled rule set
  (auto-accept from already-linked domains; queue unknowns; deny from
  blocklist). User accept/reject actions update the rule store via SPL
  facts. See [[#REQ-3905]], [[#ADR-3903]], and [[#CON-3907]].
- **Send** — build-time and serve-time post-publish hook that
  identifies new and changed external links by diffing against the
  idempotency log, discovers each target's webmention endpoint, and
  POSTs. See [[#REQ-3906]] and [[#REQ-3907]].
- **Idempotency** — sender-side persisted log at
  `.zetl/webmentions/sent.jsonl` keyed on `(source, target, content-hash)`.
  See [[#ADR-3906]].
- **Storage** — external backlinks stored as edges in the link graph
  with explicit `external_source: Url` provenance, persisted at
  `.zetl/webmentions/external-edges.jsonl`. See [[#ADR-3904]].
- **Static-build participation** — `zetl build` participates in send
  and discovery natively; receive in static mode is per [[#ADR-3902]]
  (likely via a hosted relay like [[webmention.io]] with a build-time
  pull from its JSON API).
- **Spec conformance** — emit and receive per the [[W3C]] [[Webmention]]
  REC; pass the relevant subset of the [[webmention.rocks]] test
  corpus. See [[#NFR-3905]].
- **Observability** — counters per receive outcome, send outcome,
  moderation queue depth, log lines per decision. See [[#OBS-3901]]
  ff.

**Out of scope (v1):**

- **[[ActivityPub]] / Fediverse interop** — entirely separate spec.
  Webmention does not federate identity, follows, or replies; it only
  federates links. That is intentional and aligned with zetl's focus.
- **[[OPML]]-style export of mention sets** — nice-to-have; deferred.
- **Identity verification beyond URL existence** — [[#ADR-3905]]
  evaluates whether v1 includes [[h-card]] author extraction or stays
  URL-only. The default-leaning choice is URL-only for v1, with h-card
  as a Phase 2 extension once the [[Microformats2]] parser is
  vetted.
- **WebFinger / rel=me cross-verification** — useful for
  [[Mastodon]]-style identity proof but heavy for v1.
- **Fine-grained per-page moderation policy** — v1 ships vault-wide
  moderation rules. Per-page overrides are a v2 question.
- **Email / Slack notifications on queued mentions** — surfaced via
  observability log lines and the queue UI; push notifications are out
  of scope.
- **Vouch protocol (anti-spam transitive trust)** — interesting,
  unstandardised, deferred.
- **Salmon protocol or PubSubHubbub for push** — superseded by
  Webmention itself in the IndieWeb stack.
- **Bridges from social silos (e.g., [[Bridgy]] integration)** —
  bridge tools are independent; they POST to zetl's `/webmention`
  endpoint like any other client. No new code in zetl.

### 1.4 Risks and open questions (the plan resolves these)

- **Receive in static builds: relay vs sidecar vs unsupported.** The
  largest scope decision after flow-coverage. [[#ADR-3902]].
- **Identity verification level: URL-only vs h-card vs WebFinger.**
  Heavy UX consequence; [[#ADR-3905]].
- **Storage layout: per-page sidecar vs central JSONL vs SQLite.**
  Affects both performance and git-versioning ergonomics. [[#ADR-3904]].
- **Default moderation rule set.** The bundled rules are the new-user
  experience; getting them wrong means either spam floods or silent
  drops. [[#ADR-3903]] enumerates and chooses defaults.
- **Sender idempotency log retention.** How long do we remember every
  sent (source, target, hash)? Vault-lifetime? Per-release? [[#ADR-3906]].

---

## 2. User Profiles

> Full profiles live in `/users/wiki-author-receiver/user.md`,
> `/users/federation-peer/user.md`, `/users/moderator/user.md`. The
> strawman summarises them inline; full profiles are produced by
> [[DESIGN-039-webmention#task-user-profiles]].

### 2.1 UP-3901 Wiki Author Receiver

**Role:** Self-hoster running `zetl serve` (or `--collab`) on a public
vault who now wants to see who's linking to their pages.

**Goals:** receive incoming webmentions; integrate them into the
existing backlink panel; control spam without becoming a moderation
sink; understand *why* a particular mention was auto-accepted or
queued.

**Constraints (provisional):** intermediate CLI fluency; familiar with
the existing zetl backlink view; *not* assumed to know
[[Microformats2]] or the Webmention spec text.

**Daily workflow:** writes pages, deploys, occasionally checks the
moderation queue when a notification fires; trusts the bundled rules
for the common case.

### 2.2 UP-3902 Wiki Author Sender

**Role:** Same human as UP-3901, on the publish side.

**Goals:** when their page links externally, the relevant target site's
webmention endpoint gets pinged automatically without manual
intervention; rebuilds without changes don't spam targets; removed
links produce a one-shot "remove" send so the target's UI stays in
sync.

**Constraints:** assumes the build pipeline runs without their
attention; expects the idempotency log to "just work"; will be
unhappy if `zetl build` becomes noticeably slower on every build
because of webmention sends.

### 2.3 UP-3903 Federation Peer

**Role:** Another zetl vault (or any IndieWeb-shaped publisher) that
exchanges mentions with UP-3901's vault.

**Goals:** mutual recognition — when peer A links to peer B and vice
versa, both backlinks appear in both vaults' graphs without manual
intervention; the trust posture between known peers is "auto-accept"
once established.

**Constraints:** behaviour governed entirely by the spec; UP-3901's
rules apply on receipt.

### 2.4 UP-3904 Moderator

**Role:** Managing the moderation queue, allowlist/denylist, defeasible
rules. May be the same human as UP-3901 in solo cases.

**Goals:** keep spam out without losing legitimate mentions;
understand at a glance why each queued mention is queued; promote
trusted senders to the allowlist with one click; demote spammers to
the denylist with one click; tune the rule set when the defaults
don't match this vault's pattern.

**Constraints (provisional):** vault-administrator level CLI fluency;
*may* be willing to read SPL rule definitions but should not be
required to in normal operation.

---

## 3. Happy Paths

> Full happy paths live in `/users/wiki-author-receiver/happy-paths.md`
> and the corresponding files for the other profiles. Strawman lists
> titles only.

| ID | Title | Profile | Status |
|---|---|---|---|
| HP-3901 | First incoming mention from a stranger; user clicks accept | UP-3901 + UP-3904 | Provisional |
| HP-3902 | Outgoing mention sent on first publish of a new external link | UP-3902 | Provisional |
| HP-3903 | Rebuild with no changes sends zero outbound POSTs | UP-3902 | Provisional; idempotency probe |
| HP-3904 | External link removed; sender re-POSTs; receiver removes backlink | UP-3902 + UP-3901 | Provisional |
| HP-3905 | Federation peer's mention auto-accepts (already-linked rule) | UP-3903 + UP-3904 | Provisional; SPL probe |
| HP-3906 | Spammer POSTs source with no link to target; rejected at verify | UP-3901 (defender) | Provisional; threat-model probe |
| HP-3907 | Static-build vault participates as recipient via hosted relay | UP-3901 (static) | Provisional; contingent on [[#ADR-3902]] |
| HP-3908 | Webmention POST to a private page returns oracle-resistant response | UP-3901 (defender) | Provisional; [[#REQ-3909]] / [[#T9]] |
| HP-3909 | Capability-URL page receives a mention; capability NOT echoed in response | UP-3901 (defender) | Provisional; [[#T8]] |

---

## 4. Synthetic User Simulation

`[Provisional — produced by DESIGN-039 task-synthetic-user-run.
research/SPEC-039-synthetic-user.md will hold the full transcript and
findings.]`

---

## 5. Functional Requirements

> Numbering convention: REQ-3901, REQ-3902, ... — the prefix `39` binds
> artefact ids to SPEC-039, matching the [[SPEC-035]] / [[SPEC-038-rss-support]]
> convention. Requirements below are **provisional placeholders**; the
> draft-requirements task graduates them to grounded text.

### REQ-3901: Discovery — `rel=webmention` emission

The system SHALL emit a `rel=webmention` discovery target on every
published page, in both the HTTP `Link` response header (when served
live) AND an HTML `<link rel="webmention" href="...">` tag in the
document head (always, including static builds), FOR every page in
the vault that is reachable to anonymous readers, WITH the discovery
target resolving to the canonical webmention endpoint per
[[#CON-3902]].

Trace: [[#TEST-3901]], [[#TEST-3902]], [[#CON-3902]]

### REQ-3902: Receiver endpoint — POST /webmention

The system SHALL register a `POST /webmention` HTTP endpoint that
accepts `application/x-www-form-urlencoded` requests with `source` and
`target` parameters, returning `201 Created` on synchronous accept,
`202 Accepted` on async-queued, `400 Bad Request` on malformed input,
or oracle-resistant responses per [[#REQ-3909]] for sensitive targets,
WITHIN the timeout in [[#NFR-3902]], FOR every webmention sender per
the [[W3C]] REC.

Trace: [[#TEST-3903]], [[#TEST-3904]], [[#CON-3901]]

### REQ-3903: Verification before recording

The system SHALL fetch the `source` URL via the [[SSRF]]-safe shared
fetcher (inherited from [[SPEC-038-rss-support#REQ-3811]]), parse the
returned HTML, and confirm the presence of a link to `target` BEFORE
the mention enters the moderation gate or the link graph, FOR every
incoming POST, WITH verification being a pure function of (HTML bytes,
target URL).

Trace: [[#TEST-3905]] (positive), [[#TEST-3906]] (negative — link
absent), [[#TEST-3907]] (negative — link present in `<script>` only,
not honoured)

### REQ-3904: Async verification model

If the system implements async verification (returning `202 Accepted`
per [[#REQ-3902]]), the verification job SHALL complete within the
SLO defined in [[#NFR-3903]], OR the mention SHALL be marked
`expired-verification` and the sender SHALL be allowed to re-POST.

Trace: [[#TEST-3908]]

### REQ-3905: Moderation — defeasible-rule gate

Every verified mention SHALL pass through a defeasible-rule
moderation gate, evaluated against the bundled rule set
([[#ADR-3903]]) plus any vault-local SPL rules, producing a decision
∈ {`accept`, `queue`, `deny`} BEFORE the mention is recorded as an
edge in the link graph, FOR every verified mention, WITH the
decision and its proof tree being explainable via `zetl reason
explain`.

Trace: [[#TEST-3909]] .. [[#TEST-3911]], [[#CON-3907]]

### REQ-3906: Sender idempotency

The system SHALL maintain a persisted log at
`.zetl/webmentions/sent.jsonl` recording every successfully-sent
`(source, target, content-hash, sent-at, response)` tuple, AND on
every `zetl build` / `zetl serve` publish event SHALL POST only those
`(source, target)` pairs that are NEW or whose content-hash has
CHANGED, FOR every external link in the rendered output, WITH zero
POSTs sent when no input changed (verifiable via [[#OBS-3902]]).

Trace: [[#TEST-3912]] (idempotency property test), [[#ADR-3906]]

### REQ-3907: Send on link removal

When a previously-sent `(source, target)` pair is removed from the
vault, the system SHALL re-POST to the same target's webmention
endpoint exactly once, allowing the receiver to re-fetch and remove
the backlink per the [[W3C]] REC, FOR every removal detected against
the idempotency log.

Trace: [[#TEST-3913]]

### REQ-3908: Endpoint discovery for senders

The system SHALL discover each target's webmention endpoint via
`Link` header first, falling back to HTML `<link>` and `<a>` tags,
caching successful discoveries with TTL per [[#NFR-3904]], FOR every
external link the sender intends to POST.

Trace: [[#TEST-3914]] (header preference), [[#TEST-3915]] (fallback)

### REQ-3909: Oracle-resistant response for sensitive targets

The system SHALL return an indistinguishable response shape (status
code, headers, body length distribution) for `POST /webmention`
requests targeting (a) public published pages, (b) private pages, (c)
non-existent slugs, and (d) capability-protected URLs, SUCH THAT an
attacker cannot use the endpoint as a private-page existence oracle
or a capability-token enumeration oracle, FOR every POST.

Trace: [[#TEST-3916]] (oracle-resistance probe), [[#T8]], [[#T9]]

### REQ-3910: External backlink storage

Accepted mentions SHALL be persisted as edges in the link graph with
explicit `external_source: Url` provenance, persisted on disk per
[[#ADR-3904]], AND surfaced in the existing backlink view (web UI,
TUI, `zetl backlinks` CLI) with a visible distinction between
internal and external sources, FOR every accepted mention.

Trace: [[#TEST-3917]], [[#CON-3906]]

### REQ-3911: Update and remove semantics

When a previously-accepted mention's source is re-POSTed and
re-verification finds the link still present, the system SHALL
update the existing edge's `last-seen` timestamp without creating a
duplicate. When re-verification finds the link absent (because the
source removed it), the system SHALL remove the edge.

Trace: [[#TEST-3918]], [[#TEST-3919]]

### REQ-3912: Rate limiting on receiver endpoint

`POST /webmention` SHALL enforce a rate limit per source-host AND a
global concurrency cap per [[#NFR-3906]], rejecting excess requests
with `429 Too Many Requests`, FOR every POST.

Trace: [[#TEST-3920]]

### REQ-3913: Default moderation rule set

The system SHALL bundle a default rule set covering at minimum (a)
auto-accept from domains the vault has previously linked to
outward; (b) queue mentions from unknown domains; (c) deny mentions
from a configurable blocklist; AND SHALL allow vault administrators
to override or supplement these rules via SPL in their vault, FOR
every fresh installation.

Trace: [[#TEST-3921]], [[#ADR-3903]], [[#CON-3907]]

### REQ-3914: User-decision feedback into rule store

When a moderator accepts a queued mention from a previously-unknown
domain, the system SHALL record this decision as an SPL fact in the
vault's reasoning store, with the effect that subsequent mentions
from that domain are evaluated under the updated rule set (typically
auto-accepting them after the first manual accept), FOR every
accept/reject moderation action.

Trace: [[#TEST-3922]] (state-machine test for the feedback loop),
[[#ADR-3903]]

---

## 6. Non-Functional Requirements

### NFR-3901: Receiver endpoint latency (sync mode)

`POST /webmention` SHALL return ≤ 500 ms at the 95th percentile in
synchronous mode, UNDER ≤ 50 RPS load on commodity hardware, WITH
verification fetch counted toward the latency budget.

Trace: [[#OBS-3903]], [[#TEST-3923]] (load test)

### NFR-3902: Verification fetch timeout

The verification HTTP fetch SHALL time out at 10 s and SHALL bound
the response body to 1 MiB before parsing, FOR every fetch, regardless
of `Content-Length` header (which can lie).

Trace: [[#TEST-3924]]

### NFR-3903: Async verification SLO

If async verification is used, ≥ 99% of mentions SHALL complete
verification within 60 s of POST receipt, UNDER nominal load.

Trace: [[#OBS-3904]]

### NFR-3904: Endpoint discovery cache TTL

Successful endpoint discoveries SHALL be cached with TTL ≥ 7 days
and ≤ 30 days, configurable, FOR every target host, WITH cache
invalidation on confirmed `404 Not Found` from the cached endpoint
(meaning the target moved or removed support).

Trace: [[#TEST-3925]]

### NFR-3905: Standards conformance — webmention.rocks

The system SHALL pass the applicable subset of the
[[webmention.rocks]] test corpus (sender tests where send is in
scope; receiver tests where receive is in scope), with passing
fixtures enumerated and tracked in CI; failing fixtures MUST be
documented with explicit rationale.

Trace: [[#TEST-3926]] (CI fixture run)

### NFR-3906: Rate limit thresholds

The receiver endpoint SHALL apply at minimum: 60 POSTs / minute per
source-host; 1000 POSTs / minute global; concurrent verification
fetches ≤ 8.

Trace: [[#TEST-3920]]

### NFR-3907: Idempotency log size growth

The idempotency log SHALL be append-only with a documented
compaction strategy (retain only the most-recent tuple per
`(source, target)` pair on compaction), with compaction triggered
when the log exceeds a configurable size threshold (default 100 MiB).

Trace: [[#TEST-3927]]

### NFR-3908: Failure-mode observability

Per [[SPEC-038-rss-support#NFR-3807]] alignment: every distinct REQ
failure mode SHALL emit a structured log line at `warn` or `error`
naming the violated REQ, the offending identity (source URL, target
URL), and a remediation hint.

Trace: [[#OBS-3905]]

---

## 7. Architecture Decision Records

> ADR placeholders by id and topic. Plan tasks
> [[DESIGN-039-webmention#task-adr-flow-coverage]] through
> [[DESIGN-039-webmention#task-adr-idempotency]] populate them.

### ADR-3901: Flow coverage in v1

`[Provisional — refined by DESIGN-039 task-adr-flow-coverage]`

**Status:** proposed.
**Context:** Four flows possible (receive, send, discovery,
idempotency). Smallest defensible v1 versus full federation parity.
**Decision:** *(deferred to plan task; default-leaning toward all
four since they compose cleanly)*.

### ADR-3902: Static-build receive strategy

`[Provisional — refined by DESIGN-039 task-adr-static-build-receive;
conditional on ADR-3901 admitting receive]`

**Status:** proposed.
**Context:** `zetl build` has no live HTTP server and cannot host the
receive endpoint. Three options: (A) hosted relay
([[webmention.io]]); (B) sidecar service; (C) unsupported in build
mode.
**Decision:** *(deferred)*.

### ADR-3903: Moderation policy

`[Provisional — refined by DESIGN-039 task-adr-moderation-policy]`

**Status:** proposed (default-leaning toward defeasible-rule hybrid).
**Context:** Three options — auto-accept, full manual queue,
rule-based hybrid. Hybrid defeasible-rule approach maximises
zetl-specific synergy with [[SPEC-005]].
**Decision:** *(deferred)*.

### ADR-3904: Storage model

`[Provisional — refined by DESIGN-039 task-adr-storage-model]`

**Status:** proposed.
**Context:** Per-page sidecar files vs central JSONL vs embedded
SQLite vs hybrid. Affects git-versioning, query efficiency, and
cache-invalidation contract.
**Decision:** *(deferred)*.

### ADR-3905: Identity verification level

`[Provisional — refined by DESIGN-039 task-adr-identity-verification]`

**Status:** proposed (default-leaning toward URL-only for v1).
**Context:** URL-only vs [[h-card]] extraction vs [[WebFinger]]
cross-verification. Heavy UX consequence; h-card spoofing is a real
threat.
**Decision:** *(deferred)*.

### ADR-3906: Sender idempotency mechanism

`[Provisional — refined by DESIGN-039 task-adr-idempotency]`

**Status:** proposed (default-leaning toward persisted JSONL log).
**Context:** Persisted JSONL log vs hash-only filter vs
no-tracking-relying-on-receiver-dedup.
**Decision:** *(deferred)*.

---

## 8. Contracts

> Contracts are defined per-clause so each implemented [[#REQ-3901]]
> .. [[#REQ-3914]] maps to a distinct, named clause of pre/post/error
> conditions.

### CON-3901: HTTP — POST /webmention `[Provisional — refined by DESIGN-039 task-contracts]`
### CON-3902: Discovery emission contract `[Provisional]`
### CON-3903: CLI surface — `zetl webmention <subcommand>` and build-time integration `[Provisional]`
### CON-3904: Build output paths and rendered-HTML emission `[Provisional]`
### CON-3905: Configuration — `zetl.toml [webmention]` table `[Provisional]`
### CON-3906: On-disk format — `.zetl/webmentions/*` `[Provisional]`
### CON-3907: SPL — moderation predicates and rule types `[Provisional]`
### CON-3908: Moderation queue API — list / accept / reject `[Provisional]`

---

## 9. Purity Boundary Map

> Authored in detail by [[DESIGN-039-webmention#task-purity-boundary-map]].

### Pure Core (no I/O, no shared state, deterministic)

- `webmention::verify` — given source HTML bytes + target URL,
  determine whether the source contains a link to the target.
  Deterministic, pure.
- `webmention::moderate` — given a verified mention + the SPL rule
  set + the vault's claims, produce a decision ∈ {accept, queue,
  deny}. Pure projection over the reasoning store.
- `webmention::idempotency_diff` — given current send-state and
  previous send-state, produce the set of `(source, target)` pairs
  to POST. Pure.
- `webmention::extract_external_links` — given rendered HTML +
  vault base URL, produce the list of external `(source-page-url,
  target-url)` pairs.
- `webmention::discover_endpoint` — given an HTTP response (headers
  + body bytes), produce the discovered endpoint URL.
  Header-preference, deterministic.
- *(if [[#ADR-3905]] = h-card)* `webmention::extract_hcard` — pure
  parser from source HTML to author identity record.

### Effectful Shell

- `webmention::receive` — the Axum handler for POST /webmention;
  validates, queues, calls pure core after fetching source.
- `webmention::fetch_source` — HTTP client (reuses
  [[SPEC-038-rss-support]]'s SSRF-safe fetcher).
- `webmention::send` — outbound HTTP POSTs; updates idempotency log
  on success.
- `webmention::persist` — writes accepted edges to
  `.zetl/webmentions/external-edges.jsonl`; writes sent log to
  `sent.jsonl`.
- `webmention::moderation_ui` — collab-mode admin panel showing the
  queue.

### Boundary Contracts

- `IncomingMention` (shell → core): the validated request prior to
  verification.
- `VerifiedMention` (core → shell): post-verification, pre-moderation.
- `ModerationDecision` (core → shell): the decision + proof tree.
- `OutboundMention` (core → shell): the (source, target, hash) tuple
  awaiting POST.

### Dependency Rule

Dependencies point inward: shell → core. Pure modules MUST NOT
import `tokio`, `axum`, HTTP clients, `git2`, or fs write APIs.

### Enforcement

Module visibility (`pub(crate)` on shell-side functions only),
arch-lint rule in CI prohibiting `use` from `webmention::shell` inside
`webmention::core`, and code review.

### Shared with [[SPEC-038-rss-support]]

The [[SSRF]]-safe HTTP fetcher and the HTML sanitiser are SHARED
modules introduced by SPEC-038 for inbound feed pulls. SPEC-039
depends on those modules and does not re-implement them. If
SPEC-038's inbound direction is deferred, the shared fetcher is
introduced under SPEC-039 and SPEC-038 inherits it on its later
revival.

---

## 10. Verification Strategy

| Technique | Target |
|---|---|
| Example-based testing | every [[#REQ-3901]] .. [[#REQ-3914]], decomposed into positive / negative-input / negative-output |
| Standards-conformance — [[webmention.rocks]] | the full applicable test corpus (sender + receiver fixtures); CI gate per [[#NFR-3905]] |
| Property-based testing | idempotency-diff convergence (build twice, second build produces zero POSTs); moderation-rule determinism (same input + same rules → same decision); endpoint-discovery commutativity (Link header takes precedence over HTML link tag); verification idempotence (fetch + verify is a pure function) |
| State-machine testing | moderation queue transitions (queued → accepted → recorded; queued → rejected → tombstoned); user-decision feedback loop |
| Fuzzing | receiver endpoint with malformed POST bodies, oversized payloads, malicious source URLs (SSRF probes); HTML parser edge cases on source fetch (deeply nested, unicode-bomb, billion-laughs) |
| Mutation testing | verification logic specifically (kill rate ≥ 90%); moderation logic (≥ 80%) |
| Adversarial testing | per [[DESIGN-039-webmention#task-adversarial-tests]] — verification-bypass attempts, idempotency-log corruption, SPL-injection in source URLs, capability-URL probing variants, oracle probing variants |

---

## 11. Threat Model

| ID | Threat | Surface | Mitigation (provisional) |
|---|---|---|---|
| T1 | [[SSRF]] on source-fetch verification | receive | Shared SSRF-safe fetcher inherited from [[SPEC-038-rss-support#REQ-3811]] |
| T2 | Spam volume / DoS on `/webmention` | receive | Rate-limit per source-host + global cap ([[#NFR-3906]]); denylist; queue cap |
| T3 | HTML-parser exploit on source page | receive | Same parser as [[SPEC-038-rss-support]]'s rewrite path; size cap per [[#NFR-3902]]; entity-expansion bound |
| T4 | [[Microformats2]] parser exploit (if [[#ADR-3905]] admits h-card) | receive | Sanitiser allowlist on every h-card field; size cap; per-task survey of parser fuzzing posture |
| T5 | h-card identity spoofing | receive (display) | v1: render source URL alongside h-card name so users see actual origin; v2 ([[ADR-3905]] = WebFinger): rel=me cross-verification |
| T6 | Retroactive edit (source removes link after accept) | receive | Per-spec re-verification; documented re-verify cadence; tombstone the edge on confirmed removal ([[#REQ-3911]]) |
| T7 | Replay attacks (re-POST old verified mention to bump visibility) | receive | (source, target) is the dedup key; first-seen timestamp pinned; receiver collapses duplicates |
| T8 | Capability-URL token enumeration via webmention probe | receive | Oracle-resistant response shape ([[#REQ-3909]]); never echo capability tokens in responses or queue |
| T9 | Private-page existence oracle | receive | Same indistinguishable response shape ([[#REQ-3909]]); 200 always or 401 always, never 404 distinguishably |
| T10 | Outgoing-mention privacy leak (sender reveals what they read) | send | Sender-side suppression for capability-mode pages and private vaults; configurable opt-out per page |
| T11 | Idempotency-log corruption on partial writes | send | Atomic append (write to tempfile + fsync + rename); torn-write detection on log read |
| T12 | SPL term-injection via source URL into moderation rules | moderation | URL is opaque to the SPL term language; rules quote URLs as strings, never as bare terms |

---

## 12. Observability

### OBS-3901: Receive outcome counter

Counter `zetl_webmention_receive_total{result}` incremented per POST,
with `result` ∈ {`accepted`, `queued`, `rejected_unverified`,
`rejected_blocklist`, `rejected_rate_limit`, `error`}.

### OBS-3902: Send outcome counter

Counter `zetl_webmention_send_total{result}` incremented per POST,
with `result` ∈ {`ack_201`, `ack_202`, `ack_200`, `reject_4xx`,
`error_5xx`, `network_error`, `endpoint_not_found`}.

### OBS-3903: Receiver endpoint latency histogram

Histogram `zetl_webmention_receive_duration_seconds`, bucketed for
the [[#NFR-3901]] target.

### OBS-3904: Async verification age gauge

Gauge `zetl_webmention_pending_verification_max_age_seconds` for the
[[#NFR-3903]] SLO.

### OBS-3905: Moderation queue depth gauge

Gauge `zetl_webmention_queue_depth{queue}` with `queue` ∈
{`pending_verification`, `pending_moderation`, `denied_recent`}.

### OBS-3906: Failure-mode log lines

Per [[#NFR-3908]]: structured warn/error log lines naming the violated
REQ, source URL, target URL, and remediation hint.

---

## 13. Traceability

| Artefact | Implements | Verified by |
|---|---|---|
| [[#REQ-3901]] | UP-3901 / UP-3903 (federation peers can discover) | TEST-3901, TEST-3902 |
| [[#REQ-3902]] | W3C REC normative `MUST` for receivers | TEST-3903, TEST-3904; webmention.rocks receiver corpus |
| [[#REQ-3903]] | T1, T6 mitigations | TEST-3905, TEST-3906, TEST-3907 |
| [[#REQ-3904]] | NFR-3903 SLO | TEST-3908 |
| [[#REQ-3905]] | UP-3904 daily workflow | TEST-3909..TEST-3911 |
| [[#REQ-3906]] | UP-3902 expectation; T11 mitigation | TEST-3912 |
| [[#REQ-3907]] | T6 mitigation; W3C REC delete semantics | TEST-3913 |
| [[#REQ-3908]] | UP-3902 endpoint discovery | TEST-3914, TEST-3915 |
| [[#REQ-3909]] | T8, T9 mitigations | TEST-3916 |
| [[#REQ-3910]] | UP-3901 backlink view | TEST-3917 |
| [[#REQ-3911]] | W3C REC update/delete; T6 mitigation | TEST-3918, TEST-3919 |
| [[#REQ-3912]] | T2 mitigation | TEST-3920 |
| [[#REQ-3913]] | UP-3904 default experience | TEST-3921 |
| [[#REQ-3914]] | UP-3904 feedback loop | TEST-3922 |

The full traceability table — including [[CON-3901]] .. [[CON-3908]],
[[OBS-3901]] .. [[OBS-3906]], and bug-back-links once any are filed —
is produced and validated by
[[DESIGN-039-webmention#task-phase2-quality-gates]]. Orphan REQs / CONs
/ OBSs at gate time fail the gate.

---

## 14. Quality Gates

### 14.1 USDD Phase 1 quality gates

- [ ] All requirements unambiguous — `[Provisional — produced by DESIGN-039 task-phase1-quality-gates]`
- [ ] All requirements verifiable
- [ ] All requirements atomic
- [ ] No internal conflicts
- [ ] Ambiguities resolved with measurable criteria

### 14.2 USDD Phase 2 quality gates

- [ ] Adheres to constitutional principles (1–13) — `[Provisional — produced by DESIGN-039 task-phase2-quality-gates]`
- [ ] All components have single responsibility (per [[#Purity Boundary Map]])
- [ ] All functionality exposed via well-defined interfaces (per [[#Contracts]])
- [ ] Tests derived from requirements (REQ → TEST traceability complete)
- [ ] Security controls specified with verifiable criteria (per [[#Threat Model]])
- [ ] Observability requirements captured

---

## 15. Convergence and Status Lifecycle

This document is `0.1.0-strawman` until [[DESIGN-039-webmention]]
runs. The plan promotes it through:

| Version | Trigger |
|---|---|
| `0.1.0-strawman` | this initial pass |
| `0.2.0` (status `draft`) | after Phase 1 + Phase 2 quality gates pass |
| `0.3.0-rc` | after cross-model review and adversary exhaustion |
| `1.0.0` (status `approved`) | after a human reviewer signs off |
| `implementing` | when [[IMPL-039-webmention]] begins (Phase 3) |
| `implemented` | when [[IMPL-039-webmention]]'s Phase 3 quality gates pass |

---

## 16. Open questions for human review

1. **Flow coverage in v1 — all four, or subset?** [[#ADR-3901]].
2. **Static-build receive — relay vs sidecar vs unsupported?** [[#ADR-3902]].
3. **Identity verification — URL-only vs h-card vs WebFinger?** [[#ADR-3905]].
4. **Moderation default rule set — what's bundled?** [[#ADR-3903]] +
   plan task-survey-defeasible-touchpoints output.
5. **Storage model — per-page vs central vs SQLite vs hybrid?** [[#ADR-3904]].
6. **Idempotency log retention and compaction policy.** [[#ADR-3906]].
7. **Capability-URL interaction — does T8's oracle-resistance hold
   under [[SPEC-034]]'s capability URL pattern?** Cross-spec
   verification required during cross-model review.

---

**END OF STRAWMAN.** The next document state (`0.2.0` draft) is
produced by executing [[DESIGN-039-webmention]].
