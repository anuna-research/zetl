---
title: "SPEC-036: SPAKE2 Onboarding Flow for `zetl --collab`"
version: 0.1.0-strawman
status: draft
date: 2026-04-30
audience: agent, human
parent: SPEC-020
related:
  - SPEC-020  # Multi-user collaborative editing
  - SPEC-035  # Collaborative static asset uploads
  - SPEC-018  # cap pair / pubkey handoff (reference SPAKE2 use-site)
plan: DESIGN-036-spake2-onboarding
---

# SPEC-036: SPAKE2 Onboarding Flow for `zetl --collab`

> **Strawman notice.** This document is a first-pass produced *before* the
> Phase 0 surveys and synthetic-user simulations called for by
> `plans/DESIGN-036-spake2-onboarding.spl`. Sections labelled
> **`[Provisional — refined by DESIGN-036 task X]`** are deliberate
> placeholders that the plan tasks will replace with grounded findings.
> Do not implement against this version. The version reaches `0.1.0` only
> after Phase 1 + Phase 2 quality gates pass and the human-expert review
> package is approved (per USDD §AI Trust Boundaries — cryptography is a
> no-go area requiring explicit human approval before implementation).

## Information Table

| Field          | Value                                                                       |
| -------------- | --------------------------------------------------------------------------- |
| Document ID    | SPEC-036                                                                    |
| Title          | SPAKE2 Onboarding Flow for `zetl --collab`                                  |
| Version        | 0.1.0-strawman                                                              |
| Status         | Draft (strawman; pending DESIGN-036 execution)                              |
| Author         | Agent (Claude Opus 4.7, USDD Protocol v1.3.0)                               |
| Date           | 2026-04-30                                                                  |
| Audience       | Agent, Human                                                                |
| Trace          | USDD Agent Protocol v1.3.0 §Phase 1, §Phase 2, §AI Trust Boundaries         |
| Parent         | SPEC-020 Multi-User Collaborative Editing                                   |
| Related        | SPEC-035 Asset Uploads; SPEC-018 (cap pair, prior SPAKE2 use-site)          |
| Plan           | `plans/DESIGN-036-spake2-onboarding.spl`                                    |
| Review tier    | Tier 1 (security-sensitive; cryptography + auth core)                       |

---

## 1. Overview

`zetl --collab` enables multi-user collaborative editing on a self-hosted vault.
The current onboarding ceremony, implemented in `src/user/invite.rs`, uses a
**JWT-in-URL bearer model**: the vault owner runs `zetl invite`, the server
issues a signed JWT containing scope claims and a nonce, and the URL —
including the JWT — is shared out-of-band (Slack, email, etc.) for the
collaborator to click.

This model is operationally simple and serves asynchronous onboarding
(invites that survive in inboxes for days), but it has structural costs:

- **The URL is a bearer token.** Anyone who intercepts it during transit, has
  it screen-shared, finds it in browser history, or extracts it from a
  forwarded message holds the auth.
- **Scope is asserted rather than bound.** The JWT carries scope claims
  signed by the server, but the claim is not cryptographically bound to the
  collaborator's act of redemption beyond the bearer-token property of the
  URL.
- **The mental model is foreign to small-team collaboration ceremonies.**
  Teams onboard via "tell me your username" or "I sent you an invite" or
  spoken-phrase pairing — not by passing tokens.
- **The URL-as-bearer pattern fights the synchronous case.** When the owner
  and collaborator are *already in a private channel together right now*
  (Slack DM, in-person, voice), the natural ceremony is "say a code" — not
  "I'll send you a link."

`zetl` already implements a SPAKE2-based pairing primitive in
`src/cap/pair.rs` (using magic-wormhole's `spake2` crate over Ed25519,
4-word BIP39 phrases ≈ 44 bits of entropy, single-use nonce store, HMAC
verification of the handoff). It is currently scoped to `zetl cap pair`
(pubkey handoff between two terminals) and is **not** wired into the
`--collab` invitation flow.

This specification defines the SPAKE2 onboarding flow for `--collab`,
extending the existing `cap::pair` primitive into a generalised pairing
service and selecting the correct OOB-vs-URL split, scope-binding mechanism,
and synchronous/asynchronous coverage strategy.

### 1.1 Motivation

