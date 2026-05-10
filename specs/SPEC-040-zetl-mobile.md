---
title: "SPEC-040: zetl mobile — basic local-first app"
version: 0.1.0-strawman
status: strawman
date: 2026-05-10
audience: agent, human
parent: SPEC-001
related:
  - SPEC-009  # zetl view (TUI reader — sibling reading surface)
  - SPEC-020  # collab editing — git auto-commit-on-save reused conceptually
  - SPEC-036  # SPAKE2 onboarding (alternate seed-transfer path)
  - SPEC-037  # 3D space graph (graph view inherited unchanged from serve)
  - SPEC-038  # RSS / Atom feed support (sibling reading-side primitive)
plan: DESIGN-040-zetl-mobile
---

# SPEC-040: zetl mobile — basic local-first app

> **Strawman notice.** This document is a first-pass produced *before* the
> Phase 0 surveys, prior-art research, and synthetic-user simulations called
> for by [[DESIGN-040-zetl-mobile]] (`plans/DESIGN-040-zetl-mobile.spl`).
> Sections labelled **`[Provisional — refined by DESIGN-040 task X]`** are
> deliberate placeholders that the plan tasks will replace with grounded
> findings. Do not implement against this version. The version reaches
> `0.2.0` (status `draft`) only after Phase 1 + Phase 2 quality gates pass,
> and `1.0.0` (status `approved`) only after the Tier 2 cross-model review
> and human reviewer sign-off (per [[PROTO-001]] §AI Trust Boundaries
> §Multi-Model Cognitive Diversity).

## Information Table

| Field         | Value                                                                                                             |
| ------------- | ----------------------------------------------------------------------------------------------------------------- |
| Document ID   | SPEC-040                                                                                                          |
| Title         | zetl mobile — basic local-first app                                                                               |
| Version       | 0.1.0-strawman                                                                                                    |
| Status        | Strawman (not implementable; pending [[DESIGN-040-zetl-mobile]] execution)                                        |
| Author        | Agent (Claude Opus 4.7, [[PROTO-001]] v1.6.0)                                                                     |
| Date          | 2026-05-10                                                                                                        |
| Audience      | Agent, Human                                                                                                      |
| Trace         | [[PROTO-001]] §Phase 1, §Phase 2, §AI Trust Boundaries                                                            |
| Parent        | [[SPEC-001]] — zetl Bi-directional Link Graph CLI                                                                 |
| Related       | [[SPEC-009]], [[SPEC-020]], [[SPEC-036-spake2-onboarding]], [[SPEC-037-3d-space-graph]], [[SPEC-038-rss-support]] |
| Plan          | `plans/DESIGN-040-zetl-mobile.spl` ([[DESIGN-040-zetl-mobile]])                                                   |
| Review tier   | Tier 2 (core feature; SSH key material crosses a [[Trust Boundary]] and the embedded server runs untrusted-content fetches via the share-target path) |

---

## 1. Overview

zetl today is a Rust CLI plus a local web UI ([[zetl serve]]). Captures,
edits, and reads all happen on a workstation. A user away from their desk
has no way to drop a note or web-clip into the vault.

This specification introduces **zetl mobile** — a basic, [[Local-First]]
phone app for [[iOS]] and [[Android]], built with [[Tauri Mobile]], that
**embeds [[zetl serve]] as the mobile UI** rather than reinventing the
reading and editing surface. The phone holds a full git working tree on
device, the embedded server renders pages through the existing
[[Minijinja]] templates and theme contract, and a thin set of
mobile-specific routes (capture, onboarding, sync controls) is added to
serve. Edits commit locally; opportunistic `git push` propagates to peers
(desktop, other phones) over [[SSH]]. The app's primary purpose is
**on-the-go capture**, with reading and light editing as secondary modes.

### 1.1 Motivation

- **Capture latency.** A phone is the device that's actually in the
  user's pocket. The longer it takes to drop an idea or a clipped URL
  into the vault, the more friction kills the workflow.
- **Local-first by construction.** The phone must work offline without
  depending on any specific peer being reachable. [[zetl]] has always
  been an offline-first toolchain; the mobile surface preserves that
  property.
- **Reuse, not reinvention.** [[zetl serve]] already implements
  rendering, the [[CodeMirror 6]] editor, full-text search, backlinks,
  the [[transclusion]] panel, the responsive sidebar/graph behaviour
  ([[SPEC-001]] §Graph view §Mobile), and the [[Theme Contract]]
  ([[SPEC-001]] §Theme contract versioning). Embedding it in a Tauri
  Mobile shell is dramatically less work than building a parallel
  mobile UI, and it guarantees feature parity with desktop.
- **Auth comes free.** zetl already ships `zetl derive-ssh-key`, which
  derives an [[ed25519]] SSH key from a 12-word [[BIP39]] mnemonic
  ([[SPEC-001]] §Deterministic keys). The same seed phrase that
  authenticates the desktop authenticates the phone — no new
  authentication surface.

### 1.2 Why git is the database

