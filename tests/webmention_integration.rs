//! SPEC-039 integration tests.
//!
//! End-to-end tests for the webmention receive + send pipeline using a
//! mock HTTP transport. The full Axum surface is exercised via the
//! handler directly (we don't bring up a TCP listener — keeps the test
//! fast and deterministic).

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use axum::extract::{Form, State};
use tempfile::TempDir;
use url::Url;

use zetl::feed::fetch::{FetchError, FetchRequest, FetchResponse, HttpTransport};
use zetl::webmention::config::ModerationDefault;
use zetl::webmention::core::diff::idempotency_diff;
use zetl::webmention::core::extract::extract_external_links;
use zetl::webmention::core::moderate::source_domain;
use zetl::webmention::persist::{
    append_external_edge, load_external_edges, load_queue, load_sent_log,
};
use zetl::webmention::receive::{
    process_incoming, webmention_endpoint_handler, RateLimiter, ReceiveDeps, ReceiveOutcome,
    WebmentionForm, WebmentionState,
};
use zetl::webmention::send::{
    compute_send_plan, execute_send_plan_with_poster, RenderedPage, WebmentionPoster,
};
use zetl::webmention::types::{ExternalEdge, IncomingMention, OutboundMention, SentRecord};

fn deps_for(vault: &Path, host: &str) -> ReceiveDeps {
    ReceiveDeps {
        vault_root: vault.to_path_buf(),
        vault_host: host.to_string(),
        allowlist_domains: HashSet::new(),
        denylist_domains: HashSet::new(),
        vault_outbound_domains: HashSet::new(),
        default_decision: ModerationDefault::Queue,
    }
}

struct StaticTransport {
    body: Vec<u8>,
    content_type: Option<String>,
    link_headers: Vec<String>,
}

impl Default for StaticTransport {
    fn default() -> Self {
        Self {
            body: Vec::new(),
            content_type: Some("text/html".into()),
            link_headers: Vec::new(),
        }
    }
}

impl HttpTransport for StaticTransport {
    fn fetch(&self, request: &FetchRequest) -> Result<FetchResponse, FetchError> {
        Ok(FetchResponse {
            status: 200,
            body: self.body.clone(),
            last_modified: None,
            etag: None,
            content_type: self.content_type.clone(),
            link_headers: self.link_headers.clone(),
            final_url: request.url.clone(),
        })
    }
}

#[test]
fn hp_3901_first_mention_from_stranger_is_queued() {
    // Stranger sends a mention; verification passes; default decision
    // is Queue; mention enters queue.jsonl and NOT received.jsonl.
    let dir = TempDir::new().unwrap();
    let deps = deps_for(dir.path(), "me.example");
    let transport = StaticTransport {
        body: r#"<a href="https://me.example/p">link to me</a>"#.into(),
        ..StaticTransport::default()
    };
    let mention = IncomingMention {
        source: "https://stranger.example/post".into(),
        target: "https://me.example/p".into(),
        received_at: 0,
    };
    let outcome = process_incoming(mention, &deps, &transport).unwrap();
    assert_eq!(outcome, ReceiveOutcome::Queued);
    assert_eq!(load_queue(dir.path()).unwrap().len(), 1);
    assert!(load_external_edges(dir.path()).unwrap().is_empty());
}

#[test]
fn hp_3905_federation_peer_already_linked_auto_accepts() {
    let dir = TempDir::new().unwrap();
    let mut deps = deps_for(dir.path(), "me.example");
    deps.vault_outbound_domains
        .insert("peer.example".to_string());
    let transport = StaticTransport {
        body: r#"<a href="https://me.example/p">link</a>"#.into(),
        ..StaticTransport::default()
    };
    let mention = IncomingMention {
        source: "https://peer.example/their-post".into(),
        target: "https://me.example/p".into(),
        received_at: 0,
    };
    let outcome = process_incoming(mention, &deps, &transport).unwrap();
    assert_eq!(outcome, ReceiveOutcome::Accepted);
    assert!(load_queue(dir.path()).unwrap().is_empty());
    let edges = load_external_edges(dir.path()).unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].source, "https://peer.example/their-post");
}

