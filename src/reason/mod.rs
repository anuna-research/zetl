//! Defeasible reasoning over Markdown vaults using spindle-core.
//!
//! This module is gated behind the `reason` Cargo feature flag.
//! It provides theory construction and reasoning capabilities
//! as specified in SPEC-005 §5.5.
//!
//! # Pipeline
//!
//! 1. **Parse** — feed each `SplBlock.content` to spindle-parser
//! 2. **Annotate provenance** — attach `_source_file`, `_source_line`, `_source_page`
//!    metadata to each rule/fact via spindle-core Meta API (ADR-007)
//! 3. **Combine** — union all fragments into a single Theory, detect duplicate rule labels
//! 4. **Validate** — check undefined labels in superiority relations, unreachable body literals
//! 5. **Reason** — run spindle-core StandardReasoner, produce conclusions (+D/-D/+d/-d)
//! 6. **Annotate conclusions** — trace proof sources from rule metadata

pub mod types;

use crate::cache::{CachedRuleType, TheoryCache};
use crate::types::{Diagnostic, DiagnosticLevel, ExplicitGrounding, SplBlock};
use anyhow::{Context, Result};
use spindle_core::prelude::{
    Conclusion, ConclusionType as CoreConclusionType, Literal, MetaValue, Rule,
    RuleType as CoreRuleType, Theory,
};
use spindle_parser::parse_spl;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use self::types::{
    ConclusionType, ProofSource, ProvenancedConclusion, ProvenancedFact, ProvenancedRule, RuleType,
    TheoryResult, TheorySummary,
};

// ── Source-ref classification (REQ-042) ──────────────────────────────────────

/// The parsed type of a raw source reference string from a `(meta LABEL (source ...))` form.
///
/// Used to classify strings extracted from SPL `source` meta values before any
/// resolution takes place.  Classification is purely syntactic — no file I/O
/// or hash look-ups are performed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceRefType {
    /// A BLAKE3 hash reference: 8 or more lowercase or uppercase hex characters.
    HashRef,
    /// A local block reference within the same file: starts with `^`.
    LocalRef,
    /// A cross-file block reference: `[[Page^block-id]]` pattern.
    CrossFileRef,
    /// Unrecognised format; preserved verbatim for forward-compatibility.
    Unknown,
}

/// Classify a raw source reference string by its syntactic format (REQ-042).
///
/// Rules (checked in order):
/// 1. `[[...^...]]` → [`SourceRefType::CrossFileRef`]
/// 2. `^...`        → [`SourceRefType::LocalRef`]
/// 3. 8+ hex chars  → [`SourceRefType::HashRef`]
/// 4. anything else → [`SourceRefType::Unknown`]
pub fn classify_source_ref(s: &str) -> SourceRefType {
    if s.starts_with("[[") && s.ends_with("]]") && s.contains('^') {
        return SourceRefType::CrossFileRef;
    }
    if s.starts_with('^') {
        return SourceRefType::LocalRef;
    }
    if s.len() >= 8 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F')) {
        return SourceRefType::HashRef;
    }
    SourceRefType::Unknown
}

/// Extract explicit source groundings from a locally-parsed SPL block theory (REQ-042).
///
/// Walks the theory's metadata looking for `"source"` keys.  Each label that
/// carries a `MetaValue::String` or `MetaValue::List` under `"source"` becomes
/// one [`ExplicitGrounding`] with the raw, unresolved `source_refs`.
///
/// Internal spindle labels (those starting with `_`) are skipped.
fn extract_source_groundings(theory: &Theory) -> Vec<ExplicitGrounding> {
    let mut result = Vec::new();
    for (label, meta) in theory.metadata() {
        // Skip internal provenance labels injected by the pipeline.
        if label.starts_with('_') {
            continue;
        }
        let source_refs: Vec<String> = match meta.properties.get("source") {
            Some(MetaValue::String(s)) => vec![s.clone()],
            Some(MetaValue::List(list)) => list.clone(),
            _ => continue,
        };
        if source_refs.is_empty() {
            continue;
        }
        result.push(ExplicitGrounding {
            construct: label.clone(),
            source_refs,
            targets: vec![],
        });
    }
    result
}

