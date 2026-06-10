//! SPEC-045 REQ-4510 / CON-4504 / ADR-4504 — project typed SNAKE link-graph
//! edges into SPL facts.
//!
//! Each directed edge `S --predicate--> T` whose `predicate` is a SNAKE
//! predicate (`^[a-z][a-z0-9_]*$`) becomes a binary fact
//! `(<predicate> "<source-page>" "<target-page>")` that the reasoner can chain
//! over. The projection is *sugar over SPL*: an authored fact and a
//! projected-from-wikilink fact are indistinguishable to the reasoner, which is
//! why each carries full provenance (`_source_file`, `_source_line`,
//! `_source_page`) — the asserting page+line — so any conclusion derived from a
//! projected fact remains traceable (Threat Model A, ADR-4504).
//!
//! ## Exclusions (CON-4504)
//!
//! - **Untyped edges** (`predicate == None`) are NOT projected — they remain
//!   pure navigation links and carry no logical meaning.
//! - **CURIE predicates** (e.g. `prov:wasDerivedFrom`) are NOT projected in
//!   v1. A `:` is not a legal SPL functor character, so CURIEs cannot become
//!   facts; they are collected into [`EdgeProjection::skipped_curies`] for an
//!   observability / `--verbose` channel but never reach the theory.
//!
//! ## Determinism (CON-4504)
//!
//! `LinkGraph::all_edges()` is already sorted by `(source, target, predicate,
//! line)`. We preserve that order, so the projected fact list is order-stable
//! for a fixed vault.

use crate::graph::LinkGraph;
use crate::predicates::is_snake_predicate;
use crate::types::ParsedFile;
use spindle_core::prelude::{Literal, Mode, Rule, Temporal, Theory};
use std::collections::HashMap;
use std::path::PathBuf;

/// A single SPL fact projected from a typed SNAKE edge of the link graph.
///
/// The projected literal is `(<predicate> "<source>" "<target>")`. Provenance
/// points at the *asserting* edge: `source_page` is the edge source, and
/// `source_file` / `source_line` locate the `[[wikilink]]` occurrence that
/// declared the edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedFact {
    /// The SNAKE predicate (the SPL functor name).
    pub predicate: String,
    /// The edge source page (subject).
    pub source: String,
    /// The edge target page (object).
    pub target: String,
    /// Path of the file that asserted the edge (the source page's file).
    pub source_file: PathBuf,
    /// 1-indexed line of the asserting wikilink occurrence.
    pub source_line: u32,
    /// The asserting page name (== `source`).
    pub source_page: String,
}

impl ProjectedFact {
    /// The binary SPL literal `predicate(source, target)` this fact asserts.
    ///
    /// The page names are carried as the literal's predicate-argument terms,
    /// so the reasoner sees a ground binary fact identical to one an author
    /// could have written by hand.
    pub fn to_literal(&self) -> Literal {
        Literal::new(
            &self.predicate,
            false,
            Mode::empty(),
            Temporal::empty(),
            vec![self.source.clone(), self.target.clone()],
        )
    }
}

/// A CURIE edge that was recognised but NOT projected (v1 exclusion).
///
/// Surfaced for an observability / `--verbose` channel so a curator can see
/// "N `prov:*` edges were not projected".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedCurieEdge {
    pub predicate: String,
    pub source: String,
    pub target: String,
    pub source_file: PathBuf,
    pub source_line: u32,
}

/// The outcome of projecting a link graph: the facts to add to the theory plus
/// the CURIE edges that were deliberately skipped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EdgeProjection {
    /// Projected SNAKE-edge facts, in `all_edges()` order (deterministic).
    pub facts: Vec<ProjectedFact>,
    /// CURIE edges skipped in v1 (observability only).
    pub skipped_curies: Vec<SkippedCurieEdge>,
}

/// Project every typed SNAKE edge of `graph` to a [`ProjectedFact`]
/// (SPEC-045 REQ-4510). Pure — no I/O, deterministic for a fixed vault.
///
/// `files` is used only to map a source *page name* → its *file path* (the
/// [`crate::graph::EdgeRecord`] does not carry the source file). The source
/// line comes straight from the edge record.
pub fn project_edges_to_facts(graph: &LinkGraph, files: &[ParsedFile]) -> EdgeProjection {
    // page_name → file path, for provenance. First wins on duplicate page
    // names (matches the graph node-creation order).
    let mut page_to_file: HashMap<&str, &PathBuf> = HashMap::new();
    for f in files {
        page_to_file.entry(f.page_name.as_str()).or_insert(&f.path);
    }

    let mut projection = EdgeProjection::default();

    // `all_edges()` is pre-sorted by (source, target, predicate, line), so
    // iterating in order yields a deterministic, order-stable fact list.
    for edge in graph.all_edges() {
        // Untyped edge → not a fact (CON-4504).
        let Some(predicate) = edge.predicate else {
            continue;
        };

        let source_file = page_to_file
            .get(edge.source.as_str())
            .map(|p| (*p).clone())
            .unwrap_or_default();

        if is_snake_predicate(&predicate) {
            projection.facts.push(ProjectedFact {
                predicate,
                source: edge.source.clone(),
                target: edge.target,
                source_file,
                source_line: edge.line,
                source_page: edge.source,
            });
        } else {
            // A non-snake predicate that survived parsing is a CURIE
            // (`prefix:localName`). Skip it in v1 (CON-4504) but record it.
            projection.skipped_curies.push(SkippedCurieEdge {
                predicate,
                source: edge.source.clone(),
                target: edge.target,
                source_file,
                source_line: edge.line,
                // (source_page == source, omitted from the skip record)
            });
        }
    }

    projection
}

/// Add projected edge facts to a spindle-core [`Theory`], annotating each with
/// the SAME provenance metadata (`_source_file`, `_source_line`,
/// `_source_page`) that [`super::build_theory`] attaches to authored facts.
///
/// Returns the number of facts added. Each fact is given a fresh internal
/// label `__edge_fact_<n>` (starting at `start_counter + 1`) so it never
/// collides with the `__fact_<n>` labels minted for authored SPL facts, and
/// the returned label is recorded in `label_origins` so conclusion tracing
/// (`super::trace_proof_sources`) can find the asserting page+line.
pub(crate) fn add_projected_facts_to_theory(
    theory: &mut Theory,
    facts: &[ProjectedFact],
    label_origins: &mut HashMap<String, (PathBuf, u32, String)>,
    start_counter: u32,
) -> u32 {
    let mut counter = start_counter;
    for fact in facts {
        counter += 1;
        let label = format!("__edge_fact_{counter}");
        theory.add_rule(Rule::fact(&label, fact.to_literal()));

        theory.add_meta_string(
            &label,
            "_source_file",
            &fact.source_file.display().to_string(),
        );
        theory.add_meta_string(&label, "_source_line", &fact.source_line.to_string());
        theory.add_meta_string(&label, "_source_page", &fact.source_page);

        label_origins.insert(
            label,
            (
                fact.source_file.clone(),
                fact.source_line,
                fact.source_page.clone(),
            ),
        );
    }
    counter - start_counter
}
