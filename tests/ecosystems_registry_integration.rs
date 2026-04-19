//! Integration tests for the ecosystem registry (SPEC-033 REQ-3301 /
//! CON-3301 / TEST-3301) and the matrix-lint gate guarding the
//! task-eco-registry acceptance clause "registering a new ecosystem
//! requires both code + matrix entry (lint enforced)".
//!
//! The unit tests beside the registry in `src/ecosystems/registry.rs`
//! cover pure-data invariants (unique ids, non-empty slices, canonical
//! ordering); this file is the public-facing gate that exercises the
//! registry through `zetl::ecosystems` and cross-checks the on-disk
//! `tools/zetl-ecosystem-matrix.toml` payload.

use std::collections::HashSet;
use std::path::PathBuf;

use zetl::ecosystems::{self, Ecosystem};

fn matrix_path() -> PathBuf {
    // CARGO_MANIFEST_DIR points at the crate root — the matrix file
    // lives at `tools/zetl-ecosystem-matrix.toml` relative to that.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/zetl-ecosystem-matrix.toml")
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

/// Extract the set of ecosystem ids present as `[ecosystem.<id>]`
/// sections in the matrix file.
fn matrix_section_ids(matrix: &toml::Value) -> HashSet<String> {
    let table = match matrix.get("ecosystem") {
        Some(toml::Value::Table(t)) => t,
        _ => panic!("ecosystem matrix missing top-level [ecosystem] table"),
    };
    table.keys().cloned().collect()
}

// ── TEST-3301: registry integrity ─────────────────────────────────────

#[test]
fn test_3301_registry_is_non_empty() {
    assert!(
        !ecosystems::all().is_empty(),
        "registry must contain at least one ecosystem in v1"
    );
}

#[test]
fn test_3301_every_variant_has_a_row() {
    // Exhaustive match — if a new variant lands in `Ecosystem` without
    // an `ECOSYSTEMS` row, this fails the build because `entry()`
    // panics.
    for e in [Ecosystem::Pandoc, Ecosystem::Mdbook, Ecosystem::Remark] {
        let entry = e.entry();
        assert_eq!(entry.ecosystem, e);
        assert_eq!(entry.id, e.as_str());
    }
}

#[test]
fn test_3301_by_id_resolves_every_registered_id() {
    for entry in ecosystems::all() {
        let resolved =
            ecosystems::by_id(entry.id).expect("every registry row must be by_id-lookupable");
        assert_eq!(resolved.id, entry.id);
    }
}

#[test]
fn test_3301_ids_are_unique() {
    let mut ids: HashSet<&str> = HashSet::new();
    for entry in ecosystems::all() {
        assert!(
            ids.insert(entry.id),
            "duplicate ecosystem id '{}' in registry",
            entry.id
        );
    }
}

#[test]
fn test_3301_supported_slices_are_non_empty() {
    for entry in ecosystems::all() {
        assert!(
            !entry.supported_stages.is_empty(),
            "ecosystem '{}' has empty supported_stages",
            entry.id
        );
        assert!(
            !entry.supported_ast_types.is_empty(),
            "ecosystem '{}' has empty supported_ast_types",
            entry.id
        );
    }
}

#[test]
fn test_3301_default_stage_is_supported() {
    for entry in ecosystems::all() {
        assert!(
            entry.supported_stages.contains(&entry.default_stage),
            "ecosystem '{}' default_stage {:?} not in supported_stages {:?}",
            entry.id,
            entry.default_stage,
            entry.supported_stages,
        );
    }
}

// ── Matrix-lint gate (task-eco-registry acceptance) ───────────────────

#[test]
fn matrix_lint_every_registered_ecosystem_has_a_matrix_section() {
    // Adding an entry to ECOSYSTEMS without a corresponding
    // [ecosystem.<id>] section in tools/zetl-ecosystem-matrix.toml
    // fails this test — the acceptance clause "registering a new
    // ecosystem requires both code + matrix entry (lint enforced)"
    // (SPEC-033 task-eco-registry) lives here.
    let matrix = load_matrix();
    let matrix_ids = matrix_section_ids(&matrix);
    for entry in ecosystems::all() {
        assert!(
            matrix_ids.contains(entry.id),
            "ecosystem '{}' is registered in code but has no [ecosystem.{}] \
             section in tools/zetl-ecosystem-matrix.toml — add one before merging",
            entry.id,
            entry.id,
        );
    }
}

#[test]
fn matrix_lint_every_matrix_section_is_registered() {
    // The other direction: a [ecosystem.<id>] section for an id that
    // isn't in the registry means either a typo or an ecosystem
    // removed from code without cleaning up the matrix. Both block
    // merge.
    let matrix = load_matrix();
    let matrix_ids = matrix_section_ids(&matrix);
    let registered: HashSet<&str> = ecosystems::all().iter().map(|e| e.id).collect();
    for id in &matrix_ids {
        assert!(
            registered.contains(id.as_str()),
            "tools/zetl-ecosystem-matrix.toml has section [ecosystem.{}] \
             but no matching entry in src/ecosystems/registry.rs",
            id,
        );
    }
}

#[test]
fn matrix_lint_section_counts_match() {
    // Belt-and-braces: the two sets must be exactly equal.
    let matrix = load_matrix();
    let matrix_ids = matrix_section_ids(&matrix);
    assert_eq!(
        matrix_ids.len(),
        ecosystems::all().len(),
        "registry has {} ecosystems but matrix has {} sections — they must match one-to-one",
        ecosystems::all().len(),
        matrix_ids.len(),
    );
}

// ── Cargo-feature lint (task-eco-feature-flags acceptance) ────────────

fn cargo_toml_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")
}

