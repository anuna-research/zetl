//! Ecosystem registry (SPEC-033 REQ-3301 / CON-3301).
//!
//! Enumerates the plugin ecosystems this zetl binary knows about — v1
//! ships three: Pandoc filters, mdBook preprocessors, remark plugins.
//! The registry is a `const` table: adding an ecosystem is adding an
//! entry here plus the adapter module (task-adapter-trait wires
//! ecosystems to adapter constructors). There is no runtime
//! registration — every entry is known at compile time.
//!
//! ## What lives in each entry
//!
//! Per CON-3301, every [`EcosystemEntry`] records:
//!
//! - a stable string identifier (`"pandoc"`, `"mdbook"`, `"remark"`);
//! - a human-readable display name;
//! - the [`RuntimeDep`] (binary name + minimum version) the ecosystem
//!   probes for at startup (SPEC-033 REQ-3313);
//! - the [`Stage`] this ecosystem's plugins default to when a manifest
//!   doesn't declare one explicitly (Pandoc → `transform`, mdBook →
//!   `pre-parse`, remark → `transform`);
//! - the cargo feature flag that gates the adapter's compilation, so
//!   users building a minimal zetl binary can drop whole ecosystems;
//! - the set of [`Stage`]s this ecosystem's plugins may participate in;
//! - the set of [`AstType`]s this ecosystem's adapter translates to and
//!   from.
//!
//! ## Matrix-lint contract (task-eco-registry acceptance)
//!
//! Every ecosystem id in [`ECOSYSTEMS`] must have a section in
//! `tools/zetl-ecosystem-matrix.toml` of the form `[ecosystem.<id>]`.
//! That gate lives in the integration tests beside this module.
//! Adding a new ecosystem here without a matrix entry fails the lint;
//! adding a matrix section for an unknown ecosystem fails too. Plugin
//! rows within each section are seeded by the per-ecosystem
//! matrix-seed tasks (task-pandoc-matrix-seed et al.) — this gate only
//! checks the section presence.

use crate::hooks::pipeline::Stage;
use crate::hooks::translators::AstType;

/// The plugin ecosystems zetl knows about in v1 (SPEC-033 REQ-3301).
///
/// Each variant has exactly one corresponding row in [`ECOSYSTEMS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ecosystem {
    /// Pandoc filters — `ecosystem = "pandoc"` in a manifest (REQ-3303).
    Pandoc,
    /// mdBook preprocessors — `ecosystem = "mdbook"` (REQ-3304).
    Mdbook,
    /// remark plugins — `ecosystem = "remark"` (REQ-3305).
    Remark,
}

impl Ecosystem {
    /// Stable on-disk identifier — matches the manifest `ecosystem = "..."`
    /// value and the registry lookup key.
    pub const fn as_str(self) -> &'static str {
        match self {
            Ecosystem::Pandoc => "pandoc",
            Ecosystem::Mdbook => "mdbook",
            Ecosystem::Remark => "remark",
        }
    }

    /// The registry entry for this ecosystem. Infallible — every variant
    /// has a row by construction.
    pub fn entry(self) -> &'static EcosystemEntry {
        for entry in ECOSYSTEMS {
            if entry.ecosystem as u8 == self as u8 {
                return entry;
            }
        }
        // Unreachable: every variant is in ECOSYSTEMS. Panic rather than
        // Option because a missing row is a registry-integrity bug that
        // would already fail TEST-3301.
        panic!("registry integrity: no entry for {self:?}")
    }
}

impl std::fmt::Display for Ecosystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Ecosystem {
    type Err = UnknownEcosystem;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pandoc" => Ok(Ecosystem::Pandoc),
            "mdbook" => Ok(Ecosystem::Mdbook),
            "remark" => Ok(Ecosystem::Remark),
            other => Err(UnknownEcosystem(other.to_string())),
        }
    }
}

/// Returned when a manifest's `ecosystem = "..."` value isn't in the
/// registry. Distinct from the adapter's runtime-missing diagnostic
/// (SPEC-033 REQ-3313) — that one means zetl knows the ecosystem but
/// can't find its binary; this one means the id doesn't resolve at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownEcosystem(pub String);

impl std::fmt::Display for UnknownEcosystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let known: Vec<&str> = ECOSYSTEMS.iter().map(|e| e.id).collect();
        write!(
            f,
            "unknown ecosystem '{}' (known: [{}])",
            self.0,
            known.join(", ")
        )
    }
}

impl std::error::Error for UnknownEcosystem {}

/// External runtime an ecosystem depends on (SPEC-033 REQ-3313).
///
/// The adapter probes `binary --version` at startup, parses the version,
/// and compares against `min_version`. A version below minimum or a
/// missing binary surfaces as a disabled adapter plus an actionable
/// `zetl ecosystem check` hint — not a build failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeDep {
    /// Executable looked up on `PATH`.
    pub binary: &'static str,
    /// Minimum version string accepted by the adapter. Semver-ish — the
    /// adapter decides how to parse (pandoc's 3-digit scheme, node's
    /// `vX.Y.Z`, etc.).
    pub min_version: &'static str,
}