/// Build a combined theory from extracted SPL blocks, reason over it,
/// and return provenanced conclusions with diagnostics.
///
/// This is the main entry point for the reasoning pipeline (SPEC-005 §5.5).
/// Parse errors are handled gracefully: errored blocks are excluded, and the
/// pipeline continues with a partial theory, collecting diagnostics.
pub fn build_theory(spl_blocks: &[SplBlock]) -> Result<TheoryResult> {
    if spl_blocks.is_empty() {
        return Ok(TheoryResult {
            theory: Theory::new(),
            conclusions: vec![],
            rules: vec![],
            facts: vec![],
            diagnostics: vec![],
            summary: TheorySummary {
                fact_count: 0,
                rule_count: 0,
                defeater_count: 0,
                superiority_count: 0,
                source_file_count: 0,
                definitely_provable: 0,
                definitely_not_provable: 0,
                defeasibly_provable: 0,
                defeasibly_not_provable: 0,
            },
            groundings_by_block: HashMap::new(),
        });
    }

    let mut combined = Theory::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut provenanced_rules: Vec<ProvenancedRule> = Vec::new();
    let mut provenanced_facts: Vec<ProvenancedFact> = Vec::new();
    // label -> (file, line, page) for provenance lookup during conclusion annotation
    let mut label_origins: HashMap<String, (PathBuf, u32, String)> = HashMap::new();
    let mut fact_counter = 0u32;
    // Explicit source groundings extracted per block (REQ-042).
    let mut groundings_by_block: HashMap<String, Vec<ExplicitGrounding>> = HashMap::new();

    // ── Phase 1–3: Parse, annotate provenance, combine ────────────────
    for block in spl_blocks {
        // Phase 1: Parse
        let parsed = match parse_spl(&block.content) {
            Ok(theory) => theory,
            Err(e) => {
                diagnostics.push(make_parse_diagnostic(block, &e));
                continue;
            }
        };

        // Extract explicit source groundings from this block's local theory (REQ-042).
        let block_groundings = extract_source_groundings(&parsed);
        if !block_groundings.is_empty() {
            let key = format!("{}:{}", block.source_file.display(), block.start_line);
            groundings_by_block.insert(key, block_groundings);
        }

        // Phase 2: Scan source text for construct line numbers
        let constructs = scan_construct_lines(&block.content);

        // Phase 3: Transfer rules with provenance into combined theory
        for rule in parsed.rules() {
            let content_line = find_rule_line(&constructs, rule);
            let abs_line = absolute_line(block, content_line);

            if rule.rule_type == CoreRuleType::Fact {
                // Re-label facts to avoid auto-generated label collisions across blocks
                fact_counter += 1;
                let new_label = format!("__fact_{fact_counter}");
                let head_lit = rule.head[0].clone();
                let new_rule = Rule::fact(&new_label, head_lit.clone());
                combined.add_rule(new_rule);

                combined.add_meta_string(
                    &new_label,
                    "_source_file",
                    &block.source_file.display().to_string(),
                );
                combined.add_meta_string(&new_label, "_source_line", &abs_line.to_string());
                combined.add_meta_string(&new_label, "_source_page", &block.source_page);

                provenanced_facts.push(ProvenancedFact {
                    literal: head_lit,
                    source_file: block.source_file.clone(),
                    source_line: abs_line,
                    source_page: block.source_page.clone(),
                });

                label_origins.insert(
                    new_label,
                    (
                        block.source_file.clone(),
                        abs_line,
                        block.source_page.clone(),
                    ),
                );
            } else {
                // Check for duplicate explicit labels
                if let Some((prev_file, prev_line, _)) = label_origins.get(&rule.label) {
                    diagnostics.push(Diagnostic {
                        level: DiagnosticLevel::Warning,
                        message: format!(
                            "Duplicate rule label '{}' (also defined in {}:{})",
                            rule.label,
                            prev_file.display(),
                            prev_line
                        ),
                        file: block.source_file.clone(),
                        line: abs_line,
                        column: 0,
                    });
                }

                combined.add_rule(rule.clone());

                combined.add_meta_string(
                    &rule.label,
                    "_source_file",
                    &block.source_file.display().to_string(),
                );
                combined.add_meta_string(&rule.label, "_source_line", &abs_line.to_string());
                combined.add_meta_string(&rule.label, "_source_page", &block.source_page);

                let our_rule_type = convert_rule_type(rule.rule_type);
                provenanced_rules.push(ProvenancedRule {
                    label: rule.label.clone(),
                    rule_type: our_rule_type,
                    body: rule.body.iter().cloned().collect(),
                    head: rule.head[0].clone(),
                    source_file: block.source_file.clone(),
                    source_line: abs_line,
                    source_page: block.source_page.clone(),
                });

                label_origins.insert(
                    rule.label.clone(),
                    (
                        block.source_file.clone(),
                        abs_line,
                        block.source_page.clone(),
                    ),
                );
            }
        }

        // Transfer superiority relations
        for sup in parsed.superiorities() {
            combined.add_superiority(&sup.superior, &sup.inferior);
        }
    }

    // ── Phase 4: Validate ─────────────────────────────────────────────
    validate_theory(&combined, &label_origins, &mut diagnostics);

    // ── Phase 5: Reason ───────────────────────────────────────────────
    let conclusions =
        spindle_core::reason::reason(&combined).context("spindle-core reasoning failed")?;

    // ── Phase 6: Annotate conclusions ─────────────────────────────────
    let provenanced_conclusions = annotate_conclusions(&combined, &conclusions, &label_origins);

    let summary = build_summary(&combined, &provenanced_conclusions);

    Ok(TheoryResult {
        theory: combined,
        conclusions: provenanced_conclusions,
        rules: provenanced_rules,
        facts: provenanced_facts,
        diagnostics,
        summary,
        groundings_by_block,
    })
}

/// Reconstruct a theory from cached data and re-reason (phases 5–6 only).
///
/// This skips the expensive parse + annotate + combine phases by rebuilding
/// the spindle-core `Theory` from the serialized rule/superiority data in the
/// cache.  Reasoning and conclusion annotation are always re-run.
pub fn build_theory_from_cache(cache: &TheoryCache) -> Result<TheoryResult> {
    let mut combined = Theory::new();
    let mut provenanced_rules: Vec<ProvenancedRule> = Vec::new();
    let mut provenanced_facts: Vec<ProvenancedFact> = Vec::new();
    let mut label_origins: HashMap<String, (PathBuf, u32, String)> = HashMap::new();

    for cached_rule in &cache.rules {
        let core_type = match cached_rule.rule_type {
            CachedRuleType::Fact => CoreRuleType::Fact,
            CachedRuleType::Strict => CoreRuleType::Strict,
            CachedRuleType::Defeasible => CoreRuleType::Defeasible,
            CachedRuleType::Defeater => CoreRuleType::Defeater,
        };

        let body_lits: Vec<Literal> = cached_rule.body.iter().map(|s| parse_literal(s)).collect();
        let head_lits: Vec<Literal> = cached_rule.head.iter().map(|s| parse_literal(s)).collect();

        let rule = Rule::new(
            &cached_rule.label,
            core_type,
            body_lits.clone(),
            head_lits.clone(),
        );
        combined.add_rule(rule);

        // Restore provenance metadata on the theory.
        combined.add_meta_string(
            &cached_rule.label,
            "_source_file",
            &cached_rule.source_file.display().to_string(),
        );
        combined.add_meta_string(
            &cached_rule.label,
            "_source_line",
            &cached_rule.source_line.to_string(),
        );
        combined.add_meta_string(&cached_rule.label, "_source_page", &cached_rule.source_page);

        label_origins.insert(
            cached_rule.label.clone(),
            (
                cached_rule.source_file.clone(),
                cached_rule.source_line,
                cached_rule.source_page.clone(),
            ),
        );

        if cached_rule.rule_type == CachedRuleType::Fact {
            if let Some(head_lit) = head_lits.into_iter().next() {
                provenanced_facts.push(ProvenancedFact {
                    literal: head_lit,
                    source_file: cached_rule.source_file.clone(),
                    source_line: cached_rule.source_line,
                    source_page: cached_rule.source_page.clone(),
                });
            }
        } else {
            provenanced_rules.push(ProvenancedRule {
                label: cached_rule.label.clone(),
                rule_type: match cached_rule.rule_type {
                    CachedRuleType::Strict => RuleType::Strict,
                    CachedRuleType::Defeasible => RuleType::Defeasible,
                    CachedRuleType::Defeater => RuleType::Defeater,
                    CachedRuleType::Fact => RuleType::Strict, // unreachable in this branch
                },
                body: body_lits,
                head: head_lits.into_iter().next().unwrap_or_default(),
                source_file: cached_rule.source_file.clone(),
                source_line: cached_rule.source_line,
                source_page: cached_rule.source_page.clone(),
            });
        }
    }

    // Restore superiority relations.
    for (superior, inferior) in &cache.superiorities {
        combined.add_superiority(superior, inferior);
    }

    // Phase 5: Reason
    let conclusions =
        spindle_core::reason::reason(&combined).context("spindle-core reasoning failed")?;

    // Phase 6: Annotate conclusions
    let provenanced_conclusions = annotate_conclusions(&combined, &conclusions, &label_origins);
    let summary = build_summary(&combined, &provenanced_conclusions);

    // Restore explicit source groundings from the cached spl_blocks (REQ-042).
    let groundings_by_block: HashMap<String, Vec<ExplicitGrounding>> = cache
        .spl_blocks
        .iter()
        .filter(|(_, b)| !b.explicit_groundings.is_empty())
        .map(|(k, b)| (k.clone(), b.explicit_groundings.clone()))
        .collect();

    Ok(TheoryResult {
        theory: combined,
        conclusions: provenanced_conclusions,
        rules: provenanced_rules,
        facts: provenanced_facts,
        diagnostics: cache.diagnostics.clone(),
        summary,
        groundings_by_block,
    })
}

