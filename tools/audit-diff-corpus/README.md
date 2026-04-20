# `zetl cap audit-diff` corpus

SPEC-034 REQ-3424 + ADR-3410 (BUG-016 resolution). This directory is
the versioned regression corpus for the malicious-author PR gate.

Every fixture under `fixtures/` is a self-contained two-tree sample:

```
fixtures/<NNN-slug>/
  README.md         narrative: what the sample models + which
                    detector(s) should fire
  baseline/         vault state at OLD_REF (optional; omit for
                    "newly-introduced" samples)
    page.md
  new/              vault state at NEW_REF (required)
    page.md
  expected.txt      one finding-kind tag per line; every tag must
                    appear at least once in the scan output
```

Finding-kind tags (keep in sync with
`src/cap/audit_diff.rs::FindingKind::tag`):

- `unseen-domain`
- `raw-html`
- `sanitiser-stripped`
- `dangerous-scheme`
- `dynamic-uri`

## CI gate — `audit-corpus`

`.woodpecker/ci.yaml` runs `make audit-corpus` on every push and
pull request. The step walks every fixture via
`zetl cap audit-diff --corpus-root tools/audit-diff-corpus` and fails
the build if any fixture's expected markers are missed. Do not comment
out a fixture to pass CI — add the missing detection to
`src/cap/audit_diff.rs` instead.

## Update cadence

Per SPEC-034 REQ-3424:

- Monthly review (see the calendar trigger in ADR-3410).
- Immediate update on any reported evasion — file the corpus entry
  *before* the patch that closes the detection gap.

## Adding a new fixture

1. Copy an existing fixture as the template.
2. Put the malicious content in `new/page.md`; put any reachable
   baseline state in `baseline/page.md` (an empty `baseline/` is the
   "brand-new page" case).
3. List every finding-kind tag the scanner should emit in
   `expected.txt`.
4. `cargo test --test cap_audit_diff_corpus -- --nocapture` and make
   sure the new fixture passes.
5. Commit with a commit subject of the form
   `corpus: add <slug> (<source>)` — e.g. `corpus: add 014-svg-onload
   (OWASP XSS cheatsheet)`.
