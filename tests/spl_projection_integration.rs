//! SPEC-045 REQ-4510 / CON-4504 / ADR-4504 integration tests for projecting
//! typed SNAKE link-graph edges into SPL facts.
//!
//! Gated under `--features reason` (registered with `required-features` in
//! Cargo.toml, so this whole crate only compiles under that feature).

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use zetl::graph::LinkGraph;
use zetl::reason::projection::{project_edges_to_facts, EdgeProjection};
use zetl::reason::{build_theory, build_theory_with_edges};
use zetl::types::{ParsedFile, WikiLink};

/// Build a `WikiLink` to `target` with the given predicates (empty => untyped).
fn link(target: &str, predicates: &[&str], line: u32) -> WikiLink {
    WikiLink {
        target_page: target.to_string(),
        raw_target: target.to_string(),
        heading: None,
        block_ref: None,
        alias: None,
        is_embed: false,
        line,
        column: 1,
        predicates: predicates.iter().map(|s| s.to_string()).collect(),
        annotation: None,
    }
}

/// Build a `ParsedFile` for `page` with the given outgoing links.
fn page(page: &str, links: Vec<WikiLink>) -> ParsedFile {
    ParsedFile {
        path: PathBuf::from(format!("{page}.md")),
        page_name: page.to_string(),
        links,
        spl_blocks: vec![],
        diagnostics: vec![],
        mtime: UNIX_EPOCH,
        merkle_leaves: vec![],
        file_merkle: None,
    }
}

/// A vault where "Source" links to "Target" three ways:
///   - `derived_from::[[Target]]`  -> a SNAKE typed edge   (PROJECTED)
///   - plain `[[Target]]`          -> an untyped edge      (NOT projected)
///   - `prov:wasDerivedFrom::[[Target]]` -> a CURIE edge   (NOT projected, v1)
fn sample_vault() -> Vec<ParsedFile> {
    vec![
        page(
            "Source",
            vec![
                link("Target", &["derived_from"], 3),
                link("Target", &[], 5),
                link("Target", &["prov:wasDerivedFrom"], 7),
            ],
        ),
        page("Target", vec![]),
    ]
}

fn build_projection(files: &[ParsedFile]) -> EdgeProjection {
    let graph = LinkGraph::build(files, &HashMap::new());
    project_edges_to_facts(&graph, files)
}

#[test]
fn snake_edge_yields_fact_with_provenance() {
    let files = sample_vault();
    let projection = build_projection(&files);

    assert_eq!(
        projection.facts.len(),
        1,
        "exactly one SNAKE edge should project; got {:?}",
        projection.facts
    );
    let fact = &projection.facts[0];
    assert_eq!(fact.predicate, "derived_from");
    assert_eq!(fact.source, "Source");
    assert_eq!(fact.target, "Target");
    assert_eq!(fact.source_page, "Source");
    assert_eq!(fact.source_file, PathBuf::from("Source.md"));
    assert_eq!(fact.source_line, 3);
    assert_eq!(fact.to_literal().to_string(), "derived_from(Source, Target)");
}

#[test]
fn untyped_edge_yields_no_fact() {
    let files = sample_vault();
    let projection = build_projection(&files);
    assert!(
        !projection.facts.iter().any(|f| f.source_line == 5),
        "the untyped edge on line 5 must not be projected"
    );
}

#[test]
fn curie_edge_is_skipped_not_projected() {
    let files = sample_vault();
    let projection = build_projection(&files);
    assert!(
        !projection.facts.iter().any(|f| f.predicate.contains(':')),
        "CURIE predicates must not become facts (CON-4504 v1)"
    );
    assert_eq!(projection.skipped_curies.len(), 1);
    let skipped = &projection.skipped_curies[0];
    assert_eq!(skipped.predicate, "prov:wasDerivedFrom");
    assert_eq!(skipped.source, "Source");
    assert_eq!(skipped.target, "Target");
    assert_eq!(skipped.source_line, 7);
}

#[test]
fn projection_is_deterministic_across_runs() {
    let files = sample_vault();
    let a = build_projection(&files);
    let b = build_projection(&files);
    assert_eq!(a, b, "projection must be order-stable for a fixed vault");
}

#[test]
fn multiple_snake_predicates_each_project() {
    let files = vec![
        page("A", vec![link("B", &["supersedes", "refines"], 10)]),
        page("B", vec![]),
    ];
    let projection = build_projection(&files);
    assert_eq!(projection.facts.len(), 2);
    let mut got: Vec<&str> = projection
        .facts
        .iter()
        .map(|f| f.predicate.as_str())
        .collect();
    got.sort();
    assert_eq!(got, vec!["refines", "supersedes"]);
    assert!(projection
        .facts
        .iter()
        .all(|f| f.source == "A" && f.target == "B" && f.source_line == 10));
}

#[test]
fn projected_facts_enter_theory_with_provenance_and_chain() {
    let files = sample_vault();
    let projection = build_projection(&files);

    use zetl::types::SplBlock;
    let rule_block = SplBlock {
        source_file: PathBuf::from("Rules.md"),
        source_page: "Rules".to_string(),
        start_line: 1,
        end_line: 3,
        content: "(always r1 (derived_from Source Target) traceable)\n".to_string(),
    };

    let result =
        build_theory_with_edges(&[rule_block], &projection.facts).expect("theory builds");

    let pf = result
        .facts
        .iter()
        .find(|f| f.literal.to_string() == "derived_from(Source, Target)")
        .expect("projected fact present in theory");
    assert_eq!(pf.source_page, "Source");
    assert_eq!(pf.source_line, 3);
    assert_eq!(pf.source_file, PathBuf::from("Source.md"));

    let has_source_provenance = result.facts.iter().any(|f| f.source_page == "Source");
    assert!(
        has_source_provenance,
        "conclusion must be traceable to the asserting page"
    );
}

#[test]
fn empty_edges_equivalent_to_plain_build_theory() {
    use zetl::types::SplBlock;
    let block = SplBlock {
        source_file: PathBuf::from("A.md"),
        source_page: "A".to_string(),
        start_line: 1,
        end_line: 2,
        content: "(given bird)\n".to_string(),
    };
    let plain = build_theory(std::slice::from_ref(&block)).unwrap();
    let with_edges = build_theory_with_edges(std::slice::from_ref(&block), &[]).unwrap();
    assert_eq!(plain.facts.len(), with_edges.facts.len());
    assert_eq!(plain.summary.fact_count, with_edges.summary.fact_count);
    assert_eq!(plain.conclusions.len(), with_edges.conclusions.len());
}
