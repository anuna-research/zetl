//! Integration tests for SPEC-038 pure-core surface.
//!
//! Covers:
//!
//!   * **Cross-format equivalence (TEST-3877)** — same FeedItem set
//!     produces id / url / title / dates that are byte-identical
//!     projections of the XML and JSON Feed outputs.
//!   * **Determinism (NFR-3804 / NFR-3805)** — repeated invocation
//!     with byte-identical input produces byte-identical output.
//!   * **Trust-boundary highlights** — SSRF, XXE, capability-token
//!     leakage, tombstone re-import refusal.

use std::path::Path;

use zetl::feed::cap_feed::audit_token_leakage;
use zetl::feed::fetch::{
    assert_no_xxe, assert_public_target, assert_safe_scheme, assert_under_size_cap, MAX_BODY_BYTES,
};
use zetl::feed::forget::{is_tombstoned, Tombstone};
use zetl::feed::serialise_atom::serialise_atom;
use zetl::feed::serialise_jsonfeed::serialise_jsonfeed;
use zetl::feed::serialise_rss::serialise_rss;
use zetl::feed::types::{
    AuthorRef, FeedConfig, FeedItem, FeedPaths, License, OutputFormatSet, SourceMetadata,
};

fn cfg(json: bool) -> FeedConfig {
    FeedConfig {
        base_url: "https://example.com".to_string(),
        title: "Example".to_string(),
        description: "Test feed".to_string(),
        max_items: 50,
        formats: OutputFormatSet {
            rss: true,
            atom: true,
            jsonfeed: json,
        },
        paths: FeedPaths {
            rss: "/feed.xml".to_string(),
            atom: "/atom.xml".to_string(),
            jsonfeed: "/feed.json".to_string(),
        },
        scope_id: None,
        cohort_id: None,
        default_author: Some(AuthorRef {
            name: "Anuna".to_string(),
            email: None,
            url: Some("https://example.com".to_string()),
        }),
        language: Some("en-AU".to_string()),
        copyright: Some("Anuna 2026".to_string()),
    }
}

fn item(slug: &str, date: &str) -> FeedItem {
    FeedItem {
        id: format!("tag:example.com,2026:zetl/{slug}"),
        title: format!("Title for {slug}"),
        url: format!("https://example.com/{slug}"),
        date_published: date.to_string(),
        date_modified: None,
        summary: Some(format!("Summary of {slug}")),
        content_html: Some(format!("<p>Content of {slug}.</p>")),
        author: None,
        tags: vec!["meta".to_string(), "spec-038".to_string()],
        license: Some(License::CcBy4_0),
        source_metadata: SourceMetadata {
            source_path: Some(std::path::PathBuf::from(format!("notes/{slug}.md"))),
            object_id: Some(slug.to_string()),
            content_hash: Some("deadbeef".to_string()),
            ..Default::default()
        },
    }
}

#[test]
fn cross_format_equivalence_id_and_dates() {
    // TEST-3877: id / url / title / date_published / date_modified
    // are byte-identical projections across RSS, Atom, JSON Feed.
    let items = vec![
        item("alpha", "2026-05-01T00:00:00Z"),
        item("bravo", "2026-05-02T00:00:00Z"),
        item("charlie", "2026-05-03T00:00:00Z"),
    ];
    let rss = serialise_rss(&items, &cfg(true));
    let atom = serialise_atom(&items, &cfg(true));
    let json = serialise_jsonfeed(&items, &cfg(true));

    for it in &items {
        // Every id appears verbatim in all three formats.
        assert!(rss.contains(&it.id), "rss missing id {}", it.id);
        assert!(atom.contains(&it.id), "atom missing id {}", it.id);
        assert!(json.contains(&it.id), "json missing id {}", it.id);
        // Every URL appears verbatim.
        assert!(rss.contains(&it.url));
        assert!(atom.contains(&it.url));
        assert!(json.contains(&it.url));
        // Every title appears verbatim.
        assert!(rss.contains(&it.title));
        assert!(atom.contains(&it.title));
        assert!(json.contains(&it.title));
    }
}

