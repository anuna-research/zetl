---
id: SPEC-050
title: "Component Islands & Inter-Island Messaging"
status: draft
version: 0.1.0-strawman
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
   build time                         run time (browser)
  ┌───────────────────┐          ┌──────────── SPA shell (SPEC-028) ────────────┐
  │ Island emitter    │  <script>│  window.zetl = { store(topic), bus }          │
  │ <name>.js once    │─module──▶│   ├─ retained store: last-value + replay      │
  │ per type/page     │          │   └─ ephemeral bus: fire-and-forget           │
  │ (REQ-5001)        │          │  survives client-side navigation (REQ-5007)   │
  └─────────┬─────────┘          └───────┬───────────────────────┬───────────────┘
            │ hydrate [data-z]           │ publish(topic)         │ subscribe(topic)
            ▼                    ┌────────▼────────┐      ┌────────▼────────┐
  ┌───────────────────┐         │ island A        │      │ island B        │
  │ manifest topics   │         │ publishes=[…]   │      │ subscribes=[…]  │
  │ publishes/subscribes ──────▶│ (trusted theme  │      │ (trusted theme  │
  │ → wiring graph (REQ-5008)   │  code only)     │      │  code only)     │
  └───────────────────┘         └─────────────────┘      └─────────────────┘
   persisted topics → localStorage + cross-tab `storage` event (REQ-5006)