- **Reduce the channel-confidentiality requirement.** Under SPAKE2, the
  phrase carries auth entropy; if the OOB channel is private *for one
  moment*, the auth is sound — even if the URL is later screen-shared.
- **Bind scope cryptographically.** Including scope in the SPAKE2
  domain-separation identity and HKDF info parameter makes a phrase
  issued for `scope=foo/` unusable to derive a session for `scope=bar/`.
- **Match the small-team collaboration ceremony.** Most `--collab`
  deployments are between people who already share a channel; "say a
  code right now" is a one-step UX.
- **Reuse existing crypto primitives.** `cap::pair` is implemented,
  audited at the protocol level by upstream (magic-wormhole), and gives
  us SPAKE2 + HKDF + HMAC + nonce store with no net-new cryptographic
  surface area beyond the integration glue.

### 1.2 Design Principles

1. **The phrase MUST NOT travel in the URL.** Putting the phrase in the URL
   defeats SPAKE2 — the URL becomes the bearer, with the entropy of the
   phrase. The phrase travels OOB; the URL (if any) is a generic landing
   page or carries only a session ID and the inviter's SPAKE2 commitment.
2. **Synchronous and asynchronous onboarding are different ceremonies.**
   The synchronous case (live-channel pairing) is served by SPAKE2 +
   spoken phrase. The asynchronous case (inbox-survivable invite) is
   served by a signed-bearer artefact with bound scope and short TTL —
   structurally distinct from the SPAKE2 path. They MAY coexist; the
   ADRs decide whether they MUST.
3. **Scope is bound, not asserted.** The collaborator's derived session
   key MUST be unusable for any scope other than the one the inviter
   chose, even against an attacker who can reorder, replay, or alter
   protocol messages.
4. **No new cryptographic constants without justification.** Every
   identity string, HKDF info label, phrase length, and TTL inherits
   from `cap::pair` defaults unless the threat model justifies a new
   value. New values get a fresh domain-separation tag (e.g.,
   `b"zetl/collab-pair/v1"` instead of `b"zetl/cap-pair/v1"`).
5. **Failure UX MUST NOT leak protocol-level distinctions.** A user
   typing the wrong phrase, an expired session, and a replay attempt
   all produce the same user-visible error — only the operator log
   distinguishes them.
6. **Reuse, don't reinvent.** Where `cap::pair` primitives generalise
   cleanly, extend them. Where they don't, document the divergence
   and create a parallel module rather than overload the existing one.

### 1.3 Scope

**In scope:**

- A new pairing service in the `zetl --collab` server that runs SPAKE2
  sessions on demand.
- A new CLI command `zetl collab join` for collaborators to enter the
  phrase and complete pairing.
- An extension to `zetl invite` that issues SPAKE2 phrases (alongside
  or instead of JWT-URL invitations).
- Cryptographic binding of scope (folder/page/role) to the SPAKE2
  identity string and HKDF info parameter.
- A nonce store extension (or parallel store) for collab-pairing
  sessions, isolated from the existing `cap::pair` store.
- Threat model and OBS instrumentation per USDD §Security by Design and
  §Observability Requirement.

**Out of scope:**

- Federated identity / SSO integration (deferred to a successor spec
  if user research surfaces sustained demand — see DESIGN-036 prior-art
  finding on the SSO-vs-self-hosted trade-off).
- Replacement of the existing JWT-URL invitation flow before the
  asynchronous-path ADR has decided coexistence-vs-supersede.
- Changes to `acl.rs` semantics beyond the surface required to consume
  scope-bound sessions.
- A web UI for joining (the CLI is the primary surface; a future web
  spec may add a browser-side `/join` page that drives the same protocol).
- Multi-party simultaneous pairing (one inviter, one invitee per session).

---

## 2. User Profiles

> **`[Provisional — refined by DESIGN-036 task `user-profiles`]`** Profiles
> are sketched here from riff conclusions; the plan task produces the
> grounded version after surveying current `--collab` adopters.

### 2.1 Vault Owner (carries from SPEC-020)