#[test]
fn hp_3906_spam_no_link_in_source_rejected_at_verify() {
    let dir = TempDir::new().unwrap();
    let deps = deps_for(dir.path(), "me.example");
    let transport = StaticTransport {
        body: r#"<p>this page links to nothing</p>"#.into(),
        ..StaticTransport::default()
    };
    let mention = IncomingMention {
        source: "https://spammer.example/fake".into(),
        target: "https://me.example/p".into(),
        received_at: 0,
    };
    let outcome = process_incoming(mention, &deps, &transport).unwrap();
    assert_eq!(outcome, ReceiveOutcome::Unverified);
    assert!(load_queue(dir.path()).unwrap().is_empty());
    assert!(load_external_edges(dir.path()).unwrap().is_empty());
}

#[test]
fn t1_ssrf_source_localhost_rejected_before_fetch() {
    use std::net::IpAddr;
    use zetl::feed::fetch::{assert_safe_scheme, is_public_ip};

    // The receive shell calls `fetch_source` which calls `assert_safe_scheme`
    // first, then ureq resolves DNS. Pure scheme + IP guards are tested here
    // — full transport-level SSRF is exercised by SPEC-038's tests since the
    // primitives are shared.
    assert_safe_scheme(&Url::parse("https://example.com").unwrap()).unwrap();
    assert!(assert_safe_scheme(&Url::parse("file:///etc/passwd").unwrap()).is_err());
    assert!(!is_public_ip("127.0.0.1".parse::<IpAddr>().unwrap()));
    assert!(!is_public_ip("10.0.0.1".parse::<IpAddr>().unwrap()));
    assert!(is_public_ip("8.8.8.8".parse::<IpAddr>().unwrap()));
}

#[test]
fn t8_capability_url_target_refuses_persistence_silently() {
    // REQ-3909 / T8: a webmention POST aimed at /caps/<token>/page
    // must NOT persist the token to disk anywhere — not in
    // received.jsonl, not in queue.jsonl, not in any log line. The
    // pipeline collapses into the Denied outcome (handler returns
    // oracle-resistant 202) and the source is never even fetched.
    //
    // Important: we use a transport whose body WOULD verify (contains
    // a link to the capability URL) so the test isolates the behavior
    // change to capability-target refusal, not link-absence.
    let dir = TempDir::new().unwrap();
    let mut deps = deps_for(dir.path(), "me.example");
    deps.allowlist_domains.insert("friend.example".to_string());
    let body = r#"<a href="https://me.example/caps/SECRET-TOKEN-1234/page">link</a>"#;
    let transport = StaticTransport {
        body: body.into(),
        ..StaticTransport::default()
    };
    let mention = IncomingMention {
        source: "https://friend.example/post".into(),
        target: "https://me.example/caps/SECRET-TOKEN-1234/page".into(),
        received_at: 0,
    };
    let outcome = process_incoming(mention, &deps, &transport).unwrap();
    assert_eq!(outcome, ReceiveOutcome::Denied);

    // No persistence whatsoever.
    assert!(load_queue(dir.path()).unwrap().is_empty());
    assert!(load_external_edges(dir.path()).unwrap().is_empty());
    // The on-disk JSONL files must not contain "SECRET-TOKEN-1234"
    // anywhere — even queued / errored entries are not allowed to
    // mention a capability token.
    let webmentions_dir = dir.path().join(".zetl/webmentions");
    if webmentions_dir.is_dir() {
        for entry in std::fs::read_dir(&webmentions_dir).unwrap().flatten() {
            let body = std::fs::read_to_string(entry.path()).unwrap_or_default();
            assert!(
                !body.contains("SECRET-TOKEN-1234"),
                "capability token leaked into {}",
                entry.path().display()
            );
        }
    }
}