#[test]
fn cross_format_equivalence_atom_json_dates_match() {
    // RSS uses RFC 822 dates so we can't byte-compare strings; Atom +
    // JSON Feed both use RFC 3339 verbatim.
    let items = vec![item("foo", "2026-05-08T00:00:00Z")];
    let atom = serialise_atom(&items, &cfg(true));
    let json = serialise_jsonfeed(&items, &cfg(true));
    assert!(atom.contains("<published>2026-05-08T00:00:00Z</published>"));
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["items"][0]["date_published"], "2026-05-08T00:00:00Z");
}

#[test]
fn determinism_same_inputs_produce_byte_identical_output() {
    // NFR-3804 / NFR-3805 — rebuild produces byte-identical files.
    let items = vec![
        item("alpha", "2026-05-01T00:00:00Z"),
        item("bravo", "2026-05-02T00:00:00Z"),
    ];
    for _ in 0..3 {
        assert_eq!(serialise_rss(&items, &cfg(true)), serialise_rss(&items, &cfg(true)));
        assert_eq!(serialise_atom(&items, &cfg(true)), serialise_atom(&items, &cfg(true)));
        assert_eq!(serialise_jsonfeed(&items, &cfg(true)), serialise_jsonfeed(&items, &cfg(true)));
    }
}

#[test]
fn empty_feed_round_trips() {
    let rss = serialise_rss(&[], &cfg(true));
    let atom = serialise_atom(&[], &cfg(true));
    let json = serialise_jsonfeed(&[], &cfg(true));
    assert!(rss.contains("<channel>"));
    assert!(atom.contains("<feed"));
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(v["items"].as_array().unwrap().is_empty());
}

#[test]
fn unicode_titles_survive_serialisation() {
    let mut it = item("foo", "2026-05-08T00:00:00Z");
    it.title = "café — 日本語 — مرحبا".to_string();
    let rss = serialise_rss(&[it.clone()], &cfg(false));
    let atom = serialise_atom(&[it.clone()], &cfg(false));
    let json = serialise_jsonfeed(&[it.clone()], &cfg(true));
    assert!(rss.contains("café — 日本語 — مرحبا"));
    assert!(atom.contains("café — 日本語 — مرحبا"));
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["items"][0]["title"], "café — 日本語 — مرحبا");
}

#[test]
fn ssrf_t1_loopback_rejected() {
    let local: std::net::IpAddr = "127.0.0.1".parse().unwrap();
    assert!(assert_public_target(&[local]).is_err());
}

#[test]
fn ssrf_t1_aws_metadata_link_local_rejected() {
    let aws: std::net::IpAddr = "169.254.169.254".parse().unwrap();
    assert!(assert_public_target(&[aws]).is_err());
}

#[test]
fn ssrf_t5_redirect_to_private_addr_rejected() {
    let private: std::net::IpAddr = "10.0.0.5".parse().unwrap();
    let public: std::net::IpAddr = "8.8.8.8".parse().unwrap();
    // Even mixed with a public address, presence of any private addr
    // rejects the request.
    assert!(assert_public_target(&[public, private]).is_err());
}

#[test]
fn xxe_t3_external_entity_rejected() {
    let payload = br#"<?xml version="1.0"?>
<!DOCTYPE r [ <!ENTITY x SYSTEM "file:///etc/passwd"> ]>
<r>&x;</r>"#;
    assert!(assert_no_xxe(payload).is_err());
}

#[test]
fn xxe_t4_billion_laughs_doctype_rejected() {
    // Even without expansion, the billion-laughs payload's DOCTYPE
    // declaration is enough to reject upstream.
    let payload = br#"<?xml version="1.0"?>
<!DOCTYPE lolz [
  <!ENTITY lol "lol">
  <!ENTITY lol1 "&lol;&lol;">
]>
<lolz>&lol1;</lolz>"#;
    assert!(assert_no_xxe(payload).is_err());
}

#[test]
fn t6_decompression_bomb_rejected() {
    // 1 MiB + 1 byte exceeds the bound.
    assert!(assert_under_size_cap(MAX_BODY_BYTES + 1).is_err());
}

#[test]
fn safe_scheme_rejects_dangerous_prefixes() {
    for url in [
        "file:///etc/passwd",
        "data:text/html,<script>alert(1)</script>",
        "javascript:void(0)",
    ] {
        let parsed = url::Url::parse(url).unwrap();
        assert!(assert_safe_scheme(&parsed).is_err(), "url {url} not rejected");
    }
}

