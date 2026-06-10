//! SPEC-045 REQ-4508 — predicate lints for `zetl check`.
//!
//! In the **emergent** default (no strict file) these are advisory and computed
//! against *observed corpus usage* — there is no registry, so the corpus is the
//! reference. When a strict `.zetl/predicates.toml` is present, an undeclared
//! predicate escalates to an error (`enforce = true`) or a warning
//! (`enforce = false`).

use crate::predicates::{is_curie_predicate, PredicatesConfig};
use crate::types::ParsedFile;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Severity of a predicate lint. `Error` only ever arises from a strict
/// `enforce = true` file (the team chose a controlled vocabulary).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LintLevel {
    Error,
    Warning,
    Info,
}

impl LintLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            LintLevel::Error => "Error",
            LintLevel::Warning => "Warning",
            LintLevel::Info => "Info",
        }
    }
}

/// One predicate diagnostic.
#[derive(Debug, Clone, Serialize)]
pub struct PredicateLint {
    /// Stable lint id, e.g. `"predicate-drift"`.
    pub kind: &'static str,
    pub level: LintLevel,
    pub message: String,
    pub file: PathBuf,
    pub line: u32,
}

/// Default threshold for the `relates_to::` over-use signal.
const RELATES_TO_OVERUSE_FRACTION: f64 = 0.5;
/// Frequency asymmetry for the drift typo signal: a candidate must be used at
/// least this many times more than the suspected typo.
const DRIFT_ASYMMETRY: usize = 5;

/// Compute every predicate lint for the vault (REQ-4508). Pure + deterministic.
pub fn compute_predicate_lints(
    files: &[ParsedFile],
    config: Option<&PredicatesConfig>,
) -> Vec<PredicateLint> {
    let mut lints: Vec<PredicateLint> = Vec::new();

    // Global frequency of each predicate across the corpus (the emergent
    // vocabulary IS the corpus). One count per typed-edge occurrence.
    let mut freq: BTreeMap<String, usize> = BTreeMap::new();
    // First-seen location per predicate, for single-report lints.
    let mut first_seen: BTreeMap<String, (PathBuf, u32)> = BTreeMap::new();
    for f in files {
        for link in &f.links {
            for p in &link.predicates {
                *freq.entry(p.clone()).or_insert(0) += 1;
                first_seen
                    .entry(p.clone())
                    .or_insert_with(|| (f.path.clone(), link.line));
            }
        }
    }

    // ── predicate-undeclared / -undeclared-prefix (per occurrence) ──────────
    // ── predicate-prefer-conforms-to (per occurrence) ──────────────────────
    let accept_is_a = config.map(|c| c.accept_is_a).unwrap_or(false);
    for f in files {
        for link in &f.links {
            for p in &link.predicates {
                // is_a:: → prefer conforms_to:: (advisory; ADR-4503).
                if p == "is_a" && !accept_is_a {
                    lints.push(PredicateLint {
                        kind: "predicate-prefer-conforms-to",
                        level: LintLevel::Warning,
                        message: format!(
                            "`is_a::` asserts terminal identity — prefer `conforms_to::[[X Form Contract]]` (revisable compliance)"
                        ),
                        file: f.path.clone(),
                        line: link.line,
                    });
                }

                // Strict vocabulary: undeclared predicate.
                if let Some(cfg) = config {
                    if !cfg.is_declared(p) {
                        let level = if cfg.enforce {
                            LintLevel::Error
                        } else {
                            LintLevel::Warning
                        };
                        let near = nearest(p, cfg.predicates.keys().map(|s| s.as_str()));
                        let hint = near
                            .map(|n| format!(" (did you mean `{n}`?)"))
                            .unwrap_or_default();
                        lints.push(PredicateLint {
                            kind: "predicate-undeclared",
                            level,
                            message: format!("`{p}::` is not in the declared vocabulary{hint}"),
                            file: f.path.clone(),
                            line: link.line,
                        });
                    }
                }

                // CURIE whose prefix is not declared in [prefixes]: still valid
                // (opaque), but RDF export falls back to the vault namespace.
                if is_curie_predicate(p) {
                    let prefix = p.split_once(':').map(|(pre, _)| pre).unwrap_or("");
                    let declared = config
                        .map(|c| c.prefixes.contains_key(prefix))
                        .unwrap_or(false);
                    if !declared {
                        lints.push(PredicateLint {
                            kind: "predicate-undeclared-prefix",
                            level: LintLevel::Info,
                            message: format!(
                                "CURIE prefix `{prefix}:` is not declared in [prefixes]; RDF export will fall back to the vault namespace"
                            ),
                            file: f.path.clone(),
                            line: link.line,
                        });
                    }
                }
            }
        }
    }

    // ── predicate-drift (one report per low-frequency suspected typo) ───────
    for (p, &cp) in &freq {
        if let Some((q, cq)) = freq.iter().find(|(q, &cq)| {
            *q != p && cq >= cp.saturating_mul(DRIFT_ASYMMETRY) && levenshtein(p, q) <= 2
        }) {
            let (file, line) = first_seen.get(p).cloned().unwrap_or_default();
            lints.push(PredicateLint {
                kind: "predicate-drift",
                level: LintLevel::Warning,
                message: format!(
                    "`{p}` ({cp} use{}) is close to `{q}` ({cq} uses) — possible synonym/typo; \
                     run `zetl edges --by-predicate` to audit the vocabulary",
                    if cp == 1 { "" } else { "s" }
                ),
                file,
                line,
            });
        }
    }

    // ── predicate-relates-to-overuse (per page) ─────────────────────────────
    for f in files {
        let mut typed = 0usize;
        let mut relates = 0usize;
        let mut relates_line = 0u32;
        for link in &f.links {
            for p in &link.predicates {
                typed += 1;
                if p == "relates_to" {
                    relates += 1;
                    if relates_line == 0 {
                        relates_line = link.line;
                    }
                }
            }
        }
        if typed >= 2 && (relates as f64) > (typed as f64) * RELATES_TO_OVERUSE_FRACTION {
            lints.push(PredicateLint {
                kind: "predicate-relates-to-overuse",
                level: LintLevel::Info,
                message: format!(
                    "`relates_to::` is {relates}/{typed} of this page's typed edges — consider more specific predicates"
                ),
                file: f.path.clone(),
                line: relates_line,
            });
        }
    }

    // Deterministic ordering.
    lints.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.kind.cmp(b.kind))
            .then_with(|| a.message.cmp(&b.message))
    });
    lints
}

