---
id: SPEC-041
title: "Pluggable Authentication for `zetl --collab`"
version: 0.1.0-strawman
status: draft
date: 2026-05-15
audience: agent, human
parent: SPEC-020
related:
  - SPEC-020  # Multi-user collaborative editing (current auth core)
  - SPEC-034  # Capability mode (prior art for capability URLs)
  - SPEC-036  # SPAKE2 onboarding flow (parallel onboarding work)
  - SPEC-018  # cap pair / pubkey handoff
  - SPEC-005  # Defeasible reasoning (SPL) — authorization layer, unchanged
plan: DESIGN-041-pluggable-collab-auth
---

# SPEC-041: Pluggable Authentication for `zetl --collab`

> **Strawman notice.** This document is a first-pass design produced from an
> exploration session, *before* the Phase 1 surveys, synthetic-user runs, and
> cross-model adversarial review called for by
> [[DESIGN-041-pluggable-collab-auth]]. Per [[PROTO-001]] Constitutional
> Principle 11 ([[Anti-Slop Bias]]), treat every clause here as carrying hidden
> debt until adversarial review proves otherwise. Sections marked
> **`[Provisional]`** are placeholders for grounded findings.
> [[Authentication]] is a [[PROTO-001]] §AI Trust Boundaries **Tier 1 no-go
> area**: **no implementation begins** until the human-expert review package is
> approved. The document reaches `0.1.0` only after the Phase 1 + Phase 2
> quality gates pass.

## Information Table

| Field          | Value                                                                  |
| -------------- | ---------------------------------------------------------------------- |
| Document ID    | [[SPEC-041-pluggable-collab-auth\|SPEC-041]]                            |
| Title          | Pluggable Authentication for `zetl --collab`                           |
| Version        | 0.1.0-strawman                                                         |
| Status         | Draft (strawman; pending [[DESIGN-041-pluggable-collab-auth]] execution)|
| Author         | Agent (Claude Opus 4.7, [[PROTO-001\|USDD Agent Protocol]] v1.7.0)      |
| Date           | 2026-05-15                                                             |
| Audience       | Agent, Human                                                           |
| Trace          | [[PROTO-001]] §Phase 1, §Phase 2, §AI Trust Boundaries, §LangSec        |
| Parent         | [[SPEC-020]] Multi-User Collaborative Editing                          |
| Related        | [[SPEC-034]] Capability Mode; [[SPEC-036]] SPAKE2 Onboarding; [[SPEC-005]] SPL |
| Plan           | [[DESIGN-041-pluggable-collab-auth]]                                   |
| Feature Gate   | `--features collab` (+ `collab-oidc` for the [[OIDC]] authenticator)    |
| Review tier    | Tier 1 (security-sensitive; [[Authentication]] core)                   |

---

## 1. Overview

### 1.1 Problem

`zetl serve --collab` ([[SPEC-020]]) authenticates requests through exactly two
hardcoded paths, both implemented inline in `src/web/session.rs`:

1. **Browser** — a [[WebAuthn]] passkey ceremony establishes a [[Session
   Token|session]] in the in-memory `SessionStore`, carried by the
   `zetl_session` cookie.
2. **Agent** — an [[Ed25519]]-signed [[Bearer Token]] (the "agent token"),
   verified against the `recovery_pubkey` recorded in the user's `UserProfile`.

The middleware `collab_gate` is the single chokepoint every content route
funnels through; `admin_gate` re-implements the same resolution for
`/_admin/*`; four extractors (`SessionUser`, `BearerUser`, `AuthUser`,
`SessionRole`) repeat fragments of it again.

This is operationally rigid. A team that already runs Google Workspace, Okta,
or Microsoft Entra cannot reuse it. A homelab operator who fronts zetl with
[[oauth2-proxy]], [[Authelia]], [[Tailscale]] Serve, or Cloudflare Access
cannot tell zetl to trust the proxy's verdict. A small single-team deployment
that just wants a shared password has no option. A user who wants to hand a
colleague a link that grants scoped read access — the "anyone with the link"
ceremony every collaboration tool offers — has no option. The authentication
method is a compile-time fact, not an operator choice.

### 1.2 Core Insight

**[[Authentication]] and [[Authorization]] are already cleanly separated — the
code just does not expose the seam.** Everything downstream of authentication
keys off one stable contract: a `user_id` (or, for capability URLs, a scoped
[[Principal]]) that resolves to a `Role` (Reader / Editor / Admin). [[SPL]]
policy evaluation, the agent API, `admin_gate`, and edit attribution never ask
*how* the request authenticated. Pluggability therefore does not touch the
authorization layer at all — it replaces the request → [[Principal]] step, and
nothing else.

### 1.3 Design Principles

1. **Authorization is invariant.** Every [[Authenticator]] MUST resolve to a
   [[Principal]] that either binds an existing `UserProfile` or — where the
   method's contract permits — a provisioned or pseudonymous identity. [[SPL]],
   roles, `admin_gate`, and attribution are untouched. This is the load-bearing
   invariant of the whole specification.
2. **One resolution, one [[Principal]].** A request is resolved exactly once,
   by one middleware, which stashes the result in the request extensions.
   `collab_gate`, `admin_gate`, and all extractors *read* that principal — they
   never re-parse headers. This also deletes the current triple-duplicated
   resolution logic.
3. **The default is today's behaviour, bit-for-bit.** A vault with no
   `[collab.auth]` configuration authenticates exactly as it does now (passkey
   + agent-token). The refactor is provably behaviour-preserving before any new
   method ships.
4. **Operator-ordered, first-match-wins.** The `methods` list is an ordered
   precedence chain. Each [[Authenticator]] returns "authenticated", "abstain —
   try next", or "hard reject"; the chain is deterministic, with no implicit
   fallback and no method negotiation.
5. **Trust is explicit and fails closed.** A method that trusts an upstream
   component ([[Reverse Proxy]] headers) MUST refuse to engage unless the
   operator explicitly declared that trust *and* the request provably arrived
   through the trusted hop. Misconfiguration is a startup error, never a silent
   bypass.
6. **Optional dependencies stay optional.** The [[OIDC]] authenticator pulls in
   an async HTTP client and [[OAuth2]]/[[OIDC]] crates; it lives behind the
   `collab-oidc` cargo feature so the default `collab` build is unchanged in
   dependency surface and binary size.