/// One row of the ecosystem registry (SPEC-033 CON-3301).
#[derive(Debug, Clone, Copy)]
pub struct EcosystemEntry {
    /// The strongly-typed tag. Keeps [`Ecosystem::entry`] cheap (no
    /// string compare) and lets callers match exhaustively.
    pub ecosystem: Ecosystem,
    /// Stable on-disk identifier — same as [`Ecosystem::as_str`],
    /// stored here so `&EcosystemEntry` carries everything callers need
    /// without a back-reference.
    pub id: &'static str,
    /// Human-readable display name for diagnostics and `zetl ecosystem
    /// check` output.
    pub name: &'static str,
    /// External runtime the adapter probes at startup.
    pub runtime_dep: RuntimeDep,
    /// Stage a manifest declaring this ecosystem defaults to when it
    /// doesn't set `stage = "..."` itself (REQ-3303/3304/3305).
    pub default_stage: Stage,
    /// Cargo feature flag gating the adapter's compilation. A binary
    /// built without this feature exposes the ecosystem id here for
    /// diagnostics but refuses to load its manifests (REQ-3313).
    pub feature_flag: &'static str,
    /// Every [`Stage`] this ecosystem's plugins may run in. A manifest
    /// declaring a stage outside this list is rejected at load time.
    pub supported_stages: &'static [Stage],
    /// AST shapes this ecosystem's adapter translates to and from
    /// (SPEC-033 REQ-3307 / REQ-3308). Always non-empty — every adapter
    /// at least accepts its native foreign AST.
    pub supported_ast_types: &'static [AstType],
}

/// The complete v1 registry (SPEC-033 REQ-3301 / CON-3301).
///
/// Ordering is the canonical display order used by `zetl ecosystem
/// check` and diagnostics: pandoc first, mdBook second, remark third.
/// Integration tests (TEST-3301) enforce integrity: unique ids,
/// non-empty supported_stages, non-empty supported_ast_types,
/// `default_stage ∈ supported_stages`, every id has a
/// matrix section.
pub const ECOSYSTEMS: &[EcosystemEntry] = &[
    EcosystemEntry {
        ecosystem: Ecosystem::Pandoc,
        id: "pandoc",
        name: "Pandoc filters",
        runtime_dep: RuntimeDep {
            binary: "pandoc",
            min_version: "2.11",
        },
        default_stage: Stage::Transform,
        feature_flag: "ecosystem-pandoc",
        supported_stages: &[Stage::Transform],
        supported_ast_types: &[AstType::PandocExt],
    },
    EcosystemEntry {
        ecosystem: Ecosystem::Mdbook,
        id: "mdbook",
        name: "mdBook preprocessors",
        runtime_dep: RuntimeDep {
            binary: "mdbook",
            min_version: "0.4.0",
        },
        default_stage: Stage::PreParse,
        feature_flag: "ecosystem-mdbook",
        supported_stages: &[Stage::PreParse],
        // mdBook preprocessors operate on raw Markdown strings inside a
        // `{context, book}` envelope, not on a typed AST. They share
        // zetl-ext as their boundary shape because the pre-parse stage
        // exchange is text, not AST.
        supported_ast_types: &[AstType::ZetlExt],
    },
    EcosystemEntry {
        ecosystem: Ecosystem::Remark,
        id: "remark",
        name: "remark plugins",
        runtime_dep: RuntimeDep {
            binary: "node",
            min_version: "18.0.0",
        },
        default_stage: Stage::Transform,
        feature_flag: "ecosystem-remark",
        supported_stages: &[Stage::Transform],
        supported_ast_types: &[AstType::MdastExt],
    },
];

/// Look up an ecosystem entry by its stable string id.
pub fn by_id(id: &str) -> Option<&'static EcosystemEntry> {
    ECOSYSTEMS.iter().find(|e| e.id == id)
}

