//! Section-level drift detection (REQ-043a / §6.5) and explicit-grounding
//! drift detection (REQ-043b / §6.5).
//!
//! **Section drift** is detected when the prose context surrounding an SPL
//! block changes without the SPL logic itself changing — i.e. the grounding
//! hash of the enclosing section changed but the SPL AST hash is unchanged.
//!
//! **Explicit drift** is detected when an explicit source-metadata grounding
//! target still exists in the vault but its leaf hash has changed since the
//! last `zetl build`.  Only targets that still resolve (same block-id
//! annotation or same hash prefix) are checked; targets that can no longer
//! be found are static validation errors reported by
//! [`crate::merkle::validate_source_refs`], not drift.

use crate::cache::TheoryCache;
use crate::merkle::{
    classify_source_ref, resolve_cross_file_block_id, resolve_local_block_id, SourceRefType,
    VaultHashIndex,
};
use crate::types::{
    ContentHash, DriftDiagnostic, DriftSeverity, DriftType, FileMerkle, LeafType, MerkleLeaf,
    ParsedFile,
};
use std::path::{Path, PathBuf};

/// Detect section-level drift for a single file (REQ-043a / §6.5).
///
/// Compares, for each SPL block in `current`, the freshly-computed section
/// grounding hash against the value stored in [`TheoryCache::spl_blocks`]
/// from the last `zetl build`.
///
/// **Drift condition**: grounding hash changed **AND** SPL AST hash unchanged.
///
/// **Severity**:
/// - [`DriftSeverity::Warning`] – the boundary leaves of the section
///   (`section_start` or `section_end-1`) are non-SPL leaves, meaning the
///   prose immediately surrounding the theory block may have changed.
/// - [`DriftSeverity::Info`] – both boundary leaves are SPL blocks (the
///   content change is structurally more distant from the theory).
///
/// Returns an empty `Vec` when there is no prior baseline or when no drift is
/// detected for any SPL block in the file.  Callers should skip drift
/// detection entirely when no `theory.json` exists (no baseline).
pub fn detect_section_drift(
    file: &Path,
    current: &FileMerkle,
    all_leaves: &[MerkleLeaf],
    cached_theory: &TheoryCache,
) -> Vec<DriftDiagnostic> {
    let mut diagnostics = Vec::new();

    for spl_leaf in &current.spl_leaves {
        let key = format!("{}:{}", file.display(), spl_leaf.start_line);
        let cached = match cached_theory.spl_blocks.get(&key) {
            Some(c) => c,
            None => continue, // no baseline for this SPL block
        };

        let section = &current.sections[spl_leaf.section_index];
        let current_grounding = section.grounding_hash;
        let cached_grounding = cached.section_grounding_hash;

        // No grounding change → no drift.
        if current_grounding == cached_grounding {
            continue;
        }

        // SPL AST changed → the theory itself changed, not section drift.
        if spl_leaf.ast_hash != cached.ast_hash {
            continue;
        }

        // ── Section drift confirmed ───────────────────────────────────────────
        let severity = severity_for_spl(all_leaves, spl_leaf.start_line);

        // Prefer the current heading_text populated by the scanner; fall back
        // to the heading stored in the cached theory (covers the edge case
        // where heading_text was not populated for this run).
        let section_heading = if !section.heading_text.is_empty() {
            section.heading_text.clone()
        } else {
            cached.section_heading.clone().unwrap_or_default()
        };

        let message = format!(
            "section '{}' changed since last theory build \
             (grounding hash mismatch; SPL logic unchanged)",
            section_heading
        );

        diagnostics.push(DriftDiagnostic {
            file: file.to_path_buf(),
            spl_line: spl_leaf.start_line,
            drift_type: DriftType::SectionDrift {
                section_heading: section_heading.clone(),
            },
            severity,
            message,
        });
    }

    diagnostics
}