7. **All authentication input is recognised before it is acted on.** Per
   [[PROTO-001]] Constitutional Principle 14 ([[LangSec]]), every contract that
   accepts external input ([[TOML]] config, [[Reverse Proxy]] headers, the
   password form, the [[OIDC]] callback, the [[Capability URL]] token) declares
   a formal grammar and recognises input fully before any semantic action. See
   [[#REQ-4120]].
8. **Failure UX does not leak method internals.** A wrong password, an unknown
   user, an expired [[OIDC]] `state`, a spoofed proxy header, and a revoked
   [[Capability URL]] produce the same user-visible result class. Only the
   operator log distinguishes causes.
9. **The convenience/exposure trade-off of [[Capability URL]]s is made
   explicit, not hidden.** A capability-bearing URL *is* a [[Bearer Token]] —
   [[SPEC-036]] §1 critiques exactly this property of the legacy JWT-URL invite
   flow. SPEC-041 admits the pattern only as an opt-in, scope-bound,
   short-lived, revocable method with a documented residual-risk statement (see
   [[#ADR-4110]], [[#Threat Model H]]).

### 1.4 Scope

**In scope:**

- An [[Authenticator]] trait and an ordered chain assembled from configuration.
- A single `auth_resolve` middleware producing a [[Principal]] request
  extension; refactor of `collab_gate` / `admin_gate` / extractors to consume
  it.
- Re-expression of the two existing methods (passkey+session, agent-token) as
  [[Authenticator]] implementations, with zero behaviour change.
- A `[collab.auth]` configuration block (a [[TOML]] lens, following the
  `[access]` precedent in `src/cap/public_repo.rs`).
- Four new authenticators: **[[Reverse Proxy|reverse-proxy header]]**, **static
  password**, **[[OIDC]] / [[OAuth2]]**, and **[[Capability URL]]**.
- An auto-provisioning policy for methods that authenticate principals with no
  pre-existing `UserProfile`.
- CLI surface for password credentials (`zetl collab passwd`) and capability
  URLs (`zetl collab share`).
- A [[LangSec]] grammar for every external-input contract.
- Threat model and observability instrumentation.

**Out of scope:**

- [[mTLS]] client-certificate authentication (a clean future [[Authenticator]];
  noted in [[#13. Open Questions]], not designed here).
- Pluggable *session storage*. The in-memory `SessionStore` does not survive
  restart or span replicas; a `SessionBackend` trait is a candidate successor
  spec. [[OIDC]], password, and (when session-minting) capability-URL logins
  reuse the existing `SessionStore` as-is.
- [[SCIM]] / directory sync and group-to-role mapping from an [[IdP]].
  Auto-provisioning here assigns a single default role; richer mapping is
  deferred.
- Changes to [[SPL]] / `acl.rs` semantics.
- The capability-mode ([[SPEC-034]]) *reader-identity* system, which is a
  separate, client-side, independent path and is not modified. SPEC-041's
  [[Capability URL]] authenticator is server-side and distinct (see
  [[#ADR-4110]]).
- Replacing the [[SPEC-020]] JWT-URL invitation flow or the [[SPEC-036]] SPAKE2
  flow; those remain onboarding mechanisms for the passkey method.

---

## 2. User Profiles

> **`[Provisional — refined by [[DESIGN-041-pluggable-collab-auth]] task
> user-profiles]`** Sketched from the exploration session; the plan task
> produces the grounded version after surveying current `--collab` adopters,
> per the [[PROTO-001]] §Synthetic User Protocol.

### 2.1 Self-Host Operator (carries from [[SPEC-020]])

Runs `zetl serve --collab` on their own infrastructure. Today they accept
passkeys because that is the only option. Wants to *choose* — and the choice is
driven by the identity infrastructure they already operate.

### 2.2 Workspace-Federated Team

Already runs Google Workspace / Okta / Microsoft Entra. Expects "log in with
the company account," central deprovisioning, and no per-tool password. Will
not adopt a tool that forces a parallel identity store. Served by the [[OIDC]]
authenticator.

### 2.3 Proxy-Fronted Operator

Already terminates authentication at an edge component — [[oauth2-proxy]],
[[Authelia]], [[Tailscale]] Serve, Cloudflare Access, an SSO-aware ingress.
Wants zetl to trust the proxy's verdict and read the authenticated user from a
header. Values not running authentication logic in zetl at all. Served by the
[[Reverse Proxy|reverse-proxy header]] authenticator.

### 2.4 Small-Team / Homelab Operator

Three people, one vault, no [[IdP]], no desire to operate one. Wants a shared or
per-user password and nothing more. Passkeys are friction they did not ask for.
Served by the static password authenticator.

### 2.5 Link-Holder Collaborator *(new — motivates [[Capability URL]]s)*

Receives a URL from someone with access and expects the link itself to grant
entry — the "anyone with the link can view" ceremony of every mainstream
collaboration tool. Will not create an account, will not run a CLI, may be
outside the operator's organisation entirely. Needs *scoped*, *time-bounded*
access (one folder, read-only, expires in a week), not full vault membership.
Accepts — often without articulating it — that the link is the credential.
Served by the [[Capability URL]] authenticator.

### 2.6 Agent / Programmatic Client (carries from [[SPEC-020]])

An LLM agent or script using the `/api/*` surface with a [[Bearer Token]] agent
token. Unaffected by this specification except that its resolution now flows
through the same [[Authenticator]] chain as everything else.

---

## 3. Happy Paths

> **`[Provisional — refined by [[DESIGN-041-pluggable-collab-auth]] task
> happy-paths]`** Sketched from the exploration session. The plan task produces
> enumerated failure modes and the synthetic-user run.

### 3.1 HP1: Default — No Configuration Change

**Preconditions:** Operator upgrades zetl; `.zetl/config.toml` has no
`[collab.auth]` block.

**Steps:** none. `zetl serve --collab` starts; the chain is constructed as
`["passkey", "agent-token"]`. Browser login, agent tokens, recovery,
invitations, `admin_gate` all behave exactly as the prior release.

**Postconditions:** Behaviour-identical to pre-SPEC-041. Verified by the
unchanged [[SPEC-020]] test suite running green against the refactor
([[#TEST-4103]]).

### 3.2 HP2: Proxy-Header Authentication

**Preconditions:** Operator fronts zetl with an authenticating proxy that sets
`X-Forwarded-User` and strips any client-supplied copy. zetl is reachable only
via that proxy.

**Steps:**

1. Operator sets, in `.zetl/config.toml`:
   ```toml
   [collab.auth]
   methods = ["proxy-header", "agent-token"]

   [collab.auth.proxy_header]
   user_header    = "X-Forwarded-User"
   peer_allow     = ["127.0.0.1/32", "10.0.0.0/8"]
   auto_provision = true
   ```
2. Operator starts `zetl serve --collab --trust-proxy`. Startup validation
   confirms proxy trust is enabled ([[#REQ-4106]]) — without it the server
   refuses to start and prints why.
3. A user hits the proxy, authenticates *there*, and the proxy forwards the
   request with `X-Forwarded-User: alice@example.com`.
4. `auth_resolve` walks the chain: `proxy-header` verifies the peer IP is in
   `peer_allow`, recognises the header value against its grammar, maps it to a
   `user_id`, auto-provisions a Reader `UserProfile` on first sight, and yields
   a [[Principal]].

**Postconditions:** `alice` is authenticated for the request; no zetl login
page is shown. A request from outside `peer_allow`, or carrying the header
without traversing the trusted hop, is rejected as unauthenticated.

**Failure modes (enumerated by the plan task):** proxy-header listed without
`--trust-proxy` → startup refusal; header from non-allowlisted peer → ignored;
header value fails the grammar → rejected, not normalised.

### 3.3 HP3: Static Password

**Preconditions:** Small team, no [[IdP]].

**Steps:**

1. Operator runs `zetl collab passwd add alice` and is prompted for a password
   (TTY only, never argv). An [[argon2id]] hash is written to
   `.zetl/collab/passwords.json` (mode 0600). A `UserProfile` for `alice` is
   created if absent.
2. Operator sets `methods = ["password", "agent-token"]`.
3. Alice visits the vault, is redirected to `/auth/password`, submits name +
   password over the form. The `password` authenticator recognises the form
   body against its grammar, verifies the [[argon2id]] hash in constant time,
   mints an ordinary `SessionStore` [[Session Token|session]], and sets the
   `zetl_session` cookie.

**Postconditions:** Alice has an ordinary session — identical downstream to a
passkey session. The existing `AuthRateLimiters` apply to `/auth/password`.

### 3.4 HP4: [[OIDC]] / Workspace Login

**Preconditions:** zetl built with `--features collab,collab-oidc`; team runs an
[[OIDC]] provider.

**Steps:**

1. Operator registers a client with the [[IdP]], then sets:
   ```toml
   [collab.auth]
   methods = ["oidc", "agent-token"]

   [collab.auth.oidc]
   issuer                 = "https://accounts.google.com"
   client_id              = "..."
   client_secret_file     = "~/.config/zetl/oidc-secret"
   user_id_claim          = "email"
   auto_provision         = true
   provision_domain_allow = ["example.com"]
   ```
2. Alice visits the vault, is redirected to `/auth/oidc/login`, which redirects
   to the [[IdP]] with a [[PKCE]] challenge, `state`, and `nonce`.
3. After [[IdP]] login, the [[IdP]] redirects to `/auth/oidc/callback`. zetl
   recognises the callback query against its grammar, validates `state`,
   exchanges the code with the [[PKCE]] verifier, validates the [[ID Token]]
   (issuer, audience, expiry, `nonce`, signature against the [[JWKS]]), reads
   `user_id_claim`, checks the email domain against `provision_domain_allow`,
   auto-provisions a Reader `UserProfile` on first login, and mints a
   `SessionStore` session.

**Postconditions:** Alice has an ordinary zetl session; subsequent requests need
no [[IdP]] round-trip until it expires. An [[ID Token]] with an unlisted email
domain is rejected; the operator may pre-create the profile to grant access
without widening the allowlist.

### 3.5 HP5: [[Capability URL]] — "Anyone With the Link"

**Preconditions:** A self-host operator wants to give an outside reviewer
read-only access to one folder for a week, without onboarding them.

**Steps:**

1. Operator runs `zetl collab share --scope review/draft-7/ --role reader
   --expires 7d`. The CLI mints a signed capability token (reusing the [[EdDSA]]
   server-key infrastructure in `src/user/invite.rs`) and prints a single URL:
   `https://wiki.example.com/review/draft-7/?cap=<token>`.
2. Operator sends the URL to the reviewer over whatever channel they already
   use.
3. The reviewer opens the URL. `auth_resolve` walks the chain; the
   `capability-url` authenticator recognises `?cap=<token>` against the token
   grammar, verifies the signature, checks expiry and revocation, and yields a
   *pseudonymous* scope-bound [[Principal]] ([[#REQ-4119]]) — no `UserProfile`,
   no account.
4. The reviewer reads pages under `review/draft-7/`. Any navigation outside the
   bound scope, or any write, is denied by the unchanged [[SPL]] layer because
   the [[Principal]] carries only that capability.

**Postconditions:** The reviewer read the scoped content with zero onboarding.
After 7 days, or after `zetl collab share --revoke <id>`, the same URL yields
the generic auth-failure result. The URL is a [[Bearer Token]]: anyone it is
forwarded to has the same access until expiry/revocation — a property the
operator was warned about at mint time ([[#REQ-4116]], [[#Threat Model H]]).

### 3.6 HP6: Mixed Chain

**Preconditions:** A team wants [[OIDC]] for members, agent tokens for
automation, and capability URLs for outside reviewers.

**Steps:** `methods = ["agent-token", "capability-url", "oidc"]`. Each request
walks the chain; the first [[Authenticator]] that returns a [[Principal]] wins;
the last redirect-capable method (`oidc`) provides the login redirect for
unauthenticated browsers.

**Postconditions:** All client classes work simultaneously, with deterministic,
operator-defined precedence ([[#NFR-4104]]).

---

## 4. Functional Requirements

> Numbering: SPEC-041 → REQ-41xx, mirroring the SPEC-038/039/040 pattern. Per
> [[PROTO-001]] §Numbering Rules, final IDs are confirmed by
> [[DESIGN-041-pluggable-collab-auth]] task `draft-requirements` against the
> highest existing ID at draft time. Each REQ is decomposed into positive /
> negative-input / negative-output tests per [[PROTO-001]]
> §Requirement-Targeted Test Decomposition.

### REQ-4101: Authenticator Abstraction

The system SHALL define an [[Authenticator]] trait whose contract maps an
inbound request's parts to one of three outcomes: **authenticated** (yields a
[[Principal]]), **abstain** (not this authenticator's concern — the chain
proceeds), or **reject** (a hard, chain-terminating failure). The two methods
that exist today (passkey+session, agent-token) SHALL be re-expressed as
implementations of this trait with no change to externally observable
behaviour.

**Trace:** [[#TEST-4101]], [[#CON-4101]], [[#ADR-4101]]; [[#3.1 HP1]].

### REQ-4102: Config-Driven Method Selection

The system SHALL read an ordered `[collab.auth] methods` list from
`.zetl/config.toml` and assemble the [[Authenticator]] chain in that order.
Each named method MAY have a corresponding `[collab.auth.<method>]` sub-table.
An unknown method name SHALL be a startup error, not a silent skip.

**Trace:** [[#TEST-4102]], [[#CON-4102]], [[#ADR-4101]].

### REQ-4103: Backwards-Compatible Default

WHEN `.zetl/config.toml` contains no `[collab.auth]` block, the system SHALL
behave as though `methods = ["passkey", "agent-token"]` were configured,
reproducing the pre-SPEC-041 authentication behaviour exactly — including login
redirects, recovery, invitations, and `admin_gate`.

**Trace:** [[#TEST-4103]]; [[#3.1 HP1]]; Design Principle [[#1.3 Design Principles|§1.3.3]].

### REQ-4104: Single Principal Resolution

The system SHALL resolve each request to at most one [[Principal]] exactly
once, in a single middleware (`auth_resolve`), exposed via the request
extensions. `collab_gate`, `admin_gate`, and every request extractor SHALL
consume that extension and SHALL NOT independently re-parse authentication
material from headers or cookies.

**Trace:** [[#TEST-4104]], [[#CON-4103]], [[#ADR-4102]]; Design Principle
[[#1.3 Design Principles|§1.3.2]].

### REQ-4105: Reverse-Proxy Header Authenticator

The system SHALL provide a `proxy-header` [[Authenticator]] that derives the
authenticated `user_id` from a configurable request header (default
`X-Forwarded-User`) set by a trusted upstream [[Reverse Proxy]].

**Trace:** [[#TEST-4105]], [[#CON-4108]], [[#REQ-4106]], [[#REQ-4120]]; [[#3.2 HP2]].

### REQ-4106: Proxy-Header Trust Gate

The `proxy-header` [[Authenticator]] SHALL refuse to engage UNLESS both: (a)
the server was started with proxy trust enabled (`--trust-proxy` / equivalent
config), AND (b) the request's immediate peer address matches the
operator-configured `peer_allow` [[CIDR]] list. IF `proxy-header` is listed in
`methods` but proxy trust is not enabled, the server SHALL refuse to start and
SHALL print the specific misconfiguration. The authenticator SHALL ignore any
client-supplied copy of the configured header arriving from a non-`peer_allow`
peer.

**Trace:** [[#TEST-4106]], [[#CON-4108]], [[#ADR-4103]]; [[#Threat Model A]];
Design Principle [[#1.3 Design Principles|§1.3.5]].

### REQ-4107: Static Password Authenticator

The system SHALL provide a `password` [[Authenticator]] that verifies a
submitted (user, password) pair against [[argon2id]] hashes stored in
`.zetl/collab/passwords.json`, and on success mints an ordinary `SessionStore`
[[Session Token|session]] indistinguishable downstream from a passkey session.
Verification SHALL be constant-time with respect to password content and SHALL
NOT reveal whether the failure was an unknown user or a wrong password.

**Trace:** [[#TEST-4107]], [[#CON-4105]], [[#CON-4107]], [[#ADR-4106]],
[[#REQ-4120]]; [[#3.3 HP3]].

### REQ-4108: Password Credential Management CLI

The system SHALL provide `zetl collab passwd` with `add`, `remove`, and `list`
subcommands. Passwords SHALL be read from a TTY prompt and SHALL NOT be accepted
as command-line arguments or environment variables. `add` SHALL create the
corresponding `UserProfile` if it does not exist.

**Trace:** [[#TEST-4108]], [[#CON-4106]]; [[#3.3 HP3]].

### REQ-4109: OIDC / OAuth2 Authenticator

The system SHALL provide an `oidc` [[Authenticator]] (behind the `collab-oidc`
cargo feature) implementing the [[OIDC]] [[Authorization Code Flow]] with
[[PKCE]]. It SHALL redirect unauthenticated browser requests to the configured
[[IdP]], handle the callback, and on success mint an ordinary `SessionStore`
[[Session Token|session]]. The `user_id` SHALL be derived from a configured
[[ID Token]] claim (`user_id_claim`, default `email`).

**Trace:** [[#TEST-4109]], [[#CON-4104]], [[#ADR-4104]], [[#ADR-4105]]; [[#3.4 HP4]].

### REQ-4110: OIDC Callback Security

The `oidc` [[Authenticator]] SHALL: pin the `issuer`; verify the [[ID Token]]'s
signature against the [[IdP]]'s published [[JWKS]], its `aud`, its `exp`/`iat`,
and the `nonce`; bind and verify the `state` parameter against the initiating
request; use a per-request [[PKCE]] verifier; and treat any validation failure
as an authentication failure with a generic user-visible result. `state` and
`nonce` values SHALL be single-use.

**Trace:** [[#TEST-4110]], [[#CON-4104]], [[#REQ-4120]]; [[#Threat Model B]].

### REQ-4111: Auto-Provisioning Policy

For methods that can present a [[Principal]] with no pre-existing `UserProfile`
(`proxy-header`, `oidc`), the system SHALL support an opt-in `auto_provision`
flag. WHEN enabled, a first-seen principal SHALL be provisioned with a
`UserProfile` at a fixed default role of **Reader** and never higher. The `oidc`
method SHALL additionally support a `provision_domain_allow` list; a principal
whose identity-claim domain is not listed SHALL NOT be auto-provisioned (the
operator MAY still pre-create the profile). WHEN `auto_provision` is disabled,
an unknown principal SHALL be rejected as unauthenticated.

**Trace:** [[#TEST-4111]], [[#ADR-4107]]; [[#Threat Model C]].

### REQ-4112: Stateless-Method CSRF Exemption

Each [[Authenticator]] SHALL declare whether the principals it issues are
**cookie-session-backed** or **stateless**. The [[CSRF]] guard (`csrf_guard`)
SHALL apply only to cookie-session-backed principals; stateless principals
(`agent-token`, `proxy-header`, `capability-url` in stateless mode, and any
future bearer-style method) SHALL be exempt, generalising the current
Bearer-only exemption.

**Trace:** [[#TEST-4112]], [[#CON-4101]]; [[SPEC-020]] REQ-020-064.

### REQ-4113: Authenticator Route Mounting

An [[Authenticator]] SHALL be able to contribute the public routes it needs
(login pages, [[OAuth2]] callbacks). These routes SHALL be mounted in the
always-public `auth_routes` group, behind the existing per-IP rate limiter, and
SHALL NOT be gated by `collab_gate`.

**Trace:** [[#TEST-4113]], [[#CON-4104]], [[#CON-4105]].

### REQ-4114: Startup Validation & Diagnostics

The system SHALL validate the entire `[collab.auth]` configuration at startup
and SHALL refuse to start on any of: unknown method name; a configured method
missing required sub-table fields; `proxy-header` without proxy trust; `oidc`
named while the `collab-oidc` feature is not compiled in; an unreadable
`client_secret_file` or `passwords.json`; a `capability-url` method configured
while no [[EdDSA]] server key can be loaded or created. Each refusal SHALL name
the offending key and the corrective action.

**Trace:** [[#TEST-4114]], [[#CON-4102]]; [[#REQ-4106]].

### REQ-4115: Authentication Audit Trail

The system SHALL record every authentication decision — success, abstain-to-end
(no method matched), and reject — to the operator log with: timestamp, method
id, outcome, [[Principal]] identity on success, and a cause category on
failure. The log SHALL NOT contain passwords, tokens, [[ID Token]]s, [[PKCE]]
verifiers, capability tokens, or any value derivable from them.

**Trace:** [[#TEST-4115]], [[#OBS-4103]], [[#OBS-4104]]; [[PROTO-001]]
§Observability Requirement.

### REQ-4116: Capability-URL Authenticator

The system SHALL provide a `capability-url` [[Authenticator]] that recognises a
signed capability token carried as a URL query parameter (`?cap=<token>`),
verifies its signature against the vault's [[EdDSA]] server key, and on success
yields a [[Principal]] whose authority is exactly the capability encoded in the
token. The token SHALL be a [[Bearer Token]]; the system SHALL NOT treat
possession as proof of identity, only as proof of capability. The CLI that
mints capability URLs (`zetl collab share`) SHALL emit, at mint time, an
explicit security notice that the URL is bearer authority.

**Trace:** [[#TEST-4116]], [[#CON-4109]], [[#CON-4110]], [[#ADR-4109]],
[[#ADR-4110]], [[#ADR-4111]], [[#REQ-4120]]; [[#3.5 HP5]]; [[#Threat Model H]].

### REQ-4117: Capability-URL Scope Binding and Expiry

A capability token SHALL carry a folder/page scope glob and a role, both
cryptographically bound by the signature, and an expiry as a Unix timestamp.
The `capability-url` [[Authenticator]] SHALL reject an expired token, and the
[[Principal]] it issues SHALL grant no authority beyond the token's bound scope
and role — enforced by the unchanged [[SPL]] layer, which receives the scope as
a [[Principal]] attribute. The mint CLI SHALL bound `--expires` to
**`[Provisional: 5 minutes ≤ TTL ≤ 90 days]`** with a default of
**`[Provisional: 7 days]`**.

**Trace:** [[#TEST-4117]], [[#CON-4109]], [[#CON-4110]]; [[#Threat Model H]].

### REQ-4118: Capability-URL Revocation

The system SHALL allow an operator to revoke a previously-minted capability URL
by its token id, with revocation propagating to all server-handled requests
within **`[Provisional: 60 seconds]`**. A revoked token SHALL yield the generic
authentication-failure result, indistinguishable at the user-visible layer from
an expired or malformed token. Revocation state SHALL be persisted in
`.zetl/collab/` analogously to the existing `used-nonces.json`.

**Trace:** [[#TEST-4118]], [[#CON-4109]], [[#CON-4110]]; [[#Threat Model H]].

### REQ-4119: Capability-URL Pseudonymous Principal

The `capability-url` [[Authenticator]] SHALL issue a *pseudonymous*
[[Principal]] that is NOT auto-provisioned into a `UserProfile` and SHALL NOT be
elevated by [[#REQ-4111]] auto-provisioning. The [[Principal]]'s identity SHALL
be a stable, non-guessable handle derived from the token id (for attribution
and audit), and its authority SHALL be exactly the token's bound scope+role.
A capability-URL [[Principal]] SHALL NEVER satisfy `admin_gate` regardless of
the role encoded in the token.

**Trace:** [[#TEST-4119]], [[#CON-4109]], [[#ADR-4111]]; [[#Threat Model H]].

### REQ-4120: Input Grammar Recognition (LangSec)

Every contract in [[#7. Contracts]] that accepts external input SHALL declare a
formal grammar (ABNF or schema) for that input and SHALL recognise input fully
against the grammar before any semantic action. Ad-hoc parsing, permissive
normalisation of malformed input, and string-concatenation serialisation of
structured material are prohibited at these trust boundaries. Where the same
format is consumed in more than one place (e.g. the [[EdDSA]] [[JWT]] form
shared by the agent token, the [[SPEC-020]] invite, and the capability token),
a single shared recogniser SHALL be used.

**Trace:** [[#TEST-4120]], [[#CON-4102]], [[#CON-4104]], [[#CON-4105]],
[[#CON-4108]], [[#CON-4109]]; [[PROTO-001]] §LangSec; Constitutional Principle 14.

---

## 5. Non-Functional Requirements

### NFR-4101: Auth Resolution Latency

For the non-redirect authenticators (`passkey` session validation,
`agent-token`, `proxy-header`, `password` session validation, `capability-url`
token verification), end-to-end `auth_resolve` middleware time SHALL be ≤ 1 ms
at the 95th percentile UNDER nominal server load. (The [[OIDC]] *first* login
involves an [[IdP]] round-trip and is explicitly excluded; subsequent requests
use the session path.)

**Trace:** [[#TEST-NFR-4101]], [[#OBS-4101]].

### NFR-4102: Optional-Dependency Containment

A default `--features collab` build (without `collab-oidc`) SHALL gain no new
third-party dependencies from this specification and SHALL NOT grow in binary
size beyond a negligible margin attributable to the trait refactor. The
[[OIDC]] HTTP client and [[OAuth2]]/[[OIDC]] crates SHALL be reachable ONLY
under `collab-oidc`.

**Trace:** [[#TEST-NFR-4102]], [[#ADR-4105]].

### NFR-4103: Password Hashing Cost

The `password` [[Authenticator]] SHALL use [[argon2id]] with parameters tuned
so a single verification costs **`[Provisional: ≥ 100 ms, ≤ 500 ms]`** on
reference hardware — high enough to resist offline cracking, low enough not to
enable a login-endpoint DoS amplification. Parameters SHALL be recorded in each
stored hash (the [[PHC string]] form) so they can be raised without
invalidating existing credentials.

**Trace:** [[#TEST-NFR-4103]], [[#ADR-4106]].

### NFR-4104: Ordering Determinism

For any fixed `methods` list and any fixed request, the chosen
[[Authenticator]] SHALL be deterministic and SHALL equal the first
non-abstaining authenticator in list order. No authenticator's outcome SHALL
depend on wall-clock ordering, map iteration order, or thread scheduling.

**Trace:** [[#TEST-NFR-4104]]; Design Principle [[#1.3 Design Principles|§1.3.4]].

---

## 6. Architecture Decision Records

> ADRs sketched as positions, not decided. [[DESIGN-041-pluggable-collab-auth]]
> plan tasks finalise each. Per [[PROTO-001]] Constitutional Principle 12, none
> of these is validated by this authoring session.

### ADR-4101: Trait Objects over Enum Dispatch

**Status:** Proposed (strawman default)

**Context:** The chain holds a heterogeneous, config-ordered set of
authenticators, one of which ([[OIDC]]) is feature-gated out of most builds.

**Decision:** `Vec<Box<dyn Authenticator + Send + Sync>>`. The chain builder
(config → chain) is the only site that names concrete types; the [[OIDC]] arm
is `#[cfg(feature = "collab-oidc")]` there.

**Consequences:** (+) Feature-gating is localised to one match arm. (+) Future
specs add authenticators without touching chain logic. (−) Dynamic dispatch on
the hot path — bounded by [[#NFR-4101]]; the chain is short and per-call work
dwarfs the vtable lookup. An enum would inline but would force every variant
(including [[OIDC]]'s dependency surface) into every build.

### ADR-4102: One Resolve Middleware, Principal in Extensions

**Status:** Proposed (strawman default)

**Context:** Today `collab_gate`, `admin_gate`, and four extractors each
re-derive identity from raw headers/cookies — duplicated and drift-prone.

**Decision:** A single `auth_resolve` middleware runs the chain once and inserts
`Option<Principal>` into the request extensions. `collab_gate` enforces
presence; `admin_gate` reads the principal and checks owner/admin; extractors
read the principal. Raw-header parsing lives *only* inside [[Authenticator]]
implementations.

**Consequences:** (+) One code path to audit. (+) `admin_gate`'s parallel
resolution is deleted. (−) Middleware ordering becomes load-bearing:
`auth_resolve` must run before `collab_gate`, `csrf_guard`, and route handlers
— enforced by construction in `src/web/mod.rs` and a test.

### ADR-4103: Proxy-Header Trust Model — `trust_proxy` + Peer CIDR Allowlist

**Status:** Proposed (strawman default — defence in depth)

**Context:** A forwarded-user header is trivially spoofable by any client that
can reach the server directly. The mitigation must be impossible to get subtly
wrong.

**Decision:** `proxy-header` engages only when proxy trust is enabled *and* the
request's immediate peer IP is in `peer_allow`. Naming `proxy-header` in
`methods` without `--trust-proxy` is a hard startup error
([[#REQ-4106]], [[#REQ-4114]]). The operator README states the proxy MUST strip
inbound copies of the header; zetl additionally ignores the header on any
non-allowlisted peer as a second layer.

**Consequences:** (+) No single misconfiguration yields a bypass. (+) Reuses the
existing `trust_proxy` flag in `WebState`. (−) Operators on shared-IP
infrastructure get a coarse allowlist; documented as a known limitation, with
[[mTLS]] noted as the stronger future control.

### ADR-4104: OIDC Reuses `SessionStore`; No Parallel Session Type

**Status:** Proposed (strawman default)

**Context:** After a successful [[OIDC]] callback the browser needs a durable
credential so every subsequent request does not round-trip the [[IdP]].

**Decision:** The [[OIDC]] callback mints an ordinary `SessionStore`
[[Session Token|session]] and sets the `zetl_session` cookie — the exact
artefact the passkey path produces. zetl does not store [[IdP]] refresh tokens
and does not invent an "OIDC session." Session lifetime is zetl's existing
idle/max timeout.

**Consequences:** (+) Everything downstream ([[CSRF]], `admin_gate`, `user_id`
recovery, logout) works unchanged. (+) No new persistence. (−) [[IdP]]-side
revocation does not propagate until the zetl session expires; documented, with
the `SessionBackend` successor spec as the place to shorten that window.

### ADR-4105: OIDC Behind the `collab-oidc` Cargo Feature

**Status:** Proposed (strawman default)

**Context:** [[OIDC]] needs an async HTTP client and [[OAuth2]]/[[OIDC]] crates
— real dependency weight most deployments do not need.

**Decision:** A `collab-oidc` feature, additive on top of `collab`. The `oidc`
module, its routes, and its dependencies are all `#[cfg(feature =
"collab-oidc")]`. Naming `oidc` in `methods` without the feature is a startup
error with a build-instruction hint ([[#REQ-4114]]).

**Consequences:** (+) [[#NFR-4102]] holds by construction. (−) Distributors
publish at least two build variants; `--version` output should list compiled
auth features.

### ADR-4106: Password Storage — argon2id in a 0600 JSON File

**Status:** Proposed (strawman default)

**Context:** The password method needs a credential store. zetl already keeps
collab state as JSON under `.zetl/collab/` (`used-nonces.json`,
`pending-invites.json`).

**Decision:** `.zetl/collab/passwords.json`, mode 0600, one record per user:
`{ user_id, phc }` where `phc` is the [[argon2id]] [[PHC string]] (embeds
parameters + salt, so [[#NFR-4103]] cost can be raised without invalidating
existing hashes). File-permission checks mirror the existing `server.key` 0600
enforcement in `src/user/invite.rs`.

**Consequences:** (+) Consistent with existing collab on-disk conventions. (+)
Self-describing hashes. (−) Not a real user database; a genuinely large team
should use [[OIDC]]. Documented as the intended small-team boundary.

### ADR-4107: Auto-Provisioning — Opt-In, Default Reader, Domain-Gated

**Status:** Proposed (strawman default — least privilege)

**Context:** `proxy-header` and `oidc` can authenticate a principal who has no
`UserProfile`. Either zetl rejects them (operator pre-creates every profile) or
provisions one. Provisioning above Reader, or for any identity an [[IdP]] will
mint, is a privilege-escalation footgun.

**Decision:** Auto-provisioning is opt-in per method (`auto_provision`). When
on, a first-seen principal gets a Reader profile and never higher; elevation is
an explicit operator action. `oidc` additionally requires the identity-claim
domain in `provision_domain_allow`. When off, unknown principals are rejected.
Capability-URL principals are *never* provisioned ([[#REQ-4119]]).

**Consequences:** (+) The blast radius of a misconfigured [[IdP]] client or
over-broad proxy is bounded to read access. (+) Operators retain the pre-create
escape hatch. (−) Granting Editor to a federated user is a manual step; richer
[[IdP]]-group → role mapping is explicitly deferred ([[#1.4 Scope]]).

### ADR-4108: Module Layout — `src/web/auth/`

**Status:** Proposed (strawman default)

**Decision:** A new `src/web/auth/` module: `mod.rs` ([[Authenticator]] trait,
[[Principal]], `AuthOutcome`, chain builder), `resolve.rs` (`auth_resolve`
middleware + extension plumbing), `config.rs` (`[collab.auth]` lens),
`provision.rs` (auto-provisioning), `token.rs` (the shared [[EdDSA]] [[JWT]]
recogniser, [[#REQ-4120]]), and one file per authenticator (`passkey.rs`,
`agent_token.rs`, `proxy_header.rs`, `password.rs`, `oidc.rs`,
`capability_url.rs`). `src/web/session.rs` keeps `SessionStore` and cookie
helpers; `collab_gate` / `csrf_guard` / `admin_gate` / extractors move to
`auth/` or are thinned to read the [[Principal]] extension.

**Consequences:** (+) The auth surface is one directory to audit. (−) A
non-trivial move-and-rename diff in Phase 0; mitigated by doing it with zero
behaviour change and the [[SPEC-020]] suite as the gate.

### ADR-4109: Capability-URL Token Format — Reuse the EdDSA JWT Infrastructure

**Status:** Proposed (strawman default)

**Context:** The capability token needs to be signed, compact, URL-safe, and
carry structured claims (scope, role, expiry, id). `src/user/invite.rs` already
implements [[EdDSA]] [[JWT]] encode/decode against a per-vault `server.key`,
with a single-use nonce store.

**Decision:** The capability token is an [[EdDSA]] [[JWT]] with `sub =
"zetl-capability"`, signed by the same `server.key`, produced and recognised by
the shared `auth/token.rs` recogniser ([[#REQ-4120]]). Claims: `scope` (glob),
`role`, `exp`, `jti` (token id, for revocation + the pseudonymous handle). It is
*distinct* from the [[SPEC-020]] invite [[JWT]] (`sub = "zetl-invite"`) and from
the [[SPEC-034]] client-side reader caps — same primitive, disjoint `sub`
domain, disjoint server-side state.

**Consequences:** (+) No new cryptographic surface; reuses audited
encode/decode. (+) One recogniser for all [[EdDSA]] [[JWT]] forms satisfies
[[PROTO-001]] §LangSec "one parser per language." (−) Compromise of `server.key`
forges capability tokens *and* invites — already true for invites; documented in
[[#Threat Model F]]. (−) The `sub` discriminator must be checked before claims
are trusted; the recogniser enforces this structurally.

### ADR-4110: Capability-URL Placement — Query String, Not Fragment

**Status:** Proposed (strawman default)

**Context:** [[SPEC-034]] carries reader caps in the URL *fragment* (`#k=...`)
precisely because the fragment never reaches the server — its shim is
client-side. SPEC-041's authenticator is *server-side*: it must see the token,
so the token must be in the path or query, not the fragment. That re-introduces
the "URL is a [[Bearer Token]]" property [[SPEC-036]] §1 critiques.

**Decision:** The token travels as a `?cap=<token>` query parameter. SPEC-041
accepts the bearer property as an explicit, opt-in trade-off and mitigates it:
(a) `zetl collab share` prints a bearer-authority security notice at mint time;
(b) tokens are scope-bound, short-TTL, and revocable ([[#REQ-4117]],
[[#REQ-4118]]); (c) responses to capability-URL requests set
`Referrer-Policy: no-referrer` so the token does not leak via the `Referer`
header to outbound links; (d) the token is stripped from the URL the server
logs, and never appears in the audit trail ([[#REQ-4115]]); (e) the operator
documentation states capability URLs are for low-to-moderate-sensitivity scopes
and names [[OIDC]] / passkey as the controls for anything higher.

**Consequences:** (+) A genuine "anyone with the link" capability, server-side
enforceable. (−) The residual bearer risk cannot be fully eliminated, only
bounded — stated plainly in [[#Threat Model H]] and the README. (−) Browser
history and server access logs at intermediary hops may still capture the
query; the threat-model task quantifies this.

### ADR-4111: Capability-URL Principal Model — Pseudonymous, Scope-Capped

**Status:** Proposed (strawman default — least privilege)

**Context:** A capability-URL holder is, by [[#2.5 User Profiles|Profile §2.5]],
explicitly *not* an account holder. Minting them a `UserProfile` would (a)
pollute the member list, (b) risk auto-provisioning escalation, (c) imply an
identity the token does not actually prove.

**Decision:** The `capability-url` [[Authenticator]] issues a pseudonymous
[[Principal]]: identity is a stable handle derived from `jti` (for attribution),
authority is exactly the token's `scope`+`role`, no `UserProfile` is created or
required, and the principal can never satisfy `admin_gate`
([[#REQ-4119]]). [[SPL]] evaluates it as a first-class subject carrying a
capability attribute.

**Consequences:** (+) No identity is asserted that the token does not prove. (+)
Auto-provisioning escalation ([[#Threat Model C]]) is structurally impossible
for this method. (−) [[SPL]] policy authors must handle a subject with a
capability attribute and no profile; the [[DESIGN-041-pluggable-collab-auth]]
`acl-integration` task confirms the existing `acl.rs` surface accommodates this
without semantic change.

---

## 7. Contracts

> CON entries sketched. [[DESIGN-041-pluggable-collab-auth]] task `contracts`
> finalises each with full pre/post-condition tables and error-model
> enumeration. Per [[PROTO-001]] §LangSec, every CON accepting external input
> carries a **Grammar** clause; per the CON template, pre/post-conditions are
> structured so each implemented REQ maps to a distinct clause.

### CON-4101: `Authenticator` Trait

```rust
/// Resolves one inbound request to an authenticated principal.
/// Implementations live in `src/web/auth/<method>.rs`.
trait Authenticator: Send + Sync {
    /// Stable id used in config `methods` and audit logs.
    fn id(&self) -> &'static str;

    /// Inspect the request parts. MUST be cheap for the abstain case
    /// (NFR-4101) and MUST NOT mutate server state on abstain.
    fn authenticate(&self, parts: &RequestParts) -> AuthOutcome;

    /// True if principals from this method are cookie-session-backed
    /// (CSRF applies); false for stateless/bearer-style (REQ-4112).
    fn issues_cookie_session(&self) -> bool;

    /// Where to send an unauthenticated *browser* request, if this
    /// method has a login surface. `None` for header/bearer methods.
    fn login_redirect(&self) -> Option<&str> { None }

    /// Public routes this method needs mounted (REQ-4113).
    fn routes(&self) -> Router<WebState> { Router::new() }
}

enum AuthOutcome {
    Authenticated(Principal),
    Abstain,
    Reject(AuthRejection),   // chain-terminating; generic user-visible class
}

struct Principal {
    /// Account `user_id`, OR a pseudonymous capability handle (REQ-4119).
    identity: PrincipalId,
    method: &'static str,            // authenticator id, for audit
    cookie_session: bool,            // mirrors issues_cookie_session
    /// Present only for capability-url principals: the bound scope+role.
    capability: Option<CapabilityGrant>,
}
```

**Pre-conditions:** `authenticate` is called once per request by `auth_resolve`,
before any route handler.

**Post-conditions:** (REQ-4101) `Authenticated` yields a [[Principal]] whose
`identity` either resolves to a `UserProfile`, has been auto-provisioned per
[[#REQ-4111]], or is a pseudonymous capability handle per [[#REQ-4119]], by the
time the outcome is returned. (REQ-4112) `cookie_session` is `true` iff
`issues_cookie_session()` is `true`.

**Implements:** [[#REQ-4101]], [[#REQ-4112]]. **Verified by:** [[#TEST-4101]],
[[#TEST-4112]].

### CON-4102: `[collab.auth]` Configuration Schema

**Grammar:** The input is a [[TOML]] document (RFC-8259-adjacent; the `toml`
crate is the declared recogniser). The typed lens `CollabAuthConfig` is the
schema recogniser layered on top, following the `ZetlConfigLens` /
`AccessConfig` precedent in `src/cap/public_repo.rs`: it deserialises only the
`[collab]` section and tolerates unknown *sibling* top-level keys, but rejects
unknown keys *within* `[collab.auth]` and its sub-tables (`deny_unknown_fields`)
so a typo is a startup error, not a silent default ([[#REQ-4120]]).

```toml
[collab.auth]
# Ordered precedence chain. Default when the whole block is absent:
#   ["passkey", "agent-token"]
methods = ["oidc", "capability-url", "agent-token"]

[collab.auth.proxy_header]
user_header    = "X-Forwarded-User"        # default
peer_allow     = ["127.0.0.1/32"]          # required; CIDR list
auto_provision = false                      # default

[collab.auth.password]
# no fields required; store path is fixed (.zetl/collab/passwords.json)

[collab.auth.oidc]
issuer                 = "https://accounts.google.com"
client_id              = "..."
client_secret_file     = "~/.config/zetl/oidc-secret"  # path, never inline
user_id_claim          = "email"            # default "email"
auto_provision         = false               # default
provision_domain_allow = ["example.com"]     # required if auto_provision

[collab.auth.capability_url]
default_ttl = "7d"                           # default; bounded per REQ-4117
max_ttl     = "90d"                           # default upper bound
```

**Pre-conditions:** Read once at startup.

**Post-conditions:** (REQ-4102) `methods` is parsed into an ordered list; an
unknown method name fails startup. (REQ-4114) Each named method's required
sub-table fields are present and well-typed, or startup fails naming the key.

**Implements:** [[#REQ-4102]], [[#REQ-4114]], [[#REQ-4120]]. **Verified by:**
[[#TEST-4102]], [[#TEST-4114]], [[#TEST-4120]].

### CON-4103: `auth_resolve` Middleware

Runs the chain in order; on the first `Authenticated`, inserts the
[[Principal]] into request extensions and proceeds; on `Reject`, returns the
generic failure; if all `Abstain`, inserts `None` and proceeds (so `collab_gate`
produces the redirect/401). MUST be layered before `collab_gate` and
`csrf_guard`.

**Pre-conditions:** Layered exactly once, ahead of `collab_gate`.

**Post-conditions:** (REQ-4104) every downstream handler sees exactly one
`Option<Principal>` extension, set by this middleware and by no other code.
(NFR-4104) the chosen authenticator is the first non-abstaining one in
`methods` order.

**Implements:** [[#REQ-4104]], [[#NFR-4104]]. **Verified by:** [[#TEST-4104]],
[[#TEST-NFR-4104]].

### CON-4104: HTTP — `/auth/oidc/login` + `/auth/oidc/callback`

**Grammar:** `/auth/oidc/callback` accepts a query string recognised against:

```abnf
callback-query = param *( "&" param )
param          = "code=" code / "state=" state / "error=" errcode / other
code           = 1*256( unreserved / pct-encoded )
state          = 43*43( base64url-char )      ; 256-bit opaque, fixed length
errcode        = 1*64( ALPHA / "_" )
other          = token "=" *( unreserved / pct-encoded )   ; ignored, not acted on
```

Unrecognised input → generic auth-failure; no partial action is taken on a
malformed callback.

`GET /auth/oidc/login` → 302 to the [[IdP]] authorization endpoint with `code`
response type, [[PKCE]] `code_challenge`, `state`, `nonce`.
`GET /auth/oidc/callback` → recognise query, validate `state`, exchange code
with [[PKCE]] verifier, validate the [[ID Token]] ([[#REQ-4110]]), provision per
[[#REQ-4111]], mint a session, 302 to the originally-requested path. Both
mounted in `auth_routes`, rate-limited, ungated.

**Pre-conditions:** `collab-oidc` feature compiled; `[collab.auth.oidc]` valid.

**Post-conditions:** (REQ-4109) success mints a `SessionStore` session.
(REQ-4110) any of {bad `state`, bad `nonce`, wrong `aud`, expired token, bad
signature, replayed `state`} → generic failure; operator log records the
specific cause. (REQ-4113) routes are public + rate-limited.

**Implements:** [[#REQ-4109]], [[#REQ-4110]], [[#REQ-4113]], [[#REQ-4120]].
**Verified by:** [[#TEST-4109]], [[#TEST-4110]], [[#TEST-4120]].

### CON-4105: HTTP — `/auth/password`

**Grammar:** `POST /auth/password` accepts an `application/x-www-form-urlencoded`
body recognised against:

```abnf
form-body = "user=" user "&" "password=" password
user      = 1*64( unreserved / pct-encoded )
password  = 1*256( pct-encoded / unreserved )   ; opaque; length-bounded
```

Bodies that do not match (missing field, extra field, over-length) are rejected
before any hash computation.

`GET /auth/password` → login form. `POST /auth/password` → recognise body,
constant-time [[argon2id]] verify; on success mint session + `Set-Cookie`, 302
to target; on failure re-render the form with a generic message. Mounted in
`auth_routes`, rate-limited.

**Pre-conditions:** `passwords.json` readable, mode 0600.

**Post-conditions:** (REQ-4107) success ⇒ `SessionStore` session; failure is
cause-indistinguishable (unknown user ≡ wrong password) at the user-visible
layer and in timing. (REQ-4113) route public + rate-limited.

**Implements:** [[#REQ-4107]], [[#REQ-4113]], [[#REQ-4120]]. **Verified by:**
[[#TEST-4107]], [[#TEST-4120]].

### CON-4106: CLI — `zetl collab passwd`

**Endpoint:** `zetl collab passwd add <user> | remove <user> | list`. Password
read from a TTY prompt only — never argv, never env.

**Pre-conditions:** vault collab-initialised; `passwords.json` writable.

**Post-conditions:** (REQ-4108) `add` creates the `UserProfile` if absent and
writes/updates the [[argon2id]] [[PHC string]] record under an advisory file
lock; `remove` deletes the record; `list` prints user ids only, never hashes.

**Error model:** non-zero exit + stderr for: vault not collab, permissions error
on `passwords.json`, `remove` of an unknown user.

**Implements:** [[#REQ-4108]]. **Verified by:** [[#TEST-4108]].

### CON-4107: On-Disk — Password Store

Path `.zetl/collab/passwords.json`, mode 0600 (enforced + checked, as
`server.key` is in `src/user/invite.rs`). Schema:
`[{ "user_id": "<id>", "phc": "$argon2id$v=19$m=...,t=...,p=...$<salt>$<hash>" }]`.
Mutated by the CLI under the same advisory file lock used for
`used-nonces.json`.

**Implements:** [[#REQ-4107]], [[#ADR-4106]]. **Verified by:** [[#TEST-4107]],
[[#TEST-4108]].

### CON-4108: Proxy-Header Authenticator Behaviour

**Grammar:** the configured `user_header` value is recognised against:

```abnf
proxy-user = 1*256( ALPHA / DIGIT / "." / "_" / "-" / "@" / "+" )
```

A value that does not match is rejected (the request abstains/rejects) — it is
*not* normalised or trimmed into shape ([[#REQ-4120]]).

On each request: if proxy trust is off → `Abstain` (and startup should already
have failed per [[#REQ-4106]]). If the peer IP ∉ `peer_allow` → `Abstain`
(ignore the header entirely). Else recognise `user_header`; absent/empty/
ungrammatical → `Abstain`; valid → map to `user_id`, provision per
[[#REQ-4111]], return `Authenticated` (stateless principal).

**Pre-conditions:** proxy trust enabled; `peer_allow` non-empty.

**Post-conditions:** (REQ-4105) a grammatical header from an allowlisted peer
authenticates. (REQ-4106) a header from any other peer, or absent proxy trust,
never authenticates. (REQ-4112) the principal is stateless.

**Implements:** [[#REQ-4105]], [[#REQ-4106]], [[#REQ-4111]], [[#REQ-4112]],
[[#REQ-4120]]. **Verified by:** [[#TEST-4105]], [[#TEST-4106]], [[#TEST-4120]].

### CON-4109: Capability-URL Token & Authenticator

**Grammar:** the `cap` query parameter is recognised against the shared
[[EdDSA]] [[JWT]] grammar in `auth/token.rs`:

```abnf
cap-token   = header "." payload "." signature
header      = 1*( base64url-char )          ; {"alg":"EdDSA","typ":"JWT"}
payload     = 1*( base64url-char )          ; claims object, see below
signature   = 86*86( base64url-char )       ; 64-byte Ed25519 sig, unpadded
```

The decoded `payload` MUST be a JSON object recognised against the claims
schema — `sub` (= `"zetl-capability"`, checked *before* any other claim is
trusted), `scope` (glob string), `role` (`"reader"` | `"editor"`), `exp` (Unix
integer), `jti` (128-bit hex). Unknown claim keys are rejected
(`deny_unknown_fields`). A token that fails recognition at any stage yields the
generic failure with no partial action ([[#REQ-4120]]).

On each request: if `?cap=` is absent → `Abstain`. Else recognise the token;
verify the signature against `server.key`; if `sub ≠ "zetl-capability"`,
signature invalid, `exp` passed, or `jti` ∈ the revocation set → `Abstain`
(generic — not `Reject`, so a stale link does not poison the chain for a user
who also holds another credential). Else return `Authenticated` with a
pseudonymous, scope-capped [[Principal]] ([[#REQ-4119]]).

**Pre-conditions:** `server.key` loadable; revocation set readable.

**Post-conditions:** (REQ-4116) a validly-signed unexpired unrevoked token
authenticates as a bearer capability. (REQ-4117) the issued [[Principal]]'s
authority equals the token's `scope`+`role` and an expired token never
authenticates. (REQ-4118) a revoked `jti` never authenticates, propagating
within the [[#REQ-4118]] bound. (REQ-4119) the principal carries no
`UserProfile`, is never elevated, and never satisfies `admin_gate`.

**Implements:** [[#REQ-4116]], [[#REQ-4117]], [[#REQ-4118]], [[#REQ-4119]],
[[#REQ-4120]]. **Verified by:** [[#TEST-4116]], [[#TEST-4117]], [[#TEST-4118]],
[[#TEST-4119]], [[#TEST-4120]].

### CON-4110: CLI — `zetl collab share`

**Endpoint:**
`zetl collab share --scope <glob> --role <reader|editor> [--expires <duration>]`
to mint; `zetl collab share --list` to enumerate live tokens (by `jti`, scope,
role, expiry); `zetl collab share --revoke <jti>` to revoke.

**Pre-conditions:** vault collab-initialised; the invoking operator has invite
permission; `--expires` within the [[#REQ-4117]] bounds.

**Post-conditions:** (REQ-4116) `share` prints exactly one `?cap=`-bearing URL
plus a bearer-authority security notice on stderr. (REQ-4117) the minted token
carries the requested scope/role and an `exp` within bounds. (REQ-4118)
`--revoke` adds the `jti` to the persisted revocation set; `--list` reflects
live vs revoked.

**Error model:** non-zero exit + stderr for: vault not collab, operator lacks
permission, `--expires` out of bounds, scope glob malformed, `--revoke` of an
unknown `jti`.

**Implements:** [[#REQ-4116]], [[#REQ-4117]], [[#REQ-4118]]. **Verified by:**
[[#TEST-4116]], [[#TEST-4117]], [[#TEST-4118]].

---

## 8. Test Specifications

> Per [[PROTO-001]] §Selecting a Verification Strategy: this is an
> AI-synthesised, Tier-1, security-critical specification — so
> requirement-targeted test decomposition (positive / negative-input /
> negative-output), mutation testing, fuzzing of every recogniser, and
> adversarial testing are all **mandatory**. The table is the index; each
> TEST-### records its `Validates:` attribution per [[PROTO-001]] §Traceability.

| ID                    | Technique                     | Target                                                                              | Validates                          |
| --------------------- | ----------------------------- | ----------------------------------------------------------------------------------- | ---------------------------------- |
| [[#TEST-4101]]        | example + contract            | Passkey & agent-token re-expressed as `Authenticator`, behaviour unchanged           | [[#REQ-4101]]                      |
| [[#TEST-4102]]        | example + neg-input           | `methods` parsed + ordered; unknown method → startup error                          | [[#REQ-4102]], [[#REQ-4114]]       |
| [[#TEST-4103]]        | snapshot / regression         | No `[collab.auth]` block ⇒ full [[SPEC-020]] auth suite green                        | [[#REQ-4103]]                      |
| [[#TEST-4104]]        | example                       | `Principal` resolved once; extractors read the extension, never re-parse             | [[#REQ-4104]]                      |
| [[#TEST-4105]]        | example                       | Grammatical `X-Forwarded-User` from an allowlisted peer authenticates               | [[#REQ-4105]]                      |
| [[#TEST-4106]]        | example + property            | Header from non-allowlisted peer ignored; `proxy-header` w/o trust → startup error; spoofed direct header rejected | [[#REQ-4106]] |
| [[#TEST-4107]]        | example + neg-output          | argon2id verify happy path + wrong password + unknown user, all generic              | [[#REQ-4107]]                      |
| [[#TEST-4107-timing]] | timing side-channel           | Unknown-user vs wrong-password response timing indistinguishable                     | [[#REQ-4107]]                      |
| [[#TEST-4108]]        | example                       | `passwd add/remove/list`; password never in argv/env; profile created                | [[#REQ-4108]]                      |
| [[#TEST-4109]]        | example                       | [[OIDC]] code flow end-to-end against a mock [[IdP]]                                | [[#REQ-4109]]                      |
| [[#TEST-4110]]        | example + property            | Bad `state`, bad `nonce`, wrong `aud`, expired token, replayed `state` all rejected  | [[#REQ-4110]]                      |
| [[#TEST-4111]]        | example + neg-input           | `auto_provision` on → Reader profile; off → reject; domain not in allowlist → reject | [[#REQ-4111]]                      |
| [[#TEST-4112]]        | example                       | Stateless principals bypass `csrf_guard`; cookie principals do not                   | [[#REQ-4112]]                      |
| [[#TEST-4113]]        | example                       | Each authenticator's routes mounted, public, rate-limited                            | [[#REQ-4113]]                      |
| [[#TEST-4114]]        | example + neg-input           | Each [[#REQ-4114]] misconfiguration produces a named startup error                   | [[#REQ-4114]]                      |
| [[#TEST-4115]]        | example                       | Audit log records success/abstain/reject; contains no secrets                        | [[#REQ-4115]]                      |
| [[#TEST-4116]]        | example                       | Validly-signed unexpired unrevoked `?cap=` token authenticates as a bearer capability| [[#REQ-4116]]                      |
| [[#TEST-4117]]        | example + property            | Issued authority = token scope+role; expired token never authenticates; TTL bounds enforced | [[#REQ-4117]]               |
| [[#TEST-4118]]        | example                       | Revoked `jti` rejected; revocation propagates within bound; generic at user layer    | [[#REQ-4118]]                      |
| [[#TEST-4119]]        | example + property            | Capability principal: no `UserProfile`, never elevated, never satisfies `admin_gate` | [[#REQ-4119]]                      |
| [[#TEST-4120]]        | fuzz + property (roundtrip)   | Every recogniser (config, callback query, password body, proxy header, cap token) against arbitrary bytes: never panics, never bypasses, `parse∘serialise == id` where applicable | [[#REQ-4120]] |
| [[#TEST-NFR-4101]]    | benchmark                     | `auth_resolve` ≤ 1 ms 95p for non-redirect methods                                   | [[#NFR-4101]]                      |
| [[#TEST-NFR-4102]]    | build / lint                  | Default `collab` build gains no deps; [[OIDC]] crates only under `collab-oidc`        | [[#NFR-4102]]                      |
| [[#TEST-NFR-4103]]    | benchmark                     | argon2id verify within the tuned cost band                                           | [[#NFR-4103]]                      |
| [[#TEST-NFR-4104]]    | property                      | Chosen authenticator == first non-abstaining, for random chains/requests             | [[#NFR-4104]]                      |
| TEST-mutation-resolve | mutation                      | Mutation kill rate ≥ 90% on `auth_resolve` + chain builder                           | [[#REQ-4104]] (robustness)         |
| TEST-mutation-token   | mutation                      | Mutation kill rate ≥ 90% on the shared [[EdDSA]] [[JWT]] recogniser + cap validation  | [[#REQ-4116]], [[#REQ-4120]]       |
| TEST-mutation-oidc    | mutation                      | Mutation kill rate ≥ 90% on [[OIDC]] [[ID Token]] validation                         | [[#REQ-4110]]                      |
| TEST-adversarial-041  | adversarial (cross-model)     | Fresh-context adversary attacks the REQ set for admitted-but-unintended behaviour     | all REQ-41xx                       |

---

## 9. Observability Signals

| ID             | Type   | Signal                                                                                     | Trace                              |
| -------------- | ------ | ------------------------------------------------------------------------------------------ | ---------------------------------- |
| [[#OBS-4101]]  | metric | `zetl_collab_auth_resolve_duration_seconds` histogram, label `method`                      | [[#NFR-4101]]                      |
| [[#OBS-4102]]  | metric | `zetl_collab_auth_outcomes_total{method, outcome}` (outcome ∈ authenticated/abstain/reject)| [[#REQ-4101]], [[#REQ-4115]]       |
| [[#OBS-4103]]  | log    | Operator-channel line per authentication decision, with method + cause category            | [[#REQ-4115]]                      |
| [[#OBS-4104]]  | log    | Audit line on success ([[Principal]] identity, method) and on reject (cause category)      | [[#REQ-4115]]                      |
| [[#OBS-4105]]  | log    | Startup line enumerating the assembled chain + compiled auth features                      | [[#REQ-4102]], [[#REQ-4114]]       |
| [[#OBS-4106]]  | metric | `zetl_collab_capability_tokens{state}` gauge (state ∈ live/expired/revoked)                 | [[#REQ-4118]]                      |

> The `cause` label/field on reject signals is **operator-channel only**; it
> MUST NOT be exposed on any unauthenticated HTTP-readable metrics endpoint, to
> avoid building a user-enumeration oracle that defeats [[#REQ-4107]]'s
> indistinguishability property.

---

## 10. Purity Boundary Map

> **`[Provisional — refined by [[DESIGN-041-pluggable-collab-auth]] task
> purity-boundary-map]`**

### Pure Core (no I/O, no shared state, deterministic)

- `web::auth::config::parse(toml_body) -> CollabAuthConfig` — [[TOML]] lens
  parse + structural validation; total function ([[#CON-4102]]).
- `web::auth::config::validate(cfg, compiled_features, trust_proxy)
  -> Result<(), AuthConfigError>` — the [[#REQ-4114]] rule set, pure.
- `web::auth::token::recognise(s) -> Result<EdDsaJwt, TokenError>` — the shared
  [[EdDSA]] [[JWT]] recogniser ([[#REQ-4120]]); pure, the single parser for the
  agent token, the [[SPEC-020]] invite, and the capability token.
- `web::auth::capability::decide(token, server_pubkey, now, revoked) ->
  AuthOutcome` — capability validation ([[#CON-4109]]); pure given its inputs.
- `web::auth::proxy_header::decide(header_val, peer_ip, cfg) -> AuthOutcome` —
  the [[#REQ-4106]] trust logic; pure, exhaustively testable.
- `web::auth::oidc::validate_id_token(token, jwks, expectations)
  -> Result<Claims, OidcError>` — signature/aud/exp/nonce checks; pure given the
  [[JWKS]] and expectations.
- `web::auth::password::verify(submitted, phc) -> bool` — [[argon2id]] verify;
  pure (CPU-bound, deterministic).
- `web::auth::{AuthOutcome, Principal, CapabilityGrant, AuthRejection}` — plain
  data.

### Effectful Shell (orchestrates I/O, calls pure core)

- `web::auth::resolve` — the `auth_resolve` middleware; reads the request, walks
  the chain, writes the extension.
- `web::auth::passkey` / `agent_token` — touch `SessionStore`, profile files,
  the recovery-pubkey verification path.
- `web::auth::password` store I/O — `passwords.json` read/lock/write.
- `web::auth::capability` store I/O — `server.key` load, revocation-set
  read/write, the `zetl collab share` CLI front.
- `web::auth::oidc` transport — [[JWKS]] fetch, token-endpoint exchange,
  redirect handling.
- `web::auth::provision` — reads/writes `UserProfile` files.
- `web::auth::audit` — operator log + audit log emission ([[#OBS-4103]],
  [[#OBS-4104]]).

### Boundary Contracts (data types crossing the boundary)

- `CollabAuthConfig` (shell parses bytes → pure validates → shell builds chain)
- `EdDsaJwt`, `Claims`, `CapabilityGrant` (pure → shell)
- `Principal` (pure → shell, into request extensions)
- `AuthOutcome` (pure → shell)

### Dependency Rule

Shell modules MAY import pure-core modules; the reverse MUST NOT hold. Enforced
with `clippy::disallowed_methods` on `std::fs::*`, `SystemTime::now`,
`tokio::*`, and HTTP-client crates within the pure modules — the mechanism
[[SPEC-036]] §11 specifies.

---

## 11. Threat Model (Summary)

> Detailed model lives in `research/SPEC-041-threat-model.md`, produced by
> [[DESIGN-041-pluggable-collab-auth]] task `threat-model`. This section
> summarises adversaries.

### Threat Model A — Forwarded-Header Spoofer

A client reaches the server directly (bypassing the proxy) and sets
`X-Forwarded-User` itself. **Mitigation:** [[#REQ-4106]] — `proxy-header`
engages only behind proxy trust *and* a `peer_allow` peer; the header is ignored
on every other peer; misconfiguration is a startup error. **Residual risk:** an
attacker who can originate traffic from inside `peer_allow` — documented;
[[mTLS]] noted as the stronger control.

### Threat Model B — OIDC Flow Attacks

[[CSRF]] on the callback, authorization-code injection/replay, [[ID Token]]
substitution, [[IdP]]-mixup. **Mitigation:** [[#REQ-4110]] — single-use `state`
and `nonce`, per-request [[PKCE]] verifier, pinned `issuer`, full [[ID Token]]
validation. **Residual risk:** a compromised [[IdP]] can mint identities — out
of scope; bounded by [[#REQ-4111]]'s Reader-only auto-provisioning and
`provision_domain_allow`.

### Threat Model C — Auto-Provisioning Privilege Escalation

A misconfigured [[IdP]] client or over-broad proxy authenticates unintended
principals. **Mitigation:** [[#REQ-4111]] / [[#ADR-4107]] — auto-provisioning is
opt-in, capped at Reader, and (for [[OIDC]]) domain-gated. Capability-URL
principals are *never* provisioned ([[#REQ-4119]]). The blast radius of any
provisioning mistake is read access.

### Threat Model D — Password Endpoint Abuse

Online guessing, user enumeration, login-endpoint DoS. **Mitigation:**
[[#REQ-4107]] constant-time + cause-indistinguishable verification; existing
`AuthRateLimiters` on `/auth/password`; [[#NFR-4103]] [[argon2id]] cost band
sized to resist offline cracking without enabling DoS amplification.

### Threat Model E — Method-Downgrade / Chain Confusion

An attacker shapes a request to be picked up by a weaker authenticator than
intended. **Mitigation:** [[#NFR-4104]] deterministic first-match-wins;
authenticators `Abstain` rather than guess; the operator owns the order; the
assembled chain is logged at startup ([[#OBS-4105]]).

### Threat Model F — Server-Key Compromise

`server.key` signs the agent token, the [[SPEC-020]] invite, *and* the
SPEC-041 capability token ([[#ADR-4109]]). Its compromise forges all three.
**Mitigation:** unchanged from [[SPEC-020]] — 0600 file-permission enforcement,
self-host operational hygiene; capability tokens additionally bounded by
short TTL ([[#REQ-4117]]) and revocation ([[#REQ-4118]]). **Residual risk:**
documented; key rotation is an operational procedure, not solved here.

### Threat Model G — Secret Exposure in Logs / Process State

Passwords, tokens, [[ID Token]]s, [[PKCE]] verifiers, client secrets, capability
tokens leaking into logs, argv, env, or audit trails. **Mitigation:**
[[#REQ-4108]] / [[#REQ-4115]] / [[#ADR-4110]] — TTY-only password entry,
`client_secret_file` (path, never inline), the `?cap=` token stripped from
server logs, an explicit log-redaction contract, audit lines carrying
categories not material.

### Threat Model H — Capability-URL Leakage *(the explicit trade-off)*

A `?cap=` URL *is* a [[Bearer Token]] ([[#ADR-4110]]). It can leak via
forwarding, screen-share, browser history, the `Referer` header, intermediary
access logs, or link previewers — exactly the property [[SPEC-036]] §1 raises
against the legacy JWT-URL flow. SPEC-041 does not claim to eliminate this; it
**bounds and discloses** it: opt-in only; scope-bound and role-bound so a leak
exposes only the granted slice ([[#REQ-4117]]); short default TTL and a
hard max ([[#REQ-4117]]); operator-driven revocation ([[#REQ-4118]]);
`Referrer-Policy: no-referrer` on capability-URL responses; the token stripped
from logs ([[#REQ-4115]]); a bearer-authority security notice printed at mint
time ([[#REQ-4116]]); and operator documentation that names the method's
sensitivity ceiling and points at [[OIDC]] / passkey above it. **Residual
risk:** within the TTL, anyone holding the URL has the granted access — stated
plainly. The [[DESIGN-041-pluggable-collab-auth]] `threat-model` task quantifies
the leakage vectors and confirms the mitigation set is complete.

---

## 12. Quality Attribute Checklist

> **`[Provisional — [[DESIGN-041-pluggable-collab-auth]] task
> phase1-quality-gates finalises]`** Applied to each REQ in [[#4. Functional
> Requirements]].

| REQ            | Unambiguous | Verifiable | Atomic | Consistent | Quantified         | Traceable | Error-aware |
| -------------- | :---------: | :--------: | :----: | :--------: | :----------------: | :-------: | :---------: |
| [[#REQ-4101]]  | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |
| [[#REQ-4102]]  | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |
| [[#REQ-4103]]  | ✓ | ✓ | ✓ | ✓ | n/a (binary) | ✓ | n/a |
| [[#REQ-4104]]  | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |
| [[#REQ-4105]]  | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |
| [[#REQ-4106]]  | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |
| [[#REQ-4107]]  | ✓ | ✓ | ✓ | ✓ | n/a + [[#NFR-4103]] | ✓ | ✓ |
| [[#REQ-4108]]  | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |
| [[#REQ-4109]]  | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |
| [[#REQ-4110]]  | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |
| [[#REQ-4111]]  | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |
| [[#REQ-4112]]  | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | n/a |
| [[#REQ-4113]]  | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | n/a |
| [[#REQ-4114]]  | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |
| [[#REQ-4115]]  | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |
| [[#REQ-4116]]  | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |
| [[#REQ-4117]]  | ⚠ provisional TTL bounds | ✓ | ✓ | ✓ | ⚠ provisional | ✓ | ✓ |
| [[#REQ-4118]]  | ⚠ provisional propagation bound | ✓ | ✓ | ✓ | ⚠ provisional | ✓ | ✓ |
| [[#REQ-4119]]  | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |
| [[#REQ-4120]]  | ✓ | ✓ | ✓ | ✓ | n/a | ✓ | ✓ |

Provisional ⚠ entries close once the named [[DESIGN-041-pluggable-collab-auth]]
task completes.

---

## 13. Implementation Plan

> Phased so the de-risking refactor lands and proves itself *before* any new
> authentication surface exists. Each phase is independently shippable and
> gated by its own tests. The detailed task DAG lives in
> [[DESIGN-041-pluggable-collab-auth]]; this section is the human-readable
> summary. Per [[PROTO-001]] §AI Trust Boundaries, **no phase begins** before
> the Tier-1 human-expert review of this specification.

### Phase 0 — Behaviour-Preserving Refactor *(no new methods)*

**Goal:** introduce the seam with provably zero behaviour change.

- New `src/web/auth/` module ([[#ADR-4108]]): `mod.rs` ([[Authenticator]],
  [[Principal]], `AuthOutcome`), `resolve.rs` (`auth_resolve` middleware),
  `token.rs` (the shared [[EdDSA]] [[JWT]] recogniser, extracted from
  `src/user/invite.rs` without changing its behaviour).
- `passkey.rs` and `agent_token.rs` — wrap the *existing* logic from
  `src/web/session.rs` (`SessionStore::validate`, `verify_bearer_token`) behind
  the trait. No crypto or storage changes.
- Thin `collab_gate` / `admin_gate` / extractors to read the [[Principal]]
  request extension ([[#ADR-4102]]); delete `admin_gate`'s parallel resolution.
- Wire `auth_resolve` into the router in `src/web/mod.rs` before `collab_gate`;
  add a middleware-ordering test.
- **Gate:** the entire existing [[SPEC-020]] / collab test suite passes
  unchanged ([[#TEST-4103]], [[#TEST-4101]], [[#TEST-4104]]).
- **Touches:** `src/web/session.rs`, `src/web/mod.rs`, `src/user/invite.rs`
  (extract recogniser), new `src/web/auth/*`.

### Phase 1 — Config & Chain Assembly

**Goal:** make the chain config-driven; the default reproduces Phase 0.

- `src/web/auth/config.rs` — `CollabAuthConfig` [[TOML]] lens ([[#CON-4102]]),
  `methods` parsing, `validate()` ([[#REQ-4114]] rules that apply so far),
  `deny_unknown_fields` within `[collab.auth]`.
- Chain builder: `CollabAuthConfig` → `Vec<Box<dyn Authenticator>>`; an absent
  block ⇒ `["passkey", "agent-token"]`.
- Startup wiring + [[#OBS-4105]] chain-enumeration log line.
- **Gate:** [[#TEST-4102]], [[#TEST-4114]] (partial), [[#TEST-NFR-4104]].

### Phase 2 — Reverse-Proxy Header Authenticator

**Goal:** prove the seam with the smallest real method — no crypto, no new
dependencies.

- `src/web/auth/proxy_header.rs` + `provision.rs` (auto-provisioning, shared
  with Phases 4–5).
- `[collab.auth.proxy_header]` schema; the [[#REQ-4106]] trust gate + startup
  validation; `peer_allow` [[CIDR]] matching; the header-value recogniser
  ([[#CON-4108]]).
- **Gate:** [[#TEST-4105]], [[#TEST-4106]], [[#TEST-4111]] (proxy path),
  [[#TEST-4120]] (proxy-header recogniser fuzz).

### Phase 3 — Static Password Authenticator

**Goal:** the homelab path.

- `src/web/auth/password.rs` — [[argon2id]] verify, `passwords.json` store
  ([[#CON-4107]]), 0600 enforcement, the form-body recogniser ([[#CON-4105]]).
- `/auth/password` GET/POST routes via `Authenticator::routes()`.
- `zetl collab passwd add/remove/list` CLI ([[#CON-4106]]) — TTY-only entry.
- **Gate:** [[#TEST-4107]], [[#TEST-4107-timing]], [[#TEST-4108]],
  [[#TEST-NFR-4103]], [[#TEST-4120]] (form-body fuzz).

### Phase 4 — Capability-URL Authenticator

**Goal:** the "anyone with the link" path; reuses the Phase-0 [[EdDSA]]
recogniser, no new dependencies.

- `src/web/auth/capability_url.rs` — capability claims schema atop the shared
  `token.rs` recogniser ([[#ADR-4109]], [[#CON-4109]]); `?cap=` query
  recognition; signature + `sub` + `exp` + revocation checks; the pseudonymous
  scope-capped [[Principal]] ([[#REQ-4119]]); `Referrer-Policy: no-referrer` on
  capability-URL responses ([[#ADR-4110]]).
- Revocation set persisted under `.zetl/collab/` ([[#REQ-4118]]).
- `zetl collab share --scope --role --expires | --list | --revoke` CLI
  ([[#CON-4110]]) with the bearer-authority security notice.
- [[SPL]]/`acl.rs` confirmation that a capability-attribute subject needs no
  semantic change (the `acl-integration` plan task).
- **Gate:** [[#TEST-4116]]–[[#TEST-4119]], [[#TEST-4120]] (cap-token fuzz),
  `TEST-mutation-token`.

### Phase 5 — OIDC / OAuth2 Authenticator

**Goal:** the workspace-federated path; isolated behind a cargo feature.

- New `collab-oidc` cargo feature; add the [[OAuth2]]/[[OIDC]] + HTTP-client
  deps under it only ([[#ADR-4105]], [[#NFR-4102]]).
- `src/web/auth/oidc.rs` (`#[cfg(feature = "collab-oidc")]`) —
  [[Authorization Code Flow]] + [[PKCE]], `/auth/oidc/login` + `/callback`
  routes, [[JWKS]] fetch + [[ID Token]] validation ([[#REQ-4110]]), the callback
  query recogniser ([[#CON-4104]]), claim → `user_id`, domain-gated
  auto-provision.
- `--version` lists compiled auth features.
- **Gate:** [[#TEST-4109]], [[#TEST-4110]], [[#TEST-4111]] (oidc path),
  [[#TEST-NFR-4102]], `TEST-mutation-oidc`, [[#TEST-4120]] (callback fuzz),
  against a mock [[IdP]].

### Phase 6 — Docs, Audit & Hardening

- `docs/collab-auth.md` — operator guide per method, the proxy
  header-stripping requirement, the capability-URL sensitivity ceiling, the
  chain mental model. Derived per [[PROTO-001]] §Documentation.
- `research/SPEC-041-threat-model.md` — the full threat model.
- [[#OBS-4101]]–[[#OBS-4106]] instrumentation; the audit-log redaction contract
  ([[#REQ-4115]]).
- `TEST-adversarial-041` — assemble the cross-model adversarial-review package
  for the Tier-1 human-expert gate ([[PROTO-001]] §Multi-Model Cognitive
  Diversity).
- CHANGELOG entry; specification status → `implemented`.

### Sequencing Rationale

Phase 0 carries all the architectural risk and ships with zero new behaviour —
if it cannot be made behaviour-preserving, the whole approach is reconsidered
before any feature work. Phases 2→3→4→5 are ordered by ascending cost and
dependency weight (no-deps header method → local-crypto password → reuses the
Phase-0 [[EdDSA]] recogniser for capability URLs → external-deps + network for
[[OIDC]]), so the seam is validated by three simpler methods before [[OIDC]]'s
complexity lands on it.

---

## 14. Status & Next Actions

- This strawman is an **input** to [[DESIGN-041-pluggable-collab-auth]], not an
  output. The plan's tasks refine every `[Provisional]` section and finalise
  the REQ/CON/ADR IDs against the highest existing IDs at draft time.
- **No implementation begins** until: (a) the Phase 1 + Phase 2 quality gates
  pass; (b) cross-model adversarial review completes ([[PROTO-001]]
  Constitutional Principle 12); (c) the human-expert review package is
  approved — [[Authentication]] is a [[PROTO-001]] §AI Trust Boundaries Tier-1
  no-go area.
- The capability-URL method ([[#REQ-4116]]–[[#REQ-4119]]) carries the highest
  residual risk in this specification ([[#Threat Model H]]); the
  [[DESIGN-041-pluggable-collab-auth]] `threat-model` task and the human-expert
  review MUST explicitly sign off on the convenience/exposure trade-off before
  Phase 4 is scheduled.
- After review and refinement, this document is re-issued at version `0.1.0`,
  status `approved`, with the provisional markers removed.

---

## 15. Open Questions Surfaced by This Strawman

1. **Module move vs. additive.** Phase 0 proposes moving `collab_gate` /
   `csrf_guard` / `admin_gate` / extractors into `src/web/auth/`. A
   smaller-diff alternative leaves them in `session.rs` and only adds the trait
   + `resolve.rs`. The `module-layout` plan task decides.
2. **Capability-URL session-minting variant.** [[#REQ-4116]] keeps the
   capability principal stateless (the URL is re-presented each request). An
   alternative mints a short [[Session Token|session]] on first use so the
   token leaves the URL bar sooner. Trade-off: less URL-bar exposure vs. a
   second credential to revoke. The `threat-model` task decides; strawman
   default is stateless.
3. **`provision_domain_allow` for `proxy-header` too?** [[OIDC]] has it;
   `proxy-header` currently relies on the proxy to constrain identities. Should
   zetl offer a defence-in-depth identity allowlist there as well?
4. **Multiple [[OIDC]] providers.** One `[collab.auth.oidc]` table assumes a
   single [[IdP]]. Is multi-[[IdP]] (`[[collab.auth.oidc]]`) a v0.1 need or a
   deferral?
5. **[[mTLS]] as a sixth-plus method.** Out of scope here, but the
   [[Authenticator]] trait is designed to accommodate it. Confirm the trait
   surface ([[#CON-4101]]) needs nothing added for a future `mtls` method.
6. **Logout under [[OIDC]].** zetl logout clears the local session
   ([[#ADR-4104]]). Should it also support RP-initiated logout at the [[IdP]]?
   Likely a deferral; flag for the `threat-model` task.
7. **[[#NFR-4103]] cost band** (100–500 ms) and the [[#REQ-4117]] TTL bounds are
   provisional and hardware/policy dependent — the benchmark and `threat-model`
   tasks set the real numbers.
8. **`agent-token` and [[CSRF]].** [[#REQ-4112]] generalises the Bearer
   exemption; confirm no current route *relies* on the Bearer-specific check
   beyond what `issues_cookie_session() == false` captures.
9. **Capability-URL scope vs. [[SPL]] expressivity.** [[#ADR-4111]] passes the
   bound scope to [[SPL]] as a [[Principal]] attribute. Confirm [[SPL]] can
   express "this subject may read iff the page is within its capability glob"
   without new operators — the `acl-integration` task.