/// The closest candidate within Levenshtein distance ≤ 2, if any. Shared with
/// `zetl edges`'s "did you mean" suggestion (REQ-4508 nearest-match reuse).
pub fn nearest<'a>(target: &str, candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    candidates
        .map(|c| (c, levenshtein(target, c)))
        .filter(|&(c, d)| d <= 2 && c != target)
        .min_by_key(|&(_, d)| d)
        .map(|(c, _)| c)
}

/// Classic Levenshtein edit distance (small strings; two-row DP).
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::WikiLink;
    use std::time::SystemTime;

    fn file(name: &str, links: Vec<(&str, u32, Vec<&str>)>) -> ParsedFile {
        ParsedFile {
            path: PathBuf::from(format!("{name}.md")),
            page_name: name.to_string(),
            links: links
                .into_iter()
                .map(|(target, line, preds)| WikiLink {
                    target_page: target.to_string(),
                    raw_target: target.to_string(),
                    heading: None,
                    block_ref: None,
                    alias: None,
                    is_embed: false,
                    predicates: preds.into_iter().map(String::from).collect(),
                    annotation: None,
                    line,
                    column: 1,
                })
                .collect(),
            spl_blocks: vec![],
            diagnostics: vec![],
            mtime: SystemTime::now(),
            merkle_leaves: vec![],
            file_merkle: None,
        }
    }

    #[test]
    fn drift_fires_on_low_freq_near_high_freq() {
        // informed_by ×6, inspired_by ×1 (levenshtein 4 — NOT near). Use a
        // genuine typo: informed_by vs infrmed_by (distance 1).
        let mut links = vec![("A", 1, vec!["infrmed_by"])];
        for i in 0..6 {
            links.push(("B", 10 + i, vec!["informed_by"]));
        }
        let lints = compute_predicate_lints(&[file("Note", links)], None);
        assert!(lints.iter().any(|l| l.kind == "predicate-drift"
            && l.message.contains("infrmed_by")
            && l.level == LintLevel::Warning));
    }

    #[test]
    fn is_a_suggests_conforms_to() {
        let lints = compute_predicate_lints(&[file("P", vec![("X", 3, vec!["is_a"])])], None);
        assert!(lints
            .iter()
            .any(|l| l.kind == "predicate-prefer-conforms-to" && l.line == 3));
    }

    #[test]
    fn emergent_default_never_errors() {
        let files = vec![file("P", vec![("X", 1, vec!["whatever_new"]), ("Y", 2, vec!["is_a"])])];
        let lints = compute_predicate_lints(&files, None);
        assert!(lints.iter().all(|l| l.level != LintLevel::Error));
    }

    #[test]
    fn strict_enforce_errors_on_undeclared() {
        let cfg = PredicatesConfig::from_toml_str(
            "enforce = true\n[predicates]\nderived_from = {}\n",
        )
        .unwrap();
        let files = vec![file("P", vec![("X", 5, vec!["made_up"])])];
        let lints = compute_predicate_lints(&files, Some(&cfg));
        let undeclared: Vec<_> = lints.iter().filter(|l| l.kind == "predicate-undeclared").collect();
        assert_eq!(undeclared.len(), 1);
        assert_eq!(undeclared[0].level, LintLevel::Error);
        // A declared predicate is not flagged.
        let ok = compute_predicate_lints(&[file("Q", vec![("X", 1, vec!["derived_from"])])], Some(&cfg));
        assert!(!ok.iter().any(|l| l.kind == "predicate-undeclared"));
    }

    #[test]
    fn crystallising_warns_not_errors() {
        let cfg = PredicatesConfig::from_toml_str("enforce = false\n[predicates]\nderived_from = {}\n").unwrap();
        let lints = compute_predicate_lints(&[file("P", vec![("X", 1, vec!["made_up"])])], Some(&cfg));
        let u = lints.iter().find(|l| l.kind == "predicate-undeclared").unwrap();
        assert_eq!(u.level, LintLevel::Warning);
    }

    #[test]
    fn relates_to_overuse_flagged() {
        let files = vec![file(
            "P",
            vec![
                ("A", 1, vec!["relates_to"]),
                ("B", 2, vec!["relates_to"]),
                ("C", 3, vec!["derived_from"]),
            ],
        )];
        let lints = compute_predicate_lints(&files, None);
        assert!(lints.iter().any(|l| l.kind == "predicate-relates-to-overuse"));
    }

    #[test]
    fn undeclared_prefix_info_in_emergent() {
        let lints = compute_predicate_lints(
            &[file("P", vec![("X", 1, vec!["prov:wasDerivedFrom"])])],
            None,
        );
        assert!(lints
            .iter()
            .any(|l| l.kind == "predicate-undeclared-prefix" && l.level == LintLevel::Info));
    }
}