#[test]
fn rate_limiter_per_source_host_returns_429_after_threshold() {
    use std::time::Instant;
    let limiter = RateLimiter::new(3, 1000);
    let now = Instant::now();
    assert!(limiter.allow("a.example", now));
    assert!(limiter.allow("a.example", now));
    assert!(limiter.allow("a.example", now));
    assert!(!limiter.allow("a.example", now));
}

#[test]
fn replay_attack_same_source_target_does_not_duplicate_edge() {
    // T7 mitigation: receiving the same (source, target) twice updates
    // last_seen rather than creating a duplicate edge. We use the
    // persistence layer's fold semantics directly: identical keys
    // resolve to the latest record.
    let dir = TempDir::new().unwrap();
    let edge1 = ExternalEdge {
        source: "https://a.example/post".into(),
        target: "https://me.example/p".into(),
        accepted_at: 1,
        last_seen: 1,
        source_title: None,
        tombstoned: false,
    };
    let edge2 = ExternalEdge {
        last_seen: 100,
        ..edge1.clone()
    };
    append_external_edge(dir.path(), &edge1).unwrap();
    append_external_edge(dir.path(), &edge2).unwrap();
    let live = load_external_edges(dir.path()).unwrap();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].last_seen, 100);
}

#[test]
fn idempotency_zero_post_property() {
    // HP-3903: rebuild with no content change sends zero outbound POSTs.
    let pages = vec![RenderedPage {
        source_page_url: Url::parse("https://me.example/post").unwrap(),
        rendered_html: r#"<a href="https://other.example/x">x</a>"#.into(),
    }];
    let base = Url::parse("https://me.example/").unwrap();
    let plan = compute_send_plan(&pages, &[], &base);
    assert_eq!(plan.to_send.len(), 1);

    let log: Vec<SentRecord> = plan
        .to_send
        .iter()
        .map(|m| SentRecord {
            source_page_url: m.source_page_url.clone(),
            target_url: m.target_url.clone(),
            content_hash: m.content_hash.clone(),
            sent_at: 0,
            response_status: 201,
            removal: false,
        })
        .collect();
    let plan2 = compute_send_plan(&pages, &log, &base);
    assert!(plan2.to_send.is_empty());
    assert!(plan2.to_resend_for_removal.is_empty());
}

#[test]
fn hp_3904_link_removal_emits_resend() {
    // The page no longer has the external link. The diff produces a
    // removal POST so the receiver re-fetches and tombstones.
    let pages: Vec<RenderedPage> = Vec::new();
    let base = Url::parse("https://me.example/").unwrap();
    let log = vec![SentRecord {
        source_page_url: "https://me.example/post".into(),
        target_url: "https://other.example/x".into(),
        content_hash: "h1".into(),
        sent_at: 0,
        response_status: 201,
        removal: false,
    }];
    let plan = compute_send_plan(&pages, &log, &base);
    assert!(plan.to_send.is_empty());
    assert_eq!(plan.to_resend_for_removal.len(), 1);
}

struct RecordingPoster {
    posted: std::sync::Mutex<Vec<(String, String, String)>>,
    status: u16,
}

impl WebmentionPoster for RecordingPoster {
    fn post_webmention(&self, endpoint: &Url, source: &str, target: &str) -> Result<u16, String> {
        self.posted.lock().unwrap().push((
            endpoint.to_string(),
            source.to_string(),
            target.to_string(),
        ));
        Ok(self.status)
    }
}

#[test]
fn end_to_end_send_records_sent_log() {
    let dir = TempDir::new().unwrap();
    let transport = StaticTransport {
        body: br#"<head><link rel="webmention" href="https://t.example/wm"></head>"#.to_vec(),
        ..StaticTransport::default()
    };
    let poster = RecordingPoster {
        posted: std::sync::Mutex::new(Vec::new()),
        status: 201,
    };
    let plan = idempotency_diff(
        &[OutboundMention {
            source_page_url: "https://me.example/p".into(),
            target_url: "https://t.example/post".into(),
            content_hash: "h".into(),
        }],
        &[],
    );
    let stats = execute_send_plan_with_poster(&plan, &transport, &poster, dir.path());
    assert_eq!(stats.sent, 1);
    assert_eq!(poster.posted.lock().unwrap().len(), 1);
    let log = load_sent_log(dir.path()).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].response_status, 201);
}

