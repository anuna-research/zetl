//! SPEC-033 REQ-3311 / TEST-3311 ecosystem-matrix gate.
//!
//! This file is the structural / evidence gate for every plugin row in
//! `tools/ztl-ecosystem-matrix.toml`. The per-ecosystem matrix-seed
//! tasks (pandoc / mdbook / remark) wrote the rows; the `task-eco-matrix`
//! integration binds them together by asserting:
//!
//! 1. Every row carries the REQ-3311-mandated columns (name,
//!    version_range, tier, fixture, repo, maintainer, notes).
//! 2. `tier` is one of `experimental | partial | supported`.
//! 3. `version_range` is a well-formed npm-style semver range (REQ-3314
//!    amendment to CON-3311 — `version_range` column added).
//! 4. The `fixture` path points at an existing directory on disk
//!    containing `input.md`, `expected.html`, and `README.md`.
//! 5. Per the tier-promotion checklist:
//!    - `partial` rows SHALL carry a fixture (enforced in (4)).
//!    - `supported` rows SHALL additionally declare a `[plugin.contract]`
//!      sub-table with `preserves` (non-empty `Vec<String>`) and
//!      `idempotent` (bool).
//! 6. Tier-downgrade gate (TEST-3311): a simulated before/after matrix
//!    pair where a plugin drops tier without a `tier_downgrade_rationale`
//!    field → the `check_tier_downgrade()` helper flags the offence.
//!    Running the helper on the real on-disk matrix (with itself as
//!    `before`) SHALL be a no-op.
//!
//! The registry-↔-matrix section-level lint lives in
//! `tests/ecosystems_registry_integration.rs`. This file is the
//! row-level companion.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

// ── fixture-table loaders ─────────────────────────────────────────────

fn matrix_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/ztl-ecosystem-matrix.toml")
}

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn load_matrix() -> toml::Value {
    let path = matrix_path();
    let body = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read ecosystem matrix at {}: {}",
            path.display(),
            e
        )
    });
    toml::from_str(&body).unwrap_or_else(|e| {
        panic!(
            "ecosystem matrix at {} is not valid TOML: {}",
            path.display(),
            e
        )
    })
}

// ── canonical row shape ───────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct PluginRow {
    ecosystem: String,
    name: String,
    version_range: String,
    tier: String,
    fixture: String,
    repo: String,
    maintainer: String,
    notes: String,
    contract: Option<PluginContract>,
    tier_downgrade_rationale: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PluginContract {
    preserves: Vec<String>,
    idempotent: bool,
}

fn get_str<'a>(t: &'a toml::value::Table, key: &str) -> Option<&'a str> {
    t.get(key).and_then(|v| v.as_str())
}

fn get_bool(t: &toml::value::Table, key: &str) -> Option<bool> {
    t.get(key).and_then(|v| v.as_bool())
}

fn get_string_array(t: &toml::value::Table, key: &str) -> Option<Vec<String>> {
    t.get(key).and_then(|v| v.as_array()).map(|a| {
        a.iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect()
    })
}