/// The full registry as a slice for iteration (`zetl ecosystem check`
/// and tests walk the list in canonical order).
pub fn all() -> &'static [EcosystemEntry] {
    ECOSYSTEMS
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    // ── Ecosystem enum parsing ─────────────────────────────────────────

    #[test]
    fn ecosystem_parses_every_v1_id() {
        assert_eq!("pandoc".parse::<Ecosystem>().unwrap(), Ecosystem::Pandoc);
        assert_eq!("mdbook".parse::<Ecosystem>().unwrap(), Ecosystem::Mdbook);
        assert_eq!("remark".parse::<Ecosystem>().unwrap(), Ecosystem::Remark);
    }

    #[test]
    fn ecosystem_rejects_unknown_id() {
        let err = "djot".parse::<Ecosystem>().unwrap_err();
        assert_eq!(err.0, "djot");
        let msg = err.to_string();
        assert!(msg.contains("unknown ecosystem 'djot'"));
        assert!(msg.contains("pandoc"));
        assert!(msg.contains("mdbook"));
        assert!(msg.contains("remark"));
    }

    #[test]
    fn display_matches_id() {
        assert_eq!(Ecosystem::Pandoc.to_string(), "pandoc");
        assert_eq!(Ecosystem::Mdbook.to_string(), "mdbook");
        assert_eq!(Ecosystem::Remark.to_string(), "remark");
    }

    #[test]
    fn ecosystem_entry_round_trip_matches_id() {
        for e in [Ecosystem::Pandoc, Ecosystem::Mdbook, Ecosystem::Remark] {
            let entry = e.entry();
            assert_eq!(entry.ecosystem, e);
            assert_eq!(entry.id, e.as_str());
        }
    }

    // ── Registry integrity (TEST-3301) ─────────────────────────────────

    #[test]
    fn registry_has_three_v1_entries() {
        assert_eq!(ECOSYSTEMS.len(), 3);
    }

    #[test]
    fn every_id_is_unique() {
        let mut seen: HashSet<&str> = HashSet::new();
        for entry in ECOSYSTEMS {
            assert!(
                seen.insert(entry.id),
                "duplicate ecosystem id '{}' in registry",
                entry.id
            );
        }
    }

    #[test]
    fn by_id_finds_every_entry() {
        for entry in ECOSYSTEMS {
            let found = by_id(entry.id).expect("registry entry must be lookupable by its id");
            assert_eq!(found.id, entry.id);
            assert_eq!(found.ecosystem, entry.ecosystem);
        }
    }

    #[test]
    fn by_id_returns_none_for_unknown() {
        assert!(by_id("djot").is_none());
        assert!(by_id("").is_none());
    }

    #[test]
    fn supported_stages_non_empty() {
        for entry in ECOSYSTEMS {
            assert!(
                !entry.supported_stages.is_empty(),
                "ecosystem '{}' has empty supported_stages",
                entry.id
            );
        }
    }

    #[test]
    fn supported_ast_types_non_empty() {
        for entry in ECOSYSTEMS {
            assert!(
                !entry.supported_ast_types.is_empty(),
                "ecosystem '{}' has empty supported_ast_types",
                entry.id
            );
        }
    }

    #[test]
    fn default_stage_is_in_supported_stages() {
        for entry in ECOSYSTEMS {
            assert!(
                entry.supported_stages.contains(&entry.default_stage),
                "ecosystem '{}' default_stage {} not in supported_stages {:?}",
                entry.id,
                entry.default_stage,
                entry.supported_stages,
            );
        }
    }

    #[test]
    fn feature_flag_follows_ecosystem_prefix_convention() {
        // Every feature flag is `ecosystem-<id>` so `--features
        // ecosystem-pandoc` is predictable for users and CI.
        for entry in ECOSYSTEMS {
            let expected = format!("ecosystem-{}", entry.id);
            assert_eq!(
                entry.feature_flag, expected,
                "ecosystem '{}' feature_flag should be '{}'",
                entry.id, expected
            );
        }
    }

    #[test]
    fn runtime_dep_fields_are_non_empty() {
        for entry in ECOSYSTEMS {
            assert!(
                !entry.runtime_dep.binary.is_empty(),
                "ecosystem '{}' has empty runtime binary",
                entry.id
            );
            assert!(
                !entry.runtime_dep.min_version.is_empty(),
                "ecosystem '{}' has empty runtime min_version",
                entry.id
            );
        }
    }

    #[test]
    fn canonical_ordering_is_pandoc_mdbook_remark() {
        // The canonical order surfaces in `zetl ecosystem check` and
        // diagnostics — lock it in so output stays stable.
        let ids: Vec<&str> = ECOSYSTEMS.iter().map(|e| e.id).collect();
        assert_eq!(ids, vec!["pandoc", "mdbook", "remark"]);
    }

    #[test]
    fn all_returns_the_canonical_slice() {
        assert_eq!(all().len(), ECOSYSTEMS.len());
        for (a, b) in all().iter().zip(ECOSYSTEMS.iter()) {
            assert_eq!(a.id, b.id);
        }
    }

    // ── Per-ecosystem spec-pinned invariants ───────────────────────────

    #[test]
    fn pandoc_entry_matches_spec() {
        let e = Ecosystem::Pandoc.entry();
        assert_eq!(e.id, "pandoc");
        assert_eq!(e.runtime_dep.binary, "pandoc");
        assert_eq!(e.default_stage, Stage::Transform);
        assert!(e.supported_ast_types.contains(&AstType::PandocExt));
    }

    #[test]
    fn mdbook_entry_matches_spec() {
        let e = Ecosystem::Mdbook.entry();
        assert_eq!(e.id, "mdbook");
        assert_eq!(e.runtime_dep.binary, "mdbook");
        // mdBook preprocessors operate on raw markdown → pre-parse.
        assert_eq!(e.default_stage, Stage::PreParse);
    }

    #[test]
    fn remark_entry_matches_spec() {
        let e = Ecosystem::Remark.entry();
        assert_eq!(e.id, "remark");
        assert_eq!(e.runtime_dep.binary, "node");
        assert_eq!(e.default_stage, Stage::Transform);
        assert!(e.supported_ast_types.contains(&AstType::MdastExt));
    }
}