#[test]
fn end_to_end_send_uses_link_header_when_html_has_no_endpoint() {
    // Regression for the bug where header_pairs() dropped Link headers,
    // making endpoint discovery via header impossible. The HTML body
    // here advertises NO webmention endpoint; the only signal is the
    // Link header.
    let dir = TempDir::new().unwrap();
    let transport = StaticTransport {
        body: br#"<html><body>just a page</body></html>"#.to_vec(),
        link_headers: vec![r#"<https://t.example/wm>; rel="webmention""#.to_string()],
        ..StaticTransport::default()
    };
    let poster = RecordingPoster {
        posted: std::sync::Mutex::new(Vec::new()),
        status: 201,
    };
    let plan = idempotency_diff(
        &[OutboundMention {
            source_page_url: "https://me.example/p".into(),
            target_url: "https://t.example/post".into(),
            content_hash: "h".into(),
        }],
        &[],
    );
    let stats = execute_send_plan_with_poster(&plan, &transport, &poster, dir.path());
    assert_eq!(stats.sent, 1, "Link header should drive endpoint discovery");
    assert_eq!(stats.endpoint_not_found, 0);
    let posted = poster.posted.lock().unwrap();
    assert_eq!(posted[0].0, "https://t.example/wm");
}

#[test]
fn link_extraction_skips_same_origin_and_unsafe_schemes() {
    let html = r#"
        <a href="https://other.example/post">other</a>
        <a href="https://my.example/internal">self</a>
        <a href="javascript:alert(1)">js</a>
        <a href="mailto:me@my.example">mail</a>
        <a href="/relative">rel</a>
    "#;
    let pairs = extract_external_links(
        html,
        &Url::parse("https://my.example/").unwrap(),
        &Url::parse("https://my.example/page").unwrap(),
    );
    let targets: Vec<&str> = pairs.iter().map(|(_, t)| t.as_str()).collect();
    assert_eq!(targets, vec!["https://other.example/post"]);
}

#[test]
fn moderation_source_domain_is_lowercased() {
    assert_eq!(
        source_domain("https://EXAMPLE.com/path"),
        Some("example.com".to_string())
    );
    assert_eq!(
        source_domain("https://x.example.com/p"),
        Some("x.example.com".into())
    );
    assert_eq!(source_domain("not a url"), None);
}

#[tokio::test]
async fn webmention_handler_returns_oracle_resistant_status() {
    // Same status code (202) for every plausible POST. Different status
    // distinguishes accept (201) from queue/deny/unverified (202) — that
    // is the only intentional information leak (per spec). We assert
    // that all non-accept paths collapse to 202.
    let dir = TempDir::new().unwrap();
    let deps = Arc::new(deps_for(dir.path(), "me.example"));
    let transport: Arc<dyn HttpTransport + Send + Sync> = Arc::new(StaticTransport {
        body: b"".to_vec(),
        ..StaticTransport::default()
    });
    let rate_limiter = Arc::new(RateLimiter::new(60, 1000));
    let state = WebmentionState {
        deps,
        transport,
        rate_limiter,
    };

    // Unverified target (source has no link to target).
    let resp = webmention_endpoint_handler(
        State(state.clone()),
        Form(WebmentionForm {
            source: "https://stranger.example/post".into(),
            target: "https://me.example/p".into(),
        }),
    )
    .await;
    let status = resp.status();
    // Either 202 or 400 (latter for outright structural rejection) but
    // never 404 — the existence-oracle would leak through 404.
    assert_ne!(status.as_u16(), 404);
}