Self-hosts `zetl serve --collab` on personal infrastructure. Generates
invitations for collaborators they already know and trust at a
relationship level. CLI-fluent. Has at least one private real-time
channel (Slack DM, Signal, in-person) with each collaborator they
intend to invite. Wants to grant minimal-necessary scope ("just this
folder") rather than full-vault access.

### 2.2 Synchronous Collaborator

Joining a vault while in the same private real-time channel as the
owner. Examples: paired-programming session over Zoom; office
in-person; ongoing Slack DM. Comfortable with CLI and copying a
4-word phrase ≤ 30 seconds. Expects to complete onboarding in one
contiguous moment, not over hours.

### 2.3 Asynchronous Collaborator

Receiving an invite that must sit in their inbox for hours or days
before redemption (timezone difference, batching of work, weekend).
The owner cannot remain online to drive a SPAKE2 session at the moment
of redemption. Comfortable with email and CLI; less comfortable with
synchronous ceremonies.

### 2.4 Operator / Administrator

Manages multiple `--collab` vaults (same individual or a small ops team).
Concerns: revoking access at scale, rotating server keys, auditing who
joined when. May be the same human as 2.1 in solo deployments; treated
as a distinct profile because the goals diverge.

---

## 3. Happy Paths

> **`[Provisional — refined by DESIGN-036 task `happy-paths`]`**
> Sketched from riff conclusions. The plan task produces enumerated
> failure modes and the synthetic-user run.

### 3.1 HP1: Synchronous Pairing (SPAKE2)

**Preconditions:** Owner runs `zetl serve --collab` on a vault. Owner
and Collaborator are in a private real-time channel together (Slack DM,
voice call, in-person).

**Steps:**

1. Owner runs `zetl invite --scope shared/ --pair --as alice` and
   the CLI emits: a 4-word phrase (e.g., `purple-sausage-melon-vault`)
   and a `/join` URL (generic — no phrase in it).
2. Owner says or types the phrase to Collaborator over the OOB channel.
3. Collaborator runs `zetl collab join <url>` — the CLI prompts for
   the phrase.
4. Collaborator types the phrase.
5. Both processes complete the SPAKE2 handshake; the server confirms
   via HMAC; the collaborator receives a scope-bound session credential
   and is shown the scope ("you can read+write `shared/`").

**Postconditions:** Collaborator can run `zetl serve --collab` (as a
client) or `zetl pull` against the vault, scoped to `shared/`. The
phrase has been consumed; replay yields the same generic error as a
wrong-phrase entry.

**Failure modes (enumerated by DESIGN-036 task `happy-paths`):**

- Wrong phrase typed → generic error; Owner generates a new phrase.
- Phrase expired before redemption → generic error; same recovery.
- Network interruption mid-handshake → safe to retry; nonce store
  ensures no double-redeem.
- Channel intercept (attacker shoulder-surfs the phrase) → SPAKE2's
  online single-guess property bounds the attacker to one attempt
  per server-session (see threat model §10.B).
- Server unreachable → CLI clearly distinguishes "server down" from
  "auth failed."

### 3.2 HP2: Asynchronous Onboarding (SPAKE2 vs JWT-URL — open)

**`[Provisional — DESIGN-036 task `adr-sync-vs-async` decides]`**

The riff and this strawman both flag that pure SPAKE2 *cannot* serve
the inbox-survivable case without a stand-in for the live OOB channel.
Two candidate shapes:

- **A: JWT-URL stays as the async path.** Coexists with SPAKE2 for the
  synchronous path. Documented threat model: URL is bearer; channel
  confidentiality of email = auth confidentiality.
- **B: Async-friendly SPAKE2 with a stored phrase.** The phrase is
  emitted alongside an inbox-survivable artefact (e.g., a sealed file
  the recipient decrypts with a separate password); equivalent to
  splitting the auth across two channels at rest.

ADR-### `adr-sync-vs-async` decides; this strawman defaults to **A**
(coexistence) pending the prior-art finding from
DESIGN-036 task `prior-art-research`.

### 3.3 HP3: Revocation

**Preconditions:** Owner has previously granted Collaborator scope `X`.

**Steps:**

1. Owner runs `zetl revoke --user collaborator-id --reason "left-team"`.
2. Server invalidates the collaborator's session credential and writes
   a tombstone to the revocation log.
3. Next request from Collaborator returns a generic-auth-failure error.
4. Optional: Collaborator's CLI surface notes "session ended; contact
   the owner to re-pair."

**Postconditions:** Collaborator cannot read or write any scope of the
vault. Tombstone is auditable.

### 3.4 HP4: Scope Narrowing After Grant

**`[Provisional — refined by DESIGN-036 task `happy-paths`]`** Whether
this is supported as a server-side scope reduction (downgrade existing
session) or only via revoke + re-pair. The trade-off is operator
ergonomics vs cryptographic-binding cleanliness.

---

## 4. Functional Requirements

> Numbering inherits from the latest existing spec; final IDs assigned
> by DESIGN-036 task `draft-requirements` after surveying the highest
> existing REQ-### in `specs/`.

### REQ-400: SPAKE2 Synchronous Pairing

The system SHALL provide a SPAKE2-based synchronous pairing ceremony
between Vault Owner and Synchronous Collaborator (User §2.1, §2.2),
using a 4-word BIP39 phrase as the SPAKE2 password, completable in
≤ 60 seconds at the 95th percentile WHEN both parties are online and
in a shared private real-time channel.

**Trace:** TEST-400, CON-400, OBS-400; HP1 §3.1.

### REQ-401: Phrase OOB-Only Transmission

The system SHALL ensure the SPAKE2 phrase is transmitted ONLY over an
out-of-band channel chosen by the Vault Owner. The phrase MUST NOT
appear in any URL, environment variable, command-line argument visible
to other processes, or any HTTP/WebSocket message issued by the server
or accepted from the client. The CLI MUST prompt for the phrase via
TTY input.

**Trace:** TEST-401, CON-401; Threat model §10.B.

### REQ-402: Scope Cryptographic Binding

The system SHALL bind the requested scope (folder/page/role) to the
SPAKE2-derived session key such that a phrase issued for scope `X`
cannot be used to derive a valid session for scope `Y ≠ X`, EVEN
against an attacker capable of altering, reordering, or replaying any
protocol message.

**Trace:** TEST-402, CON-402, ADR-401; Threat model §10.E.

### REQ-403: Single-Use Phrase Semantics

The system SHALL ensure each phrase is consumable at most once. A
second redemption attempt — by any party — MUST receive a generic
authentication-failure response indistinguishable from a wrong-phrase
attempt at the user-visible layer.

**Trace:** TEST-403, OBS-401.

### REQ-404: Phrase Expiry

The system SHALL expire unredeemed phrases after a default TTL of
**`[Provisional: 5 minutes]`** — refined by DESIGN-036 task
`adr-pake-vs-bearer`. Owner MAY extend or shorten the TTL via
`--ttl <duration>` flag bounded to **`[Provisional: 30s ≤ TTL ≤ 1h]`**.

**Trace:** TEST-404, CON-400.

### REQ-405: Owner-Initiated Revocation

The system SHALL allow the Vault Owner to revoke a previously-issued
session credential at any time, with revocation propagating to all
server-handled requests within **`[Provisional: 60 seconds]`** of the
revoke command's successful completion.

**Trace:** TEST-405, CON-403, NFR-402; HP3 §3.3.

### REQ-406: Failure-Message Indistinguishability

The system SHALL emit identical user-visible error text for the
distinct internal causes: wrong phrase, expired phrase, replayed
phrase, scope-mismatched phrase, server-unreachable, and rate-limited.
Only operator-channel logs (file or stderr at `--verbose`) distinguish
the underlying cause.

**Trace:** TEST-406, OBS-402; Design principle §1.2.5.

### REQ-407: Coexistence with JWT-URL Invitations

The system SHALL preserve the existing JWT-URL invitation flow
(`src/user/invite.rs`) at least until ADR-### `adr-sync-vs-async`
decides supersede vs coexist. During coexistence, both paths MUST use
disjoint nonce stores and disjoint server-side state to prevent
cross-protocol confusion.

**Trace:** TEST-407, ADR-402; §1.3 Out of Scope.

### REQ-408: Audit Trail

The system SHALL record every pairing attempt — successful, failed,
expired, or replayed — to the operator log with: timestamp, server
session ID, scope requested, outcome category, and (on success) the
collaborator's pubkey. The operator log MUST NOT contain the phrase
itself nor any value derivable from it.

**Trace:** TEST-408, OBS-403; USDD §Observability Requirement.

---

## 5. Non-Functional Requirements

### NFR-400: Phrase Entropy

Phrase entropy SHALL be ≥ 44 bits (matches `cap::pair` 4-word BIP39
default) UNDER any phrase generation path WITH 100% conformance
(no shorter phrases ever produced).

**Trace:** TEST-NFR-400; ADR-### scope-binding informs whether a
higher floor is justified.

### NFR-401: Onboarding Latency

End-to-end pairing time (Step 3 to Step 5 in HP1) SHALL be ≤ 5 seconds
UNDER nominal LAN conditions WITH 95th percentile, AND ≤ 15 seconds
UNDER WAN conditions WITH 95th percentile.

**Trace:** TEST-NFR-401, OBS-404.

### NFR-402: Revocation Propagation

Revocation propagation latency SHALL be ≤ 60 seconds UNDER nominal
server load WITH 99th percentile.

**Trace:** TEST-NFR-402, REQ-405.

### NFR-403: Failure-Message Indistinguishability — Timing

The user-visible response time for the six failure causes in REQ-406
SHALL be statistically indistinguishable to within **`[Provisional:
50ms 95th-percentile delta]`** UNDER LAN conditions, to prevent
side-channel oracle.

**Trace:** TEST-NFR-403, REQ-406.

---

## 6. Architecture Decision Records

> ADRs sketched as positions, not decided. DESIGN-036 plan tasks
> finalise each.

### ADR-400: Pairing Shape — SPAKE2 + OOB Phrase + Generic URL

**`[Provisional — refined by DESIGN-036 task `adr-pake-vs-bearer`]`**

**Status:** Proposed (strawman default)

**Context:** Three candidate shapes for the synchronous-onboarding
ceremony: (A) SPAKE2 + OOB phrase + generic `/join` URL
(Magic-Wormhole-shape); (B) SPAKE2 + URL-carrying-session-id +
OOB phrase (split-shape); (C) JWT-bearer-URL (current model).

**Decision:** (A). Phrase OOB; URL is a generic landing pointing at
the server's `/join` endpoint with no per-pairing data.

**Consequences:**

- (+) Strongest separation of channels; URL leak does not compromise
  auth.
- (+) Reuses `cap::pair` primitives directly.
- (−) Asynchronous onboarding is not served by this shape — see
  ADR-402.
- (−) Slightly more steps in the synchronous case (collaborator runs
  the join command and types the phrase, vs. clicking a link).

### ADR-401: Scope Binding via SPAKE2 Identity + HKDF Info

**`[Provisional — refined by DESIGN-036 task `adr-scope-binding`]`**

**Status:** Proposed (strawman default — defence in depth)

**Context:** Scope must be cryptographically bound (REQ-402). Three
mechanisms: (i) include scope in SPAKE2 `Identity` string; (ii)
include scope in HKDF `info` parameter for session-key derivation;
(iii) sign scope into a separate post-pairing token.

**Decision:** (i) + (ii) (defence in depth). The SPAKE2 Identity
becomes `b"zetl/collab-pair/v1|" || scope_canonical` and the HKDF
info parameter includes the same scope token. (iii) deferred unless
threat-model review surfaces a need.

**Consequences:**

- (+) Scope mismatch fails at the SPAKE2 layer (Identity mismatch
  prevents key agreement).
- (+) Defence in depth via HKDF info — even if Identity reuse is
  discovered, derived keys differ.
- (−) Scope must be canonicalised consistently on both sides. A
  separate test (TEST-402.canonicalisation) covers this.
- (−) Changing scope post-grant requires re-pairing (or a separate
  token-narrowing mechanism deferred to a successor spec).

### ADR-402: Synchronous + Asynchronous Coexistence

**`[Provisional — refined by DESIGN-036 task `adr-sync-vs-async`]`**

**Status:** Proposed (strawman default — coexist)

**Context:** Pure SPAKE2 cannot serve the asynchronous-onboarding case
where the owner is offline at the moment of redemption (Profile §2.3,
HP2). The existing JWT-URL flow does serve it.

**Decision:** Coexist. SPAKE2 is the recommended path for synchronous
pairing (Profile §2.2). JWT-URL remains for asynchronous onboarding
(Profile §2.3) with no semantic change in this spec. Both share a
common server-side ACL backend but disjoint nonce stores and disjoint
on-disk state.

**Consequences:**

- (+) Covers both user profiles without forcing a one-size-fits-all
  ceremony.
- (+) Migration path for existing users is non-breaking.
- (−) Two protocols to maintain and document.
- (−) Operator must understand which model is in play in any given
  deployment.

---

## 7. Contracts

> CON entries sketched. DESIGN-036 task `contracts` finalises each
> with full pre/post-condition tables and error-model enumeration.

### CON-400: CLI — `zetl invite --pair`

**Endpoint:** `zetl invite --scope <path> --pair [--ttl <duration>] [--as <user>]`

**Pre-conditions:** Vault initialised with `--collab`; user identified
by `--as` exists and is the vault owner OR has invite permission.

**Post-conditions:** stdout emits a 4-word phrase and a generic `/join`
URL on separate lines, with the phrase clearly labelled. Server-side
state records a pending pairing session keyed by a fresh nonce.

**Error model:** non-zero exit + stderr message for: vault not in
collab mode, user lacks permission, scope path does not exist,
TTL out of bounds.

**Implements:** REQ-400, REQ-401, REQ-404. **Verified by:** TEST-400.

### CON-401: CLI — `zetl collab join`

**Endpoint:** `zetl collab join <url>` then TTY prompt for phrase.

**Pre-conditions:** URL points at a `/join` endpoint of a reachable
collab server.

**Post-conditions:** On success, writes a session credential to
collaborator's `~/.config/zetl/sessions/<vault-fingerprint>` (mode
0600) and prints scope summary to stderr. On any failure, writes
nothing and prints a generic auth-failure message.