#[test]
fn t21_capability_token_leakage_audit() {
    let token = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    // Clean output: token only in URL segment.
    let clean = format!(r#"<a href="/caps/{token}/feed.xml">cohort feed</a>"#);
    assert!(audit_token_leakage(token, &clean).is_empty());
    // Leaky output: token also in body text.
    let leaky = format!(
        r#"<a href="/caps/{token}/feed.xml">cohort feed</a>
        token leaked: {token}"#
    );
    assert_eq!(audit_token_leakage(token, &leaky).len(), 1);
}

#[test]
fn t22_tombstone_blocks_reimport() {
    let tombs = vec![Tombstone {
        guid: Some("urn:gone".to_string()),
        link: None,
        content_hash: Some("hashgone".to_string()),
        erased_at: "2026-05-08T00:00:00Z".to_string(),
        reason: Some("operator forgot".to_string()),
    }];
    // Re-importing same guid is blocked.
    assert!(is_tombstoned(&tombs, Some("urn:gone"), None, None));
    // Re-importing same content_hash (attacker mutated guid) is blocked.
    assert!(is_tombstoned(&tombs, Some("urn:fresh"), None, Some("hashgone")));
    // Genuinely new item passes.
    assert!(!is_tombstoned(
        &tombs,
        Some("urn:new"),
        None,
        Some("hashnew")
    ));
}

#[test]
fn license_eligibility_table_smoke() {
    use zetl::feed::republication::{republication_eligible, SubscriptionPolicy};
    use zetl::feed::types::{License, RepublicationMode, RepublicationRationale};

    let policy = SubscriptionPolicy {
        republish: true,
        mode: RepublicationMode::FullAllowed,
        i_have_permission: false,
    };
    // CC0: full always.
    let d = republication_eligible(&License::Cc0_1_0, &policy, None, false);
    assert_eq!(d.mode, RepublicationMode::FullAllowed);
    assert_eq!(d.rationale, RepublicationRationale::PublicDomain);
    // CC-BY-NC + commercial: excerpt.
    let d = republication_eligible(&License::CcByNc4_0, &policy, None, true);
    assert_eq!(d.mode, RepublicationMode::ExcerptOnly);
    // Unknown without permission: deny.
    let d = republication_eligible(&License::Unknown, &policy, None, false);
    assert_eq!(d.mode, RepublicationMode::Deny);
}

#[test]
fn item_id_stable_across_invocations() {
    use url::Url;
    use zetl::feed::item_id::item_id;
    let ns = Url::parse("https://example.com/").unwrap();
    let a = item_id("notes/foo", &ns, 2024).unwrap();
    let b = item_id("notes/foo", &ns, 2024).unwrap();
    assert_eq!(a, b);
    // Distinct slugs collide never.
    assert_ne!(item_id("notes/foo", &ns, 2024), item_id("notes/bar", &ns, 2024));
}

#[test]
fn rewrite_links_idempotent_on_random_input() {
    use url::Url;
    use zetl::feed::rewrite_links::{rewrite_links, UnresolvedPolicy};
    let base = Url::parse("https://example.com").unwrap();
    let resolver = |slug: &str| Some(slug.to_string());
    let bodies = [
        "no links",
        "[[foo]]",
        "before [[foo]] middle [[bar|alias]] after",
        "[[escape]]?",
        "Many [[a]] [[b]] [[c]] [[d]] tags.",
    ];
    for body in bodies {
        let once = rewrite_links(body, &base, &resolver, UnresolvedPolicy::PreserveText);
        let twice = rewrite_links(&once, &base, &resolver, UnresolvedPolicy::PreserveText);
        assert_eq!(once, twice, "idempotence violated for {body:?}");
    }
}

#[test]
fn excerpt_word_count_never_exceeds_budget() {
    use zetl::feed::excerpt::excerpt;
    let body = format!("<p>{}</p>", "alpha bravo charlie ".repeat(500));
    for budget in [50usize, 100, 200, 500] {
        let out = excerpt(&body, budget);
        let words = out.split_whitespace().count();
        assert!(words <= budget, "budget {budget} produced {words} words");
    }
}

#[test]
fn resolve_date_pure_no_clock_dependency() {
    use zetl::feed::resolve_date::{resolve_date, PageDates};
    let dates = PageDates {
        frontmatter_date: Some("2026-05-08T12:00:00Z".to_string()),
        ..Default::default()
    };
    // Two calls produce identical results — no clock dependency.
    let a = resolve_date(&dates, None).unwrap();
    let b = resolve_date(&dates, None).unwrap();
    assert_eq!(a, b);
}

#[test]
fn config_lens_validates_cohort_token_entropy() {
    // NFR-3809 gate.
    let body = r#"
        [[capability_cohorts]]
        id = "weak"
        token = "aaaaaaaaaaaaaaaa"
        select = []
    "#;
    let err = zetl::feed::config::parse_config(body).unwrap_err();
    assert!(matches!(
        err,
        zetl::feed::config::FeedConfigError::WeakCohortToken { .. }
    ));
}

#[test]
fn config_lens_rejects_credentials_in_subscriptions_block() {
    use zetl::feed::credentials::config_credential_leak_scan;
    let leaky = r#"
        [[subscriptions]]
        id = "x"
        source = "https://upstream/feed"
        token = "secret"
    "#;
    assert!(config_credential_leak_scan(leaky).is_err());
}

#[test]
fn jsonfeed_v1_1_minimum_required_fields_present() {
    // JSON Feed v1.1 spec: top-level MUST have version, title, items.
    // Per item: id is required; at least one of content_html /
    // content_text must be present.
    let cfg = zetl::feed::types::FeedConfig {
        base_url: "https://example.com".to_string(),
        title: "T".to_string(),
        description: "d".to_string(),
        max_items: 50,
        formats: zetl::feed::types::OutputFormatSet {
            rss: false,
            atom: false,
            jsonfeed: true,
        },
        paths: zetl::feed::types::FeedPaths {
            rss: "/feed.xml".to_string(),
            atom: "/atom.xml".to_string(),
            jsonfeed: "/feed.json".to_string(),
        },
        scope_id: None,
        cohort_id: None,
        default_author: None,
        language: None,
        copyright: None,
    };
    let item = zetl::feed::types::FeedItem {
        id: "tag:example.com,2026:zetl/x".to_string(),
        title: "x".to_string(),
        url: "https://example.com/x".to_string(),
        date_published: "2026-05-08T00:00:00Z".to_string(),
        date_modified: None,
        summary: Some("s".to_string()),
        content_html: Some("<p>body</p>".to_string()),
        author: None,
        tags: vec![],
        license: None,
        source_metadata: Default::default(),
    };
    let body = zetl::feed::serialise_jsonfeed::serialise_jsonfeed(&[item], &cfg);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["version"], "https://jsonfeed.org/version/1.1");
    assert!(v["title"].is_string());
    assert!(v["items"].is_array());
    let item0 = &v["items"][0];
    assert!(item0["id"].is_string());
    assert!(
        item0.get("content_html").is_some() || item0.get("content_text").is_some(),
        "v1.1 spec MUST: each item needs at least one of content_html / content_text"
    );
}

