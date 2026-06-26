//! SPEC-050 REQ-5008/5009 — static inter-island wiring verification + audit graph.
//!
//! Given the recognised island manifests for a build, this resolves publisher→subscriber
//! edges, flags an **unpublished subscriber** (`island-topic-unpublished`, warning), an
//! **incompatible co-declared type** across any publishers/subscribers of a topic
//! (`island-topic-type-conflict`, error), and an
//! **unapproved author request** (`island-request-unapproved`, warning, vs `[security.csp]`).
//! It emits a deterministic audit (REQ-5009) extending SPEC-048 OBS-4801.
//!
//! The trusted-island `island-topic-undeclared` AST lint (REQ-5008) needs a JS parser
//! over emitted island code; v1 does not run it (a documented best-effort gap — see
//! [`WiringReport::notes`]).

use super::manifest::IslandManifest;
use super::theme::CspConfig;
use super::value_type::ValueType;
use std::collections::BTreeMap;

/// A wiring finding (REQ-5008). `fatal` findings fail the build under `--strict`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub code: String,
    pub message: String,
    pub fatal: bool,
}

/// Per-request approval status (REQ-5028).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestAudit {
    pub host: String,
    pub directive: String,
    pub approved: bool,
    pub reason: Option<String>,
}

/// Per-island audit row (REQ-5009).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IslandAudit {
    pub component: String,
    pub content_island: bool,
    pub publishes: Vec<String>,
    pub subscribes: Vec<String>,
    pub render: String,
    pub paints: bool,
    pub requests: Vec<RequestAudit>,
}

/// A resolved publisher→subscriber edge over a topic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub topic: String,
    pub publisher: String,
    pub subscriber: String,
}

/// The full wiring report (deterministic order).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WiringReport {
    pub findings: Vec<Finding>,
    pub edges: Vec<Edge>,
    pub audits: Vec<IslandAudit>,
    pub notes: Vec<String>,
}

impl WiringReport {
    /// Any fatal (error-level) finding present?
    pub fn has_fatal(&self) -> bool {
        self.findings.iter().any(|f| f.fatal)
    }
}