**Error model:** generic auth-failure for any of: wrong phrase,
expired, replayed, scope-mismatch, rate-limited. Distinct
exit-code-1 + distinct stderr for: server unreachable, malformed URL,
permissions error writing session file.

**Implements:** REQ-400, REQ-401, REQ-406. **Verified by:** TEST-400, TEST-406.

### CON-402: HTTP/WS — `/collab/pair`

**`[Provisional — refined by DESIGN-036 task `contracts`]`**

WebSocket endpoint carrying the SPAKE2 message exchange. Frame format
inherits `cap::pair`'s base64-encoded SPAKE2 messages with side-byte
prefix. Server-driven; rate-limited per source IP.

**Implements:** REQ-400, REQ-403, REQ-407. **Verified by:** TEST-407.

### CON-403: CLI — `zetl revoke`

**`[Provisional — refined by DESIGN-036 task `contracts`]`**

**Endpoint:** `zetl revoke --user <id> [--reason <text>]`

**Implements:** REQ-405. **Verified by:** TEST-405.

### CON-404: On-disk — Pending Pairing Store

**`[Provisional]`**

Path: `.zetl/collab/pairing/<nonce>.json`. Mode 0600. Schema:
`{ scope, created_at, expires_at, inviter_user_id, identity_string_canonical }`.
Pruned on TTL expiry by the same loop that prunes the existing nonce
store. Disjoint from `.zetl/caps/.pair-nonces`.