```

**Decisions** (deliberate before implementing):
[[SPEC-050-component-islands-and-messaging#ADR-5001]] shell bus, not a shared store module ·
[[SPEC-050-component-islands-and-messaging#ADR-5002]] replay-on-subscribe is the default primitive ·
[[SPEC-050-component-islands-and-messaging#ADR-5003]] islands are trusted theme code only — content-author islands forbidden in v1 ·
[[SPEC-050-component-islands-and-messaging#ADR-5005]] persisted topics carry a declared default + inline pre-paint set (FOUC).

**Load-bearing requirements:**
[[SPEC-050-component-islands-and-messaging#REQ-5001]] gated per-type island emission ·
[[SPEC-050-component-islands-and-messaging#REQ-5002]] progressive enhancement ·
[[SPEC-050-component-islands-and-messaging#REQ-5004]] shell bus (`store` + `bus`) ·
[[SPEC-050-component-islands-and-messaging#REQ-5005]] replay-on-subscribe ·
[[SPEC-050-component-islands-and-messaging#REQ-5008]] manifest topics + static wiring check ·
[[SPEC-050-component-islands-and-messaging#REQ-5010]] trusted-only island trust boundary ·
[[SPEC-050-component-islands-and-messaging#REQ-5012]] backward-compatible default.

**Open** (each blocks the Phase 2 gate — see
[[SPEC-050-component-islands-and-messaging#12. Open Questions]]):
Q1 persisted-default / FOUC mechanism · Q2 whether content-author islands ever become
sandboxed-allowed · Q3 typed vs opaque topic payloads · Q4 bus residence in the SPEC-028
shell · Q5 delivery/ordering guarantees (owner: spec author, to ground in Phase 1).

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
| Version      | 0.1.0-strawman                                                                          |
| Status       | Draft (strawman; NOT converged — pending Phase 1 + Phase 2 gates)                       |
| Author       | Agent (Claude Opus 4.8 [1M], [[PROTO-001\|USDD Agent Protocol]] v1.11.0)                |
| Date         | 2026-06-24                                                                              |
| Predecessor  | [[SPEC-048-components-and-static-overrides\|SPEC-048]] (islands/bus deferred here)      |
| Related      | [[SPEC-028]] SPA shell, [[SPEC-049]] content-author components, [[SPEC-002]] search     |
| Feature Gate | `component-islands` (island emission + hydration); `island-bus` (shell messaging)       |
| Review tier  | Tier 2 (trust boundary: client JS execution + `localStorage`/cross-tab as untrusted input) |

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

The second insight is a trust one: **a JS island is arbitrary code in the page's realm.**
String-namespacing topics cannot isolate a malicious island from a trusted topic like
`theme` — same realm, same `window.zetl`. So islands MUST be a *trusted-theme-author*
surface only; content-author components ([[SPEC-049]]) MAY NOT ship or invoke islands in
v1, which closes the escalation by construction rather than by convention
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
5. **Trusted code only.** Islands are theme-author code; content-author islands are
   forbidden in v1 ([[SPEC-050-component-islands-and-messaging#REQ-5010]]).
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
wiring graph ([[SPEC-050-component-islands-and-messaging#REQ-5009]]); the trusted-only
island trust boundary ([[SPEC-050-component-islands-and-messaging#REQ-5010]]); topic
grammar ([[SPEC-050-component-islands-and-messaging#REQ-5011]]); backward-compatible
default ([[SPEC-050-component-islands-and-messaging#REQ-5012]]).

**Out of scope:** content-author islands (forbidden in v1 — possible sandboxed successor,
`[Blocked: Q2]`); a reactive/VDOM framework; server-pushed islands or websockets (the bus
is client-local); typed/validated topic payload schemas beyond opaque JSON-serialisable
values (`[Blocked: Q3]`); cross-document (cross-origin) messaging.

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

### 3.5 HP5: A Content Page Cannot Reach the Bus
A [[SPEC-049]] content directive tries to invoke an island-bearing component. The build
refuses (`island-content-forbidden`) — content-author components cannot ship or invoke
islands in v1, so a markdown author cannot read, forge, or overwrite `theme`
([[SPEC-050-component-islands-and-messaging#REQ-5010]],
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
manifest stays an accurate contract. These declarations are advisory wiring metadata —
the runtime trust boundary is enforced by REQ-5010, not by this declaration. The manifest
keys `publishes`/`subscribes`, reserved-and-rejected in [[SPEC-048]] CON-4801, become
accepted under this spec's feature gate.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5008]], [[SPEC-050-component-islands-and-messaging#CON-5002]]; [[SPEC-050-component-islands-and-messaging#3.4 HP4]].

### REQ-5009: Island Wiring Graph (Audit)
Per build, the system SHALL emit an **island wiring graph**: for each island component,
its declared `publishes`/`subscribes` topics and the resolved publisher→subscriber edges,
plus any `island-topic-unpublished`/`island-topic-undeclared` findings, so runtime
coordination is auditable at build time. This extends [[SPEC-048]] OBS-4801.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5008]], [[SPEC-050-component-islands-and-messaging#OBS-5001]].

### REQ-5010: Trusted-Only Island Trust Boundary
Islands SHALL be a **trusted theme/component-author** surface only. A **content-author**
component ([[SPEC-049]]) SHALL NOT ship an island, and a content directive SHALL NOT be
able to invoke an island-bearing component; an attempt SHALL be a build error
(`island-content-forbidden`). Rationale: an island is arbitrary code in the page realm, so
topic-namespace "isolation" is not an enforcement boundary — only excluding untrusted
authors from the island surface actually prevents a markdown author from reading, forging,
or overwriting a trusted topic such as `theme`
([[SPEC-050-component-islands-and-messaging#Threat A]]). A sandboxed content-island model
is a possible successor (`[Blocked: Q2]`).

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5010]], [[SPEC-049]]; [[SPEC-050-component-islands-and-messaging#Threat A]]; [[SPEC-050-component-islands-and-messaging#3.5 HP5]].

### REQ-5011: Topic Grammar
A topic name SHALL match a declared grammar
([[SPEC-050-component-islands-and-messaging#CON-5001]]): a lowercase, colon-namespaced
identifier (e.g. `theme`, `search:open`). A reserved namespace prefix SHALL be set aside
for any future content-island topic set so trusted topics are syntactically partitioned
from a possible sandboxed-content successor (`[Blocked: Q2]`). A malformed topic at a
declaration or call site SHALL fail closed (`island-topic-malformed`).

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5011]], [[SPEC-050-component-islands-and-messaging#CON-5001]].

### REQ-5012: Backward-Compatible Default
WHEN a vault uses no island-bearing component, the build output SHALL be byte-identical to
a [[SPEC-048]]-only build: no `window.zetl`, no bus runtime, no island `<script>`, no
pre-paint script. All SPEC-050 behaviour SHALL be reachable only by a component shipping
`<name>.js` and being used on a page.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5012]]; [[SPEC-050-component-islands-and-messaging#3.1 HP1]].

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

### ADR-5003: Islands Are Trusted Theme Code Only; Content Islands Forbidden in v1
Only theme/component authors may ship islands; content-author components ([[SPEC-049]])
may neither ship nor invoke an island (REQ-5010). (+) Closes the bus-escalation threat
**by construction** — there is no untrusted code on `window.zetl`, so a markdown author
cannot forge `theme`; topic-namespacing becomes a wiring convenience, not a (false)
security boundary. (−) Content authors cannot add interactivity in v1. Rejected for v1:
"namespace-isolated content topics" — string namespaces do not isolate same-realm JS, so
the [[SPEC-048]] v0.1.1 claim was unsound; a real sandbox (iframe/Worker) is a heavier
successor (`[Blocked: Q2]`). This is the central security decision of the spec.

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

---

## 7. Contracts (LangSec)

> Every contract accepts author- or storage-supplied input and declares a grammar; full
> recognition precedes any action ([[PROTO-001]] §LangSec).

### CON-5001: Topic Name Grammar
**Interface:** a topic name at a manifest declaration or a runtime `store`/`bus` call.
**Grammar:**
```
topic     = [ ns ":" ] ident { ":" ident } ;
ns        = ident ;                       (* reserved prefixes partition trust domains *)
ident     = lower { lower | digit | "-" } ;
lower     = "a".."z" ; digit = "0".."9" ;
```
**Pre-conditions:** ≤ 128 bytes; matches the grammar; the reserved content-island
namespace prefix is not used by a trusted (theme-author) declaration.
**Post-conditions:** a validated topic key. **Error model:** out-of-grammar →
`island-topic-malformed` (error) at build for declarations, dropped with a console
diagnostic at runtime for call sites.
**Implements:** [[SPEC-050-component-islands-and-messaging#REQ-5011]].
**Verified by:** [[SPEC-050-component-islands-and-messaging#TEST-5011]].

### CON-5002: Island Manifest Fields (`publishes` / `subscribes` / `persisted`)
**Interface:** the island-related keys added to the [[SPEC-048]] CON-4801 manifest.
**Grammar (TOML, strict):**
```
publishes  = "publishes"  "=" "[" { topic } "]" ;
subscribes = "subscribes" "=" "[" { topic } "]" ;
island     = "[island]" , { persisted-decl } ;
persisted-decl = topic "=" "{" "persisted" "=" bool [ "," "default" "=" literal ] "}" ;
```
**Pre-conditions:** every topic matches CON-5001; a `persisted = true` topic declares a
`default`; the component ships `<name>.js` (declaring topics without an island is
`island-topic-undeclared`, warning).
**Post-conditions:** typed island metadata feeding wiring verification (REQ-5008) and the
audit graph (REQ-5009). **Error model:** malformed topic → `island-topic-malformed`
(error); persisted-without-default → `island-persisted-no-default` (error).
**Implements:** [[SPEC-050-component-islands-and-messaging#REQ-5006]], [[SPEC-050-component-islands-and-messaging#REQ-5008]].
**Verified by:** [[SPEC-050-component-islands-and-messaging#TEST-5006]], [[SPEC-050-component-islands-and-messaging#TEST-5008]].

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
**Pre-conditions:** `topic` matches CON-5001; `value`/`detail` are structured-clone-safe
(opaque JSON-serialisable in v1, `[Blocked: Q3]`); bounds within NFR-5002.
**Post-conditions:** retained-store invariants (REQ-5005); bus is non-retaining; both
survive navigation (REQ-5007). **Error model:** malformed topic or bound breach → dropped
with a console diagnostic, never a throw that breaks unrelated islands.
**Implements:** [[SPEC-050-component-islands-and-messaging#REQ-5004]], [[SPEC-050-component-islands-and-messaging#REQ-5005]], [[SPEC-050-component-islands-and-messaging#REQ-5007]].
**Verified by:** [[SPEC-050-component-islands-and-messaging#TEST-5004]], [[SPEC-050-component-islands-and-messaging#TEST-5005]].

### CON-5004: Persisted-Topic Storage Encoding
**Interface:** the `localStorage` key/value for a persisted topic and the `storage`-event
read path. **This is an untrusted-input boundary** — another tab, an extension, or a prior
version may have written the value.
**Grammar:** key = `zetl:topic:<topic>`; value = a JSON document conforming to the topic's
declared shape (v1: opaque JSON; a recogniser rejects non-conforming or oversized values).
**Pre-conditions:** the stored value parses as JSON, is ≤ the per-value cap (NFR-5002), and
conforms to the topic shape.
**Post-conditions:** a recognised value applied to the store; on any failure the declared
default is applied instead and the bad entry is overwritten. **Error model:** parse/shape/
size failure → discard + default (never apply raw — [[SPEC-050-component-islands-and-messaging#Threat C]]).
**Implements:** [[SPEC-050-component-islands-and-messaging#REQ-5006]].
**Verified by:** [[SPEC-050-component-islands-and-messaging#TEST-5006]].

---

## 8. Threat Model

Trust boundary: **theme/component authors are trusted** (they ship JS); **content authors
are excluded from the island surface** (REQ-5010); **`localStorage`/cross-tab values are
untrusted input** crossing into the runtime.

### Threat A: Island-Bus Escalation via a Content Author
A markdown author tries to ship or invoke an island to read/forge/overwrite a trusted
topic (`theme`) on the shared `window.zetl`. **Mitigation:** content-author components may
not ship or invoke islands (`island-content-forbidden`, REQ-5010) — there is no untrusted
code on the bus, closing the escalation by construction. String-namespacing is explicitly
**not** relied on as the boundary (ADR-5003).

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
A hostile `<name>.js`. **Mitigation:** islands are trusted theme-author code (same posture
as [[SPEC-048]] ADR-4801 for CSS); content authors are excluded (REQ-5010). No additional
sandbox is claimed for trusted islands.

### Threat F: Flash of Wrong Theme (FOUC) as a UX Defect
The persisted theme applies only after async hydration, flashing the wrong theme.
**Mitigation:** declared default + inline render-blocking pre-paint set (REQ-5006,
ADR-5005). Listed as a threat because a first-paint regression is a real, observable defect
the design must prevent.

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

### TEST-5010: Content-Author Island Forbidden
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5010]], [[SPEC-050-component-islands-and-messaging#Threat A]]. Positive: a theme-author island works. Negative-input: a [[SPEC-049]] content directive invoking an island-bearing component → `island-content-forbidden`. Negative-output: no content-authored code is ever emitted onto `window.zetl` (fuzz the directive surface).

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
Emit counts of `island-topic-malformed`, `island-topic-unpublished`,
`island-topic-undeclared`, `island-persisted-no-default`, and `island-content-forbidden`,
so fail-closed events are auditable.
**Trace:** [[SPEC-050-component-islands-and-messaging#REQ-5008]], [[SPEC-050-component-islands-and-messaging#REQ-5010]].

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

Net new surface is confined to (a) the ≤ 4 KiB shell bus runtime (`store`/`bus`) and
(b) the inline persisted-topic pre-paint script. Everything else composes [[SPEC-048]] and
[[SPEC-028]] primitives.

---

## 12. Open Questions

- **Q1 — Persisted-default / FOUC mechanism.** Exact shape of the inline pre-paint script
  and how the declared default is threaded into it (`[Blocked: Q1]`,
  [[SPEC-050-component-islands-and-messaging#ADR-5005]]). Phase 1: how do operators expect
  theme persistence to behave on first paint and cross-tab?
- **Q2 — Sandboxed content-author islands.** Should a later spec allow content-author
  interactivity via a real sandbox (iframe/Worker) with a bridged, capability-scoped bus?
  v1 forbids content islands entirely (`[Blocked: Q2]`,
  [[SPEC-050-component-islands-and-messaging#ADR-5003]]).
- **Q3 — Typed topic payloads.** Should topics declare a value schema (validated at the
  bus boundary) or stay opaque JSON in v1? Affects CON-5003/CON-5004 recognisers.
- **Q4 — Bus residence.** Does the bus live inside the existing SPEC-028 shell module or a
  new sibling shell module loaded alongside it? Determines load order vs the pre-paint
  script.
- **Q5 — Delivery/ordering guarantees.** Beyond per-subscriber subscription-order, are any
  cross-topic ordering or synchronous-vs-microtask delivery guarantees required? Pin in
  IMPL-050.

---

## 13. Convergence Status

**NOT converged.** A first strawman extracted from the deferred [[SPEC-048]] island/bus
material, with no adversarial pass. Before the Phase 2 gate this spec requires, at minimum:
(1) a fresh-context adversarial review ([[PROTO-001]] Principle 12) — expected to press the
trust boundary (REQ-5010/Threat A), the untrusted-storage recogniser (CON-5004/Threat C),
and the FOUC pre-paint exception (ADR-5005); (2) Phase 1 operator/theme-author profiles to
ground every `[Provisional]` value and close Q1–Q5; (3) a feasibility spike for the ≤ 4 KiB
replay-store bus against the [[SPEC-028]] shell; (4) IMPL-050 to pin the grammars, the
numeric bounds, and the pre-paint script. It depends on [[SPEC-048]] (the component +
`data-z` substrate) and [[SPEC-049]] (the content-author trust line REQ-5010 enforces) and
gates independently of both.

---

## Changelog

<details>
<summary>Revision history — 0.1.0</summary>

- **0.1.0** (2026-06-24) — *initial strawman.* Extracted and reframed from the [[SPEC-048]]
  v0.1.1 island/bus clauses (REQ-4810/4816/4817, ADR-4808) that the SPEC-048 v0.2.0
  tightening deferred. Key reframe vs that draft: islands are a **trusted-author-only**
  surface and content-author islands are **forbidden in v1** (ADR-5003/REQ-5010), closing
  the bus-escalation hole by construction rather than by topic-namespace convention; and
  `localStorage`/cross-tab values are treated as **untrusted input** with a recogniser
  (CON-5004/Threat C). Adds the FOUC pre-paint decision (ADR-5005) and bus bounds
  (NFR-5002).

</details>