/// Walk the matrix TOML and collect every `[[ecosystem.<id>.plugins]]`
/// entry into a typed [`PluginRow`]. Missing required columns panic
/// with a row-and-field-qualified message — the structural gate.
fn collect_rows(matrix: &toml::Value) -> Vec<PluginRow> {
    let eco_table = match matrix.get("ecosystem") {
        Some(toml::Value::Table(t)) => t,
        _ => panic!("ecosystem matrix missing top-level [ecosystem] table"),
    };
    let mut out: Vec<PluginRow> = Vec::new();
    for (eco_id, eco_val) in eco_table {
        let eco_t = match eco_val {
            toml::Value::Table(t) => t,
            _ => panic!("ecosystem.{eco_id} is not a table"),
        };
        let Some(plugins) = eco_t.get("plugins").and_then(|v| v.as_array()) else {
            // Ecosystem section with zero plugins is allowed for the
            // registry-lint to pass, but task-eco-matrix seeds every
            // v1 ecosystem — flag an empty section so we catch removals.
            panic!(
                "ecosystem.{eco_id} section has no `plugins` array — every v1 ecosystem \
                 carries at least one seed row per the matrix-seed tasks",
            );
        };
        for (idx, plug_val) in plugins.iter().enumerate() {
            let t = match plug_val {
                toml::Value::Table(t) => t,
                _ => panic!("ecosystem.{eco_id}.plugins[{idx}] is not a table"),
            };

            // Required string columns.
            let ctx = format!("ecosystem.{eco_id}.plugins[{idx}]");
            let name = get_str(t, "name")
                .unwrap_or_else(|| panic!("{ctx}: missing required field `name`"));
            let version_range = get_str(t, "version_range").unwrap_or_else(|| {
                panic!("{ctx}: missing required field `version_range` (REQ-3314 amendment)")
            });
            let tier = get_str(t, "tier")
                .unwrap_or_else(|| panic!("{ctx}: missing required field `tier`"));
            let fixture = get_str(t, "fixture")
                .unwrap_or_else(|| panic!("{ctx}: missing required field `fixture`"));
            let repo = get_str(t, "repo")
                .unwrap_or_else(|| panic!("{ctx}: missing required field `repo`"));
            let maintainer = get_str(t, "maintainer")
                .unwrap_or_else(|| panic!("{ctx}: missing required field `maintainer`"));
            let notes = get_str(t, "notes")
                .unwrap_or_else(|| panic!("{ctx}: missing required field `notes`"));

            // Optional contract sub-table.
            let contract = t
                .get("contract")
                .and_then(|v| v.as_table())
                .map(|ct| PluginContract {
                    preserves: get_string_array(ct, "preserves").unwrap_or_else(|| {
                        panic!(
                            "{ctx}.contract: missing required field `preserves` \
                             (Vec<String> per SPEC-033 REQ-3311 / SPEC-032 REQ-3224)"
                        )
                    }),
                    idempotent: get_bool(ct, "idempotent").unwrap_or_else(|| {
                        panic!("{ctx}.contract: missing required field `idempotent` (bool)")
                    }),
                });

            let tier_downgrade_rationale = get_str(t, "tier_downgrade_rationale").map(String::from);

            out.push(PluginRow {
                ecosystem: eco_id.clone(),
                name: name.to_string(),
                version_range: version_range.to_string(),
                tier: tier.to_string(),
                fixture: fixture.to_string(),
                repo: repo.to_string(),
                maintainer: maintainer.to_string(),
                notes: notes.to_string(),
                contract,
                tier_downgrade_rationale,
            });
        }
    }
    out
}

// ── column validators ─────────────────────────────────────────────────

/// `tier` accepts exactly three values. Anything else is a data error
/// — the promotion checklist's ladder is the single source of truth.
fn validate_tier(row: &PluginRow) {
    assert!(
        matches!(row.tier.as_str(), "experimental" | "partial" | "supported"),
        "{}/{}: tier must be one of experimental|partial|supported, got {:?}",
        row.ecosystem,
        row.name,
        row.tier,
    );
}

/// REQ-3314 amendment: `version_range` is an npm-style semver range.
/// We accept one or two space-separated predicates of the form
/// `<OP><NUMERIC VERSION>` where OP is one of `>=|<=|>|<|=|^|~` and
/// version is dot-separated numeric. No pre-release / build metadata
/// support in v1 — keeps the matrix text conservative.
fn validate_version_range(row: &PluginRow) {
    let s = row.version_range.trim();
    assert!(
        !s.is_empty(),
        "{}/{}: version_range must be non-empty",
        row.ecosystem,
        row.name,
    );
    for part in s.split_whitespace() {
        let (op, rest) = split_op(part).unwrap_or_else(|| {
            panic!(
                "{}/{}: version_range predicate {:?} does not start with a valid \
                 operator (one of >=|<=|>|<|=|^|~)",
                row.ecosystem, row.name, part,
            )
        });
        // Reject an empty operator when one was required.
        assert!(
            !op.is_empty() || starts_with_digit(part),
            "{}/{}: version_range predicate {:?} is missing its operator",
            row.ecosystem,
            row.name,
            part,
        );
        assert!(
            is_numeric_version(rest),
            "{}/{}: version_range predicate {:?} tail {:?} is not a numeric \
             dotted version",
            row.ecosystem,
            row.name,
            part,
            rest,
        );
    }
}