/// Parse a literal from its display-form string (e.g. "bird", "~flies").
fn parse_literal(s: &str) -> Literal {
    if let Some(name) = s.strip_prefix('~') {
        Literal::negated(name)
    } else {
        Literal::simple(s)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────

/// Information about a construct (fact, rule, defeater, superiority) found
/// during a line-by-line scan of SPL source text.
struct ConstructInfo {
    kind: ConstructKind,
    /// Rule label or literal name (empty for superiority relations).
    identifier: String,
    /// 1-indexed line number within the SPL block content.
    line: u32,
}

#[derive(Debug, PartialEq)]
enum ConstructKind {
    Fact,
    StrictRule,
    DefeasibleRule,
    Defeater,
    Superiority,
}

/// Scan SPL text line-by-line to find where each construct starts.
///
/// This is used to determine the source line number for each rule/fact,
/// since spindle-parser doesn't provide per-rule line info.
fn scan_construct_lines(spl_text: &str) -> Vec<ConstructInfo> {
    let mut results = Vec::new();
    for (idx, line) in spl_text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') {
            continue;
        }
        let line_num = (idx + 1) as u32;

        if let Some(rest) = trimmed.strip_prefix("(given") {
            let id = extract_first_token(rest.trim_start());
            results.push(ConstructInfo {
                kind: ConstructKind::Fact,
                identifier: id,
                line: line_num,
            });
        } else if let Some(rest) = trimmed.strip_prefix("(normally") {
            let id = extract_first_token(rest.trim_start());
            results.push(ConstructInfo {
                kind: ConstructKind::DefeasibleRule,
                identifier: id,
                line: line_num,
            });
        } else if let Some(rest) = trimmed.strip_prefix("(always") {
            let id = extract_first_token(rest.trim_start());
            results.push(ConstructInfo {
                kind: ConstructKind::StrictRule,
                identifier: id,
                line: line_num,
            });
        } else if let Some(rest) = trimmed.strip_prefix("(except") {
            let id = extract_first_token(rest.trim_start());
            results.push(ConstructInfo {
                kind: ConstructKind::Defeater,
                identifier: id,
                line: line_num,
            });
        } else if trimmed.starts_with("(prefer") {
            results.push(ConstructInfo {
                kind: ConstructKind::Superiority,
                identifier: String::new(),
                line: line_num,
            });
        }
    }
    results
}

/// Extract the first token from remaining text after a keyword.
///
/// Handles parenthesized expressions like `(not bird)` by returning
/// the full expression. For simple tokens, returns until whitespace or `)`.
fn extract_first_token(s: &str) -> String {
    if s.starts_with('(') {
        // Parenthesized expression — match balanced parens
        let mut depth = 0;
        for (i, c) in s.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return s[..=i].to_string();
                    }
                }
                _ => {}
            }
        }
    }
    // Simple token: take until whitespace or closing paren
    s.split(|c: char| c.is_whitespace() || c == ')')
        .next()
        .unwrap_or("")
        .to_string()
}

/// Find the source line number for a rule within the constructs list.
///
/// Matches by rule label for named rules, or by literal name for facts.
/// Falls back to line 1 if no match is found.
fn find_rule_line(constructs: &[ConstructInfo], rule: &Rule) -> u32 {
    if rule.rule_type == CoreRuleType::Fact {
        let head = &rule.head[0];
        let lit_name = head.name();
        let lit_display = head.to_string();
        constructs
            .iter()
            .find(|c| {
                c.kind == ConstructKind::Fact
                    && (c.identifier == lit_name || c.identifier == lit_display)
            })
            .map(|c| c.line)
            .unwrap_or(1)
    } else {
        let label = &rule.label;
        let expected_kind = match rule.rule_type {
            CoreRuleType::Strict => ConstructKind::StrictRule,
            CoreRuleType::Defeasible => ConstructKind::DefeasibleRule,
            CoreRuleType::Defeater => ConstructKind::Defeater,
            _ => return 1,
        };
        constructs
            .iter()
            .find(|c| c.kind == expected_kind && c.identifier == *label)
            .map(|c| c.line)
            .unwrap_or(1)
    }
}

/// Convert a content-relative line number to an absolute file line number.
///
/// For Markdown files: `start_line` is the fence line; content starts on the next line.
/// For `.spl` files: `start_line` is 1; content IS the file.
fn absolute_line(block: &SplBlock, content_line: u32) -> u32 {
    let is_spl_file = block.source_file.extension().and_then(|e| e.to_str()) == Some("spl");

    if is_spl_file {
        // .spl files: start_line=1, content starts at line 1
        block.start_line + content_line - 1
    } else {
        // Markdown: start_line is the fence, content starts at start_line + 1
        block.start_line + content_line
    }
}