**Implements:** REQ-403, REQ-404, REQ-407. **Verified by:** TEST-403, TEST-404.

---

## 8. Test Specifications

> TEST entries sketched. DESIGN-036 task `test-strategy` finalises
> per the verification-strategy table in USDD §Selecting a
> Verification Strategy.

| ID         | Technique                | Target                                      | REQ trace          |
| ---------- | ------------------------ | ------------------------------------------- | ------------------ |
| TEST-400   | example                  | HP1 happy path end-to-end                   | REQ-400, REQ-401   |
| TEST-401   | example + property       | Phrase never appears in URL/argv/env/log    | REQ-401            |
| TEST-402   | property                 | scope-X phrase produces no valid scope-Y key | REQ-402            |
| TEST-402.canonicalisation | example     | Scope canonicalisation agrees client/server | REQ-402            |
| TEST-403   | example                  | Replay returns generic failure              | REQ-403            |
| TEST-404   | example                  | Expired phrase rejected                     | REQ-404            |
| TEST-405   | example                  | Revocation propagates ≤ 60s                 | REQ-405, NFR-402   |
| TEST-406   | example                  | All six failure causes produce identical text | REQ-406          |
| TEST-407   | example                  | Both invitation paths usable; nonce stores disjoint | REQ-407    |
| TEST-408   | example                  | Audit log records all attempt outcomes      | REQ-408            |
| TEST-NFR-400 | property               | Generated phrases ≥ 44 bits entropy         | NFR-400            |
| TEST-NFR-401 | benchmark              | LAN/WAN onboarding latency 95p              | NFR-401            |
| TEST-NFR-403 | timing-side-channel    | 50ms-delta failure-cause indistinguishability | NFR-403, REQ-406 |
| TEST-fuzz-pairing-msg | fuzz            | SPAKE2 message decoder against random bytes | REQ-400 (robustness) |
| TEST-mutation-scope-binding | mutation  | Mutation kill rate ≥ 90% on scope-binding module | REQ-402     |
| TEST-mutation-nonce-consume | mutation  | Mutation kill rate ≥ 90% on nonce consumption | REQ-403          |
| TEST-contract-nonce-store | contract   | Single-use, monotonic, no-double-spend invariants | REQ-403      |

