# Contributing to the zetl ecosystem matrix

This doc is the authoritative tier-promotion checklist referenced by
SPEC-033 §REQ-3311. Matrix entries live in
[`tools/zetl-ecosystem-matrix.toml`](../../tools/zetl-ecosystem-matrix.toml);
their canary fixtures live under
[`tests/ecosystem-fixtures/<ecosystem>/<plugin>/`](../../tests/ecosystem-fixtures/).

The matrix is the surface the user consults when they ask
*"is zetl going to silently eat my Wikilinks if I plug in
pandoc-crossref?"* — tier is the answer. A row at `supported` says
*zetl has run the fixture end-to-end and the declared behavioural
contract holds*. A row at `experimental` says *we've documented the
plugin and wired a fixture, but the contract is inferred, not
verified*. Moving a row up the ladder is how you make that answer
stronger.

## Tiers at a glance

| Tier           | What the user can rely on                                                                                              |
|----------------|-----------------------------------------------------------------------------------------------------------------------|
| `experimental` | Plugin is declared, maintained, fixture exists. Behavioural contract inferred from CON-3221 defaults, not verified.   |
| `partial`      | Fixture is wired into a green golden-HTML runner. End-to-end render is not broken.                                     |
| `supported`    | `[plugin.contract]` declared; `preserves` list enforced; if `idempotent = true`, double-run evidence is in CI.        |
| `supported, maintainer-adopted` | Informational overlay on `supported`: a named maintainer commits to keeping the row green.              |

## Tier-promotion checklists

The lists below mirror SPEC-033 §REQ-3311 verbatim. A PR claiming a
higher tier without satisfying every item on the target-tier list is
rejected — reviewers SHOULD copy the checklist into the PR description
and tick each box against the evidence they link to.

### experimental → partial

- [ ] Matrix entry exists with `version_range`, `tier = "experimental"`,
      maintainer contact, and upstream repo URL.
- [ ] At least one working fixture in
      `tests/ecosystem-fixtures/<ecosystem>/<plugin>/`.
- [ ] Golden-HTML fixture asserts end-to-end render is not broken for
      a simple input.
- [ ] Known limitations documented in the matrix entry's `notes` field.

### partial → supported

- [ ] `[plugin.contract]` sub-table declared with at minimum `preserves`
      listing the node types that must survive.
- [ ] `contract.idempotent` declared; if `true`, verified by
      [TEST-3224-idempotent](../../specs/SPEC-032.md)'s CI double-run on
      the fixture.
- [ ] `version_range` reflects a semver-compatible span that has
      passed CI green for the current release.
- [ ] Fixture coverage expanded: at minimum one fixture exercising
      each major feature of the plugin documented in its own README.
- [ ] No open issues in the matrix entry's `notes` field marked as
      blockers.

### supported → supported, maintainer-adopted

This is an informational overlay — the tier string in the matrix
stays `supported`; the `maintained_by` field signals the adoption.

- [ ] A person or org listed in `maintained_by` who commits to
      responding to breakage within one release cycle.
- [ ] Automated refresh-of-fixtures hook scheduled to run on each
      upstream release of the plugin.

## Tier-downgrade policy

A PR that moves a matrix row *down* the ladder SHALL carry a
`tier_downgrade_rationale = "<short reason>"` field on the downgraded
row. The `TEST-3311` gate in
[`tests/ecosystem_matrix_integration.rs`](../../tests/ecosystem_matrix_integration.rs)
runs `check_tier_downgrade(before, after)` over the old and new matrix
states and fails the merge if any downgrade lacks a rationale.

Contract-field changes count as downgrades even when the tier string
stays the same. Specifically:

- Dropping a node type from `contract.preserves`.
- Flipping `contract.idempotent` from `true` to `false`.
- Removing the `[plugin.contract]` sub-table from a `supported` row.

Each of these SHALL carry its own `tier_downgrade_rationale` line.

## Worked-example PR description

Copy this template into your PR when you move a row up or down. The
checklist items reference the target tier's section above.

```markdown
## Matrix change

**Plugin:** `pandoc-crossref`
**From:** `tier = "experimental"`
**To:** `tier = "partial"`

### Evidence (promotion: experimental → partial)

- [x] Matrix entry already carries `version_range = ">=0.3.14 <0.4"`,
      maintainer contact, and upstream repo URL (unchanged).
- [x] Fixture at `tests/ecosystem-fixtures/pandoc/pandoc-crossref/`
      expanded to exercise numbered figures AND equation references.
- [x] Golden-HTML runner wired in tests/ecosystem_fixture_golden_integration.rs
      — PR adds the runner + updates the fixture's `expected.html`.
- [x] Known limitations section of `notes` updated to reflect the
      new round-trip gate.
```

## File layout contract

The structural gate in
`tests/ecosystem_matrix_integration.rs` pins the following:

- Every ecosystem registered in `src/ecosystems/registry.rs` SHALL have
  exactly one `[ecosystem.<id>]` section in the matrix file
  (enforced by the sister test
  `tests/ecosystems_registry_integration.rs`).
- Every plugin row SHALL carry the columns `name`, `version_range`,
  `tier`, `fixture`, `repo`, `maintainer`, `notes`.
- The `fixture` path SHALL point at an existing directory under
  `tests/ecosystem-fixtures/<ecosystem>/<plugin>/` containing at
  minimum `input.md`, `expected.html`, and `README.md`.
- `tier = "supported"` rows SHALL additionally declare a
  `[plugin.contract]` sub-table with a non-empty `preserves` list and
  an `idempotent: bool`.
- Each fixture README SHALL include a `Promotion notes` section
  explaining which items on the next-tier checklist the fixture
  currently clears.

## See also

- SPEC-033 §REQ-3311 — normative matrix schema + tier-promotion
  checklist.
- SPEC-033 §REQ-3314 — plugin-version drift detection (`version_range`
  column).
- SPEC-032 §REQ-3224 — behavioural contract shape (`preserves`,
  `idempotent`, `pure`, `may_restructure`, `expansion_bound`).
- `docs/ecosystems/{pandoc,mdbook,remark}.md` — per-ecosystem user
  guides.
