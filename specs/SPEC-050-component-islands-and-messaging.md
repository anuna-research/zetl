---
id: SPEC-050
title: "Component Islands & Inter-Island Messaging"
status: draft
version: 0.3.0-strawman
last-updated: 2026-06-24
audience: agent, human
---

# SPEC-050: Component Islands & Inter-Island Messaging

## Orientation

**Intent:** Let a [[SPEC-048]] component ship optional client-side behaviour (a JS
**island**) that progressively enhances its static HTML, and let independent islands
coordinate at runtime through a single shell-provided message bus — without a framework
runtime, without islands importing each other, and without breaking `file://` or
JS-disabled rendering.

**Metaphor:** *a noticeboard and a tannoy.* The retained `store` is a noticeboard — it
keeps the latest note pinned, so an island that arrives late still reads the current
value (replay-on-subscribe). The ephemeral `bus` is a tannoy announcement — heard only
by whoever is already in the room; miss it and it's gone. Islands never talk to each
other directly; they pin to and read from named channels.

**Structure** (`≤ 7` boxes; arrows = runtime message flow):

```
            ┌──────────────── SPA shell (SPEC-028), survives nav ───────────────┐
            │  window.zetl = { store(topic), bus }   — TYPED payloads (REQ-5013) │
            │   ├─ retained store: last-value + replay-on-subscribe              │
            │   └─ ephemeral bus: fire-and-forget                                │
            │                       ▲  capability bridge (reference monitor)     │
            └──────▲────────────────┼──────────── grant table + type check ──────┘
   in realm (trusted)               │ postMessage (MessageChannel port)
  ┌──────────────────┐     ┌────────┴──────────────────────────────┐
  │ trusted island   │     │  ⌗ sandboxed iframe (opaque origin)    │
  │ direct window.   │     │    content island — NO window.zetl     │
  │ zetl (REQ-5010)  │     │    (REQ-5010/5015/5016)                │
  └──────────────────┘     └───────────────────────────────────────┘
  build: emit <name>.js once/type (REQ-5001) · manifest topics+types+grants → wiring
  graph (REQ-5008/CON-5002) · persisted topics → localStorage + storage event (REQ-5006)
```