/// Verify wiring across all island manifests against the operator CSP.
pub fn verify(islands: &[IslandManifest], csp: &CspConfig) -> WiringReport {
    let mut report = WiringReport::default();

    // index publishers per topic (for edge resolution / unpublished-subscriber detection)
    let mut publishers: BTreeMap<String, Vec<(String, ValueType)>> = BTreeMap::new();
    // index EVERY topic declaration (publisher OR subscriber OR bare declarer) per topic,
    // for type-conflict detection — the runtime registers data-island-types from every
    // mounted island, so a publisher/subscriber type mismatch makes payload validation
    // hydration-order-dependent (Codex P2).
    let mut declarations: BTreeMap<String, Vec<(String, ValueType)>> = BTreeMap::new();
    // a topic that is declared persisted with a default needs no live publisher
    let mut persisted_defaulted: BTreeMap<String, bool> = BTreeMap::new();

    let mut sorted: Vec<&IslandManifest> = islands.iter().collect();
    sorted.sort_by(|a, b| a.component.cmp(&b.component));

    for im in &sorted {
        for t in &im.publishes {
            let ty = im
                .topics
                .get(t)
                .map(|d| d.ty.clone())
                .unwrap_or(ValueType::String);
            publishers
                .entry(t.clone())
                .or_default()
                .push((im.component.clone(), ty));
        }
        for (name, decl) in &im.topics {
            declarations
                .entry(name.clone())
                .or_default()
                .push((im.component.clone(), decl.ty.clone()));
            if decl.persisted && decl.default.is_some() {
                persisted_defaulted.insert(name.clone(), true);
            }
        }
    }

    // type-conflict: ANY two declarations of the same topic with incompatible types
    // (publisher↔subscriber, subscriber↔subscriber, …) are a fatal error.
    for (topic, decls) in &declarations {
        if decls.len() > 1 {
            let first = &decls[0].1;
            if let Some((who, _)) = decls.iter().find(|(_, ty)| ty != first) {
                report.findings.push(Finding {
                    code: "island-topic-type-conflict".into(),
                    message: format!(
                        "topic `{topic}` declared with incompatible types (e.g. {first:?} by `{}` vs `{who}`)",
                        decls[0].0
                    ),
                    fatal: true,
                });
            }
        }
    }

    // unpublished subscriber (warning) + resolved edges
    for im in &sorted {
        for t in &im.subscribes {
            match publishers.get(t) {
                Some(pubs) => {
                    for (publisher, _) in pubs {
                        report.edges.push(Edge {
                            topic: t.clone(),
                            publisher: publisher.clone(),
                            subscriber: im.component.clone(),
                        });
                    }
                }
                None => {
                    if !persisted_defaulted.get(t).copied().unwrap_or(false) {
                        report.findings.push(Finding {
                            code: "island-topic-unpublished".into(),
                            message: format!(
                                "`{}` subscribes to `{t}` which no island publishes",
                                im.component
                            ),
                            fatal: false,
                        });
                    }
                }
            }
        }
    }
    report.edges.sort_by(|a, b| {
        (&a.topic, &a.publisher, &a.subscriber).cmp(&(&b.topic, &b.publisher, &b.subscriber))
    });

    // per-island audit + request approval (vs CSP)
    for im in &sorted {
        let mut requests = Vec::new();
        if let Some(req) = &im.requests {
            for host in &req.connect_src {
                let approved = csp
                    .directives
                    .get("connect-src")
                    .map(|hosts| hosts.iter().any(|h| h == host))
                    .unwrap_or(false);
                if !approved {
                    report.findings.push(Finding {
                        code: "island-request-unapproved".into(),
                        message: format!(
                            "`{}` requests connect-src `{host}` not approved in [security.csp]",
                            im.component
                        ),
                        fatal: false,
                    });
                }
                requests.push(RequestAudit {
                    host: host.clone(),
                    directive: "connect-src".into(),
                    approved,
                    reason: req.reason.clone(),
                });
            }
        }
        report.audits.push(IslandAudit {
            component: im.component.clone(),
            content_island: im.content_island,
            publishes: im.publishes.clone(),
            subscribes: im.subscribes.clone(),
            render: match im.render {
                super::manifest::RenderMode::Worker => "worker".into(),
                super::manifest::RenderMode::Iframe => "iframe".into(),
            },
            paints: im.paints,
            requests,
        });
    }

    report
        .findings
        .sort_by(|a, b| (&a.code, &a.message).cmp(&(&b.code, &b.message)));
    report.notes.push(
        "trusted-island `island-topic-undeclared` AST lint not run in v1 (needs a JS parser)"
            .into(),
    );
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::components::manifest::parse_manifest;
    use crate::web::islands::manifest::parse;

    fn island(src: &str, dir: &str) -> IslandManifest {
        parse(&parse_manifest(src, dir).unwrap()).unwrap().unwrap()
    }

    #[test]
    fn resolves_edge_and_no_findings() {
        let pubr = island(
            "name = \"a\"\npublishes = [\"theme\"]\n[island.topics.theme]\ntype = \"int\"\n",
            "a",
        );
        let subr = island(
            "name = \"b\"\nsubscribes = [\"theme\"]\n[island.topics.theme]\ntype = \"int\"\n",
            "b",
        );
        let r = verify(&[pubr, subr], &CspConfig::default());
        assert_eq!(r.edges.len(), 1);
        assert_eq!(r.edges[0].publisher, "a");
        assert_eq!(r.edges[0].subscriber, "b");
        assert!(!r.has_fatal());
    }

    #[test]
    fn unpublished_subscriber_warns() {
        let subr = island(
            "name = \"b\"\nsubscribes = [\"theme\"]\n[island.topics.theme]\ntype = \"int\"\n",
            "b",
        );
        let r = verify(&[subr], &CspConfig::default());
        assert!(r
            .findings
            .iter()
            .any(|f| f.code == "island-topic-unpublished" && !f.fatal));
    }

    #[test]
    fn type_conflict_is_fatal() {
        let a = island(
            "name = \"a\"\npublishes = [\"tx:val\"]\n[island.topics.\"tx:val\"]\ntype = \"int\"\n",
            "a",
        );
        let b = island(
            "name = \"b\"\npublishes = [\"tx:val\"]\n[island.topics.\"tx:val\"]\ntype = \"bool\"\n",
            "b",
        );
        let r = verify(&[a, b], &CspConfig::default());
        assert!(r
            .findings
            .iter()
            .any(|f| f.code == "island-topic-type-conflict" && f.fatal));
        assert!(r.has_fatal());
    }

    #[test]
    fn publisher_subscriber_type_conflict_is_fatal() {
        // A publisher declares `tx:val` as int while a SUBSCRIBER declares it bool — the
        // runtime would register conflicting data-island-types by hydration order, so the
        // build must reject it (Codex P2).
        let pubr = island(
            "name = \"a\"\npublishes = [\"tx:val\"]\n[island.topics.\"tx:val\"]\ntype = \"int\"\n",
            "a",
        );
        let subr = island(
            "name = \"b\"\nsubscribes = [\"tx:val\"]\n[island.topics.\"tx:val\"]\ntype = \"bool\"\n",
            "b",
        );
        let r = verify(&[pubr, subr], &CspConfig::default());
        assert!(
            r.findings
                .iter()
                .any(|f| f.code == "island-topic-type-conflict" && f.fatal),
            "publisher↔subscriber type mismatch must be fatal: {:?}",
            r.findings
        );
    }
}
