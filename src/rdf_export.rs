//! SPEC-045 REQ-4516 / CON-4507 — semantic-web interop export.
//!
//! Projects the vault's typed edges to RDF in three serialisations
//! (N-Triples, Turtle, JSON-LD). Each typed edge becomes a triple
//! `source-page → predicate → target-page`; annotations become `rdfs:comment`
//! and provenance (`_source_page`/`_source_file`/`_source_line`) becomes PROV-O
//! on a reified statement. The predicate vocabulary is emitted as a SKOS
//! concept scheme. Un-mapped predicates and page identifiers are minted in the
//! vault's own namespace; CURIE predicates and snake predicates carrying
//! `maps_to` expand to full IRIs via `[prefixes]`.
//!
//! A SPARQL endpoint is out of scope (§1.5): this export feeds any external
//! triplestore.

use crate::graph::LinkGraph;
use crate::predicates::PredicatesConfig;
use crate::types::ParsedFile;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";
const PROV: &str = "http://www.w3.org/ns/prov#";
const SKOS: &str = "http://www.w3.org/2004/02/skos/core#";
const DCTERMS: &str = "http://purl.org/dc/terms/";

/// One RDF term.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Term {
    Iri(String),
    Literal(String),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Triple {
    s: Term,
    p: Term,
    o: Term,
}

/// Percent-encode the characters that may not appear bare in an IRI path.
fn iri_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Escape a string literal for N-Triples / Turtle.
fn escape_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

/// Resolve a predicate to its IRI: a CURIE or a snake-`maps_to` expands via
/// `[prefixes]`; everything else is minted in the vault namespace.
fn predicate_iri(predicate: &str, base: &str, config: Option<&PredicatesConfig>) -> String {
    // A snake predicate may declare `maps_to = "prefix:local"`.
    let curie: Option<String> = if predicate.contains(':') {
        Some(predicate.to_string())
    } else {
        config
            .and_then(|c| c.predicates.get(predicate))
            .and_then(|m| m.maps_to.clone())
    };
    if let Some(curie) = curie {
        if let Some((prefix, local)) = curie.split_once(':') {
            if let Some(ns) = config.and_then(|c| c.prefixes.get(prefix)) {
                return format!("{ns}{local}");
            }
        }
    }
    format!("{base}predicate/{}", iri_encode(predicate))
}

fn page_iri(page: &str, base: &str) -> String {
    format!("{base}page/{}", iri_encode(page))
}

/// Build the full triple set for the vault's typed edges + SKOS vocabulary.
fn build_triples(
    graph: &LinkGraph,
    files: &[ParsedFile],
    config: Option<&PredicatesConfig>,
    base: &str,
) -> Vec<Triple> {
    let mut triples: Vec<Triple> = Vec::new();
    let path_for = |page: &str| -> Option<String> {
        files
            .iter()
            .find(|f| f.page_name == page)
            .map(|f| f.path.to_string_lossy().to_string())
    };

    let mut vocab: BTreeSet<String> = BTreeSet::new();

    for (i, e) in graph.all_edges().iter().enumerate() {
        let Some(pred) = &e.predicate else {
            continue; // only typed edges are exported
        };
        vocab.insert(pred.clone());

        let s = Term::Iri(page_iri(&e.source, base));
        let p_iri = predicate_iri(pred, base, config);
        let p = Term::Iri(p_iri.clone());
        // A ghost/external target is still minted as a page IRI in the vault
        // namespace (it identifies the intended concept).
        let o = Term::Iri(page_iri(&e.target, base));

        // The plain assertion.
        triples.push(Triple {
            s: s.clone(),
            p: p.clone(),
            o: o.clone(),
        });

        // Reify when there is an annotation or provenance to attach.
        let stmt = Term::Iri(format!("{base}statement/{i}"));
        triples.push(Triple {
            s: stmt.clone(),
            p: Term::Iri(format!("{RDF}type")),
            o: Term::Iri(format!("{RDF}Statement")),
        });
        triples.push(Triple {
            s: stmt.clone(),
            p: Term::Iri(format!("{RDF}subject")),
            o: s.clone(),
        });
        triples.push(Triple {
            s: stmt.clone(),
            p: Term::Iri(format!("{RDF}predicate")),
            o: p.clone(),
        });
        triples.push(Triple {
            s: stmt.clone(),
            p: Term::Iri(format!("{RDF}object")),
            o: o.clone(),
        });
        // Provenance (PROV-O): the statement is attributed to the source page.
        triples.push(Triple {
            s: stmt.clone(),
            p: Term::Iri(format!("{PROV}wasAttributedTo")),
            o: s.clone(),
        });
        if let Some(path) = path_for(&e.source) {
            triples.push(Triple {
                s: stmt.clone(),
                p: Term::Iri(format!("{DCTERMS}source")),
                o: Term::Literal(format!("{path}:{}", e.line)),
            });
        }
        if let Some(annotation) = &e.annotation {
            triples.push(Triple {
                s: stmt.clone(),
                p: Term::Iri(format!("{RDFS}comment")),
                o: Term::Literal(annotation.clone()),
            });
        }
    }

    // SKOS concept scheme for the observed predicate vocabulary.
    let scheme = Term::Iri(format!("{base}vocabulary"));
    triples.push(Triple {
        s: scheme.clone(),
        p: Term::Iri(format!("{RDF}type")),
        o: Term::Iri(format!("{SKOS}ConceptScheme")),
    });
    for pred in &vocab {
        let c = Term::Iri(predicate_iri(pred, base, config));
        triples.push(Triple {
            s: c.clone(),
            p: Term::Iri(format!("{RDF}type")),
            o: Term::Iri(format!("{SKOS}Concept")),
        });
        triples.push(Triple {
            s: c.clone(),
            p: Term::Iri(format!("{SKOS}inScheme")),
            o: scheme.clone(),
        });
        let label = config
            .map(|c| c.display_label(pred))
            .unwrap_or_else(|| crate::predicates::auto_label(pred));
        triples.push(Triple {
            s: c,
            p: Term::Iri(format!("{SKOS}prefLabel")),
            o: Term::Literal(label),
        });
    }

    triples.sort();
    triples.dedup();
    triples
}

