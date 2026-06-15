---
id: SPEC-046
title: "OKF Interchange — Open Knowledge Format conformance, export, and import"
version: 0.1.2-strawman
status: draft
date: 2026-06-15
audience: agent, human
related:
  - SPEC-045  # Wikilink Predicate Language (typed edges flatten to OKF untyped links)
  - SPEC-044  # Concept Graph / SPO (the "markdown corpus → knowledge graph" sibling)
  - SPEC-001  # Link Graph CLI (the wikilink graph OKF links must bridge into)
  - SPEC-032  # Hook pipeline + zetl-ext AST (the parse boundary OKF grammars extend)
  - SPEC-026  # Vault scan exclusions (bundle ↔ vault boundary)
  - SPEC-017  # History backend (the jj snapshots that generate OKF log.md)
  - SPEC-002  # Full-text search (type becomes a search facet on import)
  - SPEC-041  # Pluggable collab auth (the identity layer a contribution-signing capability would build on)
source:
  - "Open Knowledge Format (OKF) v0.1 — GoogleCloudPlatform/knowledge-catalog: https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md"
revision_notes:
  - v0.1.0 (initial strawman, 2026-06-13): three interchange modes — conformance,
    export, import — over a shared LangSec recogniser; OKF framed as a lossy
    downcast of zetl; structural gap isolated (zetl links by page name, OKF by
    path).
  - v0.1.1 (adversarial-review revision, 2026-06-14): applied a three-lens review.
    Withdrew the v0.1.0 "OKF self-contradiction" misreading (Q1); `.md`-bearing
    links; reserved-deviation handling; x-zetl-edges trust gating; symlink/
    reserved-path threats; concrete input bounds.
  - v0.1.2 (second adversarial-review revision, 2026-06-15): applied a second
    fresh-context three-lens review whose severity *rose*, indicating non-
    convergence. KEY CHANGES — (S1a, fidelity regression) v0.1.1's "any malformed
    reserved file = ERROR" was STRICTER than OKF authorises: OKF §7 has exactly
    one MUST (ISO-8601 date headings) and §6 has none (the bold `**kind**:`
    prefix is "a convention, not a requirement"; `[Subdirectory](subdir/)`
    directory links and a `# …` log title heading are valid; "body uses one or
    more sections" is descriptive). Reserved-structure ERRORS are now limited to
    genuine OKF MUSTs; convention deviations are WARNINGS (REQ-4602, CON-4605).
    Directory-link and log-title-heading grammar added; the `.md` requirement is
    scoped to concept links, not directory entries; `okf_version` emission is
    reframed as a zetl policy over OKF's MAY. (S1b, x-zetl-edges) per direction,
    AUTHENTICITY and TRUST are separated: REQ-4617 is reframed to **inert
    preservation in v1** (x-zetl-edges is preserved as data but NEVER
    reconstructed into edges/facts; `--trust-edges` removed) because safe
    reconstruction depends on **verifiable contribution authenticity** — a
    *detached per-contribution signature that lives with the document* — which is
    a shared capability that does not yet exist (new Q7, candidate SPEC-047).
    This closes the forged-fact vector for v1 by construction (Threat E).
    SECURITY — REQ-4618 extended to reject hardlinks, FIFOs/devices/sockets, and
    TOCTOU (open-then-fstat, no lstat-then-open) and a symlinked bundle root;
    path/reserved checks now run on the fully percent-decoded, NFC, case-folded,
    lexically-normalised path, component-wise at any depth, with `~` dropped from
    the allowlist (Threats A/C/F/G; REQ-4603/4612/4619, CON-4602). UTF-8
    validation added (REQ-4601). NAMING/TRACEABILITY — `REQ-4607b`→`REQ-4622`,
    `TEST-4604b`→`TEST-4623`; TEST-4607 split (→ TEST-4622); NFR-4603 retraced to
    TEST-4621; "best-effort" removed from REQ-4605; `[Provisional]` removed from
    value slots; every TEST promoted to a `### TEST-####` heading so anchors
    resolve; YAML anchors/aliases forbidden outright (ADR-4607, NFR-4604);
    timestamp pinned to ISO-8601 *datetime*; unknown-version handling split
    minor vs major (REQ-4610). Added Q7; matrix reconciled.
---

# SPEC-046: OKF Interchange

> **Strawman notice.** Drafted from the [[Open Knowledge Format]] v0.1
> specification (the [[knowledge-catalog OKF SPEC]]) and revised twice against
> fresh-context adversarial review. It has NOT had Phase 1 surveys, synthetic-user
> runs, or stakeholder validation. Per [[PROTO-001]] Constitutional Principle 11
> ([[Anti-Slop Bias]]), treat every clause as carrying hidden debt. **The second
> review's severity rose rather than fell — this spec has not converged**;
> sections tagged **`[Blocked: Qn]`** depend on an open question
> ([[SPEC-046-okf-interchange#13. Open Questions Surfaced by This Strawman]]) and
> MUST NOT pass the Phase 2 gate until it closes ([[PROTO-001]] §Error Response).
> `[Provisional]` marks a value still to be grounded in Phase 1.
>
> **External-standard pin.** OKF is pinned at **v0.1**. Where OKF uses MAY/SHOULD/
> "convention", this spec does NOT promote those to MUST: `zetl okf check`
> conformance errors are restricted to genuine OKF MUSTs so that "conformant"
> means "an OKF consumer accepts it" ([[SPEC-046-okf-interchange#REQ-4601]],
> [[SPEC-046-okf-interchange#REQ-4602]]).

## Information Table

| Field        | Value                                                                            |
| ------------ | -------------------------------------------------------------------------------- |
| Document ID  | [[SPEC-046-okf-interchange\|SPEC-046]]                                            |
| Title        | OKF Interchange — Open Knowledge Format conformance, export, and import           |
| Version      | 0.1.2-strawman                                                                    |
| Status       | Draft (strawman; NOT converged — pending Phase 1 + Phase 2 gates)                 |
| Author       | Agent (Claude Opus 4.8, [[PROTO-001\|USDD Agent Protocol]] v1.8.0)                |
| Date         | 2026-06-15                                                                        |
| Audience     | Agent, Human                                                                      |
| Trace        | [[PROTO-001]] §Phase 1, §Phase 2, §LangSec, §AI Trust Boundaries                  |
| Source       | [[knowledge-catalog OKF SPEC]] (OKF v0.1, GoogleCloudPlatform/knowledge-catalog)  |
| Sibling      | [[SPEC-045-wikilink-predicate-language]] (typed edges), [[SPEC-044-concept-graph-spo-emergent-vocabulary]] (SPO) |
| Related      | [[SPEC-001]] Link Graph, [[SPEC-032]] AST/Hooks, [[SPEC-026]] Scan, [[SPEC-017]] History, [[SPEC-002]] Search, [[SPEC-041-pluggable-collab-auth]] Auth |
| Feature Gate | `okf` (export/import/check); export of `log.md` additionally needs `history`      |
| Review tier  | Tier 2 (a trust boundary: parsing externally-authored bundles + path links)       |

---

## 1. Overview

### 1.1 Problem

[[Open Knowledge Format]] (OKF) v0.1 is an external interchange standard: a
directory of Markdown files with YAML frontmatter, no central schema registry,
no required tooling. A directory is a **[[Knowledge Bundle]]**; each `.md` file
is a **[[Concept (OKF)|Concept]]**; a concept's **[[Concept ID]]** is its file
path within the bundle minus the `.md` extension. The only hard requirement is a
non-empty `type` field per concept. OKF is the wire format an OKF-aware catalogue
(e.g. Google Cloud's) ingests and emits.

zetl is a **superset** of OKF on nearly every axis — free-form frontmatter
([[SPEC-032]] `Frontmatter = serde_json::Map`), a [[Link Graph]] ([[SPEC-001]]),
and typed edges ([[SPEC-045-wikilink-predicate-language]]) that are strictly
*more* expressive than OKF's untyped links. But it is not interoperable, because
of concrete gaps:

1. **The link model differs.** OKF links are standard Markdown whose targets are
   **paths to the `.md` file** — bundle-absolute when leading `/` (OKF's
   *recommended*, move-stable form), otherwise relative — e.g.
   `[customers table](/tables/customers.md)`, `[neighboring concept](./other.md)`.
   OKF's `index.md` may also link **directories** as `[Subdirectory](subdir/)`.
   The [[Concept ID]] (`tables/customers`) is the `.md`-stripped *identifier*,
   not the link target. zetl's graph is built **only from `[[wikilinks]]`
   resolved by page name** ([[SPEC-001]]); a dropped-in OKF bundle renders but
   yields zero backlinks. This is the one *structural* incompatibility.
2. **`type` is required by OKF, unenforced by zetl.**
3. **[[Concept ID]] ≠ slug.** OKF Concept ID is the file path verbatim,
   case-preserved; zetl's [[slug]] lowercases and hyphenates — mismatching on
   case and Unicode normalisation ([[SPEC-046-okf-interchange#Threat Model C]]).
4. **Reserved files have prescribed structures** (OKF §6/§7), but those
   structures are mostly SHOULD/convention: §7's *only* MUST is ISO-8601 date
   headings, and the *root* `index.md` MAY carry a frontmatter block solely for
   `okf_version` (OKF §11's carve-out to §6's "Index files contain no
   frontmatter"). A conformance checker must therefore be no stricter than OKF.

The problem this spec solves: make zetl a first-class OKF **producer**,
**consumer**, and **validator** — without contaminating zetl's native model, and
without trusting an externally-authored bundle.

### 1.2 Relationship to the Typed-Edge Specs

OKF, [[SPEC-044-concept-graph-spo-emergent-vocabulary]], and
[[SPEC-045-wikilink-predicate-language]] sit on an expressiveness ladder: OKF
types *nodes* (via `type`) and leaves edges untyped; SPEC-045 types *edges*;
SPEC-044 adds vocabulary + ratification. The relationship is **downcast**: OKF
is the lowest-common-denominator interchange dialect. Exporting *loses* edge
types (they flatten — [[SPEC-046-okf-interchange#REQ-4606]]); importing *gains*
only untyped links + node `type`. This spec owns the **boundary** and does NOT
alter zetl's native graph, AST, or reasoner ([[SPEC-046-okf-interchange#ADR-4601]]).

The predicate that flattening loses can be carried through OKF as opaque
frontmatter (`x-zetl-edges`) for a zetl→OKF→zetl trip — but because a third
party can edit that frontmatter, **turning it back into live edges/facts is a
contribution-authenticity problem, not a fidelity convenience** (§1.4 principle
5). In v1 it is preserved as inert data only
([[SPEC-046-okf-interchange#REQ-4617]]); live reconstruction is deferred to a
future signing capability ([[SPEC-046-okf-interchange#Q7]]).

### 1.3 Core Insight

OKF is a strict projection of what zetl already holds; the genuinely new
capabilities are (a) **path-link resolution** (`[text](/a/b.md)` → edge to
Concept ID `a/b`) and (b) **trust-boundary hardening** (a foreign bundle is
attacker-controlled — §11). zetl reuses its AST link extraction ([[SPEC-032]])
and phantom-node / dead-link machinery ([[SPEC-001]], `src/graph.rs`); the OKF
link recogniser is a second link source feeding the same graph, scoped to OKF
mode ([[SPEC-046-okf-interchange#ADR-4603]]). One graph, two link dialects, one
hostile-input boundary.

### 1.4 Design Principles

1. **OKF is a codec, not a model.** No OKF concept leaks into the native graph,
   AST, or reasoner ([[SPEC-046-okf-interchange#ADR-4601]]).
2. **Downcast is lossy and the loss is *named*** — never silent
   ([[SPEC-046-okf-interchange#REQ-4606]], [[SPEC-046-okf-interchange#Threat Model D]]).
3. **Recognise before acting, conservatively ([[LangSec]]).** Frontmatter and
   links have declared grammars with concrete bounds
   ([[SPEC-046-okf-interchange#CON-4601]], [[SPEC-046-okf-interchange#CON-4602]],
   [[SPEC-046-okf-interchange#NFR-4604]]); recognition completes before any
   semantic action; a path escape, symlink, irregular file, over-budget input,
   or out-of-grammar structure is a **parse failure**, rejected not normalised.
4. **No stricter than OKF on conformance; tolerant only of OKF-mandated *inert*
   incompleteness.** `zetl okf check` errors are limited to OKF MUSTs
   ([[SPEC-046-okf-interchange#REQ-4601]], [[SPEC-046-okf-interchange#REQ-4602]]);
   SHOULD/convention deviations are warnings. Unknown frontmatter keys are
   preserved **inert**, never acted on. [[Postel's Law]] is rejected for
   structure ([[SPEC-046-okf-interchange#ADR-4607]]); the only tolerance is
   OKF-mandated preservation of inert unknowns.
5. **A foreign bundle is untrusted, and authenticity ≠ trust.** Two distinct
   concerns: *authenticity* (did a contribution truly come from its claimed
   author, untampered?) and *trust* (given a verified author, do we act on it?).
   v1 can verify neither for foreign contributions, so it acts on none of them:
   no foreign `x-zetl-edges` becomes an edge or fact
   ([[SPEC-046-okf-interchange#REQ-4617]]). The proper long-term primitive is a
   **detached per-contribution signature that lives with the document**, making
   provenance a verifiable property of the *contribution* (the axis
   [[SPEC-045-wikilink-predicate-language]] lacks); trust then layers cleanly on
   top ([[SPEC-046-okf-interchange#Q7]]).
6. **Identity is preserved across the boundary, OS-independently** — Concept ID
   verbatim (case-preserved, NFC), distinct from the slug
   ([[SPEC-046-okf-interchange#REQ-4603]], [[SPEC-046-okf-interchange#NFR-4602]]).
7. **No behaviour without invocation** ([[SPEC-046-okf-interchange#REQ-4616]];
   persistence of OKF link semantics is `[Blocked: Q2]`).
8. **One recogniser per OKF language** — frontmatter, links, and reserved-file
   structures each have a single recogniser shared by check, export, and import
   ([[PROTO-001]] §LangSec).

### 1.5 Scope

**In scope:** a conformance checker no stricter than OKF
([[SPEC-046-okf-interchange#REQ-4601]], [[SPEC-046-okf-interchange#REQ-4602]],
[[SPEC-046-okf-interchange#REQ-4621]]); OS-independent [[Concept ID]] derivation
([[SPEC-046-okf-interchange#REQ-4603]]); OKF export
([[SPEC-046-okf-interchange#REQ-4604]]–[[SPEC-046-okf-interchange#REQ-4610]],
[[SPEC-046-okf-interchange#REQ-4622]]); OKF import
([[SPEC-046-okf-interchange#REQ-4611]]–[[SPEC-046-okf-interchange#REQ-4617]],
[[SPEC-046-okf-interchange#REQ-4620]]); trust-boundary hardening
([[SPEC-046-okf-interchange#NFR-4604]],
[[SPEC-046-okf-interchange#REQ-4618]], [[SPEC-046-okf-interchange#REQ-4619]]);
consumer robustness ([[SPEC-046-okf-interchange#REQ-4615]]); backward-compatible
default ([[SPEC-046-okf-interchange#REQ-4616]]).

**Out of scope:** a new internal node-type system; **live reconstruction of
typed edges from foreign `x-zetl-edges`** (deferred to the
[[SPEC-046-okf-interchange#Q7]] signing capability); a contribution-signing
capability itself (its own spec — candidate SPEC-047); bidirectional live sync;
archive (zip/tar) ingestion (directory only; if added, extraction MUST validate
zip-slip and reject archive symlinks/hardlinks/device entries —
[[SPEC-046-okf-interchange#Threat Model F]] forward-guard); OKF versions other
than v0.1; inferring edge semantics from prose.

---

## 2. User Profiles

> **`[Provisional — refined by Phase 1 synthetic-user runs.]`** Defaults
> justified against these profiles ([[SPEC-046-okf-interchange#REQ-4607]] etc.)
> remain provisional until a real `users/catalogue-publisher/happy-paths.md`
> exists ([[PROTO-001]] Principle 13).

### 2.1 The Catalogue Publisher
Maintains a zetl vault as source of truth; needs `zetl export --format okf` to
emit a bundle that passes the catalogue's ingest on the first try, and to know
explicitly what the downcast lost.

### 2.2 The Catalogue Consumer
Receives an OKF bundle from elsewhere; wants to read, link, search, and reason
over it in zetl. Depends (knowingly or not) on zetl not executing or
exfiltrating anything a malicious bundle plants.

### 2.3 The Conformance Auditor
Wants a machine-checkable verdict that matches, byte-for-byte, what a downstream
OKF consumer would accept — neither stricter nor looser.

### 2.4 The Traversing Agent
An LLM agent ingesting a bundle; reads `index.md`, follows path links, weighs
concepts by `type`. Must not be steered to a homoglyph impostor concept
([[SPEC-046-okf-interchange#Threat Model C]]).

---

## 3. Happy Paths

> **`[Provisional — refined by Phase 1.]`**

### 3.1 HP1: Default — No OKF, Nothing Changes
**Pre:** never runs `zetl okf …`, never imports. **Post:** identical link graph,
backlinks, search, web output; no `type` requirement; Markdown links non-graph
as today ([[SPEC-046-okf-interchange#TEST-4616]]).

### 3.2 HP2: Audit a Directory for OKF Conformance
`zetl okf check ./catalog/` reports per-concept `okf-missing-type` /
`okf-frontmatter-parse` / `okf-encoding` (errors), and for reserved files
ISO-date / no-frontmatter violations as errors but convention deviations
(missing bold-kind, ungrouped sections) as warnings. Exit non-zero on any error
so CI gates on it; the verdict matches what an OKF consumer rejects, no stricter
([[SPEC-046-okf-interchange#REQ-4601]], [[SPEC-046-okf-interchange#REQ-4602]]).

### 3.3 HP3: Export a Vault to an OKF Bundle
`zetl export --format okf -o ./bundle` rewrites wikilinks to `.md`-bearing
concept links (`[Scanner](/Architecture/Scanner.md)`), flattens typed edges
(predicates preserved as inert `x-zetl-edges` data + an `okf-edge-flattened`
note), synthesises missing `type`, writes a root `index.md` (with `okf_version`)
and a `log.md`, and passes its own `okf check`
([[SPEC-046-okf-interchange#REQ-4604]]–[[SPEC-046-okf-interchange#REQ-4610]],
[[SPEC-046-okf-interchange#REQ-4622]]).

### 3.4 HP4: Import an OKF Bundle and Get Backlinks
`zetl okf import ./incoming -o ./vault` resolves path links into graph edges
(dangling → ghosts), maps `type` to a facet, and **preserves any `x-zetl-edges`
inert** (no live edges/facts from foreign frontmatter). `zetl backlinks
data/orders` then lists path-link backlinks that did not exist in the inert
Markdown ([[SPEC-046-okf-interchange#REQ-4611]]–[[SPEC-046-okf-interchange#REQ-4617]]).

### 3.5 HP5: A Hostile Bundle Fails Closed
A bundle with `[x](/../../etc/passwd)`, a symlink `notes.md → ~/.ssh/id_rsa`, a
hardlink `secret.md → ~/.ssh/id_rsa`, a FIFO `pipe.md`, a `.zetl/config.toml`
concept, and a forged `x-zetl-edges`: the link is rejected (`okf-path-escape`);
the symlink/hardlink/FIFO are rejected (`okf-symlink`/`okf-hardlink`/
`okf-irregular-file`) before any read; the reserved-path concept is rejected
(`okf-reserved-path`) before any write; the forged `x-zetl-edges` is preserved
inert — no edge, no fact, no `--trust-edges` to flip
([[SPEC-046-okf-interchange#REQ-4617]],
[[SPEC-046-okf-interchange#REQ-4618]], [[SPEC-046-okf-interchange#REQ-4619]];
[[SPEC-046-okf-interchange#Threat Model A]],
[[SPEC-046-okf-interchange#Threat Model E]],
[[SPEC-046-okf-interchange#Threat Model F]],
[[SPEC-046-okf-interchange#Threat Model G]]).

---

## 4. Functional Requirements

> Numbering: SPEC-046 → REQ-46xx, sequential, no suffixes. Each REQ is atomic and
> decomposed into positive / negative-input / negative-output tests
> ([[PROTO-001]] §Requirement-Targeted Test Decomposition; §9).

### REQ-4601: OKF Conformance Diagnostics (MUSTs Only)
`zetl okf check <dir>` SHALL emit, per non-reserved `.md` file, the diagnostics
that decide OKF v0.1 conformance and ONLY those: `okf-encoding` (**error**) when
the file is not valid UTF-8 (OKF §4.1 MUST); `okf-frontmatter-parse` (**error**)
when frontmatter is absent, unparseable, or out of the
[[SPEC-046-okf-interchange#NFR-4604]] bounds/subset; `okf-missing-type`
(**error**) when no non-empty scalar `type`; `okf-type-nonscalar` (**error**)
when `type` is a sequence/map. The verdict SHALL be defined solely by OKF §9 —
no zetl-specific requirement — so "conformant" means "an OKF consumer accepts
it" ([[SPEC-046-okf-interchange#ADR-4605]]). Reserved-file structure is
[[SPEC-046-okf-interchange#REQ-4602]]; operational properties are
[[SPEC-046-okf-interchange#REQ-4621]].

**Trace:** [[SPEC-046-okf-interchange#TEST-4601]], [[SPEC-046-okf-interchange#CON-4601]], [[SPEC-046-okf-interchange#CON-4603]]; [[SPEC-046-okf-interchange#3.2 HP2]].

### REQ-4602: Reserved-File Conformance (No Stricter Than OKF)
The check SHALL validate `index.md`/`log.md` against OKF §6/§7
([[SPEC-046-okf-interchange#CON-4605]]), distinguishing OKF **MUSTs** (→
`okf-reserved-structure` **error**, non-conformant) from SHOULD/convention
deviations (→ `okf-reserved-convention` **warning**, still conformant):

* **MUST (error):** a `log.md` date heading not in ISO-8601 `YYYY-MM-DD` form
  (OKF §7's sole MUST); any frontmatter in a non-root `index.md`; any frontmatter
  in `log.md`; any root-`index.md` frontmatter key other than `okf_version`.
* **Convention (warning):** missing entry descriptions; non-newest-first log
  ordering; a missing or non-bold change "kind" prefix (OKF §7: "a convention,
  not a requirement"); section-grouping style; concept vs directory link form.

A reserved file's *absence* SHALL NOT be a diagnostic (both optional). Recognition
uses [[SPEC-046-okf-interchange#CON-4605]], not ad-hoc matching.

**Trace:** [[SPEC-046-okf-interchange#TEST-4602]], [[SPEC-046-okf-interchange#CON-4605]]; [[SPEC-046-okf-interchange#3.2 HP2]].

### REQ-4603: OS-Independent Concept ID Derivation
The system SHALL derive a **[[Concept ID]]** as the bundle-relative file path
minus `.md`, **verbatim, case-preserved, NFC-normalised**. It is distinct from
the internal [[slug]]. Identity comparison (collision, link resolution) SHALL use
a declared OS-independent model: NFC byte sequences, case-folded with the
locale-independent Unicode default fold *for collision detection only* (identity
stays case-preserved). The system SHALL maintain a Concept-ID ↔ page bijection
across a round-trip ([[SPEC-046-okf-interchange#NFR-4602]]).

The system SHALL reject (`okf-invalid-concept-id`, **error**) a Concept ID that
is empty or contains a `.` or `..` segment. WHEN two concepts collide under the
model — case-only, NFC-vs-NFD, or mixed-script confusable (UTS #39 Mixed-Script
detection via the confusable-skeleton algorithm, Unicode data version pinned in
IMPL-046) — the importer SHALL emit `okf-id-collision` (**error**) for case/
normalisation collisions and `okf-id-confusable` (**warning, surfaced on
agent-facing output** — `zetl edges --json` flags the node, not just a build
log) for confusables, and SHALL NOT silently merge concepts
([[SPEC-046-okf-interchange#Threat Model C]]).

**Trace:** [[SPEC-046-okf-interchange#TEST-4603]], [[SPEC-046-okf-interchange#ADR-4602]]; [[SPEC-046-okf-interchange#3.4 HP4]].

### REQ-4604: OKF Export Produces a Conformant Bundle
Under the `okf` feature, `zetl export --format okf -o <dir>` SHALL write a
directory that passes `zetl okf check` on the same build
([[SPEC-046-okf-interchange#REQ-4601]]). Its single obligation is
**conformance-by-construction**; the constituent transforms are owned by
[[SPEC-046-okf-interchange#REQ-4605]]–[[SPEC-046-okf-interchange#REQ-4610]] and
[[SPEC-046-okf-interchange#REQ-4622]], and determinism by
[[SPEC-046-okf-interchange#NFR-4601]].

**Trace:** [[SPEC-046-okf-interchange#TEST-4604]], [[SPEC-046-okf-interchange#CON-4604]]; [[SPEC-046-okf-interchange#3.3 HP3]].

### REQ-4605: Wikilink → OKF Path-Link Rewriting
On export, each resolved `[[wikilink]]`/`[[Target|text]]` in a body SHALL become
`[text](/concept-id.md)` — the bundle-absolute path to the target's `.md` file
(OKF's recommended form), `.md` included, Concept ID case-preserved
([[SPEC-046-okf-interchange#REQ-4603]]). Display text is the alias, else the
target title (frontmatter `title`, else page name; empty → Concept ID).

An **unresolved** wikilink SHALL become `[text](/<id>.md)` where `<id>` is the
target text run through zetl's phantom-node slugification (`src/graph.rs`) — a
deterministic, specified transform — and SHALL report `okf-export-dead-link`
(info). A heading anchor (`[[Target#Section]]`) SHALL become
`[text](/concept-id.md#<fragment>)` with `<fragment>` the heading text emitted
**verbatim, percent-encoded where Markdown requires, without lowercasing**; OKF
defines no anchor semantics, so the export SHALL emit `okf-anchor-nonstandard`
(info) the first time and the consumer's renderer governs resolution. Embeds
(`![[…]]`) are `[Blocked: Q3]`.

**Trace:** [[SPEC-046-okf-interchange#TEST-4605]], [[SPEC-046-okf-interchange#CON-4604]]; [[SPEC-046-okf-interchange#3.3 HP3]].

### REQ-4606: Typed-Edge Flattening With Named Loss
On export, a typed edge `predicate::[[Target]]`
([[SPEC-045-wikilink-predicate-language]]) SHALL flatten to an untyped OKF path
link ([[SPEC-046-okf-interchange#REQ-4605]]), with the predicate preserved as
**inert data** in an `x-zetl-edges` frontmatter array
(`{predicate, target, annotation?}`, conforming to
[[SPEC-046-okf-interchange#CON-4606]]) and the flattened count reported
(`okf-edge-flattened`, info). The flattening SHALL be **named, never silent**
([[SPEC-046-okf-interchange#Threat Model D]], [[SPEC-046-okf-interchange#ADR-4604]]).
A foreign consumer sees only the untyped links and an unknown key it preserves;
zetl re-import keeps `x-zetl-edges` **inert** in v1
([[SPEC-046-okf-interchange#REQ-4617]]).

**Trace:** [[SPEC-046-okf-interchange#TEST-4606]], [[SPEC-046-okf-interchange#ADR-4604]]; [[SPEC-045-wikilink-predicate-language]].

### REQ-4607: `type` Synthesis
On export, each page SHALL have a non-empty scalar `type`: an existing one passes
through verbatim; otherwise the system SHALL set `type` to `--default-type`
(value: `"concept"`) and emit `okf-type-synthesised` (note). The default is
permitted because OKF §9 requires only a *non-empty* type (descriptive values
are SHOULD); the **diagnostic**, not any fidelity adjective, is the testable
artifact. *(Default value `"concept"` is concrete; its justification against the
dominant publisher profile is `[Provisional]` pending Phase 1.)*

**Trace:** [[SPEC-046-okf-interchange#TEST-4607]], [[SPEC-046-okf-interchange#CON-4601]]; [[SPEC-046-okf-interchange#3.3 HP3]].

### REQ-4608: Generated OKF `index.md`
On export the system SHALL write a root `index.md` per
[[SPEC-046-okf-interchange#CON-4605]]: `# Section` headings grouping
`* [Title](link) - description` entries, where a concept entry's link is
`/concept-id.md` and a directory entry's link is `subdir/` (OKF §6 form);
description from the concept's frontmatter `description` (a body-paragraph
fallback is a zetl-local heuristic, not an OKF requirement; OKF §6 sources
description from frontmatter). Grouping is **by top-level directory** (testable
default). The root `index.md` SHALL carry `okf_version`
([[SPEC-046-okf-interchange#REQ-4610]]). An **empty vault** SHALL produce a
valid `index.md` of frontmatter + empty body (CON-4605 grammar permits zero
sections). Per-directory `index.md` generation is `[Blocked: Q5]`.

**Trace:** [[SPEC-046-okf-interchange#TEST-4608]], [[SPEC-046-okf-interchange#CON-4605]]; [[SPEC-046-okf-interchange#3.3 HP3]].

### REQ-4609: Generated OKF `log.md` From History
WHEN the `history` feature is compiled in AND a jj workspace is present, on
export the system SHALL write `log.md` per
[[SPEC-046-okf-interchange#CON-4605]]: an optional leading `# <title>` heading
(OKF §7), then `## YYYY-MM-DD` headings newest-first, each with
`* [**<kind>**: ] <description>` items (the bold kind is optional — OKF §7
convention), no frontmatter. WHEN either precondition is unmet, `log.md` SHALL be
omitted and `okf-log-skipped` (note) emitted — never fabricated. Presence is a
deterministic function of the explicit (feature, workspace) inputs, folded into
[[SPEC-046-okf-interchange#NFR-4601]]'s tuple.

**Trace:** [[SPEC-046-okf-interchange#TEST-4609]], [[SPEC-046-okf-interchange#CON-4605]]; [[SPEC-017]].

### REQ-4610: `okf_version` Emission and Recognition
On export, the system SHALL declare `okf_version: "0.1"` in the **root `index.md`
frontmatter** (OKF §11's sole permitted `index.md` frontmatter). *Emitting it is
a zetl policy choice over OKF's MAY — OKF permits but does not require the
declaration; zetl always emits for unambiguous downstream consumption.* On check/
import the system SHALL recognise `okf_version` as a scalar matching `\d+\.\d+`
([[SPEC-046-okf-interchange#CON-4601]]): non-scalar/multi-line/malformed →
`okf-frontmatter-parse` (**error**); `"0.1"` → native; a **minor** unknown
(`0.x`, backward-compatible additions per OKF §11) → consume as 0.1 +
`okf-version-unknown` (warning); a **major** unknown (`≥1.0`, may break) → warn
and consume only what remains recognisable, WITHOUT claiming 0.1 semantics.
Generator and recogniser agree on placement.

**Trace:** [[SPEC-046-okf-interchange#TEST-4610]], [[SPEC-046-okf-interchange#CON-4601]].

### REQ-4611: OKF Bundle Import
Under the `okf` feature, `zetl okf import <bundle> -o <vault>` SHALL read an OKF
v0.1 *directory* bundle and produce a vault: each concept → a page (Concept ID →
slug, [[SPEC-046-okf-interchange#REQ-4603]]); frontmatter preserved
([[SPEC-046-okf-interchange#REQ-4614]]); path links → edges
([[SPEC-046-okf-interchange#REQ-4612]]); `type` → facet
([[SPEC-046-okf-interchange#REQ-4613]]). Import SHALL be deterministic per OS/
filesystem class ([[SPEC-046-okf-interchange#NFR-4601]]), tolerate OKF-permitted
incompleteness ([[SPEC-046-okf-interchange#REQ-4615]]), and fail closed on
malformed/hostile input ([[SPEC-046-okf-interchange#REQ-4618]],
[[SPEC-046-okf-interchange#REQ-4619]], [[SPEC-046-okf-interchange#NFR-4604]]).
A non-empty target vault policy is `[Blocked: Q4]`. A zero-concept bundle → an
empty-but-valid vault, not an error.

**Trace:** [[SPEC-046-okf-interchange#TEST-4611]], [[SPEC-046-okf-interchange#CON-4606]]; [[SPEC-046-okf-interchange#3.4 HP4]].

### REQ-4612: Path-Based Link Resolution (Import-Scoped)
WHEN importing (and only then — [[SPEC-046-okf-interchange#ADR-4603]]), the system
SHALL resolve bundle-path Markdown links into [[Link Graph]] edges via
[[SPEC-046-okf-interchange#CON-4602]]: a bundle-absolute target (`/a/b.md` or
`/a/b`) → Concept ID `a/b`; a relative target (`../c.md`, `./d.md`, `d.md`) →
resolved against the **linking concept's Concept-ID directory** (the same
identifier space targets resolve into — *not* the slug space), lexically
normalised within the root. The link grammar's percent-encoding SHALL be decoded
**before** the `.`/`..` normalisation and containment check (decode-then-check,
to defeat `%2e%2e` differentials). Edges are **untyped** (no prose-predicate
inference). A bundle-relative `# Citations` link to an in-bundle concept (incl.
`references/**`, [[SPEC-046-okf-interchange#REQ-4620]]) IS graphed; a citation
that is an external URL or a non-concept path is NOT graphed (treated as an
external reference). A target not resolving to a concept → ghost edge; a target
escaping the root or outside the [[SPEC-046-okf-interchange#CON-4602]] allowlist
(`\`, drive letters, UNC, non-`http(s)`/`mailto` schemes, NUL, control, overlong
UTF-8) → **rejected** (`okf-path-escape`), never read
([[SPEC-046-okf-interchange#Threat Model A]]).

**Trace:** [[SPEC-046-okf-interchange#TEST-4612]], [[SPEC-046-okf-interchange#CON-4602]], [[SPEC-046-okf-interchange#CON-4606]]; [[SPEC-046-okf-interchange#3.4 HP4]], [[SPEC-046-okf-interchange#3.5 HP5]].

### REQ-4613: OKF `type` → zetl Facet Mapping
On import, each `type` SHALL be preserved verbatim in frontmatter
([[SPEC-046-okf-interchange#REQ-4614]]) AND indexed as a **bounded scalar**
`type` field in the search index ([[SPEC-002]]) — length-capped per
[[SPEC-046-okf-interchange#NFR-4604]], control characters stripped/escaped before
indexing, a sequence/map `type` rejected (`okf-type-nonscalar`) — so an
attacker-controlled `type` cannot inject into or exhaust the index. An unknown
value is accepted verbatim (OKF tolerance); zetl validates `type` against no
registry. `type` SHALL NOT create typed *edges*
([[SPEC-046-okf-interchange#1.2 Relationship to the Typed-Edge Specs]]).

**Trace:** [[SPEC-046-okf-interchange#TEST-4613]], [[SPEC-002]]; [[SPEC-046-okf-interchange#3.4 HP4]].

### REQ-4614: Unknown-Field Preservation (Round-Trip Fidelity)
On import and export, ALL frontmatter fields — recognised, recommended, unknown —
SHALL be preserved verbatim (neither dropped, coerced, nor destructively
reordered). The codec reads only the fields it interprets (`type`, recommended
fields, `x-zetl-edges`, `okf_version`); all others are opaque, and preservation
does NOT imply interpretation ([[SPEC-046-okf-interchange#REQ-4617]]).

**Trace:** [[SPEC-046-okf-interchange#TEST-4614]], [[SPEC-046-okf-interchange#NFR-4602]]; [[SPEC-046-okf-interchange#3.4 HP4]].

### REQ-4615: Consumer Robustness (OKF-Mandated Incompleteness Only)
Per OKF §9, import/rendering SHALL treat as non-fatal (at most a diagnostic) and
ONLY these: missing recommended fields; an unknown `type`; unrecognised
frontmatter keys (preserved **inert**); broken/dangling links (→ ghosts); a
missing `index.md`/`log.md`; and — a zetl *resilience* choice distinct from the
OKF §9 list — an individual concept failing to parse (skipped, rest continues).
This is **strictly bounded** by §1.4 principles 3/4: malformed structure at the
trust boundary (path escape, symlink/hardlink/irregular file, over-budget input,
reserved-path write, non-scalar/over-budget `type`, malformed `okf_version`,
non-UTF-8, any interpreted key outside its sub-grammar) fails closed (§11).

**Trace:** [[SPEC-046-okf-interchange#TEST-4615]]; [[SPEC-046-okf-interchange#3.4 HP4]].

### REQ-4616: Backward-Compatible Default
WHEN no OKF command runs and no bundle is imported, the system SHALL behave
exactly as the pre-SPEC-046 release: no `type` requirement, no path-link in the
graph, no reserved-file reformatting, no new frontmatter key. OKF behaviour is
reachable ONLY via `zetl okf …` / `zetl export --format okf`. The persistence of
OKF link semantics in an already-imported vault on later builds is `[Blocked: Q2]`:
this guarantee holds unconditionally for never-imported vaults, and conditionally
(per the Q2 resolution) for imported ones.

**Trace:** [[SPEC-046-okf-interchange#TEST-4616]]; [[SPEC-046-okf-interchange#3.1 HP1]].

### REQ-4617: `x-zetl-edges` Is Inert in v1 (Authenticity Precondition)
On import, a concept's `x-zetl-edges` frontmatter — whatever its origin —
SHALL be **preserved as inert unknown data** ([[SPEC-046-okf-interchange#REQ-4614]])
and SHALL produce **no graph edge and no SPL fact** in v1. There is no
`--trust-edges` flag in v1. Rationale: safely turning a foreign contribution into
a live, reasoner-visible fact requires **verifiable contribution authenticity**
(did this `x-zetl-edges` entry truly come from a trusted author, untampered?),
which is a distinct concern from **trust/authorization** (whether to act on a
verified author's claim). v1 can verify neither for foreign contributions, so it
acts on none — closing the forged-fact vector by construction
([[SPEC-046-okf-interchange#Threat Model E]]). Live reconstruction is deferred
until a **detached per-contribution signature that lives with the document**
exists ([[SPEC-046-okf-interchange#Q7]]); when it does, reconstruction can be
re-enabled gated on signature verification (authenticity) with trust as an
explicit, separate policy — NOT inside the OKF importer
([[PROTO-001]] Principle 15).

**Trace:** [[SPEC-046-okf-interchange#TEST-4617]], [[SPEC-046-okf-interchange#CON-4606]], [[SPEC-046-okf-interchange#ADR-4604]]; [[SPEC-046-okf-interchange#Threat Model E]], [[SPEC-046-okf-interchange#Q7]].

### REQ-4618: Safe Read Boundary (Symlink / Hardlink / Irregular / TOCTOU)
WHEN scanning a bundle for import or `okf check`, the system SHALL admit ONLY
regular files and directories within the bundle root, enforced without a
time-of-check/time-of-use gap:

* the bundle **root** SHALL be `lstat`-checked (reject a symlinked root with
  `okf-symlink` unless `--follow-root-symlink`) and then `canonicalize`d once;
* each entry SHALL be opened with symlink-non-following semantics (`O_NOFOLLOW`,
  or `openat2` with `RESOLVE_NO_SYMLINKS|RESOLVE_BENEATH`) and classified by
  `fstat` on the **opened descriptor** (not by a separate `lstat` then `open`),
  then read from that same descriptor;
* a symbolic link → `okf-symlink`; a regular file with `st_nlink > 1` (a
  **hardlink**, indistinguishable from a regular file by type and able to alias
  an outside inode) → `okf-hardlink`; a FIFO/socket/char/block device → 
  `okf-irregular-file` (and the open SHALL be non-blocking so a FIFO cannot hang
  the scan); any entry whose descriptor resolves outside the canonical root →
  `okf-path-escape`.

This is the read-boundary counterpart to link resolution; §8's "no canonicalize"
rule applies to the **link recogniser** only ([[SPEC-046-okf-interchange#Threat Model F]]).

**Trace:** [[SPEC-046-okf-interchange#TEST-4618]]; [[SPEC-046-okf-interchange#Threat Model F]].

### REQ-4619: Vault-Write Confinement and Reserved-Path Rejection
On import, before writing any page the system SHALL compute the target path's
**fully percent-decoded, NFC-normalised, case-folded, lexically-normalised**
canonical form (the same form used for the actual write) and SHALL reject
(`okf-reserved-path`, **error**) any path whose **any component, at any depth** is
a leading-dot directory or a control file/dir (`.zetl*`, `.git*`, `.jj*`,
`.gitignore`, `.gitattributes`), or whose Concept ID maps to the reserved
filenames where they would parse as concepts. Every write SHALL be confined to
the output vault root via canonicalize-and-prefix; a path resolving outside, or
folding (under the target filesystem's case/normalisation rules) onto a reserved
path or an already-written concept, is rejected
([[SPEC-046-okf-interchange#Threat Model G]]). Reserved-file recognition is by
exact path (root-only unless [[SPEC-046-okf-interchange#Q5]]).

**Trace:** [[SPEC-046-okf-interchange#TEST-4619]]; [[SPEC-046-okf-interchange#Threat Model G]].

### REQ-4620: `references/` Citation Concepts Are First-Class
OKF §8 permits citation links into a `references/` subdirectory that mirrors
external material "as first-class OKF concepts." The system SHALL treat
`references/**.md` as **ordinary concepts** subject to the `type` requirement on
`okf check` ([[SPEC-046-okf-interchange#REQ-4601]]) and export
([[SPEC-046-okf-interchange#REQ-4607]]), and citation links to them SHALL graph
like any bundle path link ([[SPEC-046-okf-interchange#REQ-4612]]). A `references/`
file lacking `type` is `okf-missing-type` (error). (Governs the case where a
`references/` dir exists; OKF makes it one of three optional citation forms.)

**Trace:** [[SPEC-046-okf-interchange#TEST-4620]], [[SPEC-046-okf-interchange#REQ-4601]]; [[knowledge-catalog OKF SPEC]] §Citations.

### REQ-4621: `okf check` Operational Properties
`zetl okf check <dir>` SHALL be (a) **read-only**, (b) open **no network
socket**, (c) **deterministic**, (d) exit **non-zero iff** any error-level
diagnostic is present (else 0), and (e) run the **identical safe scanner** as
import ([[SPEC-046-okf-interchange#REQ-4618]]) so the auditor command cannot hang
on a FIFO or follow a swapped symlink. A non-existent `<dir>` is a usage error
(exit 2, distinct from non-conformant exit 1).

**Trace:** [[SPEC-046-okf-interchange#TEST-4621]], [[SPEC-046-okf-interchange#CON-4603]], [[SPEC-046-okf-interchange#NFR-4601]], [[SPEC-046-okf-interchange#NFR-4603]].

### REQ-4622: Frontmatter Field Mapping on Export
On export, all frontmatter other than `type` SHALL pass through unchanged
([[SPEC-046-okf-interchange#REQ-4614]]). OKF *recommended* fields SHALL be
populated from zetl equivalents only where one exists, never fabricated:
`title`/`description`/`tags` from namesakes; `timestamp` as an **ISO-8601
datetime** (OKF §4.1: "datetime of last meaningful change") sourced from the
[[SPEC-017]] history of content edits, with filesystem mtime a low-fidelity
fallback emitting `okf-timestamp-mtime` (note) only when history is unavailable.
OKF's `resource` (canonical URI) has no zetl equivalent and SHALL NOT be
synthesised (preserved only if already present).

**Trace:** [[SPEC-046-okf-interchange#TEST-4622]], [[SPEC-046-okf-interchange#CON-4604]]; [[SPEC-017]].

---

## 5. Non-Functional Requirements

### NFR-4601: Export/Import Determinism (Per OS/Filesystem Class)
A given (vault, options, feature-set) tuple SHALL produce a byte-identical bundle
across repeated exports, and a (bundle, options) pair an identical vault across
repeated imports, WITH 100% reproducibility **on the same OS/filesystem class**
(no wall-clock, no map-iteration-order, no locale-sensitive sorting). Cross-OS
byte-identity is NOT claimed (Concept-ID/case/Unicode semantics are filesystem-
dependent); the importer's **identity model is OS-independent**
([[SPEC-046-okf-interchange#REQ-4603]]) so the graph and collision verdict are
OS-invariant.

**Trace:** [[SPEC-046-okf-interchange#TEST-4601]], [[SPEC-046-okf-interchange#OBS-4601]].

### NFR-4602: Round-Trip Fidelity (Concrete Equivalence)
For a vault `V` in the OKF-expressible subset, `import(export(V))` SHALL satisfy
`eq(import(export(V)), V)`, where `eq` is: equal Concept-ID set; per-concept
frontmatter deep-equal under canonical key ordering, modulo named exceptions
(this **includes `x-zetl-edges` as data**, since v1 preserves it inertly);
equal untyped-edge set; equal node-`type` multiset. **Permitted divergences:** the
internal slug; out-of-scope content (SPL blocks, capability config);
**regenerated reserved files** (`index.md`/`log.md` are generated, not
round-tripped — hand-authored reserved content in a source bundle is lost); and
**typed-edge *liveness*** — predicates survive as inert `x-zetl-edges` data but
are NOT reconstructed into live edges in v1 (deferred,
[[SPEC-046-okf-interchange#Q7]]).

**Trace:** [[SPEC-046-okf-interchange#TEST-4614]], [[SPEC-046-okf-interchange#TEST-4606]], [[SPEC-046-okf-interchange#TEST-4617]].

### NFR-4603: Conformance-Check Latency
`zetl okf check` SHALL complete in ≤ 2 s for a 1,000-concept bundle (each ≤ 8 KiB)
at the 95th percentile over 20 runs with warm page cache on the project reference
CI runner (`[Provisional: pin exact runner class/CPU/SSD in IMPL-046]`), reusing
the existing scan+parse pass. The 1,000-concept figure is `[Provisional]` pending
the Phase 1 catalogue-size survey.

**Trace:** [[SPEC-046-okf-interchange#TEST-4621]], [[SPEC-046-okf-interchange#OBS-4602]].

### NFR-4604: Bundle Input Bounds (Fail-Closed, Streaming)
The recogniser SHALL enforce concrete fail-closed bounds, each checked **while
reading (read at most bound+1 bytes, then abort)** — never load-then-check — and
before any semantic action: per-frontmatter-document ≤ 256 KiB; YAML nesting
depth ≤ 32; per-scalar length ≤ 64 KiB; per-concept body ≤ 16 MiB; per-file
absolute ≤ 32 MiB; total bundle ≤ 5 GiB, ≤ 200,000 files, directory depth ≤ 64.
The OKF YAML subset **forbids anchors/aliases, merge keys (`<<`), and custom tags
(`!!…`)** outright ([[SPEC-046-okf-interchange#ADR-4607]]) — so the billion-laughs
expansion vector is removed by construction rather than bounded by a factor. All
numeric values are `[Provisional]` defaults to confirm in IMPL-046; the
*presence* of each bound and the streaming-enforcement and no-aliases rules are
normative.

**Trace:** [[SPEC-046-okf-interchange#TEST-4623]], [[SPEC-046-okf-interchange#OBS-4603]]; [[SPEC-046-okf-interchange#Threat Model B]].

---

## 6. Architecture Decision Records

### ADR-4601: OKF Is a Peripheral Codec, Not an Internal Model
OKF lives at the edge: import (OKF→native), export (native→OKF), check. The
native graph/AST/reasoner/link-resolution are unchanged; path-link resolution is
import-scoped ([[SPEC-046-okf-interchange#ADR-4603]]); `type`-required is
check/export-scoped ([[SPEC-046-okf-interchange#ADR-4605]]). (+) Zero blast
radius; tracks upstream as a codec change; [[PROTO-001]] Principle 15. (−)
Round-trip lossy by construction; named ([[SPEC-046-okf-interchange#ADR-4604]]).

### ADR-4602: Concept ID Is Path-Verbatim (NFC); Slug Stays Internal
OKF-facing identifier = path verbatim, NFC, case-preserved
([[SPEC-046-okf-interchange#REQ-4603]]); internal slug unchanged; codec keeps the
bijection; collisions (case/NFC/confusable) detected, never silently merged
([[SPEC-046-okf-interchange#Threat Model C]]). Rejected: global case-preserving
slugs (breaks every vault/URL); slug-as-Concept-ID (corrupts OKF identity).

### ADR-4603: Path-Link Resolution Is Import-Scoped — Persistence `[Blocked: Q2]`
The OKF path-link recogniser feeds the graph during **import**; whether an
already-imported vault keeps resolving path links on later builds, and the marker
that records the dialect, depend on [[SPEC-046-okf-interchange#Q2]]. Until Q2
closes, only import-time behaviour is decided; the persistence behaviour and its
interaction with [[SPEC-046-okf-interchange#REQ-4616]] MUST NOT pass the Phase 2
gate. The dialect, once settled, MUST be an explicit named vault parameter, not
implicit context ([[PROTO-001]] §CON context-invariance) — else an imported vault
could silently retain OKF semantics and break
[[SPEC-046-okf-interchange#REQ-4616]].

### ADR-4604: Typed Edges Flatten With Named Loss; Reconstruction Deferred to a Signing Capability
Export flattens to untyped links and preserves the predicate as inert
`x-zetl-edges` data ([[SPEC-046-okf-interchange#REQ-4606]]). **v1 never
reconstructs** ([[SPEC-046-okf-interchange#REQ-4617]]). Reconstruction was the
v0.1.1 design but the second review showed it unsound: the provenance axis it
needed ("distinct from human-authored", reasoner-scopable) **does not exist** in
[[SPEC-045-wikilink-predicate-language]] (`_source_page` = "which page", not "what
origin / signed by whom"), and a plaintext sidecar has no integrity binding (a
MITM forges edges into an operator's own bundle between a trusted export and a
trusted re-import). The correct fix separates **authenticity** (a detached
per-contribution signature that lives with the document) from **trust** (reasoner
policy over verified authorship) — a shared capability, not an OKF detail
([[SPEC-046-okf-interchange#Q7]], [[PROTO-001]] Principle 15). Rejected: encode
predicate in prose (not machine-recoverable); drop silently (integrity defect,
[[SPEC-046-okf-interchange#Threat Model D]]); auto-reconstruct or operator-flag
reconstruct (forged-fact injection, [[SPEC-046-okf-interchange#Threat Model E]]).

### ADR-4605: `type`-Required Is Scoped to Check/Export, Not Native Authoring
`type` is recognised-but-optional natively; required only by `okf check` and
synthesised on export ([[SPEC-046-okf-interchange#REQ-4607]]). Native `zetl check`
unchanged. "Conformant" = exactly OKF conformance, no zetl tax.

### ADR-4606: A `zetl okf` Subcommand Group; `export --format okf` Reuses Export
`okf` is a value of `zetl export --format`; check/import are grouped under
`zetl okf`. Export belongs on the export surface; check/import are new
responsibilities ([[PROTO-001]] Principle 15).

### ADR-4607: Constrain the YAML Input Language; Forbid Aliases; Reject Postel
Per [[PROTO-001]] §LangSec Principle 6, the OKF frontmatter input language is a
**bounded YAML subset**: mappings/sequences/scalars only; **anchors/aliases,
merge keys (`<<`), and custom tags (`!!…`) are rejected outright** (OKF
frontmatter needs none; this removes the billion-laughs and
deserialisation-RCE vectors by construction rather than by a fragile expansion
bound). Concrete size/depth/scalar bounds are enforced while streaming
([[SPEC-046-okf-interchange#NFR-4604]]). [[Postel's Law]] is NOT adopted; the
only tolerance is OKF-mandated preservation of inert unknown keys. Any
*interpreted* key (`type`, `okf_version`, `x-zetl-edges`) has its own strict
sub-grammar. (−) A bundle using aliases/merge keys/custom tags in frontmatter is
rejected even though some loaders accept it — the correct trust-boundary choice.

---

## 7. Contracts

### CON-4601: OKF Frontmatter Grammar (LangSec)
Recogniser for a concept's frontmatter, shared by check/export/import.

```abnf
concept       = utf8-file
utf8-file     = <MUST be valid UTF-8 (OKF §4.1); else okf-encoding>
                [ frontmatter ] body
frontmatter   = "---" LF yaml-subset "---" LF
yaml-subset   = <bounded YAML per ADR-4607: scalars/seqs/maps only; NO anchors/
                 aliases, NO merge keys, NO custom tags; NFR-4604 size/depth/
                 scalar bounds enforced WHILE STREAMING, before any value is read>
body          = *OCTET                              ; CommonMark; ≤ NFR-4604 body bound
type-value    = <non-empty scalar; ≤ NFR-4604 scalar bound; not a seq/map>
version-value = <scalar matching %x30-39 1*"." %x30-39 ; "0.1" native, else best-effort per REQ-4610>
```

**Post.** `Recognised{frontmatter, body}` or `OkfParseError{kind, file, line}`;
recognition complete before any semantic action; no value coerced. Reserved files
recognised by exact path, exempt from `type`; root `index.md` MAY carry a
frontmatter block containing only `okf_version`.
**Errors.** `okf-encoding`, `okf-frontmatter-parse`, `okf-missing-type`,
`okf-type-nonscalar`.
**Implements:** [[SPEC-046-okf-interchange#REQ-4601]], [[SPEC-046-okf-interchange#REQ-4610]], [[SPEC-046-okf-interchange#REQ-4613]], [[SPEC-046-okf-interchange#REQ-4614]].
**Verified by:** [[SPEC-046-okf-interchange#TEST-4601]], [[SPEC-046-okf-interchange#TEST-4623]], [[SPEC-046-okf-interchange#TEST-4610]].

### CON-4602: OKF Link Grammar and Path Resolution (LangSec)
Recogniser classifying a Markdown link destination, used only on import.
**Allowlist, not denylist**; percent-decode BEFORE normalisation/containment.

```abnf
link-dest    = bundle-abs / relative / external
bundle-abs   = "/" target
relative     = *( "./" / "../" ) target              ; base = linking concept's Concept-ID dir
external     = ( "http" [ "s" ] / "mailto" ) ":" *VCHAR   ; closed scheme set; not graphed
target       = concept-path / dir-path
concept-path = segment *( "/" segment ) [ ".md" ] [ "#" fragment ]
dir-path     = segment *( "/" segment ) "/"          ; OKF §6 directory entry → directory/index, not a concept
segment      = 1*pchar
pchar        = unreserved / pct-encoded              ; pct-encoded DECODED before checks
unreserved   = ALPHA / DIGIT / "-" / "_" / "." / NFC-non-ASCII
              ; EXCLUDES "\" "~" ":" control %x00-1F NUL and non-shortest UTF-8 (rejected pre-norm)
fragment     = *( pchar / "/" )
```

A `.`/`..` segment is allowed only as the leading `relative` prefix; embedded ones
are normalised lexically AFTER percent-decoding. Backslash, drive letters, UNC,
`~`, foreign schemes, NUL, overlong UTF-8 are out of grammar → rejected.

**Post.** Exactly one of `Edge{concept_id}` / `Dir{path}` / `External` /
`Ghost{concept_id}` / **parse-failure** `Rejected{path-escape}` (resolves outside
root or outside allowlist). `Rejected` is **never read or resolved**. Normalisation
is **lexical, filesystem-free** (no `canonicalize` here — that is the scanner's job,
[[SPEC-046-okf-interchange#REQ-4618]]).
**Errors.** `okf-path-escape` (error), `okf-import-dead-link` (info, ghost).
**Implements:** [[SPEC-046-okf-interchange#REQ-4612]].
**Verified by:** [[SPEC-046-okf-interchange#TEST-4612]].

### CON-4603: `zetl okf check` CLI
`zetl okf check <dir> [-f {auto,json,table}]`. **Post.** Conformance report
(per [[SPEC-046-okf-interchange#REQ-4601]]/[[SPEC-046-okf-interchange#REQ-4602]]):
per-file diagnostics + summary `{concepts, errors, warnings, conformant,
okf_version}`. Exit 0 iff zero errors; exit 2 for a non-existent dir. Read-only,
no network, deterministic; runs the safe scanner
([[SPEC-046-okf-interchange#REQ-4618]]). `-f json` is a stable CI schema.
**Implements:** [[SPEC-046-okf-interchange#REQ-4601]], [[SPEC-046-okf-interchange#REQ-4602]], [[SPEC-046-okf-interchange#REQ-4621]].
**Verified by:** [[SPEC-046-okf-interchange#TEST-4601]], [[SPEC-046-okf-interchange#TEST-4602]], [[SPEC-046-okf-interchange#TEST-4621]].

### CON-4604: `zetl export --format okf` Bundle Contract
`zetl export --format okf -o <dir> [--default-type <s>] [--embed-mode <m>]`.
**Post.** An OKF v0.1 bundle: one `.md` per page at its Concept-ID path; non-empty
scalar `type` ([[SPEC-046-okf-interchange#REQ-4607]]); all other fields round-trip
([[SPEC-046-okf-interchange#REQ-4614]], [[SPEC-046-okf-interchange#REQ-4622]]);
wikilinks → `.md` concept links ([[SPEC-046-okf-interchange#REQ-4605]]); typed
edges → untyped links + inert `x-zetl-edges`
([[SPEC-046-okf-interchange#REQ-4606]]); a root `index.md` with `okf_version`
([[SPEC-046-okf-interchange#REQ-4608]], [[SPEC-046-okf-interchange#REQ-4610]]); a
`log.md` when history is available ([[SPEC-046-okf-interchange#REQ-4609]]). Passes
[[SPEC-046-okf-interchange#CON-4603]]; byte-stable
([[SPEC-046-okf-interchange#NFR-4601]]).
**Implements:** [[SPEC-046-okf-interchange#REQ-4604]]–[[SPEC-046-okf-interchange#REQ-4610]], [[SPEC-046-okf-interchange#REQ-4622]].
**Verified by:** [[SPEC-046-okf-interchange#TEST-4604]]–[[SPEC-046-okf-interchange#TEST-4610]], [[SPEC-046-okf-interchange#TEST-4622]].

### CON-4605: Reserved-File Grammar (Generation + Recognition)
One grammar, two directions. Errors only on OKF MUSTs
([[SPEC-046-okf-interchange#REQ-4602]]).

```abnf
index       = [ root-version-fm ] *section          ; *section: empty index valid
root-version-fm = "---" LF "okf_version: " DQUOTE %s"0.1" DQUOTE LF "---" LF
                  ; ROOT index.md ONLY; the only frontmatter permitted in any index.md
section     = "# " title LF 1*entry
entry       = "* [" text "](" link-dest ")" [ " - " description ] LF   ; description SHOULD (warn if absent)
              ; link-dest is a concept link (/id.md) OR an OKF §6 directory link (subdir/)

log         = [ "# " title LF ] 1*day               ; optional leading title heading (OKF §7)
day         = "## " iso-date LF 1*change            ; iso-date = YYYY-MM-DD (the ONLY §7 MUST), newest-first SHOULD
change      = "* " [ "**" kind "**: " ] description LF   ; bold kind is a CONVENTION (warn-only if absent), not required
```

**Post.** Generation matches the grammar; recognition emits
`okf-reserved-structure` (**error**) only on a MUST violation (non-ISO date
heading; disallowed frontmatter) and `okf-reserved-convention` (**warning**) on a
SHOULD/convention deviation (missing description, non-newest-first, missing bold
kind).
**Implements:** [[SPEC-046-okf-interchange#REQ-4602]], [[SPEC-046-okf-interchange#REQ-4608]], [[SPEC-046-okf-interchange#REQ-4609]].
**Verified by:** [[SPEC-046-okf-interchange#TEST-4602]], [[SPEC-046-okf-interchange#TEST-4608]], [[SPEC-046-okf-interchange#TEST-4609]].

### CON-4606: `zetl okf import` Mapping + `x-zetl-edges` Grammar
`zetl okf import <bundle> -o <vault>` (no `--trust-edges` in v1).

```abnf
x-zetl-edges = "[" *edge "]"                         ; preserved INERT in v1 (REQ-4617); grammar defined for a future Q7 capability
edge         = "{" "predicate" ":" pred-name "," "target" ":" concept-id-ref
               [ "," "annotation" ":" bounded-string ] "}"
pred-name    = 1*( LALPHA / DIGIT / "_" )            ; SPEC-045 lowercase-snake; length-bounded
```

**Post.** Each concept → a page at its slug
([[SPEC-046-okf-interchange#REQ-4603]]); frontmatter preserved
([[SPEC-046-okf-interchange#REQ-4614]]); `type` indexed (bounded,
[[SPEC-046-okf-interchange#REQ-4613]]); resolved path links → untyped edges,
dangling → ghosts, escaping/out-of-allowlist → rejected
([[SPEC-046-okf-interchange#CON-4602]]); symlinks/hardlinks/irregular files →
rejected ([[SPEC-046-okf-interchange#REQ-4618]]); reserved-path IDs → rejected
([[SPEC-046-okf-interchange#REQ-4619]]); `references/**` → concepts
([[SPEC-046-okf-interchange#REQ-4620]]); **`x-zetl-edges` preserved inert** — no
edge, no fact ([[SPEC-046-okf-interchange#REQ-4617]]). Deterministic per OS class;
OKF-permitted incompleteness tolerated, malformed structure fails closed.
**Errors.** `okf-frontmatter-parse`, `okf-encoding`, `okf-id-collision`/
`okf-id-confusable`/`okf-invalid-concept-id`, `okf-symlink`/`okf-hardlink`/
`okf-irregular-file`, `okf-reserved-path`; bundle-fatal only on unreadable/
over-budget root.
**Implements:** [[SPEC-046-okf-interchange#REQ-4611]]–[[SPEC-046-okf-interchange#REQ-4620]].
**Verified by:** [[SPEC-046-okf-interchange#TEST-4611]]–[[SPEC-046-okf-interchange#TEST-4620]].

---

## 8. Purity Boundary Map

### Pure Core (no I/O, deterministic)
OKF frontmatter recogniser ([[SPEC-046-okf-interchange#CON-4601]]); OKF link
recogniser + lexical, **filesystem-free** path normaliser
([[SPEC-046-okf-interchange#CON-4602]]); Concept-ID ↔ slug mapping + NFC/case-fold/
confusable policy ([[SPEC-046-okf-interchange#REQ-4603]]); wikilink→`.md`
rewriter, edge flattener, `x-zetl-edges` validator; reserved-file generator/
recogniser ([[SPEC-046-okf-interchange#CON-4605]]); conformance evaluator; bound
checks ([[SPEC-046-okf-interchange#NFR-4604]]).

### Effectful Shell (orchestrates I/O)
Directory scan/read with **fd-based, symlink-non-following, irregular-file-
rejecting, canonicalize-and-contain** access ([[SPEC-046-okf-interchange#REQ-4618]]);
bundle write; vault write **confined to vault root**
([[SPEC-046-okf-interchange#REQ-4619]]); [[SPEC-017]] history; search-index write;
CLI/exit/render.

### Boundary Contracts
`Recognised{frontmatter, body}`; `LinkResolution`; `OkfReport`; `x-zetl-edges`
validated-but-inert array.

### Dependency Rule
Shell → core; recognisers do no I/O. **Link-recogniser normalisation MUST be
lexical (no `canonicalize`)**; **the scanner and writer MUST `canonicalize` and
verify containment** ([[SPEC-046-okf-interchange#REQ-4618]],
[[SPEC-046-okf-interchange#REQ-4619]]) — complementary, not in conflict.

### Enforcement
Module boundary (`src/okf/` core vs `src/okf/io.rs` shell) + arch-lint; property
+ fuzz tests target the pure recognisers.

---

## 9. Verification and Testing Strategy

> A parser at a trust boundary ⇒ **fuzzing + property-based roundtrip** mandatory,
> plus example-based three-way decomposition per REQ. Each TEST below is a `###`
> heading (so anchors resolve) and validates exactly ONE requirement intent (NFR
> coverage attributed separately in §12).

### TEST-4601
Validates [[SPEC-046-okf-interchange#REQ-4601]]. *Pos* every concept has `type` →
conformant. *Neg-input* unparseable YAML / non-UTF-8 → `okf-frontmatter-parse` /
`okf-encoding`, exit≠0. *Neg-output* `type: ""` MUST NOT be reported conformant.

### TEST-4602
Validates [[SPEC-046-okf-interchange#REQ-4602]]. *Pos* well-formed reserved files
→ no diagnostic; a `log.md` with no bold kind → at most a **warning** (still
conformant). *Neg-input* non-ISO `log.md` heading → `okf-reserved-structure`
**error**. *Neg-output* a missing index MUST NOT produce a diagnostic; a
convention deviation MUST NOT be reported non-conformant.

### TEST-4603
Validates [[SPEC-046-okf-interchange#REQ-4603]]. *Pos* `A/B.md` → `A/B`
(case-preserved). *Neg-input* `Foo.md`+`foo.md` and NFD/NFC `café` → 
`okf-id-collision`; Cyrillic/Latin `admin` → `okf-id-confusable`; a `.md` file
(empty ID) / `..md` → `okf-invalid-concept-id`. *Neg-output* colliding concepts
MUST NOT merge into one page.

### TEST-4604
Validates [[SPEC-046-okf-interchange#REQ-4604]]. *Pos* export output passes
`okf check`. *Neg-output* output with a `type`-less concept MUST fail the
self-check.

### TEST-4605
Validates [[SPEC-046-okf-interchange#REQ-4605]]. *Pos* `[[Scanner]]` →
`/Architecture/Scanner.md` (`.md`, case kept); unresolved `[[Foo Bar]]` →
deterministic `/<slug>.md` + `okf-export-dead-link`. *Neg-output* rewrite MUST
NOT lowercase the path or omit `.md` on a concept link.

### TEST-4606
Validates [[SPEC-046-okf-interchange#REQ-4606]]. *Pos* `supersedes::[[X]]` →
untyped link + inert `x-zetl-edges` + note. *Neg-output* MUST NOT drop the
predicate with no sidecar (silent loss).

### TEST-4607
Validates [[SPEC-046-okf-interchange#REQ-4607]]. *Pos* existing `type` passes;
absent → `--default-type` + `okf-type-synthesised`. *Neg-output* a `type`-less
page MUST NOT be exported.

### TEST-4608
Validates [[SPEC-046-okf-interchange#REQ-4608]]. *Pos* concept entries
`[Title](/id.md) - desc`, directory entries `[Sub](sub/)`; empty vault → valid
frontmatter-only index. *Neg-output* a concept entry MUST NOT omit `.md`; a
directory entry MUST NOT be forced to carry `.md`.

### TEST-4609
Validates [[SPEC-046-okf-interchange#REQ-4609]]. *Pos* history present →
newest-first ISO log, no frontmatter, optional title heading. *Neg-output*
history absent MUST NOT fabricate a log (omit + note).

### TEST-4610
Validates [[SPEC-046-okf-interchange#REQ-4610]]. *Pos* root-index
`okf_version:"0.1"` recognised. *Neg-input* `okf_version:` as a list →
`okf-frontmatter-parse`; `"0.2"` (minor) → warning + 0.1 consumption; `"1.0"`
(major) → warning + recognisable-only, no 0.1 claim. *Neg-output* a non-root
index with frontmatter MUST be reported malformed.

### TEST-4611
Validates [[SPEC-046-okf-interchange#REQ-4611]]. *Pos* a directory bundle
imports. *Neg-input* zero-concept bundle → empty valid vault; an archive path →
rejected (directory-only). *Neg-output* a partial parse failure MUST NOT abort
the whole import.

### TEST-4612
Validates [[SPEC-046-okf-interchange#REQ-4612]] / [[SPEC-046-okf-interchange#CON-4602]] / [[SPEC-046-okf-interchange#Threat Model A]].
*Pos* `/a/b.md`→edge `a/b`; `../c.md` from `a/d.md`→edge `c`; a `# Citations`
in-bundle link graphed. *Neg-input* `/../../etc/passwd`, `%2e%2e%2fetc`, `C:\x`,
`\\srv\s`, `file:///x`, `~/.ssh`, NUL → `okf-path-escape`, no fs read.
*Neg-output* a dangling in-bundle path MUST become a ghost, not resolve to an
unrelated concept.

### TEST-4613
Validates [[SPEC-046-okf-interchange#REQ-4613]]. *Pos* `type` indexed +
filterable. *Neg-input* `type:` as a map → `okf-type-nonscalar`; 1 MiB `type` →
bound-rejected. *Neg-output* `type` MUST NOT create a typed edge.

### TEST-4614
Validates [[SPEC-046-okf-interchange#REQ-4614]] / [[SPEC-046-okf-interchange#NFR-4602]].
*Property* round-trip preserves Concept-IDs, all frontmatter keys (incl.
`x-custom` and inert `x-zetl-edges`), untyped-link set, `type` multiset.
*Neg-output* an unknown field MUST NOT be dropped or coerced.

### TEST-4615
Validates [[SPEC-046-okf-interchange#REQ-4615]]. *Pos* missing recommended fields
/ unknown `type` / unknown key / broken link / missing index → non-fatal.
*Neg-output* a path escape / symlink / over-budget input MUST NOT be tolerated
(fails closed).

### TEST-4616
Validates [[SPEC-046-okf-interchange#REQ-4616]]. *Pos* the [[SPEC-001]] suite
passes with no OKF invocation. *Neg-output* a `[x](/path)` link in a native
(never-imported) vault MUST NOT be graphed, on first build or any later build.
*Neg-input* running `zetl okf check --help` MUST NOT mutate the vault.

### TEST-4617
Validates [[SPEC-046-okf-interchange#REQ-4617]] / [[SPEC-046-okf-interchange#Threat Model E]].
*Pos* an imported `x-zetl-edges` is preserved verbatim in frontmatter.
*Neg-output* a hand-authored `validated_by` `x-zetl-edges` MUST NOT produce a
graph edge or an SPL fact (no `--trust-edges` exists); MUST NOT reach the
reasoner.

### TEST-4618
Validates [[SPEC-046-okf-interchange#REQ-4618]] / [[SPEC-046-okf-interchange#Threat Model F]].
*Neg-input* a symlink file/dir → `okf-symlink`; a hardlink (`st_nlink>1`) →
`okf-hardlink`; a FIFO → `okf-irregular-file` without hanging; a symlinked
bundle root → `okf-symlink`. *Pos* a regular file imports.

### TEST-4619
Validates [[SPEC-046-okf-interchange#REQ-4619]] / [[SPEC-046-okf-interchange#Threat Model G]].
*Neg-input* Concept IDs `.zetl/config.toml`, `.ZETL/config.toml` (case-fold),
`..%2f.zetl%2fconfig` (encoded), `data/.git/hooks/pre-commit` (nested), a
root `.gitignore`, a slug resolving outside vault root → `okf-reserved-path`, not
written. *Pos* `data/x` writes within root.

### TEST-4620
Validates [[SPEC-046-okf-interchange#REQ-4620]]. *Pos* `references/rfc1.md` with
`type` is a concept; a citation link to it graphs. *Neg-input*
`references/rfc1.md` without `type` → `okf-missing-type`.

### TEST-4621
Validates [[SPEC-046-okf-interchange#REQ-4621]] / [[SPEC-046-okf-interchange#NFR-4603]].
*Pos* check is read-only, no socket, deterministic, ≤2s on the 1k fixture, exit 0
on conformant, and does not hang on a FIFO (shares the safe scanner).
*Neg-input* non-existent `<dir>` → exit 2.

### TEST-4622
Validates [[SPEC-046-okf-interchange#REQ-4622]]. *Pos* `title`/`description`/`tags`
mapped from namesakes; `timestamp` an ISO-8601 **datetime** from content history.
*Neg-output* `resource` MUST NOT be fabricated; `timestamp` MUST NOT come from
mtime when history is available; `timestamp` MUST NOT be a date-only value.

### TEST-4623
Validates [[SPEC-046-okf-interchange#NFR-4604]] / [[SPEC-046-okf-interchange#Threat Model B]].
*Neg-input* 257 KiB frontmatter; depth-33 YAML; 65 KiB scalar; a YAML
anchor/alias; a `<<` merge key; a `!!tag`; a 33 MiB file; a FIFO body — each
rejected fail-closed without OOM/hang, enforced while streaming. *Pos* a 200 KiB
frontmatter at depth 30 accepted.

### Adversarial testing (mandatory, post-acceptance)
A fresh-context (different-family) agent attacks the recognisers: homoglyph/NFD
IDs, `type` non-scalar, CRLF/BOM delimiters, encoded/looping `../`, symlink/
hardlink races, FIFO entries, `okf_version` future values, giant scalars, forged
`x-zetl-edges`. Findings → new REQ (gap) or BUG.

---

## 10. Observability

### OBS-4601: Export/Import Counters
Per run: concepts written/read, links rewritten/resolved, ghost edges, rejected
path-escapes/symlinks/hardlinks/irregular-files/reserved-paths, edges flattened,
types synthesised, `x-zetl-edges` preserved (inert). Backs
[[SPEC-046-okf-interchange#NFR-4601]] and named-loss visibility.

### OBS-4602: Conformance-Check Metrics
Per check: concept count, error/warning counts, wall-clock duration (backs
[[SPEC-046-okf-interchange#NFR-4603]]), conformant verdict.

### OBS-4603: Trust-Boundary Rejections
Counts of `okf-path-escape`, `okf-symlink`, `okf-hardlink`, `okf-irregular-file`,
`okf-reserved-path`, `okf-frontmatter-parse` (over-budget), `okf-id-collision`,
`okf-encoding`. A non-zero rate from a source is a signal
([[SPEC-046-okf-interchange#Threat Model A]]–[[SPEC-046-okf-interchange#Threat Model G]],
[[SPEC-046-okf-interchange#NFR-4604]]).

---

## 11. Threat Model

> Importing an externally-authored bundle is a **trust boundary**. Review tier 2.

### Threat Model A: Path Traversal via Bundle Links
**Threat.** `[x](/../../etc/passwd)`, `%2e%2e` encodings, `\`/drive/UNC/`file:`,
`~` targets read or link outside the root. **Mitigation.** Decode-then-check,
lexical filesystem-free normalisation, allowlist `pchar`
([[SPEC-046-okf-interchange#CON-4602]]); out-of-root/out-of-allowlist → parse
failure (`okf-path-escape`), never read. [[SPEC-046-okf-interchange#TEST-4612]].

### Threat Model B: Resource Exhaustion
**Threat.** Billion-laughs, giant scalar, unbounded body, huge/deep bundle,
FIFO-induced hang. **Mitigation.** Streaming fail-closed bounds + per-file cap;
**anchors/aliases/merge-keys/custom-tags forbidden outright**
([[SPEC-046-okf-interchange#NFR-4604]], [[SPEC-046-okf-interchange#ADR-4607]]);
irregular files rejected ([[SPEC-046-okf-interchange#REQ-4618]]).

### Threat Model C: Concept-ID Collision (Case / Unicode / Confusable)
**Threat.** Case (`Foo`/`foo`), NFC/NFD (`café`), homoglyph (`аdmin`) collapse or
impersonate — merging concepts or steering an agent to an impostor.
**Mitigation.** OS-independent identity model detects collisions
(`okf-id-collision`, error) and confusables (`okf-id-confusable`, warning,
**surfaced on agent-facing `zetl edges --json`**, UTS #39 mechanism + pinned
Unicode version); no silent merge ([[SPEC-046-okf-interchange#REQ-4603]]).

### Threat Model D: Silent Downcast Loss (Integrity)
**Threat.** Typed edges dropped silently on export. **Mitigation.** `x-zetl-edges`
sidecar + reported count ([[SPEC-046-okf-interchange#REQ-4606]]); synthesised-type
/ dead-link / anchor / mtime notes.

### Threat Model E: Forged-Fact Injection via `x-zetl-edges`
**Threat.** OKF mandates preserving unknown keys, so any bundle can author
`x-zetl-edges` asserting `conforms_to::[[Security Contract]]` /
`validated_by::[[Security Team]]`. If reconstructed, these become provenance-tagged
SPL facts feeding `zetl reason`, laundering attacker claims invisibly and
defeating the [[SPEC-045-wikilink-predicate-language]] §11 page-authored-facts
trust model. The v0.1.1 mitigation (opt-in `--trust-edges` + import-origin mark)
was unsound: SPEC-045 provenance is "which page", not "what origin", so a forged
fact is byte-indistinguishable from a real one; and a plaintext sidecar has no
integrity, so a MITM forges edges into an operator's own bundle between a trusted
export and re-import. **Mitigation (v1).** Reconstruction is **removed entirely**:
`x-zetl-edges` is inert ([[SPEC-046-okf-interchange#REQ-4617]]) — no edge, no fact,
no flag to flip. The vector is closed by construction. Safe future reconstruction
requires **authenticity** (a detached per-contribution signature that lives with
the document — making provenance a verifiable property of the *contribution*, the
axis SPEC-045 lacks) separated from **trust** (reasoner policy over verified
authorship) — a shared capability, candidate SPEC-047
([[SPEC-046-okf-interchange#Q7]], [[PROTO-001]] Principle 15).
[[SPEC-046-okf-interchange#TEST-4617]].

### Threat Model F: Host-File Access via Symlinks / Hardlinks / Irregular Files
**Threat.** A bundle entry that is a **symlink** (`notes.md → ~/.ssh/id_rsa`), a
**hardlink** to an outside inode (indistinguishable from a regular file by type),
a **symlinked directory** (`data → /`), a **symlinked root**, or a **FIFO/device**
(read hangs → DoS) lets the scanner read host content or stall — an attack
link-resolution (Threat A) never sees. **Mitigation.** fd-based,
symlink-non-following access (`O_NOFOLLOW`/`openat2 RESOLVE_NO_SYMLINKS|
RESOLVE_BENEATH`) with classification by `fstat` on the opened descriptor (no
lstat-then-open TOCTOU); reject symlinks (`okf-symlink`), `st_nlink>1` hardlinks
(`okf-hardlink`), and FIFO/socket/device (`okf-irregular-file`, non-blocking open);
root lstat-checked before canonicalize
([[SPEC-046-okf-interchange#REQ-4618]]). [[SPEC-046-okf-interchange#TEST-4618]].

### Threat Model G: Reserved-Path Write / Control-File Shadowing
**Threat.** A concept whose Concept ID is `.zetl/config.toml`, `.ZETL/…` (case
fold), `..%2f.zetl%2f…` (encoded), `data/.git/hooks/…` (nested), or a root
`.gitignore`/`.gitattributes` writes into the vault's control surface — config
takeover or hook RCE. **Mitigation.** Reserved/containment checks run on the
fully decoded, NFC, case-folded, lexically-normalised path, component-wise at any
depth, with writes confined to the vault root and guarded against the target
filesystem's folding ([[SPEC-046-okf-interchange#REQ-4619]]).
[[SPEC-046-okf-interchange#TEST-4619]].

---

## 12. Traceability

| REQ | Tests | Contracts | ADRs | Threats |
|---|---|---|---|---|
| [[SPEC-046-okf-interchange#REQ-4601]] | [[SPEC-046-okf-interchange#TEST-4601]] | [[SPEC-046-okf-interchange#CON-4601]], [[SPEC-046-okf-interchange#CON-4603]] | [[SPEC-046-okf-interchange#ADR-4605]] | — |
| [[SPEC-046-okf-interchange#REQ-4602]] | [[SPEC-046-okf-interchange#TEST-4602]] | [[SPEC-046-okf-interchange#CON-4605]] | — | — |
| [[SPEC-046-okf-interchange#REQ-4603]] | [[SPEC-046-okf-interchange#TEST-4603]] | [[SPEC-046-okf-interchange#CON-4606]] | [[SPEC-046-okf-interchange#ADR-4602]] | [[SPEC-046-okf-interchange#Threat Model C]] |
| [[SPEC-046-okf-interchange#REQ-4604]] | [[SPEC-046-okf-interchange#TEST-4604]] | [[SPEC-046-okf-interchange#CON-4604]] | [[SPEC-046-okf-interchange#ADR-4606]] | — |
| [[SPEC-046-okf-interchange#REQ-4605]] | [[SPEC-046-okf-interchange#TEST-4605]] | [[SPEC-046-okf-interchange#CON-4604]] | — | [[SPEC-046-okf-interchange#Threat Model D]] |
| [[SPEC-046-okf-interchange#REQ-4606]] | [[SPEC-046-okf-interchange#TEST-4606]] | [[SPEC-046-okf-interchange#CON-4604]], [[SPEC-046-okf-interchange#CON-4606]] | [[SPEC-046-okf-interchange#ADR-4604]] | [[SPEC-046-okf-interchange#Threat Model D]] |
| [[SPEC-046-okf-interchange#REQ-4607]] | [[SPEC-046-okf-interchange#TEST-4607]] | [[SPEC-046-okf-interchange#CON-4601]] | [[SPEC-046-okf-interchange#ADR-4605]] | — |
| [[SPEC-046-okf-interchange#REQ-4608]] | [[SPEC-046-okf-interchange#TEST-4608]] | [[SPEC-046-okf-interchange#CON-4605]] | — | — |
| [[SPEC-046-okf-interchange#REQ-4609]] | [[SPEC-046-okf-interchange#TEST-4609]] | [[SPEC-046-okf-interchange#CON-4605]] | — | — |
| [[SPEC-046-okf-interchange#REQ-4610]] | [[SPEC-046-okf-interchange#TEST-4610]] | [[SPEC-046-okf-interchange#CON-4601]] | — | — |
| [[SPEC-046-okf-interchange#REQ-4611]] | [[SPEC-046-okf-interchange#TEST-4611]] | [[SPEC-046-okf-interchange#CON-4606]] | [[SPEC-046-okf-interchange#ADR-4601]] | [[SPEC-046-okf-interchange#Threat Model B]] |
| [[SPEC-046-okf-interchange#REQ-4612]] | [[SPEC-046-okf-interchange#TEST-4612]] | [[SPEC-046-okf-interchange#CON-4602]] | [[SPEC-046-okf-interchange#ADR-4603]] | [[SPEC-046-okf-interchange#Threat Model A]] |
| [[SPEC-046-okf-interchange#REQ-4613]] | [[SPEC-046-okf-interchange#TEST-4613]] | [[SPEC-046-okf-interchange#CON-4601]], [[SPEC-046-okf-interchange#CON-4606]] | — | [[SPEC-046-okf-interchange#Threat Model B]] |
| [[SPEC-046-okf-interchange#REQ-4614]] | [[SPEC-046-okf-interchange#TEST-4614]] | [[SPEC-046-okf-interchange#CON-4606]] | [[SPEC-046-okf-interchange#ADR-4601]] | — |
| [[SPEC-046-okf-interchange#REQ-4615]] | [[SPEC-046-okf-interchange#TEST-4615]] | [[SPEC-046-okf-interchange#CON-4606]] | [[SPEC-046-okf-interchange#ADR-4607]] | [[SPEC-046-okf-interchange#Threat Model B]] |
| [[SPEC-046-okf-interchange#REQ-4616]] | [[SPEC-046-okf-interchange#TEST-4616]] | — | [[SPEC-046-okf-interchange#ADR-4603]] | — |
| [[SPEC-046-okf-interchange#REQ-4617]] | [[SPEC-046-okf-interchange#TEST-4617]] | [[SPEC-046-okf-interchange#CON-4606]] | [[SPEC-046-okf-interchange#ADR-4604]] | [[SPEC-046-okf-interchange#Threat Model E]] |
| [[SPEC-046-okf-interchange#REQ-4618]] | [[SPEC-046-okf-interchange#TEST-4618]] | [[SPEC-046-okf-interchange#CON-4606]] | — | [[SPEC-046-okf-interchange#Threat Model F]] |
| [[SPEC-046-okf-interchange#REQ-4619]] | [[SPEC-046-okf-interchange#TEST-4619]] | [[SPEC-046-okf-interchange#CON-4606]] | — | [[SPEC-046-okf-interchange#Threat Model G]] |
| [[SPEC-046-okf-interchange#REQ-4620]] | [[SPEC-046-okf-interchange#TEST-4620]] | [[SPEC-046-okf-interchange#CON-4606]] | — | — |
| [[SPEC-046-okf-interchange#REQ-4621]] | [[SPEC-046-okf-interchange#TEST-4621]] | [[SPEC-046-okf-interchange#CON-4603]] | — | [[SPEC-046-okf-interchange#Threat Model F]] |
| [[SPEC-046-okf-interchange#REQ-4622]] | [[SPEC-046-okf-interchange#TEST-4622]] | [[SPEC-046-okf-interchange#CON-4604]] | — | — |

NFRs: [[SPEC-046-okf-interchange#NFR-4601]] ([[SPEC-046-okf-interchange#TEST-4601]], [[SPEC-046-okf-interchange#OBS-4601]]);
[[SPEC-046-okf-interchange#NFR-4602]] ([[SPEC-046-okf-interchange#TEST-4614]], [[SPEC-046-okf-interchange#TEST-4606]], [[SPEC-046-okf-interchange#TEST-4617]]);
[[SPEC-046-okf-interchange#NFR-4603]] ([[SPEC-046-okf-interchange#TEST-4621]], [[SPEC-046-okf-interchange#OBS-4602]]);
[[SPEC-046-okf-interchange#NFR-4604]] ([[SPEC-046-okf-interchange#TEST-4623]], [[SPEC-046-okf-interchange#OBS-4603]]).

Blocked (MUST NOT pass Phase 2 until the question closes):
[[SPEC-046-okf-interchange#ADR-4603]]/[[SPEC-046-okf-interchange#REQ-4616]] on
[[SPEC-046-okf-interchange#Q2]]; [[SPEC-046-okf-interchange#REQ-4611]] (non-empty
target) on [[SPEC-046-okf-interchange#Q4]];
[[SPEC-046-okf-interchange#REQ-4605]] (embed) on [[SPEC-046-okf-interchange#Q3]].
([[SPEC-046-okf-interchange#Q7]] governs a *future* capability, not a v1 blocker —
REQ-4617 is settled as inert for v1.)

---

## 13. Open Questions Surfaced by This Strawman

> Resolve or explicitly defer (with rationale + owner) before leaving strawman.
> Do not proceed to plan/code on a `[Blocked]` surface until resolved
> ([[PROTO-001]] §Error Response).

### Q1 (RESOLVED): OKF `index.md` frontmatter
v0.1.0 wrongly claimed OKF was self-contradictory. OKF §6 ("Index files contain
no frontmatter") + §11 ("`okf_version` … in a bundle-root `index.md` frontmatter
block (the only place frontmatter is permitted in an `index.md`)") are
consistent. `okf_version` is placed in root-index frontmatter unconditionally
([[SPEC-046-okf-interchange#REQ-4610]]). Retained for traceability.

### Q2: The OKF link-dialect marker — BLOCKS [[SPEC-046-okf-interchange#ADR-4603]], [[SPEC-046-okf-interchange#REQ-4616]]
What records that a vault speaks OKF link dialect, and does an imported vault keep
resolving path links on later builds? **Recommendation:** an explicit
`.zetl/config.toml [okf] link_dialect = "okf"` marker (a named, visible
parameter), so import-provenance never silently retains OKF semantics. *Owner: TBD.*

### Q3: Embed (`![[…]]`) export semantics — BLOCKS [[SPEC-046-okf-interchange#REQ-4605]] embed clause
Transcluded content (loses the edge), a path link (loses transclusion), or both?
OKF has no transclusion concept. *Owner: TBD.*

### Q4: Merge policy on import into a non-empty vault — BLOCKS [[SPEC-046-okf-interchange#REQ-4611]]
Overwrite, skip-existing, or refuse? Interacts with collision
([[SPEC-046-okf-interchange#Threat Model C]]). *Owner: TBD.*

### Q5: Per-directory `index.md` generation/recognition
Generate/recognise per-directory index files or root-only? Default proposed
root-only. *Owner: TBD.*

### Q6 (RESOLVED): `# Citations` links and `references/` concepts
OKF §8 makes `references/` files first-class concepts;
[[SPEC-046-okf-interchange#REQ-4620]] `type`-checks them and
[[SPEC-046-okf-interchange#REQ-4612]] graphs in-bundle citation links.
`resource` has no zetl equivalent and is preserve-only
([[SPEC-046-okf-interchange#REQ-4622]]).

### Q7: Contribution authenticity vs trust (the safe-reconstruction precondition)
Live reconstruction of typed edges from `x-zetl-edges`
([[SPEC-046-okf-interchange#REQ-4617]]) is unsafe until a contribution carries
**verifiable authorship**. The intended primitive is a **detached per-contribution
signature that lives with the document**, making provenance a property of the
*contribution* (the axis [[SPEC-045-wikilink-predicate-language]] lacks —
`_source_page` records "which page", not "signed by whom"); **trust** then layers
as a separate reasoner policy over verified authorship. This is a **shared
capability** ([[PROTO-001]] Principle 15) — consumed by OKF import, by collab
([[SPEC-041-pluggable-collab-auth]]), and by distributed sync — and belongs in its
own spec (candidate SPEC-047), NOT in the OKF importer. Signing touches a
[[PROTO-001]] §AI-Trust-Boundaries **no-go area (cryptography)** requiring explicit
approval. Until it exists, `x-zetl-edges` stays inert. *Owner: TBD.*

---

## References

- [[knowledge-catalog OKF SPEC]] — Open Knowledge Format v0.1.
- [[PROTO-001]] — USDD Agent Protocol v1.8.0.
- [[SPEC-045-wikilink-predicate-language]] — typed named edges (the richness OKF
  downcasts; the page-authored-facts trust model Threat E re-opens; the provenance
  axis Q7 must extend).
- [[SPEC-044-concept-graph-spo-emergent-vocabulary]] — SPO concept graph.
- [[SPEC-001]] — bi-directional link graph CLI.
- [[SPEC-032]] — three-stage render hooks + zetl-ext AST.
- [[SPEC-017]] — history backend (`log.md` / `timestamp`).
- [[SPEC-026]] — vault scan exclusions.
- [[SPEC-002]] — full-text search (the `type` facet).
- [[SPEC-041-pluggable-collab-auth]] — auth/identity (a signing capability's base).
- UTS #39 — Unicode Security Mechanisms (confusable detection, Threat C).