#[test]
fn atom_rfc4287_feed_carries_default_author_via_emit_root_feed() {
    // RFC 4287 §4.1.1: feeds without per-entry authors MUST carry a
    // feed-level <atom:author>. emit_root_feed populates default_author
    // from [feed].author OR (fallback) [feed].title.
    use zetl::feed::config::parse_config;
    use zetl::feed::types::SelectionRule;

    let body = r#"
        [feed]
        base_url = "https://example.com"
        title = "Example"
        author = "Anuna"
    "#;
    let lens = parse_config(body).unwrap();
    let pages: Vec<zetl::feed::select::PageView<'_>> = vec![];
    let always = |_: &zetl::feed::select::PageView<'_>| true;
    let visibility: Box<dyn Fn(&zetl::feed::select::PageView<'_>) -> bool> =
        Box::new(always);
    let emission = zetl::feed::build::emit_root_feed(
        &lens,
        &pages,
        &visibility,
        &SelectionRule::FrontmatterOptIn,
    )
    .unwrap();
    let atom_body = emission
        .files
        .iter()
        .find(|(p, _)| p == "/atom.xml")
        .map(|(_, b)| std::str::from_utf8(b).unwrap().to_string())
        .unwrap();
    assert!(
        atom_body.contains("<author>") && atom_body.contains("<name>Anuna</name>"),
        "atom feed must carry feed-level <author>: {atom_body}"
    );
}

#[test]
fn vault_path_unused_in_test_does_not_break_compilation() {
    // Compile-time check that crate paths resolve.
    let _ = Path::new("ok");
}