fn term_ntriples(t: &Term) -> String {
    match t {
        Term::Iri(iri) => format!("<{iri}>"),
        Term::Literal(lit) => format!("\"{}\"", escape_literal(lit)),
    }
}

fn to_ntriples(triples: &[Triple]) -> String {
    let mut out = String::new();
    for t in triples {
        out.push_str(&format!(
            "{} {} {} .\n",
            term_ntriples(&t.s),
            term_ntriples(&t.p),
            term_ntriples(&t.o)
        ));
    }
    out
}

fn to_turtle(triples: &[Triple]) -> String {
    let mut out = String::new();
    out.push_str(&format!("@prefix rdf: <{RDF}> .\n"));
    out.push_str(&format!("@prefix rdfs: <{RDFS}> .\n"));
    out.push_str(&format!("@prefix prov: <{PROV}> .\n"));
    out.push_str(&format!("@prefix skos: <{SKOS}> .\n"));
    out.push_str(&format!("@prefix dcterms: <{DCTERMS}> .\n\n"));
    let prefix_of = |iri: &str| -> Option<String> {
        for (pfx, ns) in [
            ("rdf", RDF),
            ("rdfs", RDFS),
            ("prov", PROV),
            ("skos", SKOS),
            ("dcterms", DCTERMS),
        ] {
            if let Some(local) = iri.strip_prefix(ns) {
                // Only use a prefixed name when the local part is a valid PN.
                if !local.is_empty()
                    && local
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                {
                    return Some(format!("{pfx}:{local}"));
                }
            }
        }
        None
    };
    let term = |t: &Term| -> String {
        match t {
            Term::Iri(iri) => prefix_of(iri).unwrap_or_else(|| format!("<{iri}>")),
            Term::Literal(lit) => format!("\"{}\"", escape_literal(lit)),
        }
    };
    for t in triples {
        out.push_str(&format!("{} {} {} .\n", term(&t.s), term(&t.p), term(&t.o)));
    }
    out
}

fn to_jsonld(triples: &[Triple]) -> String {
    // Group triples by subject into node objects; multiple values become arrays.
    // Object IRIs render as `{"@id": ...}`, literals as bare strings.
    let mut by_subject: BTreeMap<String, BTreeMap<String, Vec<serde_json::Value>>> =
        BTreeMap::new();
    for t in triples {
        let Term::Iri(subj) = &t.s else { continue };
        let Term::Iri(pred) = &t.p else { continue };
        let value = match &t.o {
            Term::Iri(iri) => serde_json::json!({ "@id": iri }),
            Term::Literal(lit) => serde_json::Value::String(lit.clone()),
        };
        by_subject
            .entry(subj.clone())
            .or_default()
            .entry(pred.clone())
            .or_default()
            .push(value);
    }
    let graph: Vec<serde_json::Value> = by_subject
        .into_iter()
        .map(|(subj, preds)| {
            let mut obj = serde_json::Map::new();
            obj.insert("@id".to_string(), serde_json::Value::String(subj));
            for (pred, mut values) in preds {
                let v = if values.len() == 1 {
                    values.pop().unwrap()
                } else {
                    serde_json::Value::Array(values)
                };
                obj.insert(pred, v);
            }
            serde_json::Value::Object(obj)
        })
        .collect();
    let doc = serde_json::json!({
        "@context": {
            "rdf": RDF, "rdfs": RDFS, "prov": PROV, "skos": SKOS, "dcterms": DCTERMS
        },
        "@graph": graph
    });
    serde_json::to_string_pretty(&doc).unwrap_or_default()
}