/// Detect explicit-grounding drift for a single file (REQ-043b / §6.5).
///
/// For each SPL block in `current.spl_leaves`, re-resolves every cached
/// [`crate::types::ResolvedTarget`] in the corresponding
/// [`crate::cache::SplBlockCache`] against the current vault state.  Emits a
/// [`DriftDiagnostic`] when the target still exists but its leaf hash has
/// changed since the last `zetl build`.
///
/// **Drift condition**: SPL AST hash **unchanged** AND target can still be
/// resolved in the vault but its current leaf hash differs from the cached
/// `target_leaf_hash`.
///
/// **Not drift**: target no longer resolvable — this is a static validation
/// error produced by [`crate::merkle::validate_source_refs`].
///
/// **Severity**: always [`DriftSeverity::Warning`], because an explicit
/// grounding pin that silently diverged from the cited source is a meaningful
/// semantic concern.
///
/// Hash-prefix source refs (`HashRef`) are skipped for drift: if the content
/// they pin still exists the hash is identical by definition; if it is gone
/// that is a static validation error, not drift.
///
/// Returns an empty `Vec` when there is no prior baseline, when the SPL logic
/// itself changed, or when no explicit-grounding drift is detected.
pub fn detect_explicit_drift(
    file: &Path,
    current: &FileMerkle,
    cached_theory: &TheoryCache,
    files: &[ParsedFile],
    file_index: &[(String, PathBuf)],
    vault_hash_index: &VaultHashIndex,
) -> Vec<DriftDiagnostic> {
    let mut diagnostics = Vec::new();

    for spl_leaf in &current.spl_leaves {
        let key = format!("{}:{}", file.display(), spl_leaf.start_line);
        let cached = match cached_theory.spl_blocks.get(&key) {
            Some(c) => c,
            None => continue, // no baseline for this SPL block
        };

        // SPL AST changed → the theory itself changed, not grounding drift.
        if spl_leaf.ast_hash != cached.ast_hash {
            continue;
        }

        for grounding in &cached.explicit_groundings {
            for target in &grounding.targets {
                let current_hash = resolve_target_current_hash(
                    &target.source_ref,
                    &target.target_file,
                    files,
                    file_index,
                    vault_hash_index,
                );

                let current_hash = match current_hash {
                    Some(h) => h,
                    None => continue, // target not found → static error, not drift
                };

                if current_hash == target.target_leaf_hash {
                    continue; // no change
                }

                // Target exists but its hash changed → explicit grounding drift.
                diagnostics.push(DriftDiagnostic {
                    file: file.to_path_buf(),
                    spl_line: spl_leaf.start_line,
                    drift_type: DriftType::ExplicitDrift {
                        construct: grounding.construct.clone(),
                        source_ref: target.source_ref.clone(),
                    },
                    severity: DriftSeverity::Warning,
                    message: format!(
                        "explicit grounding '{}' for construct '{}' has changed \
                         since last theory build (target leaf hash mismatch)",
                        target.source_ref, grounding.construct,
                    ),
                });
            }
        }
    }

    diagnostics
}

/// Re-resolve a cached source ref against the current vault state and return
/// the current leaf hash, or `None` if the target can no longer be found.
///
/// `None` means the target is gone — a static validation error handled
/// elsewhere.  Hash-prefix refs (`HashRef`) always return `None` here because
/// they are self-verifying: the hash *is* the identity, so they cannot drift.
fn resolve_target_current_hash(
    source_ref: &str,
    target_file: &Path,
    files: &[ParsedFile],
    file_index: &[(String, PathBuf)],
    _vault_hash_index: &VaultHashIndex,
) -> Option<ContentHash> {
    match classify_source_ref(source_ref) {
        // Hash refs are content-addressed: if content exists, hash is unchanged;
        // if content is gone, that is a static error.  No drift is possible.
        SourceRefType::HashRef => None,

        SourceRefType::LocalRef => {
            let block_id = &source_ref[1..];
            let parsed = files.iter().find(|f| f.path == target_file)?;
            resolve_local_block_id(&parsed.merkle_leaves, block_id)
        }

        SourceRefType::CrossFileRef => {
            // Strip `[[` and `]]`, split on first `^`.
            let inner = &source_ref[2..source_ref.len() - 2];
            let caret = inner.find('^')?;
            let page_name = &inner[..caret];
            let block_id = &inner[caret + 1..];
            resolve_cross_file_block_id(files, file_index, page_name, block_id).ok()
        }

        // Page-only refs and unknown formats have no leaf-level identity.
        SourceRefType::PageRef | SourceRefType::Unknown => None,
    }
}

/// Compute the severity for a section-drift event by inspecting the boundary
/// leaves of the grounding section containing the SPL block at `spl_start_line`.
///
/// Returns [`DriftSeverity::Warning`] when either the first leaf
/// (`section_start`) or the last leaf (`section_end-1`) of the section is a
/// non-SPL leaf — those boundary leaves are "immediately adjacent" to the
/// theory block in its grounding context and their change is considered
/// high-impact.  Returns [`DriftSeverity::Info`] when both boundary leaves
/// are themselves SPL blocks.
fn severity_for_spl(all_leaves: &[MerkleLeaf], spl_start_line: u32) -> DriftSeverity {
    let spl_idx = match all_leaves
        .iter()
        .position(|l| l.start_line == spl_start_line && matches!(l.node_type, LeafType::SplBlock))
    {
        Some(i) => i,
        None => return DriftSeverity::Info,
    };

    let (start_idx, end_idx) = section_bounds(all_leaves, spl_idx);

    let start_is_non_spl = !matches!(all_leaves[start_idx].node_type, LeafType::SplBlock);
    // Only check end separately when it differs from start (single-leaf sections
    // would double-count the same leaf).
    let end_is_non_spl =
        end_idx != start_idx && !matches!(all_leaves[end_idx].node_type, LeafType::SplBlock);

    if start_is_non_spl || end_is_non_spl {
        DriftSeverity::Warning
    } else {
        DriftSeverity::Info
    }
}

