//! Core data types for SPL extraction and defeasible reasoning.
//!
//! These types implement the data model defined in SPEC-005 §5.3.
//! All types derive `Serialize` for JSON CLI output.

use serde::Serialize;
use spindle_core::Literal;
use std::path::PathBuf;

/// An extracted SPL block from a Markdown file or standalone `.spl` file.
#[derive(Debug, Clone, Serialize)]
pub struct SplBlock {
    /// Relative path from vault root.
    pub source_file: PathBuf,
    /// Page name (filename sans extension).
    pub source_page: String,
    /// 1-indexed line of opening ``` fence (or 1 for `.spl` files).
    pub start_line: u32,
    /// Line of closing ``` fence (or last line for `.spl` files).
    pub end_line: u32,
    /// Raw SPL text between fences.
    pub content: String,
}

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