**Decisions** (deliberate before implementing):
[[SPEC-050-component-islands-and-messaging#ADR-5001]] shell bus, not a shared store module ·
[[SPEC-050-component-islands-and-messaging#ADR-5002]] replay-on-subscribe is the default primitive ·
[[SPEC-050-component-islands-and-messaging#ADR-5003]] content islands are iframe-sandboxed with a capability-scoped bridge (trusted islands run in-realm) ·
[[SPEC-050-component-islands-and-messaging#ADR-5007]] topics are typed; payloads recognised at the bus boundary ·
[[SPEC-050-component-islands-and-messaging#ADR-5008]] the bridge is a capability (transferred port + parent reference monitor) ·
[[SPEC-050-component-islands-and-messaging#ADR-5005]] persisted topics carry a declared default + inline pre-paint set (FOUC).

**Load-bearing requirements:**
[[SPEC-050-component-islands-and-messaging#REQ-5001]] gated per-type island emission ·
[[SPEC-050-component-islands-and-messaging#REQ-5004]] shell bus (`store` + `bus`) ·
[[SPEC-050-component-islands-and-messaging#REQ-5005]] replay-on-subscribe ·
[[SPEC-050-component-islands-and-messaging#REQ-5010]] two trust tiers (in-realm vs sandboxed) ·
[[SPEC-050-component-islands-and-messaging#REQ-5013]] typed payloads ·
[[SPEC-050-component-islands-and-messaging#REQ-5015]] content-island iframe sandbox ·
[[SPEC-050-component-islands-and-messaging#REQ-5016]] capability-scoped bridge (port-identity + handshake) ·
[[SPEC-050-component-islands-and-messaging#REQ-5017]] island/port lifecycle under SPA nav ·
[[SPEC-050-component-islands-and-messaging#REQ-5012]] backward-compatible default.

**Open** (each blocks the Phase 2 gate — see
[[SPEC-050-component-islands-and-messaging#12. Open Questions]]):
Q1 persisted-default / FOUC mechanism · Q4 bus/bridge residence in the SPEC-028 shell ·
Q5 delivery/ordering + `postMessage` latency · Q6 iframe cost at scale (owner: spec author,
to ground in Phase 1). *(Q2 sandbox and Q3 typed-payloads resolved in v0.2.0.)*

**Detail:** the full requirement, contract, and test nodes follow below.

> **Conformance.** The key words MUST, MUST NOT, REQUIRED, SHALL, SHALL NOT, SHOULD,
> SHOULD NOT, RECOMMENDED, MAY, and OPTIONAL in this document are to be interpreted as
> described in BCP 14 (RFC 2119, RFC 8174) when, and only when, they appear in all
> capitals ([[PROTO-001#Requirement-Level Keywords (BCP 14)]]).

> **Strawman notice.** A *first* draft, extracted from the [[SPEC-048]] v0.1.1 island/bus
> material that the v0.2.0 tightening deferred — **NOT** converged. No Phase 1 surveys,
> no synthetic-user runs, no fresh-context adversarial review. Per [[PROTO-001]]
> Principle 11 ([[Anti-Slop Bias]]), treat every clause as carrying hidden debt until
> adversarial review proves otherwise. **`[Blocked: Qn]`** marks a clause depending on an
> open question; **`[Provisional]`** marks a value still to be grounded in Phase 1.

## Information Table

| Field        | Value                                                                                  |
| ------------ | -------------------------------------------------------------------------------------- |
| Document ID  | [[SPEC-050-component-islands-and-messaging\|SPEC-050]]                                  |
| Title        | Component Islands & Inter-Island Messaging                                              |
| Version      | 0.3.0-strawman                                                                          |
| Status       | Draft (strawman; NOT converged — pending Phase 1 + Phase 2 gates)                       |
| Author       | Agent (Claude Opus 4.8 [1M], [[PROTO-001\|USDD Agent Protocol]] v1.11.0)                |
| Date         | 2026-06-24                                                                              |
| Predecessor  | [[SPEC-048-components-and-static-overrides\|SPEC-048]] (islands/bus deferred here)      |
| Related      | [[SPEC-028]] SPA shell, [[SPEC-049]] content-author components, [[SPEC-002]] search     |
| Feature Gate | `component-islands` (island emission + hydration); `island-bus` (shell messaging)       |
| Review tier  | Tier 2 (trust boundary: untrusted content-island JS in a sandbox + capability bridge; `localStorage`/cross-tab + inbound `postMessage` as untrusted input) |

---

## 1. Overview

### 1.1 Problem

[[SPEC-048]] components render to static HTML + (unscoped) CSS, which is correct for the
dominant case and works on `file://` with JS off. But some components are inherently
interactive — a theme toggle, a copy-to-clipboard button, a collapsible, a live filter —
and a few of those must **coordinate**: a theme toggle in the nav-header must tell every
themed surface to switch; a search-open button must tell the search island to mount.

Two capabilities are therefore missing from the v1 core, and SPEC-048 deferred both here:

1. **No client-side island.** A component cannot ship behaviour. There is no convention
   for emitting a component's `<name>.js`, hydrating its instances, gating the script so
   pages that don't use it stay JS-free, or composing with the [[SPEC-028]] SPA shell's
   client-side navigation.

2. **No inter-island coordination.** Even with islands, two independent islands have no
   sanctioned way to communicate. The naïve answers all fail: a bare `CustomEvent` bus
   silently loses any signal fired before a subscriber mounted (the dominant islands bug,
   and fatal for state like the current theme); a shared imported store couples islands
   into a bundle and fights the "emit each island once" model; ad-hoc `window` globals
   have no contract, no audit, and collide.

### 1.2 Core Insight

**The hard part is not messaging — it is *late* subscribers.** Under the SPEC-028 SPA
shell, page subtrees are swapped on navigation, so islands mount, unmount, and re-mount
*after* values have already been published. Any coordination primitive whose correctness
depends on "both parties present at the moment of the signal" is wrong here. The load-
bearing primitive is therefore a **retained, replay-on-subscribe store** living on the
*persistent* shell (which survives navigation), with an ephemeral fire-and-forget `bus`
as the secondary primitive for genuinely momentary events
([[SPEC-050-component-islands-and-messaging#ADR-5002]]).

The second insight is a trust one: **a JS island is arbitrary code, and code-in-the-realm
has ambient authority.** A *trusted* theme island already controls the page, so it runs
in-realm with direct `window.zetl`. A *content-author* island ([[SPEC-049]]) is untrusted,
and string-namespacing topics cannot isolate untrusted same-realm JS from a topic like
`theme`. The boundary must therefore be a **real one**: content islands run in a sandboxed
iframe (opaque origin — no `window.zetl`), and reach the bus only through a **capability-
scoped bridge** the parent reference-monitors, over **typed** messages. Isolation
(REQ-5015) + capability scoping (REQ-5016) + payload typing (REQ-5013) are the three legs
that let untrusted authors add interactivity without being able to forge a trusted topic
([[SPEC-050-component-islands-and-messaging#ADR-5003]]).

### 1.3 Design Principles

1. **Progressive enhancement, always.** Every island enhances HTML that already works
   without it; JS-disabled and `file://` render the static component unchanged
   ([[SPEC-050-component-islands-and-messaging#REQ-5002]]).
2. **No framework runtime.** An island is a plain ES module; the build emits it once per
   component type per page and hydrates `data-z` nodes — no shared bundle, no VDOM
   ([[SPEC-050-component-islands-and-messaging#ADR-5004]]).
3. **Coordinate by topic, never by reference.** Islands publish/subscribe named topics;
   neither references the other ([[SPEC-050-component-islands-and-messaging#ADR-5001]]).
4. **Retain by default; the latecomer is the norm.** The default primitive replays the
   current value on subscribe ([[SPEC-050-component-islands-and-messaging#ADR-5002]]).
5. **Trust by isolation, not by exclusion.** Trusted theme islands run in-realm; untrusted
   content-author islands run only in an iframe sandbox reaching the bus through a
   capability-scoped bridge over typed messages — arbitrary realm access is trusted-author
   only ([[SPEC-050-component-islands-and-messaging#REQ-5010]],
   [[SPEC-050-component-islands-and-messaging#REQ-5015]],
   [[SPEC-050-component-islands-and-messaging#REQ-5016]]).
6. **Recognise before acting ([[LangSec]]).** Topic names, manifest declarations, and —
   critically — values read back from `localStorage`/`storage` events are recognised
   against a declared grammar before use
   ([[SPEC-050-component-islands-and-messaging#CON-5001]]–[[SPEC-050-component-islands-and-messaging#CON-5004]]).
7. **No behaviour without invocation.** A vault using no island behaves byte-identically
   to a [[SPEC-048]]-only build ([[SPEC-050-component-islands-and-messaging#REQ-5012]]).

### 1.4 Scope

**In scope:** optional component islands (`<name>.js`) with gated, deduped, deterministic
emission ([[SPEC-050-component-islands-and-messaging#REQ-5001]]); progressive enhancement
([[SPEC-050-component-islands-and-messaging#REQ-5002]]); SPA-shell hydration/re-hydration
([[SPEC-050-component-islands-and-messaging#REQ-5003]]); the shell-provided message bus —
retained `store` + ephemeral `bus` ([[SPEC-050-component-islands-and-messaging#REQ-5004]],
[[SPEC-050-component-islands-and-messaging#REQ-5005]]); persisted topics
([[SPEC-050-component-islands-and-messaging#REQ-5006]]); bus survival across navigation
([[SPEC-050-component-islands-and-messaging#REQ-5007]]); manifest-declared topics + static
wiring verification ([[SPEC-050-component-islands-and-messaging#REQ-5008]]); the audit
wiring graph ([[SPEC-050-component-islands-and-messaging#REQ-5009]]); the two-tier island
trust boundary ([[SPEC-050-component-islands-and-messaging#REQ-5010]]); topic grammar
([[SPEC-050-component-islands-and-messaging#REQ-5011]]); backward-compatible default
([[SPEC-050-component-islands-and-messaging#REQ-5012]]); **typed topic payloads**
([[SPEC-050-component-islands-and-messaging#REQ-5013]]); the **content-island iframe
sandbox** ([[SPEC-050-component-islands-and-messaging#REQ-5015]]); and the
**capability-scoped bridge** ([[SPEC-050-component-islands-and-messaging#REQ-5016]]).

**Out of scope:** a reactive/VDOM framework; server-pushed islands or websockets (the bus
is client-local); nested/recursive topic value schemas beyond the v1 flat-record type
language (CON-5005); a Worker-based (non-DOM) content-island variant (`[Blocked: Q2]`);
cross-document (cross-origin) messaging beyond the local capability bridge.

---

## 2. User Profiles

> **`[Provisional — refined by Phase 1 synthetic-user runs.]`**

### 2.1 The Theme Author
Ships an interactive component (theme toggle, copy button, collapsible) and wants it to
enhance the static markup, hydrate reliably under SPA navigation, and — for the toggle —
broadcast a `theme` change every surface picks up.

### 2.2 The Site Operator
Wants interactive chrome to "just work" on the deployed static site and on `file://`,
with no flash of the wrong theme on load, and with a build that stays JS-free on pages
that use no island.

### 2.3 The Reviewer / Auditor
Wants to confirm at build time that every `subscribes` topic has a publisher, that no
magic-string typo silently breaks wiring, that a content-authored page cannot reach the
trusted bus, and that values restored from `localStorage` cannot inject.

---

## 3. Happy Paths

> **`[Provisional — refined by Phase 1.]`**

### 3.1 HP1: Default — No Islands, Nothing Changes
**Pre:** no component ships `<name>.js`. **Post:** byte-identical to a SPEC-048-only
build; no `window.zetl`, no bus, no island scripts
([[SPEC-050-component-islands-and-messaging#REQ-5012]]).

### 3.2 HP2: A Self-Contained Island (Copy Button)
A `copy-button` component ships `copy-button.js`. The build emits it once as a
`<script type="module">` on pages that use the component; it hydrates each
`data-z="copy-button"` node. With JS off, the button still renders (and is inert)
([[SPEC-050-component-islands-and-messaging#REQ-5001]],
[[SPEC-050-component-islands-and-messaging#REQ-5002]]).

### 3.3 HP3: Theme Toggle Coordinates Every Surface
`theme-toggle.js` declares `publishes = ["theme"]`; other islands declare
`subscribes = ["theme"]`. Clicking the toggle calls `store("theme").set("dark")`. Every
subscriber — including one that mounts after the click, and one re-mounted after a
client-side SPA navigation — reads `"dark"` (replay-on-subscribe; the bus survives nav).
`theme` is `persisted`, so it round-trips `localStorage` and reflects a cross-tab
`storage` event ([[SPEC-050-component-islands-and-messaging#REQ-5005]],
[[SPEC-050-component-islands-and-messaging#REQ-5006]],
[[SPEC-050-component-islands-and-messaging#REQ-5007]]).

### 3.4 HP4: A Typo Is Caught at Build Time
An island declares `subscribes = ["theme"]` but no component publishes `theme`. The build
emits `island-topic-unpublished` (warning) and the wiring graph shows the dangling edge
([[SPEC-050-component-islands-and-messaging#REQ-5008]],
[[SPEC-050-component-islands-and-messaging#REQ-5009]]).

### 3.5 HP5: A Content Island Is Sandboxed and Capability-Scoped
A [[SPEC-049]] content component ships a `content:filter` island. The build mounts it in a
sandboxed iframe (opaque origin — no `window.zetl`); its only authority is a transferred
port whose grant table allows `publish content:filter` and (because the theme declared the
grant) `subscribe theme` read-only. It reads `theme` to style itself and publishes
`content:filter`; an attempt to publish `theme` is dropped at the bridge
(`island-capability-denied`) — a markdown author can never forge or overwrite a trusted
topic ([[SPEC-050-component-islands-and-messaging#REQ-5010]],
[[SPEC-050-component-islands-and-messaging#REQ-5015]],
[[SPEC-050-component-islands-and-messaging#REQ-5016]],
[[SPEC-050-component-islands-and-messaging#Threat A]]).

### 3.6 HP6: No Flash of the Wrong Theme
On first paint, the persisted `theme` value is applied by a tiny inline pre-paint script
before hydration, so a returning visitor never sees a light→dark flash
([[SPEC-050-component-islands-and-messaging#REQ-5006]],
[[SPEC-050-component-islands-and-messaging#ADR-5005]]).

---

## 4. Functional Requirements

> Numbering: SPEC-050 → REQ-50xx, sequential. Each REQ decomposes into positive /
> negative-input / negative-output tests ([[PROTO-001]] §9).

### REQ-5001: Gated, Deduplicated, Deterministic Island Emission
WHEN a [[SPEC-048]] component ships `<name>.js`, the build SHALL emit it **once per
component type used on a page** as a `<script type="module">` reference, and SHALL gate
emission so a page using no JS-bearing component loads **no** island script. The emitted
set and order SHALL be byte-identical across repeated builds (declared total order:
component name, then source layer), reusing the [[SPEC-048]] REQ-4809 dedup/emission
model. The script SHALL hydrate nodes carrying the component's `data-z="<name>"` marker.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5001]], [[SPEC-050-component-islands-and-messaging#NFR-5002]].

### REQ-5002: Progressive Enhancement
A component's static HTML + CSS SHALL render correctly with JavaScript disabled and under
`file://`; the island SHALL only **enhance** already-meaningful markup, never be the sole
source of content or navigation. An island SHALL NOT inject content that is required for
the page to be usable or indexable ([[SPEC-002]]); content an island reveals SHALL have a
no-JS-accessible fallback.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5002]], [[SPEC-028]]; [[SPEC-050-component-islands-and-messaging#3.2 HP2]].

### REQ-5003: SPA-Shell Hydration and Re-Hydration
WHEN the [[SPEC-028]] SPA shell is enabled (`[spa].enabled`), islands SHALL re-hydrate
after a client-side navigation swaps the page subtree; when the shell is off, islands
SHALL hydrate once on initial load. Hydration SHALL be **idempotent** — an already-
hydrated node SHALL NOT be double-bound — and SHALL bind only nodes within the freshly
swapped subtree on re-hydration, not the whole document.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5003]], [[SPEC-028]].

### REQ-5004: Shell-Provided Message Bus
The SPA shell SHALL expose exactly two coordination primitives on a stable global
`window.zetl`: a retained **`store(topic)`** and an ephemeral **`bus`**
([[SPEC-050-component-islands-and-messaging#CON-5003]]). Islands SHALL communicate
**only** through these — never by one island importing another, and never via a shared
reactive-store compile unit ([[SPEC-050-component-islands-and-messaging#ADR-5001]]). The
global SHALL be present only when at least one island is emitted on the page (REQ-5012).

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5004]], [[SPEC-050-component-islands-and-messaging#CON-5003]], [[SPEC-050-component-islands-and-messaging#ADR-5001]].

### REQ-5005: Retained Store With Replay-on-Subscribe
`store(topic)` SHALL be last-value-wins state with **replay on subscribe**: a subscriber
SHALL receive the topic's current value immediately on subscription AND on every
subsequent change. An island that mounts (or re-mounts after a navigation) **after** a
value was published SHALL still observe the current value, not miss it. `set(value)` with
an unchanged value MAY be coalesced (no spurious notification); ordering of notifications
to multiple subscribers SHALL be deterministic (subscription order).

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5005]], [[SPEC-050-component-islands-and-messaging#CON-5003]]; [[SPEC-050-component-islands-and-messaging#3.3 HP3]].

### REQ-5006: Persisted Topics
A topic MAY be declared **persisted**, in which case the shell SHALL back it with
`localStorage` and reflect cross-tab changes via the `storage` event. A persisted topic
SHALL declare a **default value**, applied when storage is empty, so first paint is
deterministic. Values read from `localStorage` or a `storage` event are **untrusted
input** and SHALL be recognised against the topic's declared shape before being applied
([[SPEC-050-component-islands-and-messaging#CON-5004]], [[SPEC-050-component-islands-and-messaging#Threat C]]);
a value failing recognition SHALL be discarded in favour of the declared default, never
applied raw.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5006]], [[SPEC-050-component-islands-and-messaging#CON-5004]], [[SPEC-050-component-islands-and-messaging#ADR-5005]]; [[SPEC-050-component-islands-and-messaging#Threat C]], [[SPEC-050-component-islands-and-messaging#Threat F]].

### REQ-5007: Single Bus Instance Surviving Navigation
The bus SHALL be a **single instance that lives on the persistent SPA shell**, not inside
any swapped page subtree, so retained values and subscriptions survive a client-side
navigation. A re-hydrated island re-subscribing to a topic SHALL receive the retained
value (REQ-5005), not a reset.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5007]], [[SPEC-028]].

### REQ-5008: Manifest-Declared Topics and Static Wiring Verification
A [[Component Manifest]] for an island-bearing component MAY declare `publishes` and
`subscribes` — arrays of topic names matching the topic grammar
([[SPEC-050-component-islands-and-messaging#REQ-5011]]). The build SHALL statically
verify wiring: a `subscribes` topic with **no** declared publisher in the resolved
component set SHALL be `island-topic-unpublished` (warning); a malformed topic name SHALL
be `island-topic-malformed` (error); a component whose island publishes/subscribes a
topic **absent from its own manifest** SHALL be `island-topic-undeclared` (warning) so the
manifest stays an accurate contract; a content island subscribing a **trusted** topic with
no matching `[[theme.island-grants]]` entry SHALL be `island-capability-ungranted` (error),
and a content island publishing a non-`content:` topic or lacking `sandbox = true` SHALL be
`island-content-unsandboxed` (error) ([[SPEC-050-component-islands-and-messaging#CON-5002]]).
These declarations are advisory *wiring* metadata — the runtime trust boundary is enforced
by REQ-5010/5015/5016, not by this declaration. The manifest
keys `publishes`/`subscribes`, reserved-and-rejected in [[SPEC-048]] CON-4801, become
accepted under this spec's feature gate.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5008]], [[SPEC-050-component-islands-and-messaging#CON-5002]]; [[SPEC-050-component-islands-and-messaging#3.4 HP4]].

### REQ-5009: Island Wiring Graph (Audit)
Per build, the system SHALL emit an **island wiring graph**: for each island component,
its declared `publishes`/`subscribes` topics and the resolved publisher→subscriber edges,
plus any `island-topic-unpublished`/`island-topic-undeclared` findings, so runtime
coordination is auditable at build time. This extends [[SPEC-048]] OBS-4801.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5008]], [[SPEC-050-component-islands-and-messaging#OBS-5001]].

### REQ-5010: Two Island Trust Tiers — Trusted In-Realm, Content Sandboxed
There SHALL be two island trust tiers, distinguished by author trust:

- **Trusted islands** (theme/component authors) run **in the page realm** with direct
  access to `window.zetl` ([[SPEC-050-component-islands-and-messaging#REQ-5004]]), as
  they ship code that already controls the page.
- **Content-author islands** ([[SPEC-049]]) SHALL run **only inside a sandboxed iframe**
  ([[SPEC-050-component-islands-and-messaging#REQ-5015]]) with an **opaque origin** (the
  `sandbox` attribute carries `allow-scripts` but NOT `allow-same-origin`), so the island
  has **no access to the parent realm, the parent DOM, or `window.zetl`**. Such an island
  SHALL reach the bus only through the capability-scoped bridge
  ([[SPEC-050-component-islands-and-messaging#REQ-5016]]). A content island SHALL NOT
  obtain a publish capability for a trusted (non-content-namespace) topic; it MAY be
  granted a **read-only** (subscribe) capability for a trusted topic only when the theme
  explicitly declares the grant.

This is the enforcement boundary that the v0.1.0 strawman lacked: realm isolation (not
topic-string namespacing) is what prevents a markdown author from reading, forging, or
overwriting a trusted topic such as `theme`
([[SPEC-050-component-islands-and-messaging#ADR-5003]],
[[SPEC-050-component-islands-and-messaging#Threat A]]).

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5010]], [[SPEC-050-component-islands-and-messaging#REQ-5015]], [[SPEC-050-component-islands-and-messaging#REQ-5016]], [[SPEC-049]]; [[SPEC-050-component-islands-and-messaging#Threat A]]; [[SPEC-050-component-islands-and-messaging#3.5 HP5]].

### REQ-5011: Topic Grammar and Trust-Domain Namespace
A topic name SHALL match a declared grammar
([[SPEC-050-component-islands-and-messaging#CON-5001]]): a lowercase, colon-namespaced
identifier (e.g. `theme`, `search:open`). A **reserved `content:` namespace prefix** SHALL
partition the trust domains: a content-author island MAY publish only `content:`-prefixed
topics; trusted topics (no `content:` prefix) are publishable only by trusted in-realm
islands. The namespace is a wiring/clarity aid layered on top of — never a substitute for —
the realm isolation and capability grants of
[[SPEC-050-component-islands-and-messaging#REQ-5010]]. A malformed topic at a declaration
or call site SHALL fail closed (`island-topic-malformed`).

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5011]], [[SPEC-050-component-islands-and-messaging#CON-5001]].

### REQ-5012: Backward-Compatible Default
WHEN a page uses no island-bearing component **and** declares no persisted topic, its build
output SHALL be byte-identical to a [[SPEC-048]]-only build: no `window.zetl`, no bus
runtime, no island `<script>`/iframe, no pre-paint script. The bus/bridge setup SHALL be
emitted (and the [[SPEC-028]] shell SHALL initialise `window.zetl`) **only** on pages that
emit at least one island or persisted-topic pre-paint script — i.e. the shell gates the bus
bootstrap on a build-set marker, so adding SPEC-050 to a vault does not perturb pages that
don't use it. (A page with a persisted topic but no interactive island emits *only* the
REQ-5018 pre-paint script — the one acknowledged exception.) The precise gating mechanism
depends on bus residence ([[SPEC-050-component-islands-and-messaging#12. Open Questions|Q4]])
and SHALL be pinned in IMPL-050.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5012]]; [[SPEC-050-component-islands-and-messaging#3.1 HP1]].

### REQ-5013: Typed Topic Payloads (Recognise at the Bus Boundary)
Every topic SHALL declare a **value type** in its manifest
([[SPEC-050-component-islands-and-messaging#CON-5002]],
[[SPEC-050-component-islands-and-messaging#CON-5005]]). The bus SHALL **recognise every
payload against the topic's declared type before it is stored, replayed, or delivered** —
at `store(topic).set(v)`, `bus.emit(topic, v)`, on the persisted-storage read path
([[SPEC-050-component-islands-and-messaging#REQ-5006]]), AND at the capability bridge
([[SPEC-050-component-islands-and-messaging#REQ-5016]]). A payload that does not conform
to the declared type SHALL be **rejected fail-closed** (`island-payload-type`) — dropped
with a console diagnostic in-realm, refused at the bridge for a sandboxed island, and
replaced by the declared default on a persisted read — never stored or delivered raw.
Type recognition is the LangSec discipline applied to runtime messages: no subscriber
SHALL ever receive an unrecognised value. Two publishers declaring **incompatible** types
for the same topic SHALL be a build error (`island-topic-type-conflict`).

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5013]], [[SPEC-050-component-islands-and-messaging#CON-5005]]; [[SPEC-050-component-islands-and-messaging#Threat C]], [[SPEC-050-component-islands-and-messaging#Threat H]].

### REQ-5015: Content-Island iframe Sandbox
A content-author island SHALL be mounted inside a `<iframe sandbox>` whose token set
includes `allow-scripts` and **excludes `allow-same-origin`**, giving the iframe an opaque
origin and a separate realm; the iframe SHALL NOT be granted `allow-top-navigation`,
`allow-popups`, `allow-modals`, or form/pointer-lock escalations beyond what the component
declares and the theme permits. The iframe document SHALL carry a restrictive
**Content-Security-Policy**; because zetl output is **static** (no per-request nonce is
possible), the island module SHALL be admitted by a build-computed **`'sha256-…'` hash**
source — never `'unsafe-inline'` — and remote origins SHALL be denied unless theme-declared.
The island's code, DOM, storage, and network SHALL be confined to the iframe; it SHALL
communicate with the page **only** through its capability-bridge port
([[SPEC-050-component-islands-and-messaging#REQ-5016]]). The iframe SHALL carry a `title`
(build-derived from the component name) for assistive technology, and the spec'd a11y
behaviour at the frame boundary is defined by
[[SPEC-050-component-islands-and-messaging#REQ-5020]]. The static (no-JS) rendering of the
component SHALL remain the parent-document HTML
([[SPEC-050-component-islands-and-messaging#REQ-5002]]); the iframe enhances, and its
absence (JS off / sandbox unsupported) SHALL leave the static content intact, usable, and
indexable ([[SPEC-002]]).

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5015]], [[SPEC-050-component-islands-and-messaging#REQ-5010]], [[SPEC-050-component-islands-and-messaging#REQ-5020]]; [[SPEC-050-component-islands-and-messaging#Threat A]], [[SPEC-050-component-islands-and-messaging#Threat I]], [[SPEC-050-component-islands-and-messaging#Threat J]].

### REQ-5016: Capability-Scoped Bridge
The shell SHALL connect a sandboxed content island to the bus through a **capability-scoped
bridge**: on mount, the parent creates a `MessageChannel`, records the island's grant
against `port1` in a `WeakMap`, and transfers `port2` to the iframe via the one-time,
nonce-guarded handshake of [[SPEC-050-component-islands-and-messaging#CON-5006]]; that port
is the island's **only** authority to reach the bus (possession-is-authority — [[PROTO-001]]
Principle 15 / capability security). The island SHALL be identified **solely by the object
identity of its port** (the `WeakMap` key) — **NOT** by `MessageEvent.origin` or `.source`,
which are non-discriminating for opaque-origin sandboxed frames and channel-port messages
and SHALL NOT be relied upon ([[SPEC-050-component-islands-and-messaging#Threat J]]). The
parent-side bridge SHALL hold a per-island **grant table** derived from the manifest — the
set of `(topic, direction)` pairs the island may use — and SHALL, on **every** inbound
message, enforce: (a) the message arrived on a known island port whose handshake completed;
(b) `(topic, direction)` is in that island's grant table; (c) the payload conforms to the
topic's declared type ([[SPEC-050-component-islands-and-messaging#REQ-5013]]). A message
failing any check SHALL be answered `denied` with a reason (CON-5006) and never forwarded to
the bus. The island SHALL NOT be able to enumerate, widen, or forge grants — it holds only
the port; the parent is the sole reference monitor. Grants for trusted topics SHALL be
**subscribe-only** and SHALL require an explicit `[[theme.island-grants]]` declaration
([[SPEC-050-component-islands-and-messaging#CON-5002]]); a publish grant for a non-`content:`
topic is unexpressible. The bridge SHALL **rate-coalesce** outbound `update` relays to a
subscribed island (≤ one per animation frame per topic, `[Provisional]`) so a high-frequency
trusted publisher cannot flood or finely time-channel a content island
([[SPEC-050-component-islands-and-messaging#Threat K]]); a granted subscribe to a trusted
topic is a deliberate read capability equivalent to reading that topic's value and SHALL be
surfaced in the audit graph ([[SPEC-050-component-islands-and-messaging#OBS-5001]]).

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5016]], [[SPEC-050-component-islands-and-messaging#CON-5002]], [[SPEC-050-component-islands-and-messaging#CON-5006]], [[SPEC-050-component-islands-and-messaging#REQ-5013]], [[SPEC-050-component-islands-and-messaging#REQ-5017]]; [[SPEC-050-component-islands-and-messaging#Threat A]], [[SPEC-050-component-islands-and-messaging#Threat J]], [[SPEC-050-component-islands-and-messaging#Threat K]]; [[SPEC-050-component-islands-and-messaging#3.5 HP5]].

### REQ-5017: Island & Port Lifecycle Under SPA Navigation
The shell SHALL define a deterministic island lifecycle across [[SPEC-028]] client-side
navigation, since islands mount, unmount, and re-mount as page subtrees swap. On removal of
an island's subtree the shell SHALL (a) **close both ends** of that island's
`MessageChannel` and **revoke** its grant-table/`WeakMap` entry — messages subsequently
arriving on a closed port SHALL be dropped silently (`island-port-closed`); and (b) **release
its bus subscriptions** (invoke the `unsubscribe()` handles from
[[SPEC-050-component-islands-and-messaging#CON-5003]]) so subscriber count does not grow
across navigations toward the [[SPEC-050-component-islands-and-messaging#NFR-5002]] cap. On
re-mount the shell SHALL hydrate idempotently
([[SPEC-050-component-islands-and-messaging#REQ-5003]]) and issue a **fresh** `MessageChannel`
+ grant for a sandboxed island (never reuse a prior port). Retained store *values* persist on
the nav-surviving shell ([[SPEC-050-component-islands-and-messaging#REQ-5007]]); only
per-mount *subscriptions and ports* are torn down and rebuilt.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5017]], [[SPEC-050-component-islands-and-messaging#REQ-5003]], [[SPEC-050-component-islands-and-messaging#REQ-5016]]; [[SPEC-050-component-islands-and-messaging#Threat J]].

### REQ-5018: Persisted Pre-Paint Script (Static, Self-Contained, Fail-Safe)
The render-blocking pre-paint script for persisted topics
([[SPEC-050-component-islands-and-messaging#REQ-5006]],
[[SPEC-050-component-islands-and-messaging#ADR-5005]]) SHALL be a **build-time-generated
static** snippet (no runtime code generation) that runs before the bus module loads, so it
SHALL embed **its own** minimal recogniser for each persisted topic's declared type
([[SPEC-050-component-islands-and-messaging#CON-5005]]) rather than calling the not-yet-loaded
bus. It SHALL wrap all work in `try/catch`, applying the **declared default** on any
parse/type/exception path, and SHALL apply the value only as a pre-agreed DOM signal (e.g. an
attribute on the document root) — never by interpreting the stored string as code or markup.
For static output it SHALL be admitted by a build-computed **`'sha256-…'` CSP hash**, not
`'unsafe-inline'`. This is an explicit, acknowledged exception to
[[SPEC-050-component-islands-and-messaging#1.3 Design Principles|Principle 7]]: a page
declaring a persisted topic emits this one inline script even with no interactive island; a
page with **no** persisted topic and no island emits nothing
([[SPEC-050-component-islands-and-messaging#REQ-5012]]).

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5018]], [[SPEC-050-component-islands-and-messaging#CON-5004]], [[SPEC-050-component-islands-and-messaging#CON-5005]]; [[SPEC-050-component-islands-and-messaging#Threat C]], [[SPEC-050-component-islands-and-messaging#Threat F]].

### REQ-5019: Trusted In-Realm Island Hardening
Because a trusted in-realm island has ambient authority over `window.zetl`, the shell SHALL
reduce blast radius: (a) it SHALL **freeze** the `window.zetl` capability object
(`Object.freeze`, methods non-writable/non-configurable) **before any island script runs**,
so one island cannot replace `store`/`bus` and poison the reference for the bus's own
reference monitor or later islands; and (b) every build-emitted island `<script>` (trusted
and content) SHALL carry a **Subresource Integrity** (`integrity="sha384-…"`) attribute so a
tampered or substituted island asset fails to load. This does not make a malicious trusted
island harmless (it runs first-party code by definition — [[SPEC-050-component-islands-and-messaging#Threat L]]),
but it removes the cheapest escalations (primitive replacement, asset substitution).

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5019]]; [[SPEC-050-component-islands-and-messaging#Threat L]].

### REQ-5020: Sandboxed-Island Accessibility at the Frame Boundary
A content-island iframe ([[SPEC-050-component-islands-and-messaging#REQ-5015]]) SHALL carry a
human-meaningful `title` (build-derived from the component name, author-overridable) so
assistive technology does not announce a bare "frame". The spec SHALL state the
keyboard/focus contract across the iframe boundary: tab order SHALL include the iframe in
document order, and a content island that traps focus or auto-focuses SHALL be a build
warning (`island-focus-trap`). Because the static (no-JS) component HTML is the
accessible-by-default content ([[SPEC-050-component-islands-and-messaging#REQ-5002]]), any
information an island surfaces only inside the iframe SHALL have a no-iframe equivalent in
the parent HTML (WCAG 2.2 AA, [[PROTO-001]] Principle 9).

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5020]], [[SPEC-050-component-islands-and-messaging#REQ-5002]], [[SPEC-050-component-islands-and-messaging#REQ-5015]].

---

## 5. Non-Functional Requirements

### NFR-5001: Hydration Latency
Island hydration SHALL add ≤ 50 ms to time-to-interactive at the 95th percentile for a
page with ≤ 20 island instances of ≤ 5 distinct types, measured on the project reference
CI runner (`[Provisional]`, pin in IMPL-050). The no-island page (REQ-5012) carries zero
hydration cost.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5003]], [[SPEC-050-component-islands-and-messaging#OBS-5002]].

### NFR-5002: No Framework Runtime; Bounded Bus
The shell bus runtime SHALL be ≤ 4 KiB minified (`[Provisional]`) and SHALL pull in no
third-party framework. The bus SHALL enforce fail-closed bounds: ≤ 256 distinct topics,
≤ 1024 total subscribers, ≤ 64 KiB per retained value (`[Provisional]`); a breach SHALL be
dropped with a console diagnostic, never an unbounded allocation.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5004]]; [[SPEC-050-component-islands-and-messaging#Threat D]].

### NFR-5003: Deterministic Island Asset Set
For a given (vault, theme, options) tuple, the emitted island `<script>` set and order
SHALL be byte-identical across repeated builds (no map-iteration-order, no wall-clock),
reusing the [[SPEC-048]] NFR-4802 determinism guarantee.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5001]], [[SPEC-050-component-islands-and-messaging#OBS-5002]].

---

## 6. Architecture Decision Records

### ADR-5001: Inter-Island Messaging Is a Shell Bus, Not a Shared Store Module
Islands coordinate through a shell-provided bus (`store` + `bus`) rather than each island
importing a shared reactive-store compile unit (the Astro/nanostores pattern). (+) Islands
stay mutually decoupled (publish/subscribe by topic name, no cross-import); no shared
compile unit, so the once-per-type emission model holds; one shell capability serves all
islands ([[PROTO-001]] Principle 15). (−) A retained store is marginally more than a bare
emitter. Rejected: a shared imported store (couples islands into a bundle, fights
once-per-type emission); per-island `window` globals (no contract, no audit, collide).
Carried verbatim from the deferred [[SPEC-048]] ADR-4808.

### ADR-5002: Replay-on-Subscribe Retained Store Is the Default Primitive
The default coordination primitive retains the last value and replays it on subscribe; the
ephemeral `bus` is secondary. (+) Correctness survives SPA re-hydration and late hydration
— the dominant islands failure mode (a `CustomEvent` fired before a subscriber mounts is
lost forever) is eliminated by construction; state like `theme` is exactly last-value-wins.
(−) Slightly more than a bare event emitter, and retained values must be bounded
(NFR-5002). Rejected: bare `CustomEvent` only (loses late subscribers — wrong for state).

### ADR-5003: Content Islands Are iframe-Sandboxed With a Capability-Scoped Bridge
Content-author islands are **permitted but isolated**: they run in a sandboxed iframe with
an opaque origin (no `allow-same-origin`), and reach the bus only through a capability-
scoped `postMessage` bridge ([[SPEC-050-component-islands-and-messaging#REQ-5010]],
[[SPEC-050-component-islands-and-messaging#REQ-5015]],
[[SPEC-050-component-islands-and-messaging#REQ-5016]]). (+) Closes the bus-escalation
threat with a **real enforcement boundary** — realm isolation means the island has no
reference to `window.zetl`, and the parent-side bridge is the sole reference monitor,
enforcing per-`(topic, direction)` grants and payload types on every message; a markdown
author can therefore never forge or overwrite a trusted topic. (+) Content authors regain
interactivity (the capability they're missing in a forbid-only model). (−) An iframe per
content island carries layout/perf cost and a `postMessage` hop; trusted theme islands
avoid it by running in-realm (REQ-5010). **Supersedes the v0.1.0 strawman decision** to
forbid content islands. Rejected, and why: *forbid entirely* (the strawman — needlessly
removes a capability now that a sound sandbox exists); *same-realm with topic-namespacing*
(string namespaces don't isolate same-realm JS — unsound, the original [[SPEC-048]] v0.1.1
error); *Web Worker instead of iframe* (no DOM for the island's UI). This is the central
security decision of the spec — it rests on three legs: **realm isolation** (REQ-5015),
**capability scoping** (REQ-5016), and **payload typing** (REQ-5013).

### ADR-5004: Per-Type-Once ES Module Emission, No Framework Runtime
An island is a plain ES module emitted once per component type per page, hydrating
`data-z` nodes — not a framework component. (+) Reuses the [[SPEC-048]] dedup/emission
model and the static-first output; the no-island page stays JS-free. (−) Authors write
vanilla DOM code (mitigated: the surface is small interactive widgets). Rejected: bundling
a reactive framework (breaks JS-free output, bloats the no-island page, new supply-chain
surface).

### ADR-5005: Persisted Topics Carry a Declared Default + Inline Pre-Paint Set
A persisted topic MUST declare a default, and the shell emits a tiny **inline, render-
blocking** pre-paint script that applies the stored (recognised) value before first paint
(REQ-5006). (+) Eliminates flash-of-wrong-theme without waiting for module hydration. (−)
A small inline script is the one render-blocking exception to the otherwise-deferred island
model — justified because FOUC is a first-paint problem hydration cannot solve. The stored
value is treated as untrusted input and recognised before application
([[SPEC-050-component-islands-and-messaging#Threat C]]).

### ADR-5006: SPEC-050 Number Allocation
Allocated **SPEC-050** per [[SPEC-048]] ADR-4807's successor plan (SPEC-049 content
directives, SPEC-050 islands+messaging, SPEC-051 scoped CSS). The manifest keys
`publishes`/`subscribes` — reserved-and-rejected in SPEC-048 CON-4801 — are activated here.

### ADR-5007: Topics Are Typed; Payloads Recognised at the Bus Boundary
Every topic declares a value type, and the bus recognises every payload against it before
storing, replaying, or delivering ([[SPEC-050-component-islands-and-messaging#REQ-5013]],
[[SPEC-050-component-islands-and-messaging#CON-5005]]). (+) Extends the LangSec discipline
to runtime messages — a subscriber never receives an unrecognised value, and the
capability bridge has a concrete schema to validate untrusted iframe messages against
(typing is what makes the bridge's "(b) payload conforms" check meaningful rather than
nominal). (+) Build-time detection of incompatible co-publishers
(`island-topic-type-conflict`). (−) Authors annotate a type per topic; the type language
is deliberately small (CON-5005) to stay validatable ([[PROTO-001]] LangSec principle 6 —
minimise grammatical power). Rejected: opaque/structurally-cloned values (the v0.1.0
`[Blocked: Q3]` position) — they leave the bridge unable to distinguish a valid `theme`
value from arbitrary attacker data, undermining REQ-5016.

### ADR-5008: The Bridge Is a Capability (Port + Reference Monitor), Not an API Surface
The sandboxed island's authority to reach the bus *is* a transferred `MessageChannel`
port; the parent holds the grant table and validates every message (capability security —
possession is authority, the holder cannot widen its grant). (+) No ambient authority: an
island can do exactly what its manifest declared and nothing more, checked at the boundary
on every message; trusted-topic grants are subscribe-only and explicit, so a content
island can *read* `theme` (to style itself) but never *write* it. (+) The reference monitor
is small and centralised (one bridge), not scattered per-island. (−) A `postMessage` hop
per message (acceptable for UI-coordination cadence; NFR-5002 bounds apply). **Identity is
port-object identity, not origin** (`WeakMap<port, grant>` keyed before transfer): a
sandboxed frame's origin is the indistinguishable string `"null"` and channel-port messages
have `source === null`, so an origin/source check would be a no-op — the port reference is
the only sound discriminator, delivered by a one-time nonce-guarded handshake (CON-5006) so
the pre-transfer window cannot be raced. Rejected: relying on `MessageEvent.origin`/`source`
(non-discriminating for opaque-origin frames — the trap an earlier draft fell into); exposing
a filtered `window.zetl` proxy into the iframe (requires `allow-same-origin`, which collapses
the realm isolation REQ-5015 depends on); a per-topic global callback registry (ambient,
unauditable, collision-prone).

---

## 7. Contracts (LangSec)

> Every contract accepts author- or storage-supplied input and declares a grammar; full
> recognition precedes any action ([[PROTO-001]] §LangSec).

### CON-5001: Topic Name Grammar
**Interface:** a topic name at a manifest declaration or a runtime `store`/`bus` call. The
trust partition is **encoded in the grammar** (two disjoint productions), not enforced by a
post-parse semantic check — so a recogniser that implements only the grammar already
rejects a content island's attempt to name a trusted topic.
**Grammar (LL(1), unambiguous — the `content:` prefix is a literal, not an optional `ns`):**
```
topic          = content-topic | trusted-topic ;
content-topic  = "content" ":" segment { ":" segment } ;   (* content-author islands *)
trusted-topic  = segment { ":" segment } ;                 (* MUST NOT begin with "content:" *)
segment        = lower ( lower | digit | "-" ){1,} ;       (* ≥ 2 chars; no leading/trailing "-" *)
lower          = "a".."z" ; digit = "0".."9" ;
```
**Pre-conditions:** ≤ 128 bytes total; ≤ 8 segments; matches exactly one production; a
trusted (theme-author) declaration parses as `trusted-topic` (a `content`-first name is a
`content-topic` and is rejected for a trusted declaration **structurally**); a content
island's published topics parse as `content-topic`.
**Post-conditions:** a validated topic key tagged with its trust domain (content vs
trusted), consumed by the grant table (REQ-5016) and wiring check (REQ-5008). **Error
model:** out-of-grammar or wrong-domain-for-author → `island-topic-malformed` (error) at
build for declarations, dropped with a console diagnostic at runtime for call sites.
**Implements:** [[SPEC-050-component-islands-and-messaging#REQ-5011]].
**Verified by:** [[SPEC-050-component-islands-and-messaging#TEST-5011]].

### CON-5002: Island Manifest Fields (topics, types, persistence, grants)
**Interface:** the island-related keys added to the [[SPEC-048]] CON-4801 manifest, plus
the theme-level capability grants for content islands.
**Grammar (TOML, strict):**
```
publishes   = "publishes"  "=" "[" { topic } "]" ;
subscribes  = "subscribes" "=" "[" { topic } "]" ;
sandbox     = "sandbox" "=" bool ;          (* content islands: MUST be true; trusted: absent/false *)
topics      = "[island.topics]" , { topic-decl } ;
topic-decl  = topic "=" "{" "type" "=" type-expr        (* type-expr per CON-5005 *)
                 [ "," "persisted" "=" bool ]
                 [ "," "default" "=" literal ] "}" ;
(* theme.toml only — authorises a content island to SUBSCRIBE a trusted topic: *)
grant       = "[[theme.island-grants]]" , component-line , topic-line , dir-line ;
dir-line    = "direction" "=" "\"subscribe\"" ;          (* publish grants for trusted topics are not expressible *)
```
**Pre-conditions:** every topic matches CON-5001; each published/subscribed topic has a
`[island.topics]` type declaration; a `persisted = true` topic declares a `default` whose
literal conforms to its `type`; a content-author component's `publishes` are all
`content:`-prefixed (REQ-5011) and its manifest sets `sandbox = true` (REQ-5015); a
`subscribes` of a trusted topic by a content island requires a matching
`[[theme.island-grants]]` entry.
**Post-conditions:** typed island metadata feeding wiring verification (REQ-5008), the
audit graph (REQ-5009), the bridge grant table (REQ-5016), and payload typing (REQ-5013).
**Error model:** malformed topic → `island-topic-malformed`; persisted-without-default or
default-not-of-type → `island-persisted-no-default`; content island publishing a non-`content:`
topic or lacking `sandbox = true` → `island-content-unsandboxed`; content island
subscribing a trusted topic with no grant → `island-capability-ungranted` (all build errors).
**Implements:** [[SPEC-050-component-islands-and-messaging#REQ-5006]], [[SPEC-050-component-islands-and-messaging#REQ-5008]], [[SPEC-050-component-islands-and-messaging#REQ-5013]], [[SPEC-050-component-islands-and-messaging#REQ-5016]].
**Verified by:** [[SPEC-050-component-islands-and-messaging#TEST-5006]], [[SPEC-050-component-islands-and-messaging#TEST-5008]], [[SPEC-050-component-islands-and-messaging#TEST-5016]].

### CON-5003: Bus Runtime API (`window.zetl`)
**Interface:** the client-side bus surface.
```
window.zetl.store(topic) -> {
    get():        current value (or the declared/initial default),
    set(value):   replace value, notify subscribers (coalesce if unchanged),
    subscribe(fn): fn(value) called immediately with current value, then on each change;
                   returns an unsubscribe() function
}
window.zetl.bus = {
    emit(topic, detail): fire-and-forget, NO retain, NO replay,
    on(topic, fn):       returns an off() function
}
```
**Pre-conditions:** `topic` matches CON-5001; `value`/`detail` conform to the topic's
declared type ([[SPEC-050-component-islands-and-messaging#CON-5005]], REQ-5013) and are
structured-clone-safe; bounds within NFR-5002.
**Post-conditions:** retained-store invariants (REQ-5005); bus is non-retaining; both
survive navigation (REQ-5007); every delivered value has been type-recognised.
**Error model:** malformed topic, type mismatch (`island-payload-type`), or bound breach →
dropped with a console diagnostic, never a throw that breaks unrelated islands.
**Implements:** [[SPEC-050-component-islands-and-messaging#REQ-5004]], [[SPEC-050-component-islands-and-messaging#REQ-5005]], [[SPEC-050-component-islands-and-messaging#REQ-5007]].
**Verified by:** [[SPEC-050-component-islands-and-messaging#TEST-5004]], [[SPEC-050-component-islands-and-messaging#TEST-5005]].

### CON-5004: Persisted-Topic Storage Encoding
**Interface:** the `localStorage` key/value for a persisted topic and the `storage`-event
read path. **This is an untrusted-input boundary** — another tab, an extension, or a prior
version may have written the value.
**Grammar:** key = `zetl:topic:<topic>`; value = a JSON document conforming to the topic's
declared **type** ([[SPEC-050-component-islands-and-messaging#CON-5005]]); a recogniser
rejects non-conforming or oversized values.
**Pre-conditions:** the read uses **`event.newValue`** from the `storage` event directly
(never a follow-up `localStorage.getItem`, which would re-introduce a TOCTOU race with a
concurrent cross-tab write); the value is ≤ the per-value cap (NFR-5002) and conforms to the
topic's declared type. A `newValue` of `null` (key deleted) or the empty string is treated
as "absent" and resolves to the declared default **without** a JSON parse.
**Post-conditions:** a recognised value applied to the store; on any failure (parse, type,
size, or `null`/empty) the declared **default** is applied and the bad entry is overwritten.
**Error model:** parse/type/size/null failure → discard + default (never apply raw —
[[SPEC-050-component-islands-and-messaging#Threat C]]).
**Implements:** [[SPEC-050-component-islands-and-messaging#REQ-5006]], [[SPEC-050-component-islands-and-messaging#REQ-5013]].
**Verified by:** [[SPEC-050-component-islands-and-messaging#TEST-5006]].

### CON-5005: Topic Value Type
**Interface:** the declared value type of a topic (`[island.topics].<topic>.type`), used by
the bus, the persisted-read path, and the capability bridge to recognise every payload.
**Grammar (deliberately small — LangSec principle 6):**
```
type-expr   = "string" | "bool" | "int" | "number"
            | "enum(" literal { "," literal } ")"     (* closed value set *)
            | "{" field { "," field } "}" ;           (* strict flat record *)
field       = ident ":" scalar-type ;
scalar-type = "string" | "bool" | "int" | "number" | "enum(" literal { "," literal } ")" ;
literal     = json-string ;                            (* JSON string token, double-quoted, *)
                                                       (* no control chars; enum match is case-sensitive *)
```
**Value-recognition semantics (the recogniser the bus/bridge/persisted-read apply):**
- `string` — a JSON string token (no control chars).
- `bool` — JSON `true`/`false` only.
- `int` — a JSON number with no fractional/exponent part, in `[-(2^53-1), 2^53-1]`;
  `NaN`/`Infinity`/`-Infinity`/`-0`-as-`"−0"` rejected.
- `number` — a **finite** JSON number; `NaN`/`Infinity`/`-Infinity` rejected (note
  `JSON.parse("1e400") → Infinity`, so this MUST be checked, not assumed).
- `enum(...)` — value equals one listed `literal` (case-sensitive).
- record — a JSON object whose key set **equals** the declared field set (strict: missing
  OR extra fields are a violation), each field's value conforming to its `scalar-type`. The
  recogniser MUST build the value on a null-prototype object and MUST reject `__proto__`,
  `constructor`, `prototype` as field keys ([[SPEC-050-component-islands-and-messaging#Threat H]]).
**Pre-conditions:** the type-expr parses; a topic's `default` literal conforms to it.
**Post-conditions:** a recogniser that accepts exactly the conforming values; **no nested/
recursive shapes** in v1 (records are flat — keeps validation decidable and the bridge
cheap). The recogniser is applied to the raw JSON token stream by the topic's type, never
"parse arbitrary JSON, then check" (which would admit prototype pollution before the check).
**Error model:** unparseable type-expr → `island-topic-type-invalid`; a runtime/stored/
bridged value outside the type → `island-payload-type` (drop + default/diagnostic, never
deliver).
**Implements:** [[SPEC-050-component-islands-and-messaging#REQ-5013]].
**Verified by:** [[SPEC-050-component-islands-and-messaging#TEST-5013]].

### CON-5006: Capability-Bridge `postMessage` Protocol
**Interface:** the wire protocol between a sandboxed content-island iframe and the
parent-side bridge ([[SPEC-050-component-islands-and-messaging#REQ-5016]]). **The iframe is
untrusted** — every inbound message is recognised before any bus action.

**Island identification — port identity, NOT origin (the load-bearing correction):** a
sandboxed iframe without `allow-same-origin` has an **opaque origin** (`MessageEvent.origin`
is the literal `"null"` for *every* such frame) and messages on a `MessageChannel` port
carry `source === null` — so origin/source checks are **non-discriminating and MUST NOT be
relied upon**. The bridge identifies an island **solely by the object identity of its
transferred `MessageChannel` port**, which the parent created and holds in a
`WeakMap<MessagePort, Grant>` keyed *before* transfer. Each island has its own
`MessageChannel`; the parent listens on `port1` and transfers `port2` to exactly one iframe.

**Handshake (closes the one interception window):** the parent transfers the port **inside
the iframe's `srcdoc`/initial document via a one-time `init` message bearing a random
`nonce`**; the iframe MUST echo the nonce in a `ready` message on the port before the parent
processes any bus-affecting message, and the parent ignores `ready` with a wrong/replayed
nonce. No port is transferred over `window.postMessage` to a yet-unidentified frame.

**Grammar (messages on the transferred port; `value` is recognised *as the topic's type* —
CON-5005 — not as arbitrary JSON):**
```
init       = "{" "\"op\":\"init\","       "\"nonce\":" json-string "}" ;   (* parent → iframe, with the port *)
ready      = "{" "\"op\":\"ready\","      "\"nonce\":" json-string "}" ;   (* iframe → parent, echoes nonce *)
publish    = "{" "\"op\":\"publish\","    "\"topic\":" topic "," "\"value\":" typed-value "}" ;
emit       = "{" "\"op\":\"emit\","       "\"topic\":" topic "," "\"value\":" typed-value "}" ;
subscribe  = "{" "\"op\":\"subscribe\","  "\"topic\":" topic "}" ;
unsubscribe= "{" "\"op\":\"unsubscribe\"," "\"topic\":" topic "}" ;
(* parent → iframe responses / relays *)
ack        = "{" "\"op\":\"ack\","        "\"topic\":" topic "}" ;          (* subscribe granted *)
denied     = "{" "\"op\":\"denied\","     "\"topic\":" topic "," "\"reason\":" reason "}" ;
update     = "{" "\"op\":\"update\","     "\"topic\":" topic "," "\"value\":" typed-value "}" ;
reason     = "\"ungranted\"" | "\"type\"" | "\"cap-exceeded\"" | "\"malformed\"" ;
typed-value= (* the JSON encoding of the topic's declared type, recognised per CON-5005;
                NOT the full JSON grammar — recognised structurally against the type *)
```
**Pre-conditions (enforced by the parent on every inbound msg):** the message arrives on the
island's own port object (WeakMap hit); the handshake `ready`/nonce has completed; `op`/
`topic` parse (CON-5001); `(topic, op-direction)` is in the island's grant table (REQ-5016);
`value` conforms to the topic's type (CON-5005); size/rate within NFR-5002 (and the
outbound `update` relay is rate-coalesced — REQ-5016).
**Post-conditions:** a conforming, granted request is forwarded to the real bus; a granted
`subscribe` is answered with `ack` then `update`s (each re-recognised); a refusal is
answered with `denied` + reason (so silence ≠ denial — closes the probe oracle); a closed
port (post-teardown, REQ-5017) drops messages silently.
**Error model:** unknown port / failed handshake → ignored; ungranted topic/direction →
`denied:ungranted` (+ `island-capability-denied` counter); type mismatch →
`denied:type` (+ `island-payload-type`); cap breach → `denied:cap-exceeded`; malformed →
`denied:malformed`. No inbound message ever reaches the bus without passing all checks.
**Implements:** [[SPEC-050-component-islands-and-messaging#REQ-5016]], [[SPEC-050-component-islands-and-messaging#REQ-5017]].
**Verified by:** [[SPEC-050-component-islands-and-messaging#TEST-5016]].

---

## 8. Threat Model

Trust boundary: **theme/component authors are trusted** (they ship in-realm JS);
**content-author island code is untrusted** and runs only in a sandboxed iframe reaching
the bus through a capability bridge (REQ-5010/5015/5016); **`localStorage`/cross-tab values
and all inbound bridge messages are untrusted input** crossing into the runtime.

### Threat A: Island-Bus Escalation via a Content Author
A markdown author ships a content island trying to read/forge/overwrite a trusted topic
(`theme`) on `window.zetl`. **Mitigation (defense in depth, three legs):** (1) **realm
isolation** — the island runs in a sandboxed iframe with an opaque origin and no
`allow-same-origin`, so it holds no reference to the parent realm or `window.zetl`
(REQ-5015); (2) **capability scoping** — its only authority is a transferred port whose
grant table the parent enforces on every message; trusted-topic grants are subscribe-only
and theme-declared, so it can at most *read* `theme`, never *write* it (REQ-5016); (3)
**payload typing** — even granted messages must conform to the topic's declared type
(REQ-5013). String-namespacing is a clarity aid, explicitly **not** the boundary
(ADR-5003). Supersedes the v0.1.0 forbid-only mitigation.

### Threat B: Silent Mis-Wiring (Topic Typo)
A `subscribes`/`publishes` magic-string typo silently breaks coordination. **Mitigation:**
static wiring verification at build (`island-topic-unpublished` / `-undeclared` /
`-malformed`) + the audit wiring graph (REQ-5008/5009).

### Threat C: `localStorage` Poisoning / Cross-Tab Injection
A persisted value written by another tab/extension/version is malformed, oversized, or a
hostile payload, hoping to be applied raw on read or `storage` event. **Mitigation:**
CON-5004 recognises every stored value before application; non-conforming/oversized values
are discarded for the declared default and overwritten — storage is treated as untrusted
input ([[PROTO-001]] §LangSec).

### Threat D: Bus Resource Exhaustion
An island registers unbounded topics/subscribers or sets a huge retained value to exhaust
memory. **Mitigation:** NFR-5002 fail-closed bounds; over-cap operations dropped with a
diagnostic.

### Threat E: Script Injection via an Island Module
A hostile `<name>.js`. **Mitigation:** a *trusted* island is theme-author code (same
posture as [[SPEC-048]] ADR-4801 for CSS) and runs in-realm. A *content* island is
untrusted but confined to the iframe sandbox (REQ-5015), so hostile code there cannot reach
the parent realm, the DOM, or the bus except through the capability bridge.

### Threat F: Flash of Wrong Theme (FOUC) as a UX Defect
The persisted theme applies only after async hydration, flashing the wrong theme.
**Mitigation:** declared default + inline render-blocking pre-paint set (REQ-5006,
ADR-5005). Listed as a threat because a first-paint regression is a real, observable defect
the design must prevent.

### Threat H: Payload Type Confusion
A publisher (or a poisoned persisted/bridged value) sends a value of the wrong shape for a
topic, hoping a subscriber mis-handles it (e.g. an object where `theme` expects
`enum("light","dark")`). **Mitigation:** REQ-5013 / CON-5005 recognise every payload
against the topic's declared type at the bus, the persisted-read path, and the bridge;
non-conforming values are dropped/defaulted, never delivered.

### Threat I: iframe Sandbox Escape / Escalation
A content island tries to break out of the sandbox — navigating the top frame, opening
popups, or reaching the parent DOM. **Mitigation:** the `sandbox` token set grants only
`allow-scripts` (no `allow-same-origin`, `allow-top-navigation`, `allow-popups`,
`allow-modals`), and a restrictive CSP confines network/inline execution (REQ-5015); the
opaque origin denies same-origin DOM/storage access by construction.

### Threat J: `postMessage` Spoofing / Confused-Source Injection
A script (another `"null"`-origin frame, an extension, or a stale/torn-down island) posts
messages to the bridge pretending to be a granted island. **Note the trap this corrects:**
a sandboxed iframe's `MessageEvent.origin` is the literal `"null"` for *every* such frame
and channel-port messages carry `source === null`, so **origin/source checks are
non-discriminating and are explicitly NOT the mechanism**. **Mitigation:** the bridge
accepts messages only on each island's **own `MessageChannel` port object** (the
`WeakMap<port, grant>` key created before transfer — REQ-5016), the port is delivered by a
**one-time nonce-guarded handshake** so the pre-transfer window can't be raced (CON-5006),
and a torn-down island's port is **closed and revoked** (REQ-5017) so stale messages are
dropped. Port-object identity, not origin, is the discriminator.

### Threat L: Compromised / Supply-Chained Trusted Island
A trusted in-realm island ships malicious or supply-chain-compromised JS — it has ambient
`window.zetl` authority and could replace bus primitives, read every retained value, or
publish any topic. **Mitigation (blast-radius reduction, not elimination):** the shell
**freezes** `window.zetl` before any island runs so primitives can't be replaced, and every
island `<script>` carries **SRI** so a substituted asset fails to load (REQ-5019). Residual
risk is explicit and accepted: a trusted island is first-party code by definition; full
defense would require sandboxing trusted islands too, which is out of scope (theme authors
are the trust root, as for theme CSS/templates in [[SPEC-048]]). Operators reduce residual
risk by vetting theme islands and avoiding remote island imports.

### Threat K: Capability Over-Grant, Over-Read, or Relay Flooding
A theme grants a content island more authority than intended (e.g. publish on `theme`), or a
*granted* subscribe is abused as an oracle — finely timing trusted-topic updates to
side-channel user activity, or a high-frequency trusted publisher floods the island.
**Mitigation:** trusted-topic publish grants are **inexpressible** in the manifest grammar
(subscribe-only, CON-5002); content islands publish only `content:` topics; grants are
theme-declared and enumerated in the audit graph (REQ-5009/OBS-5001). For the granted-read
risk, the bridge **rate-coalesces** outbound `update` relays (≤ one per animation frame per
topic — REQ-5016), blunting both flooding and fine-grained timing; and the spec records that
a granted subscribe **is** a deliberate read of that topic (equivalent to reading its
persisted value), so themes grant it consciously. The parent bridge is the sole reference
monitor — no island can widen its own grant (REQ-5016).

---

## 9. Test Specifications

> Positive / negative-input / negative-output per [[PROTO-001]] §9. AI-synthesised spec →
> adversarial testing is **mandatory** before convergence; browser-level tests reuse the
> [[SPEC-028]] harness.

### TEST-5001: Gated Deduped Deterministic Emission
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5001]], [[SPEC-050-component-islands-and-messaging#NFR-5003]]. Positive: a JS-bearing component used 3× emits one module script. Negative-input: a page with no JS component emits no island script. Negative-output: two builds are byte-identical (stable order).

### TEST-5002: Progressive Enhancement
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5002]]. Positive: component renders + enhances with JS on. Negative-input: JS disabled → static HTML+CSS renders and is usable. Negative-output: no island-only content is missing from the static/indexed HTML.

### TEST-5003: SPA Hydration + Re-Hydration + Latency
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5003]], [[SPEC-050-component-islands-and-messaging#NFR-5001]]. Positive: islands hydrate on load; re-hydrate after a client-side nav. Negative-input: an already-hydrated node is not double-bound. Negative-output: hydration latency breach fails the NFR gate.

### TEST-5004: Bus Presence + Bounds
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5004]], [[SPEC-050-component-islands-and-messaging#CON-5003]], [[SPEC-050-component-islands-and-messaging#NFR-5002]]. Positive: `window.zetl.store`/`bus` exist when an island is present. Negative-input: over-cap topics/subscribers dropped with a diagnostic, no throw. Negative-output: `bus` does not retain (a later subscriber gets nothing).

### TEST-5005: Replay-on-Subscribe vs Ephemeral
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5005]], [[SPEC-050-component-islands-and-messaging#REQ-5007]]. Positive: a subscriber mounting AFTER `store('theme').set('dark')` — and one re-hydrated after a simulated nav — both read `'dark'`. Negative-input: unchanged `set` coalesces. Negative-output: a `bus.emit` fired before any listener existed is NOT replayed (distinguishes `bus` from `store`).

### TEST-5006: Persisted Topics + Untrusted-Storage Recognition
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5006]], [[SPEC-050-component-islands-and-messaging#CON-5004]], [[SPEC-050-component-islands-and-messaging#Threat C]]. Positive: a persisted `theme` round-trips `localStorage` and reflects a `storage` event. Negative-input: a malformed/oversized stored value is discarded for the declared default (not applied raw); `persisted` without `default` → `island-persisted-no-default`. Negative-output: no flash — the pre-paint script applies the stored value before first paint (HP6).

### TEST-5008: Manifest Topics + Wiring Verification + Graph
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5008]], [[SPEC-050-component-islands-and-messaging#REQ-5009]], [[SPEC-050-component-islands-and-messaging#CON-5002]]. Positive: publisher/subscriber pair resolves; wiring graph shows the edge. Negative-input: malformed topic → `island-topic-malformed`; subscriber with no publisher → `island-topic-unpublished` (warning); island publishing an undeclared topic → `island-topic-undeclared` (warning). Negative-output: the graph lists every dangling edge.

### TEST-5010: Two Trust Tiers — In-Realm vs Sandboxed
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5010]], [[SPEC-050-component-islands-and-messaging#Threat A]]. Positive: a theme island runs in-realm with direct `window.zetl`; a content island renders inside a sandboxed iframe. Negative-input: a content component without `sandbox = true`, or publishing a non-`content:` topic → build error. Negative-output: the content iframe has no reference to the parent `window.zetl` (opaque origin; `window.parent.zetl` is unreachable).

### TEST-5013: Typed Payload Recognition
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5013]], [[SPEC-050-component-islands-and-messaging#CON-5005]], [[SPEC-050-component-islands-and-messaging#Threat H]]. Positive: an `enum("light","dark")` `theme` accepts `"dark"`. Negative-input: `set("blue")` or an object → `island-payload-type`, dropped, subscribers unaffected; two publishers declaring incompatible types → `island-topic-type-conflict` at build. Negative-output: no subscriber, persisted read, or bridge delivery ever yields an unrecognised value.

### TEST-5015: Content-Island iframe Sandbox
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5015]], [[SPEC-050-component-islands-and-messaging#Threat I]]. Positive: the island enhances inside the sandboxed iframe. Negative-input: sandbox token set lacks `allow-same-origin`/`allow-top-navigation`/`allow-popups`; CSP present. Negative-output: with JS off or sandbox unsupported, the parent-document static HTML renders, is usable, and is indexable; a top-navigation/parent-DOM attempt from inside fails.

### TEST-5016: Capability-Scoped Bridge (Port-Identity, Handshake, Grants)
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5016]], [[SPEC-050-component-islands-and-messaging#CON-5006]], [[SPEC-050-component-islands-and-messaging#Threat A]], [[SPEC-050-component-islands-and-messaging#Threat J]], [[SPEC-050-component-islands-and-messaging#Threat K]]. Positive: a content island completes the nonce handshake, publishes its `content:filter` grant, and reads a theme-granted `theme` subscribe (gets `ack` then rate-coalesced `update`s). Negative-input: publishing `theme` (no publish grant) → `denied:ungranted`; a message before/with a wrong handshake nonce → ignored; a message posted to `window` (origin `"null"`) or on any port other than the island's own → ignored (the bridge keys on **port object identity**, asserting an origin check would NOT distinguish it); a publish grant for a trusted topic is unexpressible in the manifest grammar. Negative-output: no ungranted/unhandshaken/type-invalid message ever reaches the real bus (fuzz the `postMessage` protocol incl. `__proto__` payloads).

### TEST-5017: Island & Port Lifecycle Under SPA Nav
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5017]], [[SPEC-050-component-islands-and-messaging#Threat J]]. Positive: navigating away closes the island's port and releases its subscriptions; navigating back issues a fresh port + grant and re-hydrates idempotently. Negative-input: a message on a closed (torn-down) port → dropped (`island-port-closed`). Negative-output: repeated nav cycles do not grow the subscriber count toward the NFR-5002 cap (assert bounded after N cycles); a retained store value survives the nav (REQ-5007) while subscriptions do not.

### TEST-5018: Persisted Pre-Paint Script
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5018]], [[SPEC-050-component-islands-and-messaging#CON-5005]], [[SPEC-050-component-islands-and-messaging#Threat F]]. Positive: a returning visitor's persisted `theme` is applied before first paint (no flash); the snippet is admitted by its `'sha256-…'` CSP source. Negative-input: a poisoned/oversized/`null` stored value → declared default applied, no exception escapes the `try/catch`. Negative-output: the snippet never interprets the stored string as code/markup; a page with no persisted topic and no island emits no pre-paint script (REQ-5012).

### TEST-5019: Trusted-Island Hardening
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5019]], [[SPEC-050-component-islands-and-messaging#Threat L]]. Positive: `window.zetl` is frozen before islands run; emitted island `<script>`s carry `integrity`. Negative-input: an island attempting `window.zetl.store = …` fails (frozen); a tampered island asset (hash mismatch) fails to load. Negative-output: a later island still sees the genuine `store`/`bus` primitives after an earlier island's mutation attempt.

### TEST-5020: Sandboxed-Island Accessibility
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5020]], [[SPEC-050-component-islands-and-messaging#REQ-5002]]. Positive: the iframe carries a meaningful `title`; the frame sits in document tab order. Negative-input: a content island that auto-focuses/traps focus → `island-focus-trap` warning. Negative-output: information shown only inside the iframe has a parent-HTML (no-JS) equivalent (WCAG 2.2 AA).

### TEST-5011: Topic Grammar
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5011]], [[SPEC-050-component-islands-and-messaging#CON-5001]]. Positive: `theme`, `search:open` accepted. Negative-input: `Theme`, `a b`, an over-long, or reserved-namespace-by-trusted topic → `island-topic-malformed`. Negative-output: a malformed runtime topic is dropped, not silently coerced.

### TEST-5012: Backward-Compatible Default
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5012]]. Positive: a no-island vault builds byte-identically to the SPEC-048 baseline (golden). Negative-input: no `window.zetl`/bus/pre-paint script when no island is used. Negative-output: an unused build is unchanged.

---

## 10. Observability

### OBS-5001: Island Wiring Graph
Per build, emit the island components, their declared `publishes`/`subscribes`, resolved
publisher→subscriber edges, and any `island-topic-unpublished`/`-undeclared` findings
(extends [[SPEC-048]] OBS-4801).
**Trace:** [[SPEC-050-component-islands-and-messaging#REQ-5008]], [[SPEC-050-component-islands-and-messaging#REQ-5009]].

### OBS-5002: Island Asset Set + Dedup Ratio
Emit the set of island scripts emitted, the dedup ratio (distinct scripts vs island
instances), and a stable hash of each, to detect nondeterminism regressions.
**Trace:** [[SPEC-050-component-islands-and-messaging#NFR-5003]].

### OBS-5003: Bound Rejections
Emit counts of the **build** errors `island-topic-malformed`, `island-topic-unpublished`,
`island-topic-undeclared`, `island-persisted-no-default`, `island-topic-type-conflict`,
`island-topic-type-invalid`, `island-content-unsandboxed`, `island-capability-ungranted`,
and the `island-focus-trap` warning; plus **runtime** counters (dev console / optional debug
channel) for `island-payload-type`, `island-capability-denied` (by `denied` reason), and
`island-port-closed`, so fail-closed events are auditable. The audit wiring graph (OBS-5001)
additionally lists, per content island, its iframe-sandbox status and its granted
`(topic, direction)` capabilities.
**Trace:** [[SPEC-050-component-islands-and-messaging#REQ-5008]], [[SPEC-050-component-islands-and-messaging#REQ-5010]], [[SPEC-050-component-islands-and-messaging#REQ-5013]], [[SPEC-050-component-islands-and-messaging#REQ-5016]], [[SPEC-050-component-islands-and-messaging#REQ-5017]].

---

## 11. Composition-First Feasibility (Principle 15)

| Capability | Existing primitive attempted | Outcome / placement |
| ---------- | ---------------------------- | ------------------- |
| Per-type-once asset emission | [[SPEC-048]] REQ-4809 CSS dedup/emission model | **Compose** — islands reuse the same dedup + determinism path ([[SPEC-050-component-islands-and-messaging#REQ-5001]]) |
| Hydration marker | [[SPEC-048]] `data-z="<name>"` component marker | **Compose** — hydrate `data-z` nodes; no new marker ([[SPEC-050-component-islands-and-messaging#REQ-5001]]) |
| Persistent client runtime | [[SPEC-028]] SPA shell (already nav-surviving) | **Extend** — add `store`/`bus` to the existing shell, not a per-island module ([[SPEC-050-component-islands-and-messaging#ADR-5001]]) |
| Manifest topic fields | [[SPEC-048]] CON-4801 manifest (keys reserved there) | **Extend** — activate `publishes`/`subscribes`/`[island]` ([[SPEC-050-component-islands-and-messaging#CON-5002]]) |
| Wiring audit | [[SPEC-048]] OBS-4801 component stats | **Extend** — add the island wiring graph ([[SPEC-050-component-islands-and-messaging#REQ-5009]]) |
| Retained replay-store | *(none)* | **New** — the genuinely new runtime; justified because no existing primitive survives SPA re-hydration ([[SPEC-050-component-islands-and-messaging#ADR-5002]]) |
| Untrusted-island isolation | platform `<iframe sandbox>` (opaque origin) | **Compose (platform)** — the browser's own sandbox is the isolation primitive; no custom sandbox built ([[SPEC-050-component-islands-and-messaging#REQ-5015]], [[PROTO-001]] Simplicity-Ladder rung 3) |
| Sandbox↔bus channel | platform `MessageChannel` + `postMessage` | **Compose (platform)** — transferred port *is* the capability; parent is the reference monitor ([[SPEC-050-component-islands-and-messaging#REQ-5016]]) |
| Payload recognition | the `[island.topics]` type + a small recogniser | **New** — a deliberately tiny type language (CON-5005), the LangSec recogniser for runtime messages ([[SPEC-050-component-islands-and-messaging#REQ-5013]]) |

Net new surface is confined to (a) the ≤ 4 KiB shell bus runtime (`store`/`bus` + the
capability-bridge reference monitor), (b) the inline persisted-topic pre-paint script, and
(c) the small topic-type recogniser. Realm isolation and the sandbox↔bus channel reuse
**platform** primitives (`<iframe sandbox>`, `MessageChannel`) rather than anything bespoke;
everything else composes [[SPEC-048]] and [[SPEC-028]].

---

## 12. Open Questions

- **Q1 — Persisted-default / FOUC mechanism.** Exact shape of the inline pre-paint script
  and how the declared default is threaded into it (`[Blocked: Q1]`,
  [[SPEC-050-component-islands-and-messaging#ADR-5005]]). Phase 1: how do operators expect
  theme persistence to behave on first paint and cross-tab?
- **Q2 — Sandboxed content-author islands.** *Resolved (v0.2.0):* content islands are
  **permitted, in an `<iframe sandbox>` with a capability-scoped bridge**
  ([[SPEC-050-component-islands-and-messaging#REQ-5010]],
  [[SPEC-050-component-islands-and-messaging#REQ-5015]],
  [[SPEC-050-component-islands-and-messaging#REQ-5016]],
  [[SPEC-050-component-islands-and-messaging#ADR-5003]]), superseding the strawman's
  forbid-only stance. *Still open:* whether a Worker-based variant is also offered for
  non-DOM content islands, and the exact iframe layout/auto-resize ergonomics.
- **Q3 — Typed topic payloads.** *Resolved (v0.2.0):* topics are **typed**; the bus, the
  persisted-read path, and the bridge recognise every payload against a small declared type
  ([[SPEC-050-component-islands-and-messaging#REQ-5013]],
  [[SPEC-050-component-islands-and-messaging#CON-5005]]). *Still open:* whether the type
  language needs nested/record shapes beyond the v1 flat-record cap.
- **Q4 — Bus residence.** Does the bus + bridge reference-monitor live inside the existing
  SPEC-028 shell module or a new sibling shell module? Determines load order vs the
  pre-paint script.
- **Q5 — Delivery/ordering guarantees.** Beyond per-subscriber subscription-order, are any
  cross-topic ordering or synchronous-vs-microtask delivery guarantees required (incl. the
  added `postMessage` hop latency for sandboxed islands)? Pin in IMPL-050.
- **Q6 — iframe cost at scale.** Per-content-island iframes carry layout/memory cost; is a
  shared-iframe-per-page (multiplexed bridge) needed when a page has many content islands?
  Ground against Phase 1 page profiles.

---

## 13. Convergence Status

**NOT converged** (but materially closer). A strawman revised to a sandbox + capability +
typing model, then hardened in v0.3.0 by **one fresh-context, cross-model adversarial review**
([[PROTO-001]] Principle 12) whose 14 findings (4 Blocking / 6 Major / 4 Minor) are now
resolved in-spec — most importantly the F1 origin-vs-port-identity defect (the bridge can't
use `MessageEvent.origin`/`source` for opaque-origin frames; it now keys on port identity via
a nonce handshake). Before the Phase 2 gate this spec still requires: (1) a **second**
independent review from a clean context (ideally a different model again, or a human security
domain expert — this is a Tier-2 trust boundary) to confirm the F1/F3/F4/F6 fixes are sound
and not merely relocated, with **adversarial fuzzing** of the bridge `postMessage` protocol
and the type recogniser; (2) Phase 1 operator/theme-author profiles to ground every
`[Provisional]` value and close Q1/Q4/Q5/Q6; (3) feasibility spikes for the ≤ 4 KiB
replay-store bus and the iframe capability bridge (incl. the nonce handshake + port teardown)
against the [[SPEC-028]] shell; (4) IMPL-050 to pin the grammars (topic-type, bridge-protocol,
topic-name), the numeric bounds, the sandbox token set + `sha256` CSP, the pre-paint script,
and the lifecycle teardown. It depends on [[SPEC-048]] (the component + `data-z` substrate) and
[[SPEC-049]] (the content-author component surface the sandbox isolates) and gates
independently of both.

---

## Changelog

<details>
<summary>Revision history — 0.1.0 → 0.3.0</summary>

- **0.3.0** (2026-06-24) — *normative; adversarial-review fixes.* Applied a fresh-context
  (cross-model) adversarial review (4 Blocking / 6 Major / 4 Minor). **F1 (most dangerous):**
  the bridge can't identify islands by `MessageEvent.origin`/`source` — a sandboxed frame's
  origin is the literal `"null"` and channel-port `source` is `null`; rewrote REQ-5016 /
  CON-5006 / ADR-5008 / Threat J to identify by **port-object identity** (`WeakMap` keyed
  before transfer) via a **nonce-guarded handshake**. **F3/F8:** completed the LangSec
  grammars — CON-5006 `value` is recognised *as the topic type* (not bare JSON; null-proto,
  `__proto__` rejected) with `ack`/`denied`/`unsubscribe`; CON-5005 now defines `literal`,
  `int`/`number` bounds (reject `NaN`/`Infinity`), and strict flat records. **F4:** CON-5001
  grammar made unambiguous with the `content:`/trusted partition encoded structurally. **F6:**
  added REQ-5017 (port/subscription lifecycle + teardown under SPA nav). **F7:** added
  REQ-5018 (static, self-contained, `sha256`-CSP pre-paint script; acknowledged Principle-7
  exception). **F2:** added REQ-5019 (freeze `window.zetl` + SRI) and Threat L (supply-chain
  residual). **F5:** rate-coalesced subscribe relay + read-disclosure note (REQ-5016, Threat
  K). **F9:** fixed Design Principle 5 (still said "forbidden"). **F10:** CON-5004 uses
  `event.newValue`, handles `null` delete (TOCTOU). **F11/F13:** REQ-5012 gating clause;
  REQ-5008 error taxonomy. **F14:** added REQ-5020 (iframe a11y). Added TEST-5017–5020.
- **0.2.0** (2026-06-24) — *normative; security-model revision.* Replaced the v0.1.0
  "content islands forbidden" stance with a **sandbox + capability + typing** model on
  stakeholder direction: content-author islands now run in an `<iframe sandbox>` (opaque
  origin, no `window.zetl` — REQ-5015) and reach the bus only via a **capability-scoped
  `postMessage` bridge** the parent reference-monitors (REQ-5016, CON-5006); topics are
  **typed** and every payload is recognised at the bus, persisted-read, and bridge
  boundaries (REQ-5013, CON-5005). Trusted theme islands still run in-realm (REQ-5010, two
  tiers). Reversed ADR-5003; added ADR-5007 (typing) + ADR-5008 (bridge-as-capability);
  extended CON-5002 (typed topics + theme-declared subscribe-only grants); added Threats
  H (type confusion), I (sandbox escape), J (postMessage spoofing), K (capability
  over-grant); added TEST-5013/5015/5016. **Resolves Q2 (sandbox) and Q3 (typed)**; adds
  Q6 (iframe cost at scale). Net-new surface still small — realm isolation and the channel
  reuse platform `<iframe sandbox>` + `MessageChannel`.
- **0.1.0** (2026-06-24) — *initial strawman.* Extracted and reframed from the [[SPEC-048]]
  v0.1.1 island/bus clauses (REQ-4810/4816/4817, ADR-4808) that the SPEC-048 v0.2.0
  tightening deferred. Key reframe vs that draft: islands are a **trusted-author-only**
  surface and content-author islands are **forbidden in v1** (ADR-5003/REQ-5010), closing
  the bus-escalation hole by construction rather than by topic-namespace convention; and
  `localStorage`/cross-tab values are treated as **untrusted input** with a recogniser
  (CON-5004/Threat C). Adds the FOUC pre-paint decision (ADR-5005) and bus bounds
  (NFR-5002).

</details>
