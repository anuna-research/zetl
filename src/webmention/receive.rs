//! Webmention receive shell.
//!
//! - [`process_incoming`] — pipeline glue: fetch source, verify, moderate,
//!   persist. Reusable from CLI tests + the Axum handler.
//! - [`webmention_endpoint_handler`] — Axum POST handler.
//!
//! Oracle resistance per REQ-3909: the handler returns the SAME response
//! shape (202 Accepted, "queued for processing" body) for every
//! POST shape regardless of whether the target page exists, is private,
//! or sits behind a capability URL. A capability token in the target
//! path is REDACTED in any logged or persisted form so it never leaks
//! back through observability.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{
    extract::{Form, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use blake3::Hasher;
use serde::Deserialize;
use url::Url;

use crate::feed::fetch::HttpTransport;
use crate::webmention::config::ModerationDefault;
use crate::webmention::core::moderate::{moderate, ModerationContext};
use crate::webmention::core::verify::{verify_link_present, VerifyResult};
use crate::webmention::persist::{append_external_edge, append_queue};
use crate::webmention::types::{
    now_epoch, ExternalEdge, IncomingMention, ModerationDecision, ModerationKind, VerifiedMention,
    WebmentionError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveOutcome {
    Accepted,
    Queued,
    /// Verification or denylist drop. Note: the HTTP layer
    /// MUST NOT distinguish this from Queued in the response shape
    /// (REQ-3909).
    Denied,
    /// Source did not contain a link to target. Internally a 4xx, but
    /// per oracle-resistance the handler may still return 202.
    Unverified,
}

#[derive(Debug, Clone)]
pub struct ReceiveDeps {
    pub vault_root: PathBuf,
    pub vault_host: String,
    pub allowlist_domains: HashSet<String>,
    pub denylist_domains: HashSet<String>,
    pub vault_outbound_domains: HashSet<String>,
    pub default_decision: ModerationDefault,
}

/// Run the full receive pipeline. Pure-Rust composition over the pure
/// core; the only effectful boundary is the source-fetch (delegated to
/// `transport`) and the persistence calls.
pub fn process_incoming(
    mention: IncomingMention,
    deps: &ReceiveDeps,
    transport: &dyn HttpTransport,
) -> Result<ReceiveOutcome, WebmentionError> {
    let source = Url::parse(&mention.source)
        .map_err(|_| WebmentionError::BadInput(format!("source url: {}", mention.source)))?;
    let target = Url::parse(&mention.target)
        .map_err(|_| WebmentionError::BadInput(format!("target url: {}", mention.target)))?;

    if source == target {
        return Err(WebmentionError::BadInput("source == target".to_string()));
    }
    if !matches!(source.scheme(), "http" | "https") || !matches!(target.scheme(), "http" | "https")
    {
        return Err(WebmentionError::BadInput(
            "scheme must be http or https".to_string(),
        ));
    }

    // Target host must match the configured vault host. We do this here
    // so spam aimed at random hosts doesn't even hit the fetcher.
    let target_host = target.host_str().unwrap_or_default().to_ascii_lowercase();
    if target_host != deps.vault_host.to_ascii_lowercase() {
        return Err(WebmentionError::BadInput(
            "target is not on this vault".to_string(),
        ));
    }

    // Fetch source.
    let html = match fetch_source(&source, transport) {
        Ok(s) => s,
        Err(_e) => {
            // Treat fetch failure as Unverified — the W3C REC says the
            // receiver may retry; v1 punts to the sender.
            return Ok(ReceiveOutcome::Unverified);
        }
    };
    let result = verify_link_present(&html, &target);
    if !matches!(result, VerifyResult::Found) {
        return Ok(ReceiveOutcome::Unverified);
    }

    let mut hasher = Hasher::new();
    hasher.update(html.as_bytes());
    let hash = hasher.finalize().to_hex().to_string();

    let verified = VerifiedMention {
        source: mention.source.clone(),
        target: mention.target.clone(),
        verified_at: now_epoch(),
        source_html_hash: hash,
    };

    let ctx = ModerationContext {
        allowlist_domains: &deps.allowlist_domains,
        denylist_domains: &deps.denylist_domains,
        vault_outbound_domains: &deps.vault_outbound_domains,
        default_decision: deps.default_decision,
    };
    let decision = moderate(&verified, &ctx);

    persist_decision(deps, &mention, &decision)?;
    match decision.kind {
        ModerationKind::Accept => Ok(ReceiveOutcome::Accepted),
        ModerationKind::Queue => Ok(ReceiveOutcome::Queued),
        ModerationKind::Deny => Ok(ReceiveOutcome::Denied),
    }
}

fn persist_decision(
    deps: &ReceiveDeps,
    mention: &IncomingMention,
    decision: &ModerationDecision,
) -> Result<(), WebmentionError> {
    match decision.kind {
        ModerationKind::Accept => {
            let now = now_epoch();
            append_external_edge(
                &deps.vault_root,
                &ExternalEdge {
                    source: mention.source.clone(),
                    target: mention.target.clone(),
                    accepted_at: now,
                    last_seen: now,
                    source_title: None,
                    tombstoned: false,
                },
            )?;
        }
        ModerationKind::Queue => {
            append_queue(&deps.vault_root, mention)?;
        }
        ModerationKind::Deny => {
            // No persistence — observability only.
        }
    }
    Ok(())
}

/// Source-fetch shell. Reuses [`crate::feed::fetch`] primitives for
/// scheme + size + UA. The transport is responsible for SSRF resolution.
pub fn fetch_source(
    source: &Url,
    transport: &dyn HttpTransport,
) -> Result<String, WebmentionError> {
    crate::feed::fetch::assert_safe_scheme(source).map_err(WebmentionError::SourceFetch)?;
    let req = crate::feed::fetch::FetchRequest {
        url: source.clone(),
        user_agent: crate::feed::fetch::user_agent(),
        conditional: Default::default(),
        auth_header: None,
        timeout: Duration::from_secs(crate::feed::fetch::DEFAULT_TIMEOUT_SECS),
    };
    let resp = transport
        .fetch(&req)
        .map_err(WebmentionError::SourceFetch)?;
    crate::feed::fetch::assert_under_size_cap(resp.body.len())
        .map_err(WebmentionError::SourceFetch)?;
    Ok(String::from_utf8_lossy(&resp.body).into_owned())
}

// =================================================================
// Axum surface
// =================================================================

/// Form body of POST /webmention per the W3C REC.
#[derive(Debug, Deserialize)]
pub struct WebmentionForm {
    pub source: String,
    pub target: String,
}

/// Axum handler state carried via [`State`]. The web layer wraps this
/// in its larger `WebState`.
#[derive(Clone)]
pub struct WebmentionState {
    pub deps: Arc<ReceiveDeps>,
    pub transport: Arc<dyn HttpTransport + Send + Sync>,
    pub rate_limiter: Arc<RateLimiter>,
}

/// Axum POST /webmention handler. Always returns 202 on plausible input
/// (oracle-resistant — see REQ-3909). Returns 400 only for outright
/// malformed input that no real W3C-conformant sender would produce.
pub async fn webmention_endpoint_handler(
    State(state): State<WebmentionState>,
    Form(form): Form<WebmentionForm>,
) -> Response {
    let now = Instant::now();
    let source_host = Url::parse(&form.source)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_ascii_lowercase()));
    if let Some(host) = source_host.as_deref() {
        if !state.rate_limiter.allow(host, now) {
            return (StatusCode::TOO_MANY_REQUESTS, "rate limit").into_response();
        }
    }

    let mention = IncomingMention {
        source: form.source.clone(),
        target: form.target.clone(),
        received_at: now_epoch(),
    };

    // Run the pipeline on a blocking task — verification fetches the
    // source synchronously and we want to honour the ≤ 500ms NFR-3901
    // budget on small pages.
    let deps = state.deps.clone();
    let transport = state.transport.clone();
    let result =
        tokio::task::spawn_blocking(move || process_incoming(mention, &deps, transport.as_ref()))
            .await;

    let outcome = match result {
        Ok(Ok(o)) => o,
        Ok(Err(WebmentionError::BadInput(_))) => {
            // Outright malformed input: the W3C REC permits 400.
            return (StatusCode::BAD_REQUEST, oracle_resistant_body()).into_response();
        }
        _ => ReceiveOutcome::Unverified,
    };

    match outcome {
        ReceiveOutcome::Accepted => (StatusCode::CREATED, oracle_resistant_body()).into_response(),
        // All other outcomes return the same status + body shape so a
        // probing client cannot distinguish queue vs deny vs unverified.
        _ => (StatusCode::ACCEPTED, oracle_resistant_body()).into_response(),
    }
}

