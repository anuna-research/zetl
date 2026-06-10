//! SPEC-045 REQ-4516: `zetl export --rdf {ntriples,turtle,jsonld}`.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use tempfile::TempDir;

fn vault() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("Decision.md"),
        "- derived_from::[[Retro]]\n  - because it shaped the plan\n- contradicts::[[Old]]\n",
    )
    .unwrap();
    fs::write(dir.path().join("Retro.md"), "# Retro\n").unwrap();
    dir
}

fn run(dir: &TempDir, fmt: &str) -> String {
    let out = cargo_bin_cmd!("zetl")
        .args(["-d", dir.path().to_str().unwrap(), "export", "--rdf", fmt])
        .assert()
        .success();
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
}

#[test]
fn ntriples_has_typed_edge_provenance_and_skos() {
    let dir = vault();
    let nt = run(&dir, "ntriples");
    assert!(nt.contains("page/Decision> <") && nt.contains("predicate/derived_from> <"));
    assert!(nt.contains("prov#wasAttributedTo"));
    assert!(nt.contains("skos/core#Concept"));
    assert!(nt.contains("rdf-schema#comment> \"because it shaped the plan\""));
}

#[test]
fn turtle_has_prefixes() {
    let dir = vault();
    let ttl = run(&dir, "turtle");
    assert!(ttl.contains("@prefix skos:"));
    assert!(ttl.contains("@prefix prov:"));
}

#[test]
fn jsonld_parses_and_has_graph() {
    let dir = vault();
    let jl = run(&dir, "jsonld");
    let v: serde_json::Value = serde_json::from_str(&jl).unwrap();
    assert!(v["@graph"]
        .as_array()
        .map(|a| !a.is_empty())
        .unwrap_or(false));
}

#[test]
fn default_export_without_rdf_is_unchanged_json() {
    let dir = vault();
    let out = cargo_bin_cmd!("zetl")
        .args(["-f", "json", "-d", dir.path().to_str().unwrap(), "export"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    // Still the link-graph export shape, not RDF.
    assert!(v["edge_count"].is_number());
    assert!(v["nodes"].is_array());
}