---

## 9. Observability Signals

| ID       | Type    | Signal                                                 | REQ trace |
| -------- | ------- | ------------------------------------------------------ | --------- |
| OBS-400  | metric  | `zetl_collab_pairing_started_total{scope_class}`       | REQ-400   |
| OBS-401  | metric  | `zetl_collab_pairing_failed_total{cause}` (operator-only label `cause`) | REQ-403, REQ-406 |
| OBS-402  | log     | Operator-channel log line per pairing outcome with cause | REQ-406, REQ-408 |
| OBS-403  | log     | Audit log per attempt (timestamp, scope, outcome, pubkey-on-success) | REQ-408 |
| OBS-404  | metric  | `zetl_collab_pairing_duration_seconds` histogram      | NFR-401   |

> Note: `cause` label on OBS-401 is **operator-channel only**; it MUST
> NOT be exposed via any HTTP-readable metrics endpoint without
> operator authentication, to prevent the metric becoming an external
> oracle that defeats REQ-406.

---

## 10. Threat Model (Summary)

> Detailed threat model lives in
> `research/SPEC-036-threat-model.md`, produced by DESIGN-036 task
> `threat-model`. This section summarises adversaries and notes
> open questions.

### A. Passive Network Observer

Observes URL channel and (where TLS unterminated upstream) HTTP/WS
traffic. Cannot recover phrase from URL (REQ-401) nor from SPAKE2
messages (protocol property). Mitigation: REQ-401 + SPAKE2.

