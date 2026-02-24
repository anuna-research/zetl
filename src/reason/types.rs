//! Core data types for SPL extraction and defeasible reasoning.
//!
//! These types implement the data model defined in SPEC-005 §5.3.
//! All output types derive `Serialize` for JSON CLI output.

use serde::Serialize;
use spindle_core::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::types::{Diagnostic, ExplicitGrounding};

/// The type of a rule in the defeasible logic theory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum RuleType {
    /// Strict rule (`->`): cannot be defeated.
    Strict,
    /// Defeasible rule (`=>`): can be defeated by stronger evidence.
    Defeasible,
    /// Defeater (`~>`): blocks conclusions but doesn't prove anything.
    Defeater,
}

/// A rule with source provenance.
#[derive(Debug, Clone, Serialize)]
pub struct ProvenancedRule {
    /// Rule label (e.g., "r1", "r-prefer-redis").
    pub label: String,
    /// Type of rule.
    pub rule_type: RuleType,
    /// Body literals (antecedents/premises).
    pub body: Vec<Literal>,
    /// Head literal (consequent/conclusion).
    pub head: Literal,
    /// Source file relative to vault root.
    pub source_file: PathBuf,
    /// Absolute line number in the source file.
    pub source_line: u32,
    /// Page name.
    pub source_page: String,
}

/// A fact with source provenance.
#[derive(Debug, Clone, Serialize)]
pub struct ProvenancedFact {
    /// The asserted literal.
    pub literal: Literal,
    /// Source file relative to vault root.
    pub source_file: PathBuf,
    /// Absolute line number in the source file.
    pub source_line: u32,
    /// Page name.
    pub source_page: String,
}

/// The type of a conclusion in defeasible logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum ConclusionType {
    /// Definitely provable.
    #[serde(rename = "+D")]
    DefinitelyProvable,
    /// Definitely not provable.
    #[serde(rename = "-D")]
    DefinitelyNotProvable,
    /// Defeasibly provable.
    #[serde(rename = "+d")]
    DefeasiblyProvable,
    /// Defeasibly not provable.
    #[serde(rename = "-d")]
    DefeasiblyNotProvable,
}

/// A conclusion with its proof provenance.
#[derive(Debug, Clone, Serialize)]
pub struct ProvenancedConclusion {
    /// The concluded literal (display form).
    pub literal: String,
    /// Type of conclusion (+D, -D, +d, -d).
    pub conclusion_type: ConclusionType,
    /// Documents contributing to this conclusion.
    pub proof_sources: Vec<ProofSource>,
}

/// A source document reference within a proof.
#[derive(Debug, Clone, Serialize)]
pub struct ProofSource {
    /// Page name.
    pub page: String,
    /// File path relative to vault root.
    pub path: PathBuf,
    /// Line number in the source file.
    pub line: u32,
    /// Rule label (if from a rule, not a bare fact).
    pub rule_label: Option<String>,
    /// How this source contributes to the conclusion.
    ///
    /// One of: `"fact"`, `"strict_rule"`, `"defeasible_rule"`,
    /// `"defeater"`, `"superiority"`.
    pub contribution: String,
}

/// Result of building and reasoning over a theory from SPL blocks.
///
/// Returned by [`build_theory`](super::build_theory). Contains the combined
/// spindle-core `Theory`, provenanced conclusions, and diagnostics.
pub struct TheoryResult {
    /// The combined spindle-core Theory.
    pub theory: Theory,
    /// Provenanced conclusions from defeasible reasoning.
    pub conclusions: Vec<ProvenancedConclusion>,
    /// Provenanced rules (non-fact) extracted from SPL blocks.
    pub rules: Vec<ProvenancedRule>,
    /// Provenanced facts extracted from SPL blocks.
    pub facts: Vec<ProvenancedFact>,
    /// Parse errors, validation warnings, and other diagnostics.
    pub diagnostics: Vec<Diagnostic>,
    /// Summary statistics.
    pub summary: TheorySummary,
    /// Explicit source groundings extracted from `(meta LABEL (source ...))` forms (REQ-042).
    ///
    /// Keyed by `"<path>:<start_line>"` matching the SPL block that contained
    /// the meta declaration. Values are the raw groundings with unresolved `source_refs`.
    pub groundings_by_block: HashMap<String, Vec<ExplicitGrounding>>,
}

/// Summary statistics for the theory and reasoning results.
#[derive(Debug, Clone, Serialize)]
pub struct TheorySummary {
    /// Number of facts in the theory.
    pub fact_count: usize,
    /// Number of non-fact rules (strict + defeasible).
    pub rule_count: usize,
    /// Number of defeater rules.
    pub defeater_count: usize,
    /// Number of superiority relations.
    pub superiority_count: usize,
    /// Number of distinct source files contributing SPL.
    pub source_file_count: usize,
    /// Number of +D conclusions.
    pub definitely_provable: usize,
    /// Number of -D conclusions.
    pub definitely_not_provable: usize,
    /// Number of +d conclusions.
    pub defeasibly_provable: usize,
    /// Number of -d conclusions.
    pub defeasibly_not_provable: usize,
}