/// Create a Diagnostic from a spindle-parser parse error.
fn make_parse_diagnostic(block: &SplBlock, error: &spindle_parser::ParseError) -> Diagnostic {
    let (parser_line, message) = parse_error_info(error);
    let abs_line = if parser_line > 0 {
        absolute_line(block, parser_line)
    } else {
        block.start_line
    };

    Diagnostic {
        level: DiagnosticLevel::Error,
        message: format!("SPL parse error: {message}"),
        file: block.source_file.clone(),
        line: abs_line,
        column: 0,
    }
}

/// Extract line number and message from a ParseError.
fn parse_error_info(error: &spindle_parser::ParseError) -> (u32, String) {
    let line = error.line().unwrap_or(0) as u32;
    let message = format!("{error}");
    (line, message)
}

/// Convert spindle-core's RuleType to our RuleType (excluding Fact).
fn convert_rule_type(rt: CoreRuleType) -> RuleType {
    match rt {
        CoreRuleType::Strict => RuleType::Strict,
        CoreRuleType::Defeasible => RuleType::Defeasible,
        CoreRuleType::Defeater => RuleType::Defeater,
        // Facts are handled separately; this shouldn't be called for facts
        CoreRuleType::Fact => RuleType::Strict,
    }
}

/// Convert spindle-core's ConclusionType to ours.
fn convert_conclusion_type(ct: CoreConclusionType) -> ConclusionType {
    match ct {
        CoreConclusionType::DefinitelyProvable => ConclusionType::DefinitelyProvable,
        CoreConclusionType::DefinitelyNotProvable => ConclusionType::DefinitelyNotProvable,
        CoreConclusionType::DefeasiblyProvable => ConclusionType::DefeasiblyProvable,
        CoreConclusionType::DefeasiblyNotProvable => ConclusionType::DefeasiblyNotProvable,
    }
}

/// Phase 4: Validate the combined theory.
///
/// Checks for:
/// - Undefined labels in superiority relations
/// - Body literals that appear in no rule head or fact (possible typos)
fn validate_theory(
    theory: &Theory,
    label_origins: &HashMap<String, (PathBuf, u32, String)>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let rule_labels: HashSet<&str> = theory.rules().map(|r| r.label.as_str()).collect();

    // Check 1: Undefined labels in superiority relations
    for sup in theory.superiorities() {
        if !rule_labels.contains(sup.superior.as_str()) {
            diagnostics.push(Diagnostic {
                level: DiagnosticLevel::Warning,
                message: format!(
                    "Superiority references undefined rule label '{}'",
                    sup.superior
                ),
                file: PathBuf::new(),
                line: 0,
                column: 0,
            });
        }
        if !rule_labels.contains(sup.inferior.as_str()) {
            diagnostics.push(Diagnostic {
                level: DiagnosticLevel::Warning,
                message: format!(
                    "Superiority references undefined rule label '{}'",
                    sup.inferior
                ),
                file: PathBuf::new(),
                line: 0,
                column: 0,
            });
        }
    }

    // Check 2: Body literals with no matching head or fact
    let all_heads: HashSet<String> = theory
        .rules()
        .flat_map(|r| r.head.iter().map(|h| h.to_string()))
        .collect();

    for rule in theory.rules() {
        if rule.rule_type == CoreRuleType::Fact {
            continue;
        }
        for body_lit in &rule.body {
            let lit_str = body_lit.to_string();
            if !all_heads.contains(&lit_str) {
                if let Some((file, line, _)) = label_origins.get(&rule.label) {
                    diagnostics.push(Diagnostic {
                        level: DiagnosticLevel::Warning,
                        message: format!(
                            "Body literal '{}' in rule '{}' appears in no rule head or fact (possible typo)",
                            lit_str, rule.label
                        ),
                        file: file.clone(),
                        line: *line,
                        column: 0,
                    });
                }
            }
        }
    }
}

/// Phase 6: Annotate conclusions with proof provenance.
///
/// For each conclusion, trace back through the theory's metadata to find
/// the source documents that contributed to the proof.
fn annotate_conclusions(
    theory: &Theory,
    conclusions: &[Conclusion],
    label_origins: &HashMap<String, (PathBuf, u32, String)>,
) -> Vec<ProvenancedConclusion> {
    conclusions
        .iter()
        .map(|c| {
            let proof_sources = trace_proof_sources(theory, c, label_origins);
            ProvenancedConclusion {
                literal: c.literal.to_string(),
                conclusion_type: convert_conclusion_type(c.conclusion_type),
                proof_sources,
            }
        })
        .collect()
}

/// Trace proof sources for a single conclusion.
fn trace_proof_sources(
    theory: &Theory,
    conclusion: &Conclusion,
    label_origins: &HashMap<String, (PathBuf, u32, String)>,
) -> Vec<ProofSource> {
    let mut sources = Vec::new();

    // If the conclusion was derived by a specific rule, trace that rule
    if let Some(ref rule_label) = conclusion.rule_label {
        if let Some((file, line, page)) = label_origins.get(rule_label.as_str()) {
            let contribution = if let Some(rule) = theory.get_rule(rule_label) {
                match rule.rule_type {
                    CoreRuleType::Fact => "fact",
                    CoreRuleType::Strict => "strict_rule",
                    CoreRuleType::Defeasible => "defeasible_rule",
                    CoreRuleType::Defeater => "defeater",
                }
            } else {
                "unknown"
            };

            sources.push(ProofSource {
                page: page.clone(),
                path: file.clone(),
                line: *line,
                rule_label: Some(rule_label.clone()),
                contribution: contribution.to_string(),
            });

            // Also trace body literal sources for non-fact rules
            if let Some(rule) = theory.get_rule(rule_label) {
                if rule.rule_type != CoreRuleType::Fact {
                    add_body_fact_sources(theory, rule, label_origins, &mut sources);
                }
            }
        }
    } else {
        // No rule label — check if the literal is directly a fact
        let lit_str = conclusion.literal.to_string();
        for (label, (file, line, page)) in label_origins {
            if let Some(rule) = theory.get_rule(label) {
                if rule.rule_type == CoreRuleType::Fact
                    && rule.head.first().map(|h| h.to_string()).as_deref() == Some(&lit_str)
                {
                    sources.push(ProofSource {
                        page: page.clone(),
                        path: file.clone(),
                        line: *line,
                        rule_label: None,
                        contribution: "fact".to_string(),
                    });
                    break;
                }
            }
        }
    }

    sources
}