### B. Active OOB-Channel MitM

Compromises Slack/iMessage/email between Owner and Collaborator.
Recovers phrase. Per SPAKE2's online-single-guess property, attacker
gets one attempt to redeem before Owner notices the legitimate
collaborator's redeem-failure. Mitigation: REQ-403 single-use; OBS-401
detection signal; documented residual risk in operator README.

### C. Active URL-Channel MitM

Intercepts URL. Under ADR-400 (no phrase in URL) the URL alone is
useless. Mitigation: REQ-401 + ADR-400.

### D. Compromised Invitee Device

Post-pairing, attacker has the session credential. Out of scope of
this spec; standard credential-revocation (REQ-405) is the recovery
path.

### E. Active Protocol-Layer MitM

Attacker between server and a redeeming collaborator alters scope
in protocol messages. Mitigation: REQ-402 cryptographic scope binding.

### F. Compromised Vault Server

Attacker controls server, issues over-scoped credentials. Out of
scope of this spec; mitigated only by self-host operational
hygiene + auditing OBS-403.

### G. Phrase Phishing — `zetl collab join` to Wrong Server

**`[Open question — refined by DESIGN-036 task `threat-model`]`**

Attacker tricks Collaborator into running `zetl collab join` against
an attacker-controlled server, types the legitimate phrase, attacker
relays it to the real server. Analogous to WebAuthn's
attestation problem. Candidate mitigation: server-fingerprint OOB
verification step (Owner shares server fingerprint with Collaborator
alongside the phrase; CLI verifies). May be deferred to v0.2.

---

## 11. Purity Boundary Map

> **`[Provisional — refined by DESIGN-036 task `purity-boundary-map`]`**

### Pure Core (no I/O, no shared state, deterministic)

- `collab::pair::phrase::generate(rng) -> Phrase` — wraps existing BIP39
  generator; deterministic given RNG.
- `collab::pair::phrase::canonicalise_scope(scope: &str) -> ScopeCanon`
  — total function, no I/O.
- `collab::pair::identity::for_scope(version: u32, scope: &ScopeCanon)
  -> SpakeIdentity` — computes the SPAKE2 identity string.
- `collab::pair::session::Session::start_symmetric(...)` — wraps
  `cap::pair`'s SPAKE2 driver.