/// Return the inclusive `[start_idx, end_idx]` leaf-range for the grounding
/// section that contains the SPL block at `spl_leaf_idx`.
///
/// Mirrors the boundary-computation logic of
/// [`crate::merkle::grounding_section`] without producing a [`Section`] value.
fn section_bounds(leaves: &[MerkleLeaf], spl_leaf_idx: usize) -> (usize, usize) {
    // Walk backwards to find the nearest preceding heading (section_start).
    let (start_idx, heading_level) = match (0..spl_leaf_idx)
        .rev()
        .find(|&i| matches!(leaves[i].node_type, LeafType::Heading { .. }))
    {
        Some(idx) => {
            let lv = match leaves[idx].node_type {
                LeafType::Heading { level } => level,
                _ => unreachable!(),
            };
            (idx, lv)
        }
        None => (0, 0u8), // preamble: no preceding heading
    };

    let end_idx = if heading_level == 0 {
        // Preamble section ends just before the first heading of any level.
        match leaves
            .iter()
            .position(|l| matches!(l.node_type, LeafType::Heading { .. }))
        {
            Some(first_heading) => first_heading.saturating_sub(1),
            None => leaves.len().saturating_sub(1),
        }
    } else {
        // Non-preamble: ends just before the next heading at the same or
        // higher structural level (lower or equal numeric level).
        let search_from = start_idx + 1;
        match leaves[search_from..].iter().position(
            |l| matches!(l.node_type, LeafType::Heading { level: lv } if lv <= heading_level),
        ) {
            Some(offset) => (search_from + offset).saturating_sub(1),
            None => leaves.len().saturating_sub(1),
        }
    };

    (start_idx, end_idx)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ContentHash, FileMerkle, Section, SplLeafCached};
    use std::path::PathBuf;

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn make_leaf(node_type: LeafType, start_line: u32) -> MerkleLeaf {
        MerkleLeaf {
            node_type,
            start_line,
            end_line: start_line,
            hash: [0u8; 32],
            spl_hashes: None,
            block_id: None,
        }
    }

    fn h(byte: u8) -> ContentHash {
        [byte; 32]
    }

    /// Construct a minimal `TheoryCache` via JSON deserialization.
    ///
    /// `spl_blocks` maps `"path:line"` keys to `(ast_hash, grounding_hash,
    /// section_heading)` triples (all as `[u8; 32]`).
    fn make_theory_cache(
        spl_blocks: Vec<(&str, ContentHash, ContentHash, Option<&str>)>,
    ) -> TheoryCache {
        let mut blocks_json = serde_json::Map::new();
        for (key, ast, grounding, heading) in spl_blocks {
            let ast_hex: String = ast.iter().map(|b| format!("{:02x}", b)).collect();
            let grounding_hex: String = grounding.iter().map(|b| format!("{:02x}", b)).collect();
            // content_hash = ast_hash for simplicity in tests
            let block = serde_json::json!({
                "ast_hash": ast_hex,
                "content_hash": ast_hex,
                "section_heading": heading,
                "section_grounding_hash": grounding_hex,
                "explicit_groundings": []
            });
            blocks_json.insert(key.to_string(), block);
        }
        let json = serde_json::json!({
            "version": 2u32,
            "spl_file_mtimes": {},
            "rules": [],
            "superiorities": [],
            "diagnostics": [],
            "spl_blocks": blocks_json,
        });
        serde_json::from_value(json).expect("TheoryCache deserialization failed")
    }

    fn make_file_merkle(sections: Vec<Section>, spl_leaves: Vec<SplLeafCached>) -> FileMerkle {
        FileMerkle {
            root_hash: [0u8; 32],
            sections,
            spl_leaves,
        }
    }

    fn make_section(
        heading_line: u32,
        heading_text: &str,
        heading_level: u8,
        leaf_range: (usize, usize),
        grounding_hash: ContentHash,
    ) -> Section {
        Section {
            heading_line,
            heading_text: heading_text.to_string(),
            heading_level,
            leaf_range,
            grounding_hash,
        }
    }

    fn make_spl_leaf(
        start_line: u32,
        ast_hash: ContentHash,
        section_index: usize,
    ) -> SplLeafCached {
        SplLeafCached {
            start_line,
            content_hash: ast_hash,
            ast_hash,
            section_index,
            explicit_groundings: vec![],
        }
    }

    // ── section_bounds ─────────────────────────────────────────────────────────

    #[test]
    fn section_bounds_preamble_no_headings() {
        // [Para@1, SPL@3] — preamble, no headings anywhere
        let leaves = vec![
            make_leaf(LeafType::Paragraph, 1),
            make_leaf(LeafType::SplBlock, 3),
        ];
        let (start, end) = section_bounds(&leaves, 1);
        assert_eq!(start, 0, "preamble starts at leaf 0");
        assert_eq!(end, 1, "preamble ends at last leaf");
    }

    #[test]
    fn section_bounds_preamble_stops_before_heading() {
        // [Para@1, SPL@3, H2@5] — preamble ends before H2
        let leaves = vec![
            make_leaf(LeafType::Paragraph, 1),
            make_leaf(LeafType::SplBlock, 3),
            make_leaf(LeafType::Heading { level: 2 }, 5),
        ];
        let (start, end) = section_bounds(&leaves, 1);
        assert_eq!(start, 0);
        assert_eq!(end, 1, "preamble ends at leaf before H2");
    }

    #[test]
    fn section_bounds_under_heading() {
        // [H2@1, Para@3, SPL@5] — grounded by H2@1
        let leaves = vec![
            make_leaf(LeafType::Heading { level: 2 }, 1),
            make_leaf(LeafType::Paragraph, 3),
            make_leaf(LeafType::SplBlock, 5),
        ];
        let (start, end) = section_bounds(&leaves, 2);
        assert_eq!(start, 0, "section starts at the H2");
        assert_eq!(
            end, 2,
            "section ends at the last leaf (no terminating heading)"
        );
    }

    #[test]
    fn section_bounds_terminates_at_same_level_heading() {
        // [H2@1, Para@2, SPL@4, H2@10, Para@11]
        let leaves = vec![
            make_leaf(LeafType::Heading { level: 2 }, 1),
            make_leaf(LeafType::Paragraph, 2),
            make_leaf(LeafType::SplBlock, 4),
            make_leaf(LeafType::Heading { level: 2 }, 10),
            make_leaf(LeafType::Paragraph, 11),
        ];
        let (start, end) = section_bounds(&leaves, 2);
        assert_eq!(start, 0);
        assert_eq!(end, 2, "section ends just before the second H2");
    }

    // ── severity_for_spl ───────────────────────────────────────────────────────

    #[test]
    fn severity_warning_when_section_start_is_heading() {
        // [H2@1, SPL@3] — section_start is the heading (non-SPL) → Warning
        let leaves = vec![
            make_leaf(LeafType::Heading { level: 2 }, 1),
            make_leaf(LeafType::SplBlock, 3),
        ];
        assert!(matches!(
            severity_for_spl(&leaves, 3),
            DriftSeverity::Warning
        ));
    }

    #[test]
    fn severity_warning_when_section_end_is_paragraph() {
        // [H2@1, SPL@3, Para@5] — section_end is Para (non-SPL) → Warning
        let leaves = vec![
            make_leaf(LeafType::Heading { level: 2 }, 1),
            make_leaf(LeafType::SplBlock, 3),
            make_leaf(LeafType::Paragraph, 5),
        ];
        assert!(matches!(
            severity_for_spl(&leaves, 3),
            DriftSeverity::Warning
        ));
    }

    #[test]
    fn severity_info_when_both_boundaries_are_spl() {
        // [SPL@1, SPL@3] — preamble, both start and end are SPL → Info
        let leaves = vec![
            make_leaf(LeafType::SplBlock, 1),
            make_leaf(LeafType::SplBlock, 3),
        ];
        // spl_idx=0: section_bounds preamble with no heading → (0, 1)
        // start=SPL, end=SPL → Info
        assert!(matches!(severity_for_spl(&leaves, 1), DriftSeverity::Info));
    }

    #[test]
    fn severity_info_when_spl_line_not_found() {
        let leaves = vec![make_leaf(LeafType::Paragraph, 1)];
        assert!(matches!(
            severity_for_spl(&leaves, 999),
            DriftSeverity::Info
        ));
    }

    // ── detect_section_drift ───────────────────────────────────────────────────

    #[test]
    fn no_drift_when_no_theory_baseline() {
        // Empty spl_blocks in theory cache → no diagnostics
        let file = PathBuf::from("notes/arch.md");
        let spl_leaf = make_spl_leaf(5, h(0xAA), 0);
        let section = make_section(1, "Background", 2, (0, 1), h(0x11));
        let fm = make_file_merkle(vec![section], vec![spl_leaf]);
        let leaves = vec![
            make_leaf(LeafType::Heading { level: 2 }, 1),
            make_leaf(LeafType::SplBlock, 5),
        ];
        let theory = make_theory_cache(vec![]); // no baseline

        let result = detect_section_drift(&file, &fm, &leaves, &theory);
        assert!(result.is_empty(), "no baseline → no drift diagnostics");
    }

    #[test]
    fn no_drift_when_grounding_hash_unchanged() {
        let file = PathBuf::from("notes/arch.md");
        let ast = h(0xAA);
        let grounding = h(0x11);
        let spl_leaf = make_spl_leaf(5, ast, 0);
        let section = make_section(1, "Background", 2, (0, 1), grounding);
        let fm = make_file_merkle(vec![section], vec![spl_leaf]);
        let leaves = vec![
            make_leaf(LeafType::Heading { level: 2 }, 1),
            make_leaf(LeafType::SplBlock, 5),
        ];
        // Cached with same grounding hash
        let theory = make_theory_cache(vec![(
            "notes/arch.md:5",
            ast,
            grounding,
            Some("Background"),
        )]);

        let result = detect_section_drift(&file, &fm, &leaves, &theory);
        assert!(result.is_empty(), "unchanged grounding → no drift");
    }

    #[test]
    fn no_drift_when_ast_hash_also_changed() {
        let file = PathBuf::from("notes/arch.md");
        let current_ast = h(0xBB);
        let cached_ast = h(0xAA); // different
        let current_grounding = h(0x22);
        let cached_grounding = h(0x11); // different
        let spl_leaf = make_spl_leaf(5, current_ast, 0);
        let section = make_section(1, "Background", 2, (0, 1), current_grounding);
        let fm = make_file_merkle(vec![section], vec![spl_leaf]);
        let leaves = vec![
            make_leaf(LeafType::Heading { level: 2 }, 1),
            make_leaf(LeafType::SplBlock, 5),
        ];
        // Cached with different grounding AND different AST → not section drift
        let theory = make_theory_cache(vec![(
            "notes/arch.md:5",
            cached_ast,
            cached_grounding,
            Some("Background"),
        )]);

        let result = detect_section_drift(&file, &fm, &leaves, &theory);
        assert!(
            result.is_empty(),
            "grounding changed but AST also changed → not section drift"
        );
    }

    #[test]
    fn section_drift_detected_grounding_changed_ast_unchanged() {
        let file = PathBuf::from("notes/arch.md");
        let ast = h(0xAA); // unchanged
        let current_grounding = h(0x22);
        let cached_grounding = h(0x11); // different
        let spl_leaf = make_spl_leaf(5, ast, 0);
        let section = make_section(1, "Background", 2, (0, 1), current_grounding);
        let fm = make_file_merkle(vec![section], vec![spl_leaf]);
        let leaves = vec![
            make_leaf(LeafType::Heading { level: 2 }, 1),
            make_leaf(LeafType::SplBlock, 5),
        ];
        let theory = make_theory_cache(vec![(
            "notes/arch.md:5",
            ast,
            cached_grounding,
            Some("Background"),
        )]);

        let result = detect_section_drift(&file, &fm, &leaves, &theory);
        assert_eq!(result.len(), 1, "one drift diagnostic expected");

        let d = &result[0];
        assert_eq!(d.file, file);
        assert_eq!(d.spl_line, 5);
        assert!(
            matches!(&d.drift_type, DriftType::SectionDrift { section_heading } if section_heading == "Background"),
            "drift type should be SectionDrift with the heading"
        );
        assert!(d.message.contains("Background"));
        assert!(d.message.contains("grounding hash mismatch"));
    }

    #[test]
    fn section_drift_severity_warning_for_heading_bounded_section() {
        // [H2@1, SPL@5] — heading is section_start (non-SPL) → Warning
        let file = PathBuf::from("notes/arch.md");
        let ast = h(0xAA);
        let spl_leaf = make_spl_leaf(5, ast, 0);
        let section = make_section(1, "Perf", 2, (0, 1), h(0x22));
        let fm = make_file_merkle(vec![section], vec![spl_leaf]);
        let leaves = vec![
            make_leaf(LeafType::Heading { level: 2 }, 1),
            make_leaf(LeafType::SplBlock, 5),
        ];
        let theory = make_theory_cache(vec![("notes/arch.md:5", ast, h(0x11), Some("Perf"))]);

        let result = detect_section_drift(&file, &fm, &leaves, &theory);
        assert_eq!(result.len(), 1);
        assert!(matches!(result[0].severity, DriftSeverity::Warning));
    }

    #[test]
    fn section_drift_uses_cached_heading_when_current_is_empty() {
        let file = PathBuf::from("notes/arch.md");
        let ast = h(0xAA);
        let spl_leaf = make_spl_leaf(5, ast, 0);
        // heading_text is empty in the current scan
        let section = make_section(1, "", 2, (0, 1), h(0x22));
        let fm = make_file_merkle(vec![section], vec![spl_leaf]);
        let leaves = vec![
            make_leaf(LeafType::Heading { level: 2 }, 1),
            make_leaf(LeafType::SplBlock, 5),
        ];
        let theory = make_theory_cache(vec![(
            "notes/arch.md:5",
            ast,
            h(0x11),
            Some("CachedHeading"),
        )]);

        let result = detect_section_drift(&file, &fm, &leaves, &theory);
        assert_eq!(result.len(), 1);
        assert!(
            matches!(&result[0].drift_type, DriftType::SectionDrift { section_heading } if section_heading == "CachedHeading"),
            "should fall back to cached heading when current is empty"
        );
    }

    #[test]
    fn section_drift_multiple_spl_blocks_partial_drift() {
        // Two SPL blocks in the file. Only one has section drift.
        let file = PathBuf::from("notes/arch.md");
        let ast1 = h(0xA1);
        let ast2 = h(0xA2);
        let grounding1_current = h(0x21);
        let grounding1_cached = h(0x11); // different → drift for spl1
        let grounding2 = h(0x33); // unchanged

        let spl1 = make_spl_leaf(5, ast1, 0);
        let spl2 = make_spl_leaf(10, ast2, 1);
        let section1 = make_section(1, "Alpha", 2, (0, 1), grounding1_current);
        let section2 = make_section(8, "Beta", 2, (1, 2), grounding2);
        let fm = make_file_merkle(vec![section1, section2], vec![spl1, spl2]);

        let leaves = vec![
            make_leaf(LeafType::Heading { level: 2 }, 1),
            make_leaf(LeafType::SplBlock, 5),
            make_leaf(LeafType::Heading { level: 2 }, 8),
            make_leaf(LeafType::SplBlock, 10),
        ];

        let theory = make_theory_cache(vec![
            ("notes/arch.md:5", ast1, grounding1_cached, Some("Alpha")),
            ("notes/arch.md:10", ast2, grounding2, Some("Beta")), // same → no drift
        ]);

        let result = detect_section_drift(&file, &fm, &leaves, &theory);
        assert_eq!(result.len(), 1, "only spl1 drifted");
        assert_eq!(result[0].spl_line, 5);
        assert!(
            matches!(&result[0].drift_type, DriftType::SectionDrift { section_heading } if section_heading == "Alpha")
        );
    }

    // ── detect_explicit_drift helpers ──────────────────────────────────────────

    use crate::merkle::build_vault_hash_index;
    use crate::types::ParsedFile;
    use std::time::SystemTime;

    fn make_parsed_file(path: &str, leaves: Vec<MerkleLeaf>) -> ParsedFile {
        ParsedFile {
            path: PathBuf::from(path),
            page_name: path.to_string(),
            links: vec![],
            spl_blocks: vec![],
            diagnostics: vec![],
            mtime: SystemTime::UNIX_EPOCH,
            merkle_leaves: leaves,
            file_merkle: None,
        }
    }

    fn make_leaf_with_block_id(start_line: u32, hash: ContentHash, block_id: &str) -> MerkleLeaf {
        MerkleLeaf {
            node_type: LeafType::Paragraph,
            start_line,
            end_line: start_line,
            hash,
            spl_hashes: None,
            block_id: Some(block_id.to_string()),
        }
    }

    /// Build a `TheoryCache` that has explicit groundings attached to one SPL
    /// block.  The `groundings` vec contains `(construct, source_ref,
    /// target_file, target_leaf_hash)` tuples that are embedded as
    /// `ResolvedTarget` entries.
    fn make_theory_cache_with_explicit(
        key: &str,
        ast_hash: ContentHash,
        grounding_hash: ContentHash,
        groundings: Vec<(&str, &str, &str, ContentHash)>,
    ) -> TheoryCache {
        let ast_hex: String = ast_hash.iter().map(|b| format!("{:02x}", b)).collect();
        let grounding_hex: String = grounding_hash
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect();

        let explicit: Vec<serde_json::Value> = groundings
            .iter()
            .map(|(construct, source_ref, target_file, target_hash)| {
                let target_hash_hex: String =
                    target_hash.iter().map(|b| format!("{:02x}", b)).collect();
                serde_json::json!({
                    "construct": construct,
                    "source_refs": [source_ref],
                    "targets": [{
                        "source_ref": source_ref,
                        "target_file": target_file,
                        "target_leaf_hash": target_hash_hex,
                    }]
                })
            })
            .collect();

        let block = serde_json::json!({
            "ast_hash": ast_hex,
            "content_hash": ast_hex,
            "section_heading": null,
            "section_grounding_hash": grounding_hex,
            "explicit_groundings": explicit,
        });

        let mut blocks_json = serde_json::Map::new();
        blocks_json.insert(key.to_string(), block);

        let json = serde_json::json!({
            "version": 2u32,
            "spl_file_mtimes": {},
            "rules": [],
            "superiorities": [],
            "diagnostics": [],
            "spl_blocks": blocks_json,
        });
        serde_json::from_value(json).expect("TheoryCache deserialization failed")
    }

    // ── detect_explicit_drift tests ────────────────────────────────────────────

    #[test]
    fn explicit_drift_no_baseline() {
        // No theory cache entry → no drift.
        let file = PathBuf::from("notes/arch.md");
        let ast = h(0xAA);
        let spl_leaf = make_spl_leaf(5, ast, 0);
        let section = make_section(1, "S", 2, (0, 1), h(0x11));
        let fm = make_file_merkle(vec![section], vec![spl_leaf]);
        let theory = make_theory_cache(vec![]); // empty
        let files: Vec<ParsedFile> = vec![];
        let file_index: Vec<(String, PathBuf)> = vec![];
        let index = build_vault_hash_index(&files);

        let result = detect_explicit_drift(&file, &fm, &theory, &files, &file_index, &index);
        assert!(result.is_empty(), "no baseline → no drift");
    }

    #[test]
    fn explicit_drift_no_explicit_groundings() {
        // Theory cache entry exists but has no explicit groundings → no drift.
        let file = PathBuf::from("notes/arch.md");
        let ast = h(0xAA);
        let spl_leaf = make_spl_leaf(5, ast, 0);
        let section = make_section(1, "S", 2, (0, 1), h(0x11));
        let fm = make_file_merkle(vec![section], vec![spl_leaf]);
        let theory = make_theory_cache(vec![("notes/arch.md:5", ast, h(0x11), None)]);
        let files: Vec<ParsedFile> = vec![];
        let file_index: Vec<(String, PathBuf)> = vec![];
        let index = build_vault_hash_index(&files);

        let result = detect_explicit_drift(&file, &fm, &theory, &files, &file_index, &index);
        assert!(result.is_empty(), "no explicit groundings → no drift");
    }

    #[test]
    fn explicit_drift_ast_changed_skips_grounding_check() {
        // Even if a target drifted, when AST changed the SPL itself changed —
        // not grounding drift.
        let file = PathBuf::from("notes/arch.md");
        let old_ast = h(0xAA);
        let new_ast = h(0xBB); // different from cached
        let old_target_hash = h(0x11);
        let new_target_hash = h(0x22); // target has also changed

        let spl_leaf = make_spl_leaf(5, new_ast, 0); // current AST ≠ cached
        let section = make_section(1, "S", 2, (0, 1), h(0x33));
        let fm = make_file_merkle(vec![section], vec![spl_leaf]);

        let theory = make_theory_cache_with_explicit(
            "notes/arch.md:5",
            old_ast,
            h(0x33),
            vec![("my_fact", "^para-ref", "notes/arch.md", old_target_hash)],
        );

        let target_file_leaves = vec![make_leaf_with_block_id(3, new_target_hash, "para-ref")];
        let files = vec![make_parsed_file("notes/arch.md", target_file_leaves)];
        let file_index: Vec<(String, PathBuf)> = vec![];
        let index = build_vault_hash_index(&files);

        let result = detect_explicit_drift(&file, &fm, &theory, &files, &file_index, &index);
        assert!(result.is_empty(), "AST changed → not grounding drift");
    }

    #[test]
    fn explicit_drift_local_ref_unchanged() {
        // Local block-id ref; target hash identical → no drift.
        let file = PathBuf::from("notes/arch.md");
        let ast = h(0xAA);
        let target_hash = h(0x11);

        let spl_leaf = make_spl_leaf(5, ast, 0);
        let section = make_section(1, "S", 2, (0, 1), h(0x33));
        let fm = make_file_merkle(vec![section], vec![spl_leaf]);

        let theory = make_theory_cache_with_explicit(
            "notes/arch.md:5",
            ast,
            h(0x33),
            vec![("my_fact", "^para-ref", "notes/arch.md", target_hash)],
        );

        // Current leaf has the SAME hash as cached.
        let target_file_leaves = vec![make_leaf_with_block_id(3, target_hash, "para-ref")];
        let files = vec![make_parsed_file("notes/arch.md", target_file_leaves)];
        let file_index: Vec<(String, PathBuf)> = vec![];
        let index = build_vault_hash_index(&files);

        let result = detect_explicit_drift(&file, &fm, &theory, &files, &file_index, &index);
        assert!(result.is_empty(), "unchanged target hash → no drift");
    }

    #[test]
    fn explicit_drift_local_ref_changed() {
        // Local block-id ref; target hash changed → ExplicitDrift Warning.
        let file = PathBuf::from("notes/arch.md");
        let ast = h(0xAA);
        let old_target_hash = h(0x11);
        let new_target_hash = h(0x22); // content changed

        let spl_leaf = make_spl_leaf(5, ast, 0);
        let section = make_section(1, "S", 2, (0, 1), h(0x33));
        let fm = make_file_merkle(vec![section], vec![spl_leaf]);

        let theory = make_theory_cache_with_explicit(
            "notes/arch.md:5",
            ast,
            h(0x33),
            vec![("my_fact", "^para-ref", "notes/arch.md", old_target_hash)],
        );

        // Current leaf has a DIFFERENT hash.
        let target_file_leaves = vec![make_leaf_with_block_id(3, new_target_hash, "para-ref")];
        let files = vec![make_parsed_file("notes/arch.md", target_file_leaves)];
        let file_index: Vec<(String, PathBuf)> = vec![];
        let index = build_vault_hash_index(&files);

        let result = detect_explicit_drift(&file, &fm, &theory, &files, &file_index, &index);
        assert_eq!(result.len(), 1, "one explicit drift diagnostic");

        let d = &result[0];
        assert_eq!(d.file, file);
        assert_eq!(d.spl_line, 5);
        assert!(matches!(d.severity, DriftSeverity::Warning));
        assert!(
            matches!(&d.drift_type, DriftType::ExplicitDrift { construct, source_ref }
                if construct == "my_fact" && source_ref == "^para-ref"),
            "drift type should be ExplicitDrift with correct construct and source_ref"
        );
        assert!(d.message.contains("^para-ref"));
        assert!(d.message.contains("my_fact"));
    }

    #[test]
    fn explicit_drift_local_ref_block_id_gone() {
        // Local block-id ref; block-id no longer in vault → no drift (static error).
        let file = PathBuf::from("notes/arch.md");
        let ast = h(0xAA);
        let old_target_hash = h(0x11);

        let spl_leaf = make_spl_leaf(5, ast, 0);
        let section = make_section(1, "S", 2, (0, 1), h(0x33));
        let fm = make_file_merkle(vec![section], vec![spl_leaf]);

        let theory = make_theory_cache_with_explicit(
            "notes/arch.md:5",
            ast,
            h(0x33),
            vec![("my_fact", "^para-ref", "notes/arch.md", old_target_hash)],
        );

        // Target file exists but the block-id annotation is gone.
        let target_file_leaves = vec![make_leaf(LeafType::Paragraph, 3)]; // no block_id
        let files = vec![make_parsed_file("notes/arch.md", target_file_leaves)];
        let file_index: Vec<(String, PathBuf)> = vec![];
        let index = build_vault_hash_index(&files);

        let result = detect_explicit_drift(&file, &fm, &theory, &files, &file_index, &index);
        assert!(result.is_empty(), "block-id gone → static error, not drift");
    }

    #[test]
    fn explicit_drift_cross_file_ref_changed() {
        // Cross-file `[[Page^block-id]]` ref; target hash changed → Warning.
        let spl_file = PathBuf::from("notes/arch.md");
        let target_file_path = PathBuf::from("notes/source.md");
        let ast = h(0xAA);
        let old_target_hash = h(0x11);
        let new_target_hash = h(0x22);

        let spl_leaf = make_spl_leaf(5, ast, 0);
        let section = make_section(1, "S", 2, (0, 1), h(0x33));
        let fm = make_file_merkle(vec![section], vec![spl_leaf]);

        let theory = make_theory_cache_with_explicit(
            "notes/arch.md:5",
            ast,
            h(0x33),
            vec![(
                "cross_fact",
                "[[Source^evidence-block]]",
                "notes/source.md",
                old_target_hash,
            )],
        );

        // The referenced block in the target file now has a different hash.
        let target_leaves = vec![make_leaf_with_block_id(
            7,
            new_target_hash,
            "evidence-block",
        )];
        let files = vec![
            make_parsed_file("notes/arch.md", vec![]),
            make_parsed_file("notes/source.md", target_leaves),
        ];
        let file_index: Vec<(String, PathBuf)> =
            vec![("Source".to_string(), target_file_path.clone())];
        let index = build_vault_hash_index(&files);

        let result = detect_explicit_drift(&spl_file, &fm, &theory, &files, &file_index, &index);
        assert_eq!(result.len(), 1);
        assert!(matches!(result[0].severity, DriftSeverity::Warning));
        assert!(
            matches!(&result[0].drift_type, DriftType::ExplicitDrift { construct, source_ref }
                if construct == "cross_fact" && source_ref == "[[Source^evidence-block]]")
        );
    }

    #[test]
    fn explicit_drift_hash_ref_never_drifts() {
        // Hash-prefix refs are self-verifying: re-resolution is skipped → no drift.
        let file = PathBuf::from("notes/arch.md");
        let ast = h(0xAA);
        let old_target_hash = h(0x11);
        // Build a hash ref from the old target (first 8 hex chars of 0x11...11).
        let hash_ref = "1111111111111111"; // 16 hex chars, matches h(0x11)

        let spl_leaf = make_spl_leaf(5, ast, 0);
        let section = make_section(1, "S", 2, (0, 1), h(0x33));
        let fm = make_file_merkle(vec![section], vec![spl_leaf]);

        let theory = make_theory_cache_with_explicit(
            "notes/arch.md:5",
            ast,
            h(0x33),
            vec![("hash_fact", hash_ref, "notes/arch.md", old_target_hash)],
        );

        // Vault has content with a different hash — but hash refs are skipped.
        let new_target_hash = h(0x22);
        let files = vec![make_parsed_file(
            "notes/arch.md",
            vec![make_leaf_with_block_id(3, new_target_hash, "some-id")],
        )];
        let file_index: Vec<(String, PathBuf)> = vec![];
        let index = build_vault_hash_index(&files);

        let result = detect_explicit_drift(&file, &fm, &theory, &files, &file_index, &index);
        assert!(result.is_empty(), "hash refs never drift");
    }
}