fn load_cargo_features() -> toml::value::Table {
    let body = std::fs::read_to_string(cargo_toml_path()).expect("Cargo.toml must be readable");
    let parsed: toml::Value = toml::from_str(&body).expect("Cargo.toml must parse as TOML");
    match parsed.get("features") {
        Some(toml::Value::Table(t)) => t.clone(),
        _ => panic!("Cargo.toml is missing the [features] table"),
    }
}

#[test]
fn cargo_features_cover_every_registered_ecosystem() {
    // Every `feature_flag` declared in `src/ecosystems/registry.rs` must
    // exist as a `[features]` entry in Cargo.toml — otherwise the
    // `--features ecosystem-<id>` cargo invocation SPEC-033 REQ-3302
    // advertises to users (and the `zetl ecosystem check` diagnostic
    // surfaces in its install hint) would silently resolve to a typo.
    let features = load_cargo_features();
    for entry in ecosystems::all() {
        assert!(
            features.contains_key(entry.feature_flag),
            "ecosystem '{}' declares feature_flag '{}' but Cargo.toml [features] has no such \
             entry — add `{} = []` (or its transitive deps) before merging",
            entry.id,
            entry.feature_flag,
            entry.feature_flag,
        );
    }
}

#[test]
fn ecosystems_v1_umbrella_enables_every_per_ecosystem_flag() {
    // SPEC-033 §12 + plans/IMPL-032-033.spl task-eco-feature-flags
    // acceptance: `cargo build --features ecosystems-v1` must enable all
    // three per-ecosystem flags at once. Encode that as a transitive
    // check on the Cargo.toml [features] table.
    let features = load_cargo_features();
    let umbrella = features
        .get("ecosystems-v1")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| {
            panic!(
                "Cargo.toml must declare an `ecosystems-v1` umbrella feature (see \
                 SPEC-033 §12) enabling every per-ecosystem flag"
            )
        });
    let enabled: HashSet<String> = umbrella
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    for entry in ecosystems::all() {
        assert!(
            enabled.contains(entry.feature_flag),
            "`ecosystems-v1` umbrella does not enable '{}' — expected all three \
             per-ecosystem flags (got {:?})",
            entry.feature_flag,
            enabled,
        );
    }
}