- `collab::pair::hkdf::derive_session_key(shared, scope) -> SessionKey`.
- `collab::pair::error::classify(...) -> FailureCategory` — maps
  internal cause to operator-channel category; user-visible message is
  constant (REQ-406).

### Effectful Shell (orchestrates I/O, calls pure core)

- `collab::pair::store` — pending-pairing on-disk persistence; nonce
  consumption; TTL pruning.
- `collab::pair::server` — HTTP/WS handler accepting `/collab/pair`
  frames.
- `collab::pair::cli::invite_pair_cmd` — CLI front for `zetl invite --pair`.
- `collab::pair::cli::join_cmd` — CLI front for `zetl collab join`.
- `collab::pair::audit` — operator log + audit log emission (OBS-402,
  OBS-403).

### Boundary Contracts (data types crossing the boundary)

- `Phrase` (pure → shell, briefly; never persisted in pure form by shell)
- `SpakeIdentity` (pure → shell)
- `SpakeMessage` (shell ↔ shell over wire; opaque to pure core)
- `SessionKey` (pure → shell, written to file only by shell)
- `FailureCategory` (pure → shell, emitted only to operator log)

### Dependency Rule

`collab::pair::server` and `collab::pair::cli::*` MAY import from
`collab::pair::{phrase, identity, session, hkdf, error}`. The reverse
MUST NOT hold. Enforcement: `clippy::disallowed_methods` on
`std::fs::*`, `std::time::SystemTime::now`, `tokio::*`, and HTTP
crates within the pure modules.

---

## 12. Quality Attribute Checklist

> **`[Provisional — DESIGN-036 task `phase1-quality-gates` finalises]`**

Applied to each REQ-### in §4:

| REQ | Unambiguous | Verifiable | Atomic | Consistent | Quantified | Traceable | Error-aware |
| --- | :---------: | :--------: | :----: | :--------: | :--------: | :-------: | :---------: |
| 400 | ✓ | ✓ | ✓ | ✓ | ✓ (60s 95p) | ✓ | ✓ |
| 401 | ✓ | ✓ | ✓ | ✓ | n/a (binary) | ✓ | ✓ |
| 402 | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |
| 403 | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |
| 404 | ⚠ provisional TTL | ✓ | ✓ | ✓ | ⚠ | ✓ | ✓ |
| 405 | ✓ | ✓ | ✓ | ✓ | ✓ (60s) | ✓ | ✓ |
| 406 | ✓ | ✓ | ✓ | ✓ | n/a + NFR-403 | ✓ | ✓ |
| 407 | ⚠ pending ADR-402 | ✓ | ✓ | ✓ | n/a | ✓ | n/a |
| 408 | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |

Provisional ⚠ entries close once the named DESIGN-036 task completes.

---

## 13. Open Questions Surfaced by This Strawman

1. **Async path:** ADR-402 defaults to coexistence; the prior-art
   research task may surface evidence to deprecate JWT-URL outright.
2. **Phrase length:** 4 words = 44 bits matches `cap::pair`. Is 44
   bits sufficient given the online-single-guess SPAKE2 property AND
   the rate-limit of the pairing endpoint? The threat-model task
   answers.
3. **Scope canonicalisation:** Do we canonicalise on bytes
   (UTF-8 normalised), on a parsed path tree, or on a hash of
   resolved page IDs? Affects ADR-401 directly.
4. **Server fingerprint OOB step (G):** ship in v0.1 or defer to v0.2?
   Adds one OOB element; reduces phishing risk; complicates UX.
5. **Session credential format:** new format, or extend the existing
   `cap::pair`-derived session shape? Affects `acl.rs` integration.
6. **TTL bounds:** 30s ≤ TTL ≤ 1h is provisional. The 1h upper
   bound starts to push against the synchronous-only assumption;
   the lower bound is plausibly too aggressive for non-CLI-fluent
   collaborators.

---

## 14. Status & Next Actions

- This strawman is an **input** to `plans/DESIGN-036-spake2-onboarding.spl`,
  not an output. The plan's tasks refine each Provisional section.
- **No implementation begins** until: (a) Phase 1 + Phase 2 quality
  gates pass; (b) cross-model adversarial review completes; (c)
  human-expert review package is approved (USDD §AI Trust Boundaries
  no-go gate for cryptography + auth core).
- After review and refinement, this document is re-issued at version
  `0.1.0` with status `approved` and the provisional markers removed.