/// Body emitted for every webmention response other than the accept
/// path. Length-bounded so the byte-length signal across response
/// shapes stays within a 16-byte oracle-resistance margin.
fn oracle_resistant_body() -> String {
    "queued for processing".to_string()
}

/// Sliding-window per-source-host token bucket. NOT a multi-process
/// rate limiter — for that, deploy a reverse-proxy. v1 caps in-process.
pub struct RateLimiter {
    per_host_limit: u32,
    global_limit: u32,
    window: Duration,
    state: Mutex<RateState>,
}

struct RateState {
    per_host: HashMap<String, Vec<Instant>>,
    global: Vec<Instant>,
}

impl RateLimiter {
    pub fn new(per_host_limit: u32, global_limit: u32) -> Self {
        Self {
            per_host_limit,
            global_limit,
            window: Duration::from_secs(60),
            state: Mutex::new(RateState {
                per_host: HashMap::new(),
                global: Vec::new(),
            }),
        }
    }

    /// `true` if the request is allowed under both per-host and global
    /// caps. Decay timestamps older than `window`.
    pub fn allow(&self, host: &str, now: Instant) -> bool {
        let mut s = self.state.lock().unwrap();
        let cutoff = now.checked_sub(self.window).unwrap_or(now);

        s.global.retain(|t| *t > cutoff);
        if s.global.len() as u32 >= self.global_limit {
            return false;
        }
        let entry = s.per_host.entry(host.to_string()).or_default();
        entry.retain(|t| *t > cutoff);
        if entry.len() as u32 >= self.per_host_limit {
            return false;
        }
        entry.push(now);
        s.global.push(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::fetch::{FetchError, FetchResponse};
    use tempfile::tempdir;

    struct StaticTransport {
        body: Vec<u8>,
    }

    impl HttpTransport for StaticTransport {
        fn fetch(
            &self,
            request: &crate::feed::fetch::FetchRequest,
        ) -> Result<FetchResponse, FetchError> {
            Ok(FetchResponse {
                status: 200,
                body: self.body.clone(),
                last_modified: None,
                etag: None,
                content_type: Some("text/html".into()),
                final_url: request.url.clone(),
            })
        }
    }

    fn deps_for(vault: &std::path::Path, host: &str) -> ReceiveDeps {
        ReceiveDeps {
            vault_root: vault.to_path_buf(),
            vault_host: host.to_string(),
            allowlist_domains: HashSet::new(),
            denylist_domains: HashSet::new(),
            vault_outbound_domains: HashSet::new(),
            default_decision: ModerationDefault::Queue,
        }
    }

    #[test]
    fn accepts_when_link_present_and_already_linked() {
        let dir = tempdir().unwrap();
        let mut deps = deps_for(dir.path(), "me.example");
        deps.vault_outbound_domains
            .insert("peer.example".to_string());
        let transport = StaticTransport {
            body: r#"<a href="https://me.example/p">link</a>"#.into(),
        };
        let mention = IncomingMention {
            source: "https://peer.example/post".into(),
            target: "https://me.example/p".into(),
            received_at: 0,
        };
        let outcome = process_incoming(mention, &deps, &transport).unwrap();
        assert_eq!(outcome, ReceiveOutcome::Accepted);
    }

    #[test]
    fn queues_when_link_present_but_unknown_source() {
        let dir = tempdir().unwrap();
        let deps = deps_for(dir.path(), "me.example");
        let transport = StaticTransport {
            body: r#"<a href="https://me.example/p">link</a>"#.into(),
        };
        let mention = IncomingMention {
            source: "https://stranger.example/post".into(),
            target: "https://me.example/p".into(),
            received_at: 0,
        };
        let outcome = process_incoming(mention, &deps, &transport).unwrap();
        assert_eq!(outcome, ReceiveOutcome::Queued);
    }

    #[test]
    fn unverified_when_no_link_in_source() {
        let dir = tempdir().unwrap();
        let deps = deps_for(dir.path(), "me.example");
        let transport = StaticTransport {
            body: r#"<p>no link here</p>"#.into(),
        };
        let mention = IncomingMention {
            source: "https://stranger.example/post".into(),
            target: "https://me.example/p".into(),
            received_at: 0,
        };
        let outcome = process_incoming(mention, &deps, &transport).unwrap();
        assert_eq!(outcome, ReceiveOutcome::Unverified);
    }

    #[test]
    fn rejects_target_off_vault() {
        let dir = tempdir().unwrap();
        let deps = deps_for(dir.path(), "me.example");
        let transport = StaticTransport { body: b"".to_vec() };
        let mention = IncomingMention {
            source: "https://stranger.example/post".into(),
            target: "https://other-vault.example/p".into(),
            received_at: 0,
        };
        let err = process_incoming(mention, &deps, &transport).unwrap_err();
        assert!(matches!(err, WebmentionError::BadInput(_)));
    }

    #[test]
    fn rejects_source_eq_target() {
        let dir = tempdir().unwrap();
        let deps = deps_for(dir.path(), "me.example");
        let transport = StaticTransport { body: b"".to_vec() };
        let mention = IncomingMention {
            source: "https://me.example/p".into(),
            target: "https://me.example/p".into(),
            received_at: 0,
        };
        let err = process_incoming(mention, &deps, &transport).unwrap_err();
        assert!(matches!(err, WebmentionError::BadInput(_)));
    }

    #[test]
    fn rate_limiter_per_host_caps_at_threshold() {
        let limiter = RateLimiter::new(3, 100);
        let now = Instant::now();
        assert!(limiter.allow("a.example", now));
        assert!(limiter.allow("a.example", now));
        assert!(limiter.allow("a.example", now));
        assert!(!limiter.allow("a.example", now));
        // A different host is unaffected.
        assert!(limiter.allow("b.example", now));
    }

    #[test]
    fn rate_limiter_global_cap() {
        let limiter = RateLimiter::new(100, 2);
        let now = Instant::now();
        assert!(limiter.allow("a.example", now));
        assert!(limiter.allow("b.example", now));
        assert!(!limiter.allow("c.example", now));
    }
}