/// Render the typed-edge RDF export in the requested format.
pub fn export_rdf(
    graph: &LinkGraph,
    files: &[ParsedFile],
    config: Option<&PredicatesConfig>,
    base_iri: &str,
    format: crate::cli::RdfFormat,
) -> String {
    // Normalise the base to end with `/` so IRI concatenation is well-formed.
    let base = if base_iri.ends_with('/') {
        base_iri.to_string()
    } else {
        format!("{base_iri}/")
    };
    let triples = build_triples(graph, files, config, &base);
    match format {
        crate::cli::RdfFormat::Ntriples => to_ntriples(&triples),
        crate::cli::RdfFormat::Turtle => to_turtle(&triples),
        crate::cli::RdfFormat::Jsonld => to_jsonld(&triples),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::WikiLink;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::SystemTime;

    fn typed_file(name: &str, links: Vec<(&str, u32, Vec<&str>, Option<&str>)>) -> ParsedFile {
        ParsedFile {
            path: PathBuf::from(format!("{name}.md")),
            page_name: name.to_string(),
            links: links
                .into_iter()
                .map(|(target, line, preds, ann)| WikiLink {
                    target_page: target.to_string(),
                    raw_target: target.to_string(),
                    heading: None,
                    block_ref: None,
                    alias: None,
                    is_embed: false,
                    predicates: preds.into_iter().map(String::from).collect(),
                    annotation: ann.map(String::from),
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

    fn graph_for(files: &[ParsedFile]) -> LinkGraph {
        let mut resolved = HashMap::new();
        for f in files {
            for l in &f.links {
                resolved.insert(l.raw_target.clone(), l.target_page.clone());
            }
        }
        LinkGraph::build(files, &resolved)
    }

    #[test]
    fn ntriples_emits_typed_edge_and_skos() {
        let files = vec![
            typed_file(
                "Decision",
                vec![("Retro", 5, vec!["derived_from"], Some("why"))],
            ),
            typed_file("Retro", vec![]),
        ];
        let g = graph_for(&files);
        let out = export_rdf(
            &g,
            &files,
            None,
            "http://x/",
            crate::cli::RdfFormat::Ntriples,
        );
        // The plain assertion.
        assert!(out.contains(
            "<http://x/page/Decision> <http://x/predicate/derived_from> <http://x/page/Retro> ."
        ));
        // Reified provenance + annotation.
        assert!(out.contains("www.w3.org/ns/prov#wasAttributedTo"));
        assert!(out.contains("rdf-schema#comment> \"why\""));
        // SKOS vocabulary.
        assert!(out.contains("skos/core#Concept"));
        // Untyped edges are NOT exported.
        assert!(!out.contains("predicate/null"));
    }

    #[test]
    fn turtle_uses_prefixes_and_maps_to() {
        let cfg = PredicatesConfig::from_toml_str(
            "[prefixes]\nprov = \"http://www.w3.org/ns/prov#\"\n\n[predicates]\nderived_from = { maps_to = \"prov:wasDerivedFrom\" }\n",
        )
        .unwrap();
        let files = vec![
            typed_file("A", vec![("B", 1, vec!["derived_from"], None)]),
            typed_file("B", vec![]),
        ];
        let g = graph_for(&files);
        let out = export_rdf(
            &g,
            &files,
            Some(&cfg),
            "http://x/",
            crate::cli::RdfFormat::Turtle,
        );
        // maps_to expands the snake predicate to the prov IRI (prefixed form).
        assert!(out.contains("prov:wasDerivedFrom"));
        assert!(out.contains("@prefix prov:"));
    }

    #[test]
    fn jsonld_is_valid_json_with_graph() {
        let files = vec![
            typed_file("A", vec![("B", 1, vec!["contradicts"], None)]),
            typed_file("B", vec![]),
        ];
        let g = graph_for(&files);
        let out = export_rdf(&g, &files, None, "http://x/", crate::cli::RdfFormat::Jsonld);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!(v["@graph"].is_array());
        assert!(v["@context"]["skos"].is_string());
    }

    #[test]
    fn output_is_deterministic() {
        let files = vec![
            typed_file(
                "A",
                vec![("B", 1, vec!["x"], None), ("C", 2, vec!["y"], None)],
            ),
            typed_file("B", vec![]),
            typed_file("C", vec![]),
        ];
        let g = graph_for(&files);
        let a = export_rdf(
            &g,
            &files,
            None,
            "http://x/",
            crate::cli::RdfFormat::Ntriples,
        );
        let b = export_rdf(
            &g,
            &files,
            None,
            "http://x/",
            crate::cli::RdfFormat::Ntriples,
        );
        assert_eq!(a, b);
    }
}