fn split_op(s: &str) -> Option<(&str, &str)> {
    // Order matters — two-char operators first.
    for op in [">=", "<=", "==", ">", "<", "=", "^", "~"] {
        if let Some(rest) = s.strip_prefix(op) {
            return Some((op, rest));
        }
    }
    // Bare numeric version (implicit `=`).
    if starts_with_digit(s) {
        return Some(("", s));
    }
    None
}

fn starts_with_digit(s: &str) -> bool {
    s.chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(false)
}

fn is_numeric_version(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let parts: Vec<&str> = s.split('.').collect();
    if parts.is_empty() || parts.len() > 4 {
        return false;
    }
    parts
        .iter()
        .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// The `fixture` column points at a directory, relative to the crate
/// root, that SHALL exist and carry the canary trio.
fn validate_fixture_path(row: &PluginRow) {
    let fx = crate_root().join(&row.fixture);
    assert!(
        fx.is_dir(),
        "{}/{}: fixture path {:?} does not exist as a directory on disk",
        row.ecosystem,
        row.name,
        fx.display(),
    );
    for required in ["input.md", "expected.html", "README.md"] {
        let p = fx.join(required);
        assert!(
            p.is_file(),
            "{}/{}: fixture directory {:?} missing required file {:?}",
            row.ecosystem,
            row.name,
            fx.display(),
            required,
        );
    }
}

/// Fixture dir convention: `tests/ecosystem-fixtures/<eco>/<plugin>/`.
/// Not normative in SPEC-033, but the seed tasks all follow it, so we
/// pin it here to catch typos / placement drift early.
fn validate_fixture_convention(row: &PluginRow) {
    let expected = format!("tests/ecosystem-fixtures/{}/{}/", row.ecosystem, row.name);
    assert_eq!(
        row.fixture, expected,
        "{}/{}: fixture path {:?} does not follow the tests/ecosystem-fixtures/\
         <eco>/<plugin>/ convention (expected {:?})",
        row.ecosystem, row.name, row.fixture, expected,
    );
}

/// Promotion checklist: `tier = "supported"` requires a contract
/// sub-table with a non-empty `preserves` list and an `idempotent`
/// bool. `tier = "partial"` only requires the fixture (validated
/// separately). `tier = "experimental"` has no contract requirement.
fn validate_tier_requirements(row: &PluginRow) {
    match row.tier.as_str() {
        "supported" => {
            let c = row.contract.as_ref().unwrap_or_else(|| {
                panic!(
                    "{}/{}: tier=supported requires a [plugin.contract] sub-table \
                     (preserves + idempotent) per REQ-3311 promotion checklist",
                    row.ecosystem, row.name,
                )
            });
            assert!(
                !c.preserves.is_empty(),
                "{}/{}: tier=supported requires contract.preserves to list at \
                 least one node type that must survive",
                row.ecosystem,
                row.name,
            );
        }
        "partial" => { /* fixture already validated */ }
        "experimental" => { /* evidence-light by design */ }
        _ => unreachable!("validate_tier covered this"),
    }
}

// ── test surface ──────────────────────────────────────────────────────

#[test]
fn matrix_rows_have_required_columns() {
    let matrix = load_matrix();
    let rows = collect_rows(&matrix);
    assert!(
        !rows.is_empty(),
        "ecosystem matrix has zero plugin rows; the v1 seeds (pandoc / mdbook / \
         remark) should land at least ten rows between them"
    );
    // `collect_rows` panics on missing columns; reaching here means all
    // rows are structurally complete. Surface a summary line for
    // running with `-- --nocapture`.
    let mut by_eco: BTreeMap<String, usize> = BTreeMap::new();
    for r in &rows {
        *by_eco.entry(r.ecosystem.clone()).or_default() += 1;
    }
    for (eco, n) in &by_eco {
        println!("matrix: {n} rows for ecosystem {eco}");
    }
}

#[test]
fn matrix_rows_have_valid_tier_values() {
    for row in collect_rows(&load_matrix()) {
        validate_tier(&row);
    }
}

#[test]
fn matrix_rows_have_valid_version_ranges() {
    for row in collect_rows(&load_matrix()) {
        validate_version_range(&row);
    }
}

#[test]
fn matrix_fixture_paths_exist_and_follow_convention() {
    for row in collect_rows(&load_matrix()) {
        validate_fixture_convention(&row);
        validate_fixture_path(&row);
    }
}

#[test]
fn matrix_tier_contract_coupling() {
    // REQ-3311 promotion checklist gate: contract evidence strength
    // tracks tier.
    for row in collect_rows(&load_matrix()) {
        validate_tier_requirements(&row);
    }
}

#[test]
fn matrix_names_are_unique_within_ecosystem() {
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    for row in collect_rows(&load_matrix()) {
        let key = (row.ecosystem.clone(), row.name.clone());
        assert!(
            seen.insert(key.clone()),
            "duplicate plugin row {}/{} in matrix — plugin names must be \
             unique within an ecosystem section",
            key.0,
            key.1,
        );
    }
}

// ── TEST-3311 tier-downgrade simulation ───────────────────────────────

#[derive(Debug, PartialEq, Eq)]
struct TierDowngrade {
    ecosystem: String,
    plugin: String,
    from_tier: String,
    to_tier: String,
}

fn tier_rank(t: &str) -> u8 {
    match t {
        "experimental" => 0,
        "partial" => 1,
        "supported" => 2,
        _ => u8::MAX,
    }
}

/// Given two matrices (before, after), return every plugin row that
/// strictly decreased in tier rank *without* carrying a non-empty
/// `tier_downgrade_rationale` field in the `after` row. CI gate: any
/// entry in the returned vec SHALL fail the merge.
///
/// Contract-field changes (e.g. dropping a `preserves` entry from a
/// supported row's contract) are also treated as tier downgrades per
/// REQ-3311 — we model that as "contract got weaker" and reuse the
/// same rationale field.
fn check_tier_downgrade(before: &[PluginRow], after: &[PluginRow]) -> Vec<TierDowngrade> {
    let mut out: Vec<TierDowngrade> = Vec::new();
    let before_index: BTreeMap<(&str, &str), &PluginRow> = before
        .iter()
        .map(|r| ((r.ecosystem.as_str(), r.name.as_str()), r))
        .collect();
    for after_row in after {
        let Some(before_row) =
            before_index.get(&(after_row.ecosystem.as_str(), after_row.name.as_str()))
        else {
            continue;
        };
        let decreased_tier = tier_rank(&after_row.tier) < tier_rank(&before_row.tier);
        let contract_weakened = match (&before_row.contract, &after_row.contract) {
            (Some(b), Some(a)) => {
                let a_set: BTreeSet<&String> = a.preserves.iter().collect();
                b.preserves.iter().any(|name| !a_set.contains(name))
                    || (b.idempotent && !a.idempotent)
            }
            (Some(_), None) => true,
            _ => false,
        };
        let downgraded = decreased_tier || contract_weakened;
        let has_rationale = after_row
            .tier_downgrade_rationale
            .as_ref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        if downgraded && !has_rationale {
            out.push(TierDowngrade {
                ecosystem: after_row.ecosystem.clone(),
                plugin: after_row.name.clone(),
                from_tier: before_row.tier.clone(),
                to_tier: after_row.tier.clone(),
            });
        }
    }
    out
}

#[test]
fn test_3311_tier_downgrade_without_rationale_flags_offender() {
    // Simulated PR scenario: pandoc-crossref drops supported → partial
    // with no rationale. Gate flags it.
    let before = vec![PluginRow {
        ecosystem: "pandoc".into(),
        name: "pandoc-crossref".into(),
        version_range: ">=0.3 <0.4".into(),
        tier: "supported".into(),
        fixture: "tests/ecosystem-fixtures/pandoc/pandoc-crossref/".into(),
        repo: "https://example".into(),
        maintainer: "x".into(),
        notes: "n".into(),
        contract: Some(PluginContract {
            preserves: vec!["Wikilink".into()],
            idempotent: true,
        }),
        tier_downgrade_rationale: None,
    }];
    let after = vec![PluginRow {
        tier: "partial".into(),
        contract: None,
        tier_downgrade_rationale: None,
        ..before[0].clone()
    }];
    let offenders = check_tier_downgrade(&before, &after);
    assert_eq!(
        offenders.len(),
        1,
        "unrationalised tier downgrade must be flagged; got {offenders:?}",
    );
    assert_eq!(offenders[0].from_tier, "supported");
    assert_eq!(offenders[0].to_tier, "partial");
}

#[test]
fn test_3311_tier_downgrade_with_rationale_is_allowed() {
    // Same scenario as above but the PR author populated
    // `tier_downgrade_rationale`. Gate returns empty.
    let before = vec![PluginRow {
        ecosystem: "pandoc".into(),
        name: "pandoc-crossref".into(),
        version_range: ">=0.3 <0.4".into(),
        tier: "supported".into(),
        fixture: "tests/ecosystem-fixtures/pandoc/pandoc-crossref/".into(),
        repo: "https://example".into(),
        maintainer: "x".into(),
        notes: "n".into(),
        contract: Some(PluginContract {
            preserves: vec!["Wikilink".into()],
            idempotent: true,
        }),
        tier_downgrade_rationale: None,
    }];
    let after = vec![PluginRow {
        tier: "partial".into(),
        contract: None,
        tier_downgrade_rationale: Some(
            "Upstream 0.4 pandoc-types bump broke translator round-trip; downgrade to \
             partial until CON-3221 marker convention is re-aligned."
                .into(),
        ),
        ..before[0].clone()
    }];
    assert!(
        check_tier_downgrade(&before, &after).is_empty(),
        "downgrade with rationale SHALL NOT be flagged"
    );
}

#[test]
fn test_3311_contract_preserves_removal_counts_as_downgrade() {
    // REQ-3311: "Contract-field changes (e.g. dropping `preserves =
    // ['Wikilink']`) are treated as tier downgrades and gated
    // identically." Simulate a supported-tier plugin whose contract
    // silently drops Wikilink preservation — gate fires.
    let before = vec![PluginRow {
        ecosystem: "pandoc".into(),
        name: "pandoc-crossref".into(),
        version_range: ">=0.3 <0.4".into(),
        tier: "supported".into(),
        fixture: "tests/ecosystem-fixtures/pandoc/pandoc-crossref/".into(),
        repo: "https://example".into(),
        maintainer: "x".into(),
        notes: "n".into(),
        contract: Some(PluginContract {
            preserves: vec!["Wikilink".into(), "Embed".into()],
            idempotent: true,
        }),
        tier_downgrade_rationale: None,
    }];
    let after = vec![PluginRow {
        contract: Some(PluginContract {
            preserves: vec!["Embed".into()],
            idempotent: true,
        }),
        ..before[0].clone()
    }];
    let offenders = check_tier_downgrade(&before, &after);
    assert_eq!(offenders.len(), 1, "preserves-shrink must be flagged");
}

#[test]
fn test_3311_tier_promotion_is_not_flagged() {
    // An experimental → partial promotion with a new fixture is not a
    // downgrade. Gate stays silent.
    let before = vec![PluginRow {
        ecosystem: "pandoc".into(),
        name: "pandoc-crossref".into(),
        version_range: ">=0.3 <0.4".into(),
        tier: "experimental".into(),
        fixture: "tests/ecosystem-fixtures/pandoc/pandoc-crossref/".into(),
        repo: "https://example".into(),
        maintainer: "x".into(),
        notes: "n".into(),
        contract: None,
        tier_downgrade_rationale: None,
    }];
    let after = vec![PluginRow {
        tier: "partial".into(),
        ..before[0].clone()
    }];
    assert!(
        check_tier_downgrade(&before, &after).is_empty(),
        "tier promotion SHALL NOT be flagged",
    );
}

#[test]
fn test_3311_self_diff_is_noop_on_real_matrix() {
    // Meta-gate: running the downgrade check with the real matrix as
    // both `before` and `after` SHALL return an empty vec. Guards
    // against false positives from stray rationale-field drift.
    let rows = collect_rows(&load_matrix());
    let offenders = check_tier_downgrade(&rows, &rows);
    assert!(
        offenders.is_empty(),
        "self-diff of the on-disk matrix must be empty; got {offenders:?}",
    );
}

#[test]
fn test_3311_v1_seed_tiers_are_all_experimental() {
    // Pin the v1 seed state: the matrix-seed tasks deliberately landed
    // every row at tier = "experimental" with the promotion path
    // deferred to this task + downstream adapter work. A future PR
    // lifting a row to `partial` / `supported` will intentionally
    // tip this test over — that is the moment to confirm the
    // accompanying evidence (fixture wired into a golden-HTML runner
    // for `partial`; `[plugin.contract]` declared for `supported`)
    // is in place. This gate makes the promotion deliberate.
    for row in collect_rows(&load_matrix()) {
        assert_eq!(
            row.tier, "experimental",
            "{}/{}: v1 seed rows land at tier=experimental; promotion past \
             that SHALL update this pin and the accompanying promotion \
             checklist evidence",
            row.ecosystem, row.name,
        );
    }
}

// ── documentation gate ────────────────────────────────────────────────

#[test]
fn tier_promotion_checklist_doc_exists() {
    // REQ-3311: "The checklist lives in
    // docs/ecosystems/matrix-contribution.md and is kept in sync with
    // this REQ." Enforce that the doc exists and references the three
    // tier-transition headings.
    let doc_path = crate_root().join("docs/ecosystems/matrix-contribution.md");
    let body = std::fs::read_to_string(&doc_path).unwrap_or_else(|e| {
        panic!(
            "REQ-3311 requires docs/ecosystems/matrix-contribution.md to exist \
             (read error: {e})"
        )
    });
    for heading in [
        "experimental → partial",
        "partial → supported",
        "supported → supported, maintainer-adopted",
    ] {
        assert!(
            body.contains(heading),
            "docs/ecosystems/matrix-contribution.md is missing the {heading:?} \
             promotion checklist section required by REQ-3311",
        );
    }
}

// ── helper for fixture-sanity on the README promotion-notes section ──

fn fixture_readme_path(row: &PluginRow) -> PathBuf {
    crate_root().join(&row.fixture).join("README.md")
}

#[test]
fn fixture_readmes_reference_promotion_checklist() {
    // Each fixture README explains what the fixture proves and which
    // promotion box it's blocked on. The seed tasks standardised the
    // "Promotion notes" section header; this gate catches future
    // fixtures that forget to include the hand-off.
    for row in collect_rows(&load_matrix()) {
        let path = fixture_readme_path(&row);
        let body = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "{}/{}: fixture README at {:?} unreadable ({})",
                row.ecosystem,
                row.name,
                path.display(),
                e,
            )
        });
        assert!(
            body.contains("Promotion notes"),
            "{}/{}: fixture README {:?} is missing the 'Promotion notes' \
             section required by the matrix-seed convention",
            row.ecosystem,
            row.name,
            path.display(),
        );
    }
}