The phone never speaks a zetl-specific server protocol to the outside
world. The git remote is the database; every peer (desktop, phone,
[[zetl serve]] on a server, CI builder) is a clone. This is consistent
with how zetl's `--collab` mode already auto-commits on every save
([[SPEC-020]]) and means the mobile app introduces zero new server
infrastructure beyond what the user already runs (or self-hosts on
[[Codeberg]] / [[Gitea]] / similar). Conflict resolution is delegated
to git; merge UIs on a phone screen are explicitly out of scope for
v0.1 (see [[#ADR-4004]]).

### 1.3 Design principles

1. **Embed, don't reimplement.** The mobile app embeds a single-user
   build of [[zetl serve]] and uses its existing Minijinja templates,
   themes, editor, search, and graph as the entire UI. Mobile-specific
   surfaces (capture, onboarding, sync controls) are new routes inside
   serve, not a parallel client.
2. **Capture is sacred.** No path through the system may lose a
   captured note. Atomic writes, share-extension payloads written
   before the share-sheet dismisses, and capture-queue rows drained
   only after a successful push (see [[#REQ-4008]] and [[#NFR-4002]]).
3. **Single source of truth.** The git remote is the database. The
   phone is one peer among many; no peer is privileged.
4. **Sync is explicit.** Pull on app open and push after save are
   the default; manual buttons are always available. No background
   sync daemons in v0.1 (see [[#ADR-4003]]).
5. **Boundaries are typed.** The [[Tauri Command]] surface is
   intentionally tiny: keychain access, git sync triggers, share-target
   handoff, and the embedded server's lifecycle. Everything UI-shaped
   goes through the embedded server.
6. **Conflict policy is humble.** v0.1 is fast-forward only. The phone
   is the worst place to merge text; it surfaces a "resolve on desktop"
   message instead (see [[#ADR-4004]]).

### 1.4 Scope

**In scope (v0.1):**

- iOS + Android, single [[Tauri Mobile]] codebase ([[#REQ-4001]])
- Full `git clone` of the vault on device ([[#REQ-4002]],
  [[#ADR-4002]])
- SSH auth via key derived from BIP39 seed ([[#REQ-4003]],
  [[#CON-4002]])
- **Embedded [[zetl serve]] as the UI host** — single-user mode,
  127.0.0.1-bound (or `tauri://` custom protocol), feature-gated
  build that disables `--collab` / `mcp` / `reason` / `history`
  ([[#REQ-4004]], [[#ADR-4001]])
- **Mobile-specific serve routes / templates** — `/_mobile/capture`,
  `/_mobile/onboarding`, `/_mobile/sync` ([[#REQ-4005]],
  [[#CON-4004]])
- **Capture flow** — quick-create from a [[Floating Action Button]]
  on the existing serve chrome and from the OS share sheet (iOS
  Share Extension, Android `ACTION_SEND`) ([[#REQ-4006]],
  [[#REQ-4007]])
- **Read & edit** — delegated to existing serve UI verbatim
  (Minijinja templates, themes, [[CodeMirror 6]] editor, backlinks,
  search, [[transclusion]] panel, [[Sigma.js]] graph below
  responsive breakpoint)
- **Sync flow** — auto pull-on-open (FF-only), opportunistic push
  after every save, manual pull/push controls in `/_mobile/sync`
  ([[#REQ-4009]], [[#REQ-4010]], [[#ADR-4003]])
- **Onboarding** — paste 12-word [[BIP39]] mnemonic, paste git
  remote URL, clone vault ([[#REQ-4011]])
- **Themes** — full desktop theme contract works unmodified;
  responsive CSS already collapses sidebar / docked graph below
  `--zetl-graph-widget-breakpoint`
- **Durability invariants** ([[#NFR-4002]])

**Out of scope (v0.1):**

- **No collab WebSocket / [[CRDT]].** [[SPEC-020]]'s [[Peritext]]
  engine and [[WebAuthn]] passkey flow are gated off in the mobile
  serve build — single-writer-per-page; git is the conflict ground
  truth.
- **No reasoning** (`zetl reason`). `--features reason` is disabled.
- **No `zetl serve` MCP server**. `--features mcp` is disabled.
- **No history feature**. `--features history` is disabled in v0.1
  to keep clone size and indexing cost predictable; revisit in v0.2.
- **No image / file attachments.** Text only. ([[git-LFS]] support
  is a future SPEC-040.x.)
- **No background sync.** Pull/push run only in foreground — no iOS
  `BGAppRefresh`, no Android `WorkManager` ([[#ADR-4003]]).
- **No sparse checkout.** Vault must fit on phone storage. v0.1
  documents a "fits on phone" assumption (see [[#NFR-4004]]).
- **No merge UI.** Non-FF pulls and divergent pushes block with a
  "resolve on desktop" message ([[#ADR-4004]]).
- **No mobile-specific theme contract.** The full desktop
  [[Theme Contract]] applies. Themes that override desktop-only
  templates (e.g. `_graph.html`) work unchanged; the responsive CSS
  already handles small viewports.
- **No app-store distribution decision.** This spec covers the app;
  the release channel (TestFlight / direct APK / store) is decided
  in [[DESIGN-040-zetl-mobile]].

### 1.5 Risks and open questions (the plan resolves these)

- **Tauri Mobile maturity (as of 2026-05).** [Provisional — refined
  by [[DESIGN-040-zetl-mobile#task-tauri-mobile-maturity-survey]]] —
  document the actual state of iOS/Android signing, plugin ecosystem,
  and known issues so [[#NFR-4005]] (build pipeline reliability) has
  a measurable target.
- **In-process server vs custom protocol handler.** [[Tauri]]
  supports `tauri://` custom protocols that bypass HTTP entirely.
  Whether to bind the embedded serve to `127.0.0.1:<port>` or to
  serve responses through a Tauri custom protocol is a security /
  perf trade-off resolved in
  [[DESIGN-040-zetl-mobile#task-server-binding]].
- **`--features serve` modularity.** Compiling [[zetl serve]] without
  `--collab` / `mcp` / `reason` / `history` may need feature-flag
  refactoring in the existing crate. Scoped in
  [[DESIGN-040-zetl-mobile#task-feature-gates]].
- **Vault size envelope.** A 5 000-page vault clones fine on mobile
  networks; a 50 000-page vault may strain mobile storage and clone
  time. Sparse checkout is a v0.2 question. [[#NFR-4004]].
- **Share-extension UI scope.** Minimal vs richer. v0.1 spec assumes
  minimal (write to app-group inbox, finalize on relaunch). A richer
  share-ext UI is a v0.2 improvement.

---

## 2. User profiles

> Full profiles live in `users/zetl-mobile-capturer/user.md`,
> `users/zetl-mobile-reader/user.md`, and
> `users/zetl-mobile-cross-device-editor/user.md`. The strawman
> summarises them inline; full profiles are produced by
> [[DESIGN-040-zetl-mobile#task-user-profiles]].

### 2.1 UP-4001 On-the-go capturer (primary)

**Role:** Existing zetl user with a vault on desktop, who wants to drop
notes and clipped URLs from a phone whenever an idea strikes.

**Goals:** capture in <5 seconds from cold-launch to "saved"; capture
from any app via the share sheet; trust that captures are durable
through airplane-mode periods, low battery, and force-quits.

**Constraints (provisional):** intermediate technical fluency;
comfortable with git on desktop; familiar with zetl's [[wikilink]]
vocabulary.

**Daily workflow:** opens app once or twice a day to capture or briefly
read; never does sustained writing on the phone; expects edits to land
on desktop within hours, not seconds.

### 2.2 UP-4002 Mobile reader

**Role:** Same human as UP-4001, on the read-side.

**Goals:** browse recently-edited and pinned pages while away from
desktop; follow [[wikilink]]s and read backlinks; search the vault.

**Constraints:** poor or absent connectivity is the norm, not an edge
case; reading must be instant and offline.

### 2.3 UP-4003 Cross-device editor (occasional)

**Role:** Same human, occasionally making small edits to existing pages
from phone.

**Goals:** fix typos, add a sentence, tag a note, link two existing
pages — all while away from desktop; commit gets attribution and
history.

**Constraints:** does *not* expect a desktop-grade editor experience;
will not attempt structural rewrites; tolerates wonky autocomplete.

---

## 3. Requirements

### 3.1 Functional requirements

#### REQ-4001: Cross-platform parity

The system SHALL provide identical core features (capture, read, edit,
sync) on both [[iOS]] and [[Android]] FROM a single [[Tauri Mobile]]
codebase WITH platform-specific glue isolated to share-extension and
keychain modules.

Trace: [[#TEST-4001]], [[#ADR-4005]]

#### REQ-4002: On-device git working tree

The system SHALL maintain a full git working tree of the user's vault
at the platform-appropriate app-data directory ON first run AFTER
successful [[#REQ-4011|onboarding]].

Trace: [[#TEST-4002]], [[#ADR-4002]]

#### REQ-4003: SSH authentication via BIP39 seed

The system SHALL derive an [[ed25519]] SSH key pair from a 12-word
[[BIP39]] mnemonic via [[SLIP-0010]] path `m/44'/2'/0'` AND store the
private key in the platform secure-element store ([[iOS Keychain]] or
[[Android Keystore]]) WITH biometric or device-passcode gating.

Trace: [[#TEST-4003]], [[#CON-4002]], [[SPEC-001]] §Deterministic keys

#### REQ-4004: Embedded zetl serve as UI host

The system SHALL embed a single-user build of [[zetl serve]] as the
sole UI rendering layer. The embedded server SHALL be feature-gated
to disable `--collab`, `mcp`, `reason`, and `history`. The
[[WebView]] SHALL load pages from the embedded server (binding TBD
per [[#open-questions]]) AND SHALL NOT introduce a parallel
mobile-specific UI layer in [[TypeScript]] or any other framework.

Trace: [[#TEST-4004]], [[#ADR-4001]], [[#CON-4004]]

#### REQ-4005: Mobile-specific serve routes

The embedded server SHALL expose three additional [[Minijinja]] /
route surfaces under a `_mobile/` prefix, in addition to the existing
serve UI:

| Route                  | Purpose                                                              |
| ---------------------- | -------------------------------------------------------------------- |
| `GET /_mobile/onboarding` | Guided seed-import + remote-URL + clone wizard                    |
| `GET /_mobile/capture`    | Capture screen (FAB target + share-extension landing)             |
| `GET /_mobile/sync`       | Sync status + manual pull/push controls                           |
| `POST /_mobile/sync/pull` | Trigger a pull (FF-only)                                          |
| `POST /_mobile/sync/push` | Trigger a push                                                    |
| `POST /_mobile/capture`   | Write captured note to vault, commit, queue push                  |

These routes SHALL render through the active theme's templates
(falling back to bundled defaults), inheriting all theme variables
and chrome already in use by the desktop UI.

Trace: [[#TEST-4005]], [[#CON-4004]]

#### REQ-4006: FAB-driven capture

The system SHALL present a [[Floating Action Button]] on every
primary serve screen (page list, page view, search) that links to
`/_mobile/capture` AND opens the capture view WITHIN 200 ms of tap
(see [[#NFR-4001]]) AND SHALL save a new markdown file with auto-
titled `Inbox YYYY-MM-DD-HHMM` slug if the user does not provide a
title.

Trace: [[#TEST-4006]], [[#CON-4004]]

#### REQ-4007: Share-target capture

The system SHALL register an [[iOS Share Extension]] target and an
[[Android Share Activity]] with `ACTION_SEND` filter that ACCEPTS
text/plain, text/uri-list, and the page-title from a browser share
AND writes the payload to an app-group inbox file BEFORE the share
sheet dismisses. On next app launch, the embedded server SHALL drain
the inbox via the [[Tauri Command]] `drain_share_inbox()`, render
`/_mobile/capture` with the payload prefilled, and surface a toast
"N captured note(s) from Share" if N ≥ 1.

Trace: [[#TEST-4007]], [[#CON-4003]], [[#NFR-4002]]

#### REQ-4008: Capture durability

The system SHALL never lose a captured note. A successful return from
`POST /_mobile/capture` SHALL imply (a) the file is fsync'd to disk
and (b) a git commit referencing it has been written to the local
repo. Pushed-state is independent and may follow asynchronously.

Trace: [[#TEST-4008]], [[#NFR-4002]]

#### REQ-4009: Auto pull on app open

The system SHALL invoke `git fetch` followed by `git merge --ff-only`
on the configured remote WHEN the app foregrounds AND the device is
online AND the user has not opted out via `/_mobile/sync` settings.
The pull SHALL execute via the [[Tauri Command]] `pull()`; the
embedded server SHALL refresh its [[LinkGraph]] index after a
successful FF pull.

Trace: [[#TEST-4009]], [[#CON-4001]], [[#CON-4002]], [[#ADR-4003]]

#### REQ-4010: Opportunistic push after save

The system SHALL attempt `git push` immediately after every successful
save (existing serve `PUT /api/pages/{slug}`) or capture
(`POST /_mobile/capture`) IFF the device is online, AND otherwise
SHALL leave the local commit in place AND retry on the next online
event.

Trace: [[#TEST-4010]], [[#CON-4001]], [[#CON-4002]], [[#ADR-4003]]

#### REQ-4011: Onboarding flow

The system SHALL guide a first-run user through three steps,
delivered via the `/_mobile/onboarding` template: (1) paste or scan a
12-word [[BIP39]] mnemonic, (2) paste a git remote URL, (3) initiate
`git clone`, AND surface the derived SSH public key for the user to
add to their git host BEFORE the clone step proceeds.

Trace: [[#TEST-4011]], [[#CON-4001]], [[#CON-4002]], [[#REQ-4003]]

### 3.2 Non-functional requirements

#### NFR-4001: Latency

| Operation                   | Target                                                |
| --------------------------- | ----------------------------------------------------- |
| Cold-launch to PageList     | ≤ 1.5 s WITH p95 on iPhone 13 / Pixel 6 baseline      |
| FAB tap to `/_mobile/capture` | ≤ 200 ms WITH p95                                   |
| Page render (10 KB note)    | inherits serve target — measure ≤ 100 ms p95 (cached)|
| Search over 10 000 pages    | inherits serve target — measure ≤ 500 ms p95         |
| Save → commit               | ≤ 250 ms WITH p95                                    |

Trace: [[#TEST-4101]], [[#OBS-4001]]

#### NFR-4002: Durability

The system SHALL guarantee at-most-once capture loss only in the
presence of physical device destruction. Every capture path SHALL be
atomic at the filesystem level (tmp-write → fsync → rename) AND every
share-extension payload SHALL be written before the share sheet
dismisses, regardless of subsequent app-suspend or force-quit.

Trace: [[#TEST-4102]]

#### NFR-4003: Privacy

The system SHALL NOT emit telemetry, crash reports, or any payload
containing vault content to any non-user-controlled endpoint. The only
network destinations SHALL be (a) the git remote configured by the user
and (b) optional URL-fetch during share-extension preview generation
(documented and disable-able). The embedded serve SHALL bind to
loopback only — no LAN exposure.

Trace: [[#TEST-4103]], [[#OBS-4002]]

#### NFR-4004: Vault size envelope

The system SHALL declare itself **suitable for vaults ≤ 50 MB working
tree, ≤ 10 000 pages**. Vaults larger than this MAY work but are not
covered by NFR-4001 latency targets and are not part of the v0.1
manual-QA matrix. Sparse checkout is deferred to v0.2.

Trace: [[#TEST-4104]]

#### NFR-4005: Build pipeline reliability

The system's [[Tauri Mobile]] build pipeline SHALL produce signed iOS
and Android artefacts in CI WITH a per-tag success rate ≥ 95 % over
rolling 30-day windows.

[Provisional — refined by [[DESIGN-040-zetl-mobile#task-tauri-mobile-maturity-survey]]]

Trace: [[#TEST-4105]], [[#OBS-4003]]

#### NFR-4006: Accessibility

The system SHALL meet [[WCAG 2.2 AA]] for all surfaces (per
[[PROTO-001]] Constitutional Principle 9) AND SHALL respect the
OS-level text-size and reduced-motion preferences. Most accessibility
properties are inherited from [[zetl serve]]; the mobile-specific
routes (`/_mobile/*`) SHALL be audited separately.

Trace: [[#TEST-4106]]

---

## 4. Architecture

### 4.1 System diagram

```
┌─────────────────── 📱 phone (Tauri Mobile app) ────────────────────┐
│                                                                    │
│   WebView                                                          │
│      │                                                             │
│      │ tauri:// (or http://127.0.0.1:port)                         │
│      ▼                                                             │
│   ┌── embedded zetl serve (single-user, axum) ─────────────────┐  │
│   │  • Minijinja templates + active theme                       │  │
│   │  • REST API (/api/pages, /api/search, /api/graph…)          │  │
│   │  • CodeMirror 6 editor (existing)                           │  │
│   │  • Backlinks, search, wikilink nav, transclusion (existing) │  │
│   │  • SPA shell + responsive CSS (existing)                    │  │
│   │                                                             │  │
│   │  + new mobile routes:                                       │  │
│   │     /_mobile/onboarding   /_mobile/capture                  │  │
│   │     /_mobile/sync         POST /_mobile/sync/{pull,push}    │  │
│   │     POST /_mobile/capture                                   │  │
│   └─────────────────────────────────────────────────────────────┘  │
│      │                                                             │
│      │ direct fn calls (same process)                              │
│      ▼                                                             │
│   ┌── mobile-specific Rust additions ───────────────────────────┐  │
│   │  • git: clone / pull (FF-only) / push (git2-rs)             │  │
│   │  • keys: BIP39 → ed25519 → keychain                         │  │
│   │  • inbox: drain app-group share-extension payloads          │  │
│   │  • lifecycle: spawn / shutdown embedded serve               │  │
│   └─────────────────────────────────────────────────────────────┘  │
│      │                              │                              │
│      ▼                              ▼                              │
│   📁 vault (git working tree)    🔐 iOS Keychain / Android Keystore│
│       app data dir                  (SSH private key)              │
│         │                                                          │
└─────────┼──────────────────────────────────────────────────────────┘
          │ SSH (push / pull)
          ▼
       🌐 git remote ← canonical DB
          ▲
          │ pull on desktop (manual or hook)
       🖥️ desktop: same vault, indexed by `zetl index`
```

### 4.2 Architecture decisions

#### ADR-4001: Embed `zetl serve` as the mobile UI host

**Status:** Accepted (strawman; subject to
[[DESIGN-040-zetl-mobile]] review).

**Context:** The mobile app needs a UI for reading, editing, search,
backlinks, and the new capture / onboarding / sync flows. Two
approaches exist: (a) build a fresh mobile UI in TypeScript / Solid /
Flutter / similar that talks to a Rust core via [[Tauri Command]]s,
or (b) embed [[zetl serve]] as a process inside the Tauri Mobile
app and let the WebView load pages from it.

**Decision:** Embed [[zetl serve]] in single-user mode. The WebView
loads pages from the embedded server. Mobile-specific UI surfaces
(capture, onboarding, sync controls) are added as new
[[Minijinja]] templates and routes inside serve, not as a parallel
mobile UI layer.

**Rationale:**

- zetl serve already implements rendering, the [[CodeMirror 6]]
  editor, full-text search, backlinks, [[transclusion]], the
  [[Sigma.js]] graph widget, and the [[Theme Contract]]. None of
  this needs to be rebuilt.
- The responsive CSS already collapses sidebar and docked graph
  below `--zetl-graph-widget-breakpoint` (default 900 px), so
  serve's UI is already viable on mobile viewports.
- Themes work unmodified — the entire desktop [[Theme Contract]]
  applies. No mobile-specific theme contract needed.
- Mobile-specific UX (FAB, capture screen, onboarding wizard) is
  handled by adding ~3 new templates, not by writing a new client.
- Native features that need platform glue (keychain, share-target)
  remain on a small [[Tauri Command]] surface — kept tiny on
  purpose ([[#CON-4001]]).

**Consequences:**

- The embedded serve must build without `--collab` / `mcp` /
  `reason` / `history`. Existing feature gates may need refactoring;
  scoped in [[DESIGN-040-zetl-mobile#task-feature-gates]].
- Server binding choice (loopback HTTP vs `tauri://` custom
  protocol) is an open question. Both are viable; perf and the
  platform "is this allowed?" review may differ. Resolved in
  [[DESIGN-040-zetl-mobile#task-server-binding]].
- The mobile binary contains a full HTTP server stack ([[axum]] +
  [[tokio]]). Binary size grows; verified against [[#NFR-4001]]
  cold-launch budget.
- All new mobile-only UI lives in templates, which means theme
  authors can override `_mobile/*.html` if they choose — same
  override mechanism as for desktop templates.

Implements: [[#REQ-4004]], [[#REQ-4005]]

#### ADR-4002: Git remote as the database; phone holds full clone

**Status:** Accepted.

**Context:** The phone needs sync with the vault. Three classes of
solution exist: (a) HTTP API to a running [[zetl serve]] on the
desktop, (b) OS-level file sync ([[Syncthing]], [[iCloud Drive]],
Drive), (c) git as the sync substrate.

**Decision:** Use git as the substrate; phone holds a full working
tree on device. The git remote is the canonical store; every peer is
a clone.

**Rationale:** zetl already auto-commits on every save in `--collab`
mode ([[SPEC-020]]) and ships `zetl derive-ssh-key` as a first-class
command. Git as the sync substrate (i) reuses both, (ii) introduces
zero new server infrastructure, (iii) yields a truly [[Local-First]]
story (phone is just another peer), (iv) gives free history and
attribution.

**Consequences:** [[git2-rs]] / [[libgit2]] becomes a hard dependency
of the mobile-additions Rust module, adding ~500 KB to the binary.
Vaults larger than mobile storage are out of scope (mitigated by
[[#NFR-4004]]). Conflict resolution must have a UX answer (see
[[#ADR-4004]]).

Implements: [[#REQ-4002]], [[#REQ-4010]]

#### ADR-4003: Sync is foreground-only and explicit

**Status:** Accepted.

**Context:** Background sync (iOS `BGAppRefresh`, Android
`WorkManager`) is technically possible but introduces non-trivial
complexity around permissions, OS scheduling, push triggers, and
battery profiles.

**Decision:** v0.1 syncs only when the app is in the foreground:
auto pull on app open, opportunistic push after save, manual buttons
in `/_mobile/sync`.

**Rationale:** "Basic" scope. Foreground sync is good enough for
capture-first usage, where users open the app to capture and
naturally trigger the sync round-trip. Background sync is a v0.2
question.

**Consequences:** A capture made offline that is never followed by an
app-open while online will never push. Documented; users who care can
simply open the app on Wi-Fi.

Implements: [[#REQ-4009]], [[#REQ-4010]]

#### ADR-4004: Conflict policy is fast-forward only

**Status:** Accepted.

**Context:** Phone screen is the worst place to resolve a non-trivial
text merge. Capture-first usage means conflicts are dominated by the
empty case (new files don't conflict).

**Decision:** v0.1 phone refuses non-FF pulls and divergent pushes.
On non-FF, surface a modal pointing the user to desktop resolution.
Block push until the next pull is FF.

**Rationale:** Capture conflicts are rare; phone-side merging is
high-risk (small screen, no real diff tools). A merge UI on phone is
a v0.2 problem at the earliest.

**Consequences:** A user who edits the same page on both phone and
desktop must resolve on desktop. Surfaced in onboarding documentation.
Local commits are never lost — only push is delayed.

Implements: [[#REQ-4009]], [[#REQ-4010]]

#### ADR-4005: Tauri Mobile vs alternatives

**Status:** Accepted (with maturity caveat).

**Context:** [[Tauri Mobile]], [[Capacitor]], [[React Native]],
[[Flutter]], and pure-native were the candidate stacks.

**Decision:** [[Tauri Mobile]].

**Rationale:** [[Tauri Mobile]] is the only stack that lets us link
the existing zetl Rust crate (including its [[axum]]-based serve) as
a library inside the app process; alternatives would require either
running serve as a sidecar binary (awkward on iOS) or rewriting the
UI surface ([[#ADR-4001]] explicitly avoids this). Tauri's WebView
host is well-suited to embedding a local HTTP server or a custom
protocol handler. One codebase for both stores.

**Consequences:** [[Tauri Mobile]] is younger than [[Capacitor]] /
[[React Native]] / [[Flutter]]; build-pipeline reliability is captured
as a measurable NFR ([[#NFR-4005]]) rather than assumed.

Implements: [[#REQ-4001]]

### 4.3 Purity Boundary Map

Separates deterministic, side-effect-free computation from I/O.

#### Pure core (no I/O, no shared state, deterministic)

| Module / function                          | What it computes                                                 |
| ------------------------------------------ | ---------------------------------------------------------------- |
| `keys::derive_ssh(mnemonic)`               | BIP39 → SLIP-0010 → ed25519 keypair                              |
| `mobile::auto_slug(title, now)`            | Capture filename slug computation                                |
| `mobile::auto_title(content)`              | First-line-or-fallback auto-title                                |
| (existing serve pure functions)            | [[Frontmatter]] parsing, link extraction, rendering              |

#### Effectful shell (orchestrates I/O, calls pure core)

| Module / function                  | What effects it performs                              |
| ---------------------------------- | ----------------------------------------------------- |
| `vault::write_atomic(path, bytes)` | Tmp-write, fsync, rename                              |
| `git::commit / pull / push`        | libgit2 calls, network I/O                            |
| `keys::keychain_store / load`      | Platform secure-element calls                         |
| `inbox::drain()`                   | Reads app-group `inbox.jsonl`                         |
| `lifecycle::spawn_serve()`         | Starts embedded axum server                           |
| `commands::*`                      | Tauri command boundary; orchestrates the above        |
| `mobile-routes::*`                 | Serve handlers for `/_mobile/*` — call into core      |

#### Boundary contracts (data types crossing the boundary)

- `CaptureRequest { content, title? }` — UI → vault
- `SyncStatus { ahead, behind, dirty, last_pull, last_push }` — git → UI
- `SshKeypair { priv_pem, pub_openssh }` — keys → keychain (private
  side never crosses to UI)
- `ShareInboxEntry { kind, title, body, received_at }` — share-ext → capture

#### Dependency rule

Dependencies point inward: `commands` and `mobile-routes` →
`vault` / `git` / `keys` / `inbox` / `lifecycle` (effectful shell)
→ pure core. Pure core MUST NOT import from the shell. Shell modules
MUST NOT import the `commands` module.

#### Enforcement

Crate-level module visibility (`pub(crate)`) on shell-only types;
[[arch-lint]]-equivalent CI rule for "pure must not depend on shell".

---

## 5. Components

### 5.1 Tauri Mobile shell

The outer shell is a [[Tauri Mobile]] app whose only responsibilities
are: (a) host a [[WebView]], (b) spawn the embedded serve at startup,
(c) expose the small [[Tauri Command]] surface in [[#CON-4001]], (d)
provide native platform glue (keychain, share-extension intake).

### 5.2 Embedded `zetl serve` (single-user)

A feature-gated build of the existing [[zetl serve]] binary. Disabled
features: `--collab`, `mcp`, `reason`, `history`. Bound to loopback
or served via a [[Tauri]] custom protocol per
[[DESIGN-040-zetl-mobile#task-server-binding]]. All existing serve
endpoints (`/api/pages`, `/api/search`, `/api/graph`, the page
templates, the editor, the SPA shell) are inherited unchanged. The
[[Theme Contract]] applies as on desktop.

### 5.3 Mobile-specific serve extensions

| Component               | Responsibility                                                                                  |
| ----------------------- | ----------------------------------------------------------------------------------------------- |
| `_mobile/onboarding.html` | Minijinja template — guided seed-import + remote-URL + clone wizard                           |
| `_mobile/capture.html`    | Minijinja template — quick-capture form (FAB target + share-extension landing)                |
| `_mobile/sync.html`       | Minijinja template — sync status + manual pull/push buttons                                   |
| `mobile-routes` module    | Rust serve handlers for the new routes; thin adapters from HTTP to `git` / `keys` / `inbox`   |
| FAB injection             | Existing serve `base.html` gets a small FAB block that renders only when `mode == "mobile"`   |

### 5.4 Mobile-specific Rust additions

| Module      | Responsibility                                                                                  |
| ----------- | ----------------------------------------------------------------------------------------------- |
| `git`       | [[git2-rs]] wrapper: `clone`, `pull` (FF-only), `push`, `commit`, `status`. SSH credential cb   |
| `keys`      | [[BIP39]] → [[SLIP-0010]] → [[ed25519]] SSH key. Re-uses existing zetl key derivation code      |
| `inbox`     | Read app-group `inbox.jsonl`; deliver entries to capture-route on app launch                    |
| `lifecycle` | Spawn / shutdown embedded serve; bind to loopback or register Tauri custom protocol             |
| `state`     | SQLite ([[rusqlite]]): `last_pulled_rev`, `pending_push_count`, `settings`, `log_ring`          |
| `commands`  | Tauri command handlers — pure adapter from JSON args to module calls                            |

### 5.5 Platform glue (Swift / Kotlin)

| Component              | Responsibility                                                                              |
| ---------------------- | ------------------------------------------------------------------------------------------- |
| `keychain` (iOS)       | Store/retrieve SSH private key in `kSecClassKey` with biometrics gate                       |
| `keystore` (Android)   | Store/retrieve SSH private key in [[Android Keystore]] with `setUserAuthenticationRequired` |
| `share-ext` (iOS)      | [[iOS Share Extension]] target writes incoming text/URL to app-group `inbox.jsonl`          |
| `share-act` (Android)  | Activity with `ACTION_SEND` intent-filter; appends payload to `inbox.jsonl` on launch       |

Platform plugins are abstracted behind a Rust trait so unit tests
substitute an in-memory implementation.

---

## 6. Contracts

### CON-4001: Tauri command interface (small surface)

```
import_seed(mnemonic)   → { ssh_pubkey }
clone(remote_url)       → { progress_stream → done }
pull()                  → { pulled_revs, conflicts? }
push()                  → { pushed: bool, error? }
status()                → { ahead, behind, dirty, last_pull, last_push }
drain_share_inbox()     → [ ShareInboxEntry ]
serve_url()             → { url } // tauri:// or http://127.0.0.1:port
```

Pre-conditions: invoked only from the in-process WebView. `pull` and
`push` SHALL fail-fast if the keychain rejects the SSH key fetch.

Post-conditions: every command returning a `commit_id` (transitively)
SHALL reference a commit reachable from `HEAD` of the local repo.
Errors SHALL surface through Tauri's `Result` mechanism, never
silently swallowed.

Error model: typed error enum mirrored in the WebView; serve mobile
templates map errors to either toast, modal, or inline form error per
[[#§8 Error handling]].

Implements: [[#REQ-4003]], [[#REQ-4007]], [[#REQ-4009]],
[[#REQ-4010]], [[#REQ-4011]]

Verified by: [[#TEST-4001]] – [[#TEST-4011]] (snapshot tests on return
shape) and [[#TEST-4101]] (latency).

### CON-4002: SSH credential callback

The git2-rs credential callback SHALL load the private key from the
platform keychain WITHIN the credential-callback closure (not before),
SHALL never log the private key, and SHALL fail-fast with a structured
error if the keychain is locked or the key is missing.

Implements: [[#REQ-4003]], [[#REQ-4009]], [[#REQ-4010]], [[#REQ-4011]]

Verified by: [[#TEST-4003]], [[#TEST-4103]]

### CON-4003: Share-extension app-group inbox

iOS Share Extension and Android share-target SHALL write payloads to
a shared app-group / external-storage inbox file with the schema:

```json
{ "received_at": "<ISO 8601>",
  "kind": "text" | "url" | "url_with_title",
  "title": "...",
  "body": "..." }
```

Multiple payloads accumulate as JSON-lines in `inbox.jsonl`. The
`drain_share_inbox()` Tauri command empties the file atomically and
returns the entries; the mobile-route handler routes them to
`/_mobile/capture`.

Implements: [[#REQ-4007]], [[#REQ-4008]]

Verified by: [[#TEST-4007]], [[#TEST-4008]], [[#TEST-4102]]

### CON-4004: Mobile-specific serve routes

```
GET  /_mobile/onboarding   → onboarding wizard view
GET  /_mobile/capture      → capture form (optionally prefilled by query string)
GET  /_mobile/sync         → sync status + manual buttons
POST /_mobile/sync/pull    → invoke `pull()` Tauri command, redirect to /_mobile/sync
POST /_mobile/sync/push    → invoke `push()` Tauri command, redirect to /_mobile/sync
POST /_mobile/capture      → write file, commit, redirect to /pages/{slug}
```

Pre-conditions: routes SHALL be registered only when the build is
mobile-flavoured (gated by a `serve-mobile` cargo feature). Routes
SHALL accept requests only from the loopback address (or
[[Tauri]] custom-protocol origin per
[[DESIGN-040-zetl-mobile#task-server-binding]]).

Post-conditions: `POST /_mobile/capture` SHALL satisfy
[[#REQ-4008]] capture durability before returning success.

Implements: [[#REQ-4005]], [[#REQ-4006]], [[#REQ-4011]]

Verified by: [[#TEST-4005]], [[#TEST-4006]], [[#TEST-4011]]

### CON-4005: SQLite schema for `state`

```sql
CREATE TABLE settings    (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE log_ring    (id INTEGER PRIMARY KEY, ts TEXT NOT NULL, level TEXT, msg TEXT);
CREATE TABLE sync_meta   (key TEXT PRIMARY KEY, value TEXT NOT NULL);
-- log_ring trimmed to last 200 rows by trigger
-- sync_meta keys: last_pulled_rev, pending_push_count, last_push_at, last_pull_at
```

Migrations follow the `sqlx migrate`-style versioned-file convention.

Implements: durability invariants in [[#NFR-4002]]; diagnostics ring
buffer in [[#§8.2 Logging]].

Verified by: [[#TEST-4102]], [[#TEST-4103]]

---

## 7. Data flow

### 7.1 First-run onboarding

```
Cold launch → Tauri shell → spawn_serve() → WebView loads /_mobile/onboarding
  (mobile-routes detects no remote configured, redirects to onboarding wizard)

Step 1: paste seed
  POST /_mobile/onboarding/seed
    serve handler calls import_seed Tauri command
      Rust: keys::derive_ssh() → keychain::store(priv)
      return { ssh_pubkey }
    template re-renders showing pubkey + "Add this to your git host" copy button

Step 2: paste git remote URL
  POST /_mobile/onboarding/remote
    serve handler stores URL in state.settings, redirects to step 3

Step 3: clone
  POST /_mobile/onboarding/clone
    serve handler invokes clone() Tauri command (streams progress events to template)
      Rust: git::clone() with credential cb that loads key from keychain
      On done: trigger zetl::scanner::index() over working tree
    Redirect to / (existing serve page list)
```

Implements: [[#REQ-4011]], [[#REQ-4002]], [[#REQ-4003]]

### 7.2 Capture (FAB or share-target)

```
[FAB tap]    → GET /_mobile/capture (empty form)
[Share-ext]  → on app launch, mobile-routes handler calls drain_share_inbox()
              → if non-empty, redirect to GET /_mobile/capture?from=share
                with payload prefilled in template
              → toast "N captured note(s) from Share"
User edits title (or accepts auto-title from first-line / "Inbox YYYY-MM-DD-HHMM")
User submits → POST /_mobile/capture
  serve handler:
    Rust mobile module: vault::write_atomic + git::commit("capture: <slug>")
    If online: git::push() (best-effort, fire-and-forget)
    If offline: increment state.sync_meta.pending_push_count
  Redirect to /pages/{slug} (existing serve page view)
```

Implements: [[#REQ-4006]], [[#REQ-4007]], [[#REQ-4008]], [[#REQ-4010]]

### 7.3 Read & edit (delegated to existing serve UI)

No new design required. The user lands on `/` (page list) or
`/pages/{slug}` (page view) via standard serve navigation. Reading
goes through the existing rendered-page template; editing goes
through the existing [[CodeMirror 6]] editor; saving goes through
`PUT /api/pages/{slug}`. After successful save, a small mobile hook
(serve middleware gated by `serve-mobile` feature) calls the `push()`
Tauri command in the background to satisfy [[#REQ-4010]].

Implements: existing serve REQs; mobile delegation noted in
[[#REQ-4004]]

### 7.4 Pull on app open / manual

```
App.onForeground() → mobile-routes::on_foreground()
  Calls status() Tauri command
  If behind > 0 && online → invoke pull() Tauri command
  Else: SyncBar widget in /_mobile/sync shows "Last synced: …" (manual button)

pull()
  Rust: git::fetch(), then merge --ff-only
  If FF: trigger zetl::scanner::reindex() over changed files
  If non-FF: return { conflicts: true }
Mobile sync template:
  success → toast "Pulled N updates"
  conflict → modal "Push blocked — resolve on desktop, then pull again"
```

Implements: [[#REQ-4009]], [[#ADR-4004]]

### 7.5 Push

```
push() Tauri command
  If conflicts flag set from prior pull → refuse, surface "pull first"
  Else: git::push() with SSH credential cb
  On 2xx: state.sync_meta.pending_push_count := 0
  On non-FF rejection: set conflicts flag, surface "pull first"
  On network error: leave pending_push_count, retry on next online event
```

Implements: [[#REQ-4010]], [[#ADR-4004]]

---

## 8. Error handling

Three principles drive the design: **never lose a capture**
([[#NFR-4002]]), **fail loudly only when the user must act**,
**diagnostics on demand**.

### 8.1 Failure modes

| Failure                                | UI surface                                                    | Recovery                                                                                          |
| -------------------------------------- | ------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| Network down on push                   | `/_mobile/sync` shows "N pending — offline"                   | Commits stay in local repo. Auto-retry on next online event.                                      |
| SSH key rejected by remote             | Modal: "Authentication failed — re-add key" + copyable pubkey | User adds key on host; tap Retry. No automatic retry.                                             |
| Non-FF on pull                         | Modal: "Resolve on desktop, then pull again"                  | Push blocked until next FF pull succeeds.                                                         |
| Non-FF on push                         | Toast: "Remote has new changes — pulling…" auto-pull          | Auto-pull; if FF → re-push; else falls into row above.                                            |
| Bad mnemonic on import                 | Inline form error                                             | User retries. No partial state stored.                                                            |
| Malformed git remote URL               | Inline form error                                             | User edits URL; previous valid value preserved.                                                   |
| Clone fails (404 / auth)               | Clear progress UI with error reason                           | User retries with corrected URL or key. Partial clone wiped automatically before retry.           |
| Disk full on save                      | Modal: "Storage full — free space and tap Retry"              | Atomic save means no half-written state. Capture content kept in memory until save succeeds.      |
| Slug collision on capture              | Silent: append `-2`, `-3`… to slug                            | Title preserved verbatim; slug is just a filename suffix.                                         |
| Keychain access denied                 | Modal: "Allow keychain access in Settings"                    | No fallback — without keychain, no auth. Surface deep-link to OS settings.                        |
| Share-ext payload while app suspended  | App opens → drain_share_inbox → toast: "1 captured note from Share" | Share Extension wrote to app-group inbox before share-sheet dismissed; app drains on launch. |
| Embedded serve fails to spawn          | Full-screen native error: "Server failed to start" + log link | Tauri shell shows native diagnostic UI; surface log buffer.                                       |
| Rust panic in command or serve handler | Toast: "Something went wrong" + "Diagnostics" link            | Tauri command boundary catches; UI stays alive. Diagnostics shows last 200 log lines.             |

### 8.2 Logging

- [[tracing]] in the Rust core (matches zetl convention).
- Last 200 lines kept in a SQLite ring buffer ([[#CON-4005]]
  `log_ring`).
- Diagnostics surfaced via `/_mobile/sync#diagnostics` — a section in
  the existing template — with a "Copy to clipboard" action.
- No remote telemetry, no automatic crash uploads ([[#NFR-4003]]).

### 8.3 Durability invariants

- A successful `POST /_mobile/capture` or `PUT /api/pages/{slug}`
  means the file is written and committed locally — push can fail
  later, the data is still safe.
- Saves are atomic: write tmp, fsync, rename.
- Share-extension payloads are written to the app-group inbox
  **before** the share sheet dismisses.
- `pending_push_count` is decremented to zero only after a successful
  push for the current HEAD.

Trace: [[#REQ-4008]], [[#NFR-4002]]

---

## 9. Testing

Test strategy follows [[PROTO-001]] §Verification — every REQ gets at
least one positive, one negative-input where applicable, and one
negative-output where applicable. Mutation testing is mandatory on
the pure core (`mobile::auto_slug`, `mobile::auto_title`,
`keys::derive_ssh`) per [[PROTO-001]] §Mutation Testing.

### 9.1 Test catalogue (extract)

| ID         | REQ      | Form                | Description                                                                                                  |
| ---------- | -------- | ------------------- | ------------------------------------------------------------------------------------------------------------ |
| TEST-4001  | REQ-4001 | example             | Both iOS and Android builds expose the full mobile surface; smoke-launch both.                               |
| TEST-4002  | REQ-4002 | example             | After successful clone, app data dir contains `.git` and at least one `.md` file.                            |
| TEST-4003  | REQ-4003 | golden + property   | Derived pubkey for fixture mnemonic equals `zetl derive-ssh-key` output. Property: deterministic.            |
| TEST-4004  | REQ-4004 | example             | Mobile build produces an embedded serve with `--collab` / `mcp` / `reason` / `history` disabled. WebView loads `/`. |
| TEST-4005  | REQ-4005 | example             | `/_mobile/onboarding`, `/_mobile/capture`, `/_mobile/sync` render through the active theme without 500s.     |
| TEST-4006  | REQ-4006 | example + latency   | FAB tap to `/_mobile/capture` ≤ 200 ms (p95). Auto-title falls back to `Inbox YYYY-MM-DD-HHMM`.              |
| TEST-4007  | REQ-4007 | example             | iOS share from Safari → Capture screen prefilled with URL + page title. Android `ACTION_SEND` ditto.         |
| TEST-4008  | REQ-4008 | crash injection     | Force-kill mid-capture → restart → no half-written file, capture either fully present or absent.             |
| TEST-4009  | REQ-4009 | example             | App open with FF updates → auto-pull, toast "Pulled N updates". With non-FF → modal, no merge.               |
| TEST-4010  | REQ-4010 | example             | Save online → push within 5 s. Save offline → commit local, push on next online event.                      |
| TEST-4011  | REQ-4011 | example + form-error| Onboarding happy path; bad mnemonic gives inline error; bad URL gives inline error.                          |
| TEST-4101  | NFR-4001 | latency suite       | Measure each operation against target on iPhone 13 / Pixel 6; p95 within budget.                             |
| TEST-4102  | NFR-4002 | crash injection     | Random `kill -9` during capture / save / share-ext: zero data loss across N=1000 trials.                     |
| TEST-4103  | NFR-4003 | network audit       | Run app under proxy; no traffic to non-configured destinations during capture / read / save.                 |
| TEST-4104  | NFR-4004 | scale               | Vaults of 1k / 5k / 10k pages: latency targets hold; vault of 50k pages: documented degradation.             |
| TEST-4105  | NFR-4005 | CI                  | Tagged-release pipeline: signed iOS .ipa and signed Android .apk produced; success rate ≥ 95 %.              |
| TEST-4106  | NFR-4006 | accessibility       | axe / Accessibility Inspector run on every screen including `/_mobile/*`: 0 critical violations.             |

Trace: every test ID is bidirectionally linked to its driving REQ /
NFR via [[#§11 Traceability]].

### 9.2 Verification strategy

- **Pure core:** [[Property-Based Testing]] (`proptest`) for slug
  collision, frontmatter round-trip, link-graph invariants. Mutation
  testing (cargo-mutants) on the same modules.
- **Effectful shell:** Integration tests against temp bare repos
  (real libgit2). Platform plugins behind traits → in-memory impl
  for tests.
- **Mobile-specific serve routes:** axum integration tests against a
  temp working tree, exercising every route in [[#CON-4004]].
- **End-to-end:** Tauri Mobile project also builds for desktop with
  stubbed platform plugins → fastest E2E. [[Maestro]] flows on real
  iOS / Android nightly.
- **Adversarial testing** ([[PROTO-001]] §Adversarial Testing):
  required before v0.2 promotion; scoped in
  [[DESIGN-040-zetl-mobile#task-adversarial-testing]].

---

## 10. Observability

#### OBS-4001: Per-operation latency histogram

The system SHALL emit (locally; per [[#NFR-4003]] no remote
telemetry) a latency histogram per command in [[#CON-4001]] AND per
mobile route in [[#CON-4004]], accessible via
`/_mobile/sync#diagnostics`. Histograms inform release-gate
evaluation against [[#NFR-4001]].

#### OBS-4002: Network destination audit log

The system SHALL log every outbound network destination (host:port,
direction, byte count, no payload) to the `log_ring` per
[[#NFR-4003]], auditable via the diagnostics view.

#### OBS-4003: Build-pipeline success counter

CI SHALL emit a per-build success/failure counter tagged with
platform (iOS / Android), build stage, and tag/branch. The 30-day
rolling success rate is the gate for [[#NFR-4005]].

---

## 11. Traceability

| REQ / NFR | CONs                                         | TESTs                                  | OBS                  |
| --------- | -------------------------------------------- | -------------------------------------- | -------------------- |
| REQ-4001  | (cross-cutting)                              | TEST-4001, TEST-4105                   | OBS-4003             |
| REQ-4002  | (cross-cutting; uses CON-4002 for clone)     | TEST-4002                              |                      |
| REQ-4003  | CON-4002                                     | TEST-4003                              |                      |
| REQ-4004  | CON-4001, CON-4004                           | TEST-4004                              |                      |
| REQ-4005  | CON-4004                                     | TEST-4005                              |                      |
| REQ-4006  | CON-4004                                     | TEST-4006                              | OBS-4001             |
| REQ-4007  | CON-4003                                     | TEST-4007                              |                      |
| REQ-4008  | CON-4003, CON-4005                           | TEST-4008, TEST-4102                   |                      |
| REQ-4009  | CON-4001, CON-4002                           | TEST-4009                              | OBS-4001             |
| REQ-4010  | CON-4001, CON-4002                           | TEST-4010                              | OBS-4001             |
| REQ-4011  | CON-4001, CON-4002, CON-4004                 | TEST-4011                              |                      |
| NFR-4001  | (cross-cutting)                              | TEST-4101                              | OBS-4001             |
| NFR-4002  | CON-4003, CON-4005                           | TEST-4102                              |                      |
| NFR-4003  | (cross-cutting)                              | TEST-4103                              | OBS-4002             |
| NFR-4004  | (cross-cutting)                              | TEST-4104                              |                      |
| NFR-4005  | (cross-cutting)                              | TEST-4105                              | OBS-4003             |
| NFR-4006  | (cross-cutting)                              | TEST-4106                              |                      |

---

## 12. Acceptance criteria

This specification is acceptable for promotion to `0.2.0` (status
`draft`) when:

- [[DESIGN-040-zetl-mobile]] Phase 0 surveys (Tauri Mobile maturity,
  feature-gate refactoring, server-binding choice) have replaced the
  `[Provisional]` sections with grounded findings.
- Synthetic-user walkthroughs against the three user profiles
  ([[#UP-4001]]–[[#UP-4003]]) reach the [[Adversary Exhaustion]]
  convergence signal ([[PROTO-001]] §Success Criteria).
- Every REQ has a positive + (where applicable) negative-input +
  negative-output [[TEST-###]].
- Mutation kill rate on the pure core ≥ 80 %.
- All wikilink targets in this document either exist as pages or are
  recorded as deferred-concept entries in the vault (per
  [[PROTO-001]] §Wikilinks Required In Downstream Outputs).

This specification is acceptable for promotion to `1.0.0` (status
`approved`) only after Tier 2 cross-model adversarial review and
human reviewer sign-off.