/// Add proof sources for the body literals of a rule (the facts that feed into it).
fn add_body_fact_sources(
    theory: &Theory,
    rule: &Rule,
    label_origins: &HashMap<String, (PathBuf, u32, String)>,
    sources: &mut Vec<ProofSource>,
) {
    for body_lit in &rule.body {
        let body_str = body_lit.to_string();
        // Find facts matching this body literal
        for fact in theory.facts() {
            if fact.head.first().map(|h| h.to_string()).as_deref() == Some(&body_str) {
                if let Some((file, line, page)) = label_origins.get(&fact.label) {
                    sources.push(ProofSource {
                        page: page.clone(),
                        path: file.clone(),
                        line: *line,
                        rule_label: None,
                        contribution: "fact".to_string(),
                    });
                }
                break; // One match per body literal
            }
        }
    }
}

/// Build summary statistics from the theory and conclusions.
fn build_summary(theory: &Theory, conclusions: &[ProvenancedConclusion]) -> TheorySummary {
    let source_files: HashSet<&str> = theory
        .metadata()
        .values()
        .filter_map(|meta| {
            meta.properties.get("_source_file").and_then(|v| match v {
                MetaValue::String(s) => Some(s.as_str()),
                _ => None,
            })
        })
        .collect();

    let mut definitely_provable = 0;
    let mut definitely_not_provable = 0;
    let mut defeasibly_provable = 0;
    let mut defeasibly_not_provable = 0;

    for c in conclusions {
        match c.conclusion_type {
            ConclusionType::DefinitelyProvable => definitely_provable += 1,
            ConclusionType::DefinitelyNotProvable => definitely_not_provable += 1,
            ConclusionType::DefeasiblyProvable => defeasibly_provable += 1,
            ConclusionType::DefeasiblyNotProvable => defeasibly_not_provable += 1,
        }
    }

    TheorySummary {
        fact_count: theory.facts().count(),
        rule_count: theory
            .rules()
            .filter(|r| {
                r.rule_type == CoreRuleType::Strict || r.rule_type == CoreRuleType::Defeasible
            })
            .count(),
        defeater_count: theory
            .rules()
            .filter(|r| r.rule_type == CoreRuleType::Defeater)
            .count(),
        superiority_count: theory.superiorities().len(),
        source_file_count: source_files.len(),
        definitely_provable,
        definitely_not_provable,
        defeasibly_provable,
        defeasibly_not_provable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_block(file: &str, page: &str, start: u32, content: &str) -> SplBlock {
        let end = start + content.lines().count() as u32 + 1;
        SplBlock {
            source_file: PathBuf::from(file),
            source_page: page.to_string(),
            start_line: start,
            end_line: end,
            content: content.to_string(),
        }
    }

    fn make_spl_file_block(file: &str, page: &str, content: &str) -> SplBlock {
        let end = content.lines().count().max(1) as u32;
        SplBlock {
            source_file: PathBuf::from(file),
            source_page: page.to_string(),
            start_line: 1,
            end_line: end,
            content: content.to_string(),
        }
    }

    #[test]
    fn empty_blocks_returns_empty_result() {
        let result = build_theory(&[]).unwrap();
        assert!(result.conclusions.is_empty());
        assert!(result.rules.is_empty());
        assert!(result.facts.is_empty());
        assert!(result.diagnostics.is_empty());
        assert_eq!(result.summary.fact_count, 0);
    }

    #[test]
    fn single_fact_block() {
        let block = make_block("A.md", "A", 5, "(given bird)\n");
        let result = build_theory(&[block]).unwrap();

        assert_eq!(result.facts.len(), 1);
        assert_eq!(result.facts[0].source_file, PathBuf::from("A.md"));
        assert_eq!(result.facts[0].source_line, 6); // 5 + 1
        assert_eq!(result.facts[0].source_page, "A");
        assert_eq!(result.summary.fact_count, 1);

        // Should have +D conclusion for bird
        let bird_conclusion = result.conclusions.iter().find(|c| {
            c.literal == "bird" && c.conclusion_type == ConclusionType::DefinitelyProvable
        });
        assert!(bird_conclusion.is_some(), "bird should be +D");
    }

    #[test]
    fn multi_document_theory() {
        let blocks = vec![
            make_block("A.md", "A", 5, "(given bird)\n"),
            make_block("B.md", "B", 10, "(normally r1 bird flies)\n"),
        ];
        let result = build_theory(&blocks).unwrap();

        assert_eq!(result.facts.len(), 1);
        assert_eq!(result.rules.len(), 1);
        assert_eq!(result.rules[0].label, "r1");
        assert_eq!(result.rules[0].source_file, PathBuf::from("B.md"));
        assert_eq!(result.rules[0].source_line, 11); // 10 + 1

        // bird should be +D, flies should be +d
        let bird = result.conclusions.iter().find(|c| {
            c.literal == "bird" && c.conclusion_type == ConclusionType::DefinitelyProvable
        });
        assert!(bird.is_some(), "bird should be +D");

        let flies = result.conclusions.iter().find(|c| {
            c.literal == "flies" && c.conclusion_type == ConclusionType::DefeasiblyProvable
        });
        assert!(flies.is_some(), "flies should be +d");
    }

    #[test]
    fn parse_error_excludes_block() {
        let blocks = vec![
            make_block("A.md", "A", 5, "(given bird)\n"),
            make_block("B.md", "B", 10, "(invalid syntax here\n"),
            make_block("C.md", "C", 20, "(given penguin)\n"),
        ];
        let result = build_theory(&blocks).unwrap();

        // A and C should contribute, B should be excluded
        assert_eq!(result.facts.len(), 2);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.level == DiagnosticLevel::Error && d.file == PathBuf::from("B.md")),
            "Should have parse error for B.md"
        );
    }

    #[test]
    fn superiority_and_defeat() {
        let blocks = vec![
            make_block("Birds.md", "Birds", 5, "(given bird)\n(given penguin)\n"),
            make_block("Flight.md", "Flight", 10, "(normally r1 bird flies)\n"),
            make_block(
                "Penguins.md",
                "Penguins",
                15,
                "(normally r2 penguin (not flies))\n(prefer r2 r1)\n",
            ),
        ];
        let result = build_theory(&blocks).unwrap();

        assert_eq!(result.summary.superiority_count, 1);

        // ~flies should be +d since r2 defeats r1
        let not_flies = result.conclusions.iter().find(|c| {
            c.literal.contains("flies")
                && c.literal.starts_with('~')
                && c.conclusion_type == ConclusionType::DefeasiblyProvable
        });
        assert!(
            not_flies.is_some(),
            "~flies should be +d; conclusions: {:?}",
            result
                .conclusions
                .iter()
                .map(|c| format!("{} {:?}", c.literal, c.conclusion_type))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn spl_file_line_numbers() {
        let block = make_spl_file_block(
            "theories/caching.spl",
            "caching",
            "; comment\n(given eval-redis)\n",
        );
        let result = build_theory(&[block]).unwrap();

        assert_eq!(result.facts.len(), 1);
        // Line 2 in the .spl file (line 1 is comment, line 2 is fact)
        assert_eq!(result.facts[0].source_line, 2);
    }

    #[test]
    fn duplicate_rule_label_warns() {
        let blocks = vec![
            make_block("A.md", "A", 5, "(normally r1 a b)\n"),
            make_block("B.md", "B", 10, "(normally r1 c d)\n"),
        ];
        let result = build_theory(&blocks).unwrap();

        let dup_warning = result
            .diagnostics
            .iter()
            .find(|d| d.message.contains("Duplicate rule label 'r1'"));
        assert!(
            dup_warning.is_some(),
            "Should warn about duplicate label r1"
        );
    }

    #[test]
    fn demo_vault_integration() {
        // Tests the exact SPL from the demo vault
        let blocks = vec![
            make_block(
                "decisions/Redis vs Memcached.md",
                "Redis vs Memcached",
                11,
                "(given evaluated-redis)\n(given redis-supports-persistence)\n(normally r-prefer-redis\n  (and evaluated-redis redis-supports-persistence)\n  decided-use-redis)\n",
            ),
            make_block(
                "audits/License Audit.md",
                "License Audit",
                6,
                "(given discovered-license-risk)\n(except d-license-risk discovered-license-risk (not decided-use-redis))\n",
            ),
            make_spl_file_block(
                "theories/caching.spl",
                "caching",
                "; Caching theory — standalone SPL file\n; Complements decisions in Redis vs Memcached\n\n; Memcached is a simpler alternative for stateless caching\n(given evaluated-memcached)\n\n(normally r-prefer-memcached\n  evaluated-memcached\n  decided-use-memcached)\n\n; When both options are viable, prefer Redis over Memcached\n(prefer r-prefer-redis r-prefer-memcached)\n",
            ),
            make_block(
                "processes/Deployment Checklist.md",
                "Deployment Checklist",
                8,
                "(normally r-ready-prod\n  (and verified-load-test verified-security-audit)\n  ready-for-production)\n",
            ),
        ];

        let result = build_theory(&blocks).unwrap();

        // Verify provenance
        let redis_fact = result
            .facts
            .iter()
            .find(|f| f.literal.to_string() == "evaluated-redis")
            .expect("should have evaluated-redis fact");
        assert_eq!(
            redis_fact.source_file,
            PathBuf::from("decisions/Redis vs Memcached.md")
        );
        assert_eq!(redis_fact.source_line, 12); // start_line=11 + content_line=1

        let license_fact = result
            .facts
            .iter()
            .find(|f| f.literal.to_string() == "discovered-license-risk")
            .expect("should have discovered-license-risk fact");
        assert_eq!(
            license_fact.source_file,
            PathBuf::from("audits/License Audit.md")
        );
        assert_eq!(license_fact.source_line, 7); // start_line=6 + 1

        let redis_rule = result
            .rules
            .iter()
            .find(|r| r.label == "r-prefer-redis")
            .expect("should have r-prefer-redis rule");
        assert_eq!(redis_rule.source_line, 14); // start_line=11 + 3

        let prod_rule = result
            .rules
            .iter()
            .find(|r| r.label == "r-ready-prod")
            .expect("should have r-ready-prod rule");
        assert_eq!(prod_rule.source_line, 9); // start_line=8 + 1

        // Verify summary counts
        assert_eq!(result.summary.fact_count, 4);
        assert_eq!(result.summary.superiority_count, 1);

        // decided-use-memcached should be +d (not defeated)
        let memcached = result.conclusions.iter().find(|c| {
            c.literal == "decided-use-memcached"
                && c.conclusion_type == ConclusionType::DefeasiblyProvable
        });
        assert!(
            memcached.is_some(),
            "decided-use-memcached should be +d; conclusions: {:?}",
            result
                .conclusions
                .iter()
                .map(|c| format!("{} {:?}", c.literal, c.conclusion_type))
                .collect::<Vec<_>>()
        );

        // No parse errors expected
        let parse_errors: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.level == DiagnosticLevel::Error)
            .collect();
        assert!(
            parse_errors.is_empty(),
            "no parse errors expected: {:?}",
            parse_errors
        );
    }

    #[test]
    fn scan_construct_lines_basic() {
        let spl = "; comment\n(given bird)\n(normally r1 bird flies)\n(prefer r2 r1)\n";
        let constructs = scan_construct_lines(spl);
        assert_eq!(constructs.len(), 3);
        assert_eq!(constructs[0].kind, ConstructKind::Fact);
        assert_eq!(constructs[0].identifier, "bird");
        assert_eq!(constructs[0].line, 2);
        assert_eq!(constructs[1].kind, ConstructKind::DefeasibleRule);
        assert_eq!(constructs[1].identifier, "r1");
        assert_eq!(constructs[1].line, 3);
        assert_eq!(constructs[2].kind, ConstructKind::Superiority);
        assert_eq!(constructs[2].line, 4);
    }

    #[test]
    fn absolute_line_markdown() {
        let block = make_block("test.md", "test", 5, "(given bird)\n");
        assert_eq!(absolute_line(&block, 1), 6); // 5 + 1
        assert_eq!(absolute_line(&block, 2), 7); // 5 + 2
    }

    #[test]
    fn absolute_line_spl_file() {
        let block = make_spl_file_block("test.spl", "test", "(given bird)\n");
        assert_eq!(absolute_line(&block, 1), 1); // 1 + 1 - 1
        assert_eq!(absolute_line(&block, 2), 2); // 1 + 2 - 1
    }

    #[test]
    fn theory_cache_round_trip() {
        use crate::cache::build_theory_cache;
        use crate::types::ParsedFile;
        use std::time::{Duration, UNIX_EPOCH};

        // Build theory the normal way.
        let blocks = vec![
            make_block("A.md", "A", 5, "(given bird)\n"),
            make_block("B.md", "B", 10, "(normally r1 bird flies)\n"),
        ];
        let result = build_theory(&blocks).unwrap();

        // Collect conclusions from the original build.
        let mut original_conclusions: Vec<(String, String)> = result
            .conclusions
            .iter()
            .map(|c| (c.literal.clone(), format!("{:?}", c.conclusion_type)))
            .collect();
        original_conclusions.sort();

        // Create fake ParsedFiles to simulate the pipeline (for mtime tracking).
        let mtime = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let files = vec![
            ParsedFile {
                path: PathBuf::from("A.md"),
                page_name: "A".to_string(),
                links: vec![],
                spl_blocks: vec![blocks[0].clone()],
                diagnostics: vec![],
                mtime,
                merkle_leaves: vec![],
                file_merkle: None,
            },
            ParsedFile {
                path: PathBuf::from("B.md"),
                page_name: "B".to_string(),
                links: vec![],
                spl_blocks: vec![blocks[1].clone()],
                diagnostics: vec![],
                mtime,
                merkle_leaves: vec![],
                file_merkle: None,
            },
        ];

        // Build cache.
        let cache = build_theory_cache(
            &result.theory,
            &result.diagnostics,
            &files,
            &result.groundings_by_block,
        );

        // Reconstruct from cache and re-reason.
        let cached_result = build_theory_from_cache(&cache).unwrap();

        // Verify same conclusions.
        let mut cached_conclusions: Vec<(String, String)> = cached_result
            .conclusions
            .iter()
            .map(|c| (c.literal.clone(), format!("{:?}", c.conclusion_type)))
            .collect();
        cached_conclusions.sort();

        assert_eq!(
            original_conclusions, cached_conclusions,
            "Conclusions should match between direct build and cache-based build"
        );

        // Verify same fact/rule counts.
        assert_eq!(result.facts.len(), cached_result.facts.len());
        assert_eq!(result.rules.len(), cached_result.rules.len());
        assert_eq!(
            result.summary.superiority_count,
            cached_result.summary.superiority_count
        );
    }

    #[test]
    fn theory_cache_round_trip_with_superiority() {
        use crate::cache::build_theory_cache;
        use crate::types::ParsedFile;
        use std::time::{Duration, UNIX_EPOCH};

        let blocks = vec![
            make_block("Birds.md", "Birds", 5, "(given bird)\n(given penguin)\n"),
            make_block("Flight.md", "Flight", 10, "(normally r1 bird flies)\n"),
            make_block(
                "Penguins.md",
                "Penguins",
                15,
                "(normally r2 penguin (not flies))\n(prefer r2 r1)\n",
            ),
        ];

        let result = build_theory(&blocks).unwrap();

        let mtime = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let files: Vec<ParsedFile> = blocks
            .iter()
            .map(|b| ParsedFile {
                path: b.source_file.clone(),
                page_name: b.source_page.clone(),
                links: vec![],
                spl_blocks: vec![b.clone()],
                diagnostics: vec![],
                mtime,
                merkle_leaves: vec![],
                file_merkle: None,
            })
            .collect();

        let cache = build_theory_cache(
            &result.theory,
            &result.diagnostics,
            &files,
            &result.groundings_by_block,
        );
        let cached_result = build_theory_from_cache(&cache).unwrap();

        // ~flies should be +d in both
        let has_not_flies = |r: &TheoryResult| {
            r.conclusions.iter().any(|c| {
                c.literal.contains("flies")
                    && c.literal.starts_with('~')
                    && c.conclusion_type == ConclusionType::DefeasiblyProvable
            })
        };

        assert!(has_not_flies(&result), "Original should have ~flies as +d");
        assert!(
            has_not_flies(&cached_result),
            "Cached should have ~flies as +d"
        );

        assert_eq!(
            result.summary.superiority_count,
            cached_result.summary.superiority_count
        );
    }

    #[test]
    fn theory_cache_round_trip_serialization() {
        use crate::cache::{
            build_theory_cache, collect_spl_ast_hashes, load_theory_cache, save_theory_cache,
            theory_cache_valid,
        };
        use crate::types::ParsedFile;
        use std::time::{Duration, UNIX_EPOCH};

        let blocks = vec![
            make_block("A.md", "A", 5, "(given bird)\n"),
            make_block("B.md", "B", 10, "(normally r1 bird flies)\n"),
        ];
        let result = build_theory(&blocks).unwrap();

        let mtime = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let files: Vec<ParsedFile> = blocks
            .iter()
            .map(|b| ParsedFile {
                path: b.source_file.clone(),
                page_name: b.source_page.clone(),
                links: vec![],
                spl_blocks: vec![b.clone()],
                diagnostics: vec![],
                mtime,
                merkle_leaves: vec![],
                file_merkle: None,
            })
            .collect();

        // Save to disk, load back, verify.
        let dir = tempfile::TempDir::new().unwrap();
        let vault = dir.path();

        let cache = build_theory_cache(
            &result.theory,
            &result.diagnostics,
            &files,
            &result.groundings_by_block,
        );
        save_theory_cache(vault, &cache).unwrap();

        let loaded = load_theory_cache(vault)
            .unwrap()
            .expect("cache should exist");
        let current_hashes = collect_spl_ast_hashes(&files);
        assert!(theory_cache_valid(&current_hashes, &loaded));

        let cached_result = build_theory_from_cache(&loaded).unwrap();
        assert_eq!(result.conclusions.len(), cached_result.conclusions.len());
        assert_eq!(result.facts.len(), cached_result.facts.len());
        assert_eq!(result.rules.len(), cached_result.rules.len());
    }

    // ── REQ-042: Source extraction tests ─────────────────────────────────────

    #[test]
    fn classify_source_ref_hash() {
        // 8 or more hex chars → HashRef
        assert_eq!(classify_source_ref("abcdef12"), SourceRefType::HashRef);
        assert_eq!(
            classify_source_ref("a1b2c3d4e5f6a7b8"),
            SourceRefType::HashRef
        );
        // Upper-case hex is also accepted
        assert_eq!(classify_source_ref("ABCDEF12"), SourceRefType::HashRef);
        // Fewer than 8 hex chars → Unknown
        assert_eq!(classify_source_ref("abcdef1"), SourceRefType::Unknown);
    }

    #[test]
    fn classify_source_ref_local() {
        assert_eq!(classify_source_ref("^block-id"), SourceRefType::LocalRef);
        assert_eq!(classify_source_ref("^abc123"), SourceRefType::LocalRef);
    }

    #[test]
    fn classify_source_ref_cross_file() {
        assert_eq!(
            classify_source_ref("[[Page^block-id]]"),
            SourceRefType::CrossFileRef
        );
        assert_eq!(
            classify_source_ref("[[Architecture^intro]]"),
            SourceRefType::CrossFileRef
        );
    }

    #[test]
    fn classify_source_ref_unknown() {
        assert_eq!(classify_source_ref("just-text"), SourceRefType::Unknown);
        assert_eq!(classify_source_ref(""), SourceRefType::Unknown);
        // [[...]] without ^ is not a valid cross-file ref
        assert_eq!(classify_source_ref("[[Page]]"), SourceRefType::Unknown);
    }

    #[test]
    fn source_extraction_single_string() {
        // A block with (meta r1 (source "abcdef1234567890"))
        let spl = "(normally r1 bird flies)\n(meta r1 (source abcdef1234567890))\n";
        let block = make_block("notes/A.md", "A", 10, spl);
        let result = build_theory(&[block]).unwrap();

        // One grounding for key "notes/A.md:10"
        let groundings = result.groundings_by_block.get("notes/A.md:10");
        assert!(groundings.is_some(), "should have groundings for the block");
        let groundings = groundings.unwrap();
        assert_eq!(groundings.len(), 1);
        assert_eq!(groundings[0].construct, "r1");
        assert_eq!(groundings[0].source_refs, vec!["abcdef1234567890"]);
        assert!(groundings[0].targets.is_empty(), "no resolution yet");
    }

    #[test]
    fn source_extraction_list_value() {
        // A block with (meta r1 (source ("ref1" "ref2"))) — quoted strings
        // because `^` and `[` are not valid SPL atom characters.
        let spl =
            "(normally r1 bird flies)\n(meta r1 (source (\"^block-id\" \"[[Page^other]]\")) )\n";
        let block = make_block("notes/B.md", "B", 5, spl);
        let result = build_theory(&[block]).unwrap();

        let groundings = result
            .groundings_by_block
            .get("notes/B.md:5")
            .expect("should have groundings");
        assert_eq!(groundings.len(), 1);
        assert_eq!(groundings[0].construct, "r1");
        assert_eq!(
            groundings[0].source_refs,
            vec!["^block-id", "[[Page^other]]"]
        );
    }

    #[test]
    fn source_extraction_multiple_labels() {
        // Two rules each with their own source meta.
        // `^some-block` is quoted because `^` is not a valid SPL atom character.
        let spl = "(normally r1 a b)\n(normally r2 c d)\n\
                   (meta r1 (source abcdef1234567890))\n\
                   (meta r2 (source \"^some-block\"))\n";
        let block = make_block("notes/C.md", "C", 1, spl);
        let result = build_theory(&[block]).unwrap();

        let groundings = result
            .groundings_by_block
            .get("notes/C.md:1")
            .expect("should have groundings");
        assert_eq!(groundings.len(), 2, "one grounding per labelled meta");

        let r1 = groundings.iter().find(|g| g.construct == "r1");
        let r2 = groundings.iter().find(|g| g.construct == "r2");
        assert!(r1.is_some(), "should have r1 grounding");
        assert!(r2.is_some(), "should have r2 grounding");
    }

    #[test]
    fn source_extraction_no_source_meta() {
        // Block with meta but no "source" key — should not produce groundings
        let spl = "(normally r1 bird flies)\n(meta r1 (description \"A bird rule\"))\n";
        let block = make_block("notes/D.md", "D", 1, spl);
        let result = build_theory(&[block]).unwrap();
        assert!(
            result.groundings_by_block.is_empty(),
            "no source meta → no groundings"
        );
    }

    #[test]
    fn source_extraction_persists_through_cache() {
        use crate::cache::{build_theory_cache, load_theory_cache, save_theory_cache};
        use crate::types::ParsedFile;
        use std::time::{Duration, UNIX_EPOCH};

        let spl = "(normally r1 bird flies)\n(meta r1 (source abcdef1234567890))\n";
        let block = make_block("notes/A.md", "A", 10, spl);
        let result = build_theory(&[block.clone()]).unwrap();

        assert!(
            result.groundings_by_block.contains_key("notes/A.md:10"),
            "groundings should be keyed by path:start_line"
        );

        let mtime = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let files = vec![ParsedFile {
            path: PathBuf::from("notes/A.md"),
            page_name: "A".to_string(),
            links: vec![],
            spl_blocks: vec![block],
            diagnostics: vec![],
            mtime,
            merkle_leaves: vec![],
            file_merkle: None,
        }];

        let dir = tempfile::TempDir::new().unwrap();
        let vault = dir.path();

        let cache = build_theory_cache(
            &result.theory,
            &result.diagnostics,
            &files,
            &result.groundings_by_block,
        );
        save_theory_cache(vault, &cache).unwrap();

        // Reload and verify groundings are in spl_blocks
        let loaded = load_theory_cache(vault).unwrap().expect("cache should exist");
        let spl_block = loaded
            .spl_blocks
            .get("notes/A.md:10")
            .expect("should have spl_block entry");
        assert_eq!(spl_block.explicit_groundings.len(), 1);
        assert_eq!(spl_block.explicit_groundings[0].construct, "r1");
        assert_eq!(
            spl_block.explicit_groundings[0].source_refs,
            vec!["abcdef1234567890"]
        );

        // Rebuild from cache — groundings_by_block should be populated
        let from_cache = build_theory_from_cache(&loaded).unwrap();
        let cached_groundings = from_cache
            .groundings_by_block
            .get("notes/A.md:10")
            .expect("groundings should survive cache round-trip");
        assert_eq!(cached_groundings[0].construct, "r1");
        assert_eq!(cached_groundings[0].source_refs, vec!["abcdef1234567890"]);
    }
}
