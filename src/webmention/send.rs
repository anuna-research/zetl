//! Outbound webmention send pipeline.
//!
//! `compute_send_plan` is pure (delegates to [`super::core::diff`]);
//! `execute_send_plan` is the effectful shell that actually issues the
//! POSTs against an [`crate::feed::fetch::HttpTransport`].

use std::path::Path;

use blake3::Hasher;
use url::Url;

use crate::feed::fetch::{FetchRequest, FetchResponse, HttpTransport};
use crate::webmention::core::diff::{idempotency_diff, IdempotencyDiff};
use crate::webmention::core::discover::discover_endpoint;
use crate::webmention::core::extract::extract_external_links;
use crate::webmention::persist::append_sent_record;
use crate::webmention::types::{now_epoch, OutboundMention, SentRecord};

/// One rendered page plus its source URL — the input to send-plan
/// computation.
#[derive(Debug, Clone)]
pub struct RenderedPage {
    pub source_page_url: Url,
    pub rendered_html: String,
}

/// blake3 of a stable representation of the link's surrounding context.
/// In v1 we hash the entire rendered HTML — simple and correct: any
/// change to the page invalidates every external link's hash and the
/// receiver re-fetches. Future work could narrow this to the surrounding
/// paragraph for fewer false-positive resends.
fn content_hash(rendered_html: &str) -> String {
    let mut h = Hasher::new();
    h.update(rendered_html.as_bytes());
    h.finalize().to_hex().to_string()
}

/// Compute the build's outbound-POST plan against `previous_log`.
/// Pure modulo `previous_log` ordering.
pub fn compute_send_plan(
    pages: &[RenderedPage],
    previous_log: &[SentRecord],
    vault_base_url: &Url,
) -> IdempotencyDiff {
    let mut current: Vec<OutboundMention> = Vec::new();
    for page in pages {
        let hash = content_hash(&page.rendered_html);
        let pairs =
            extract_external_links(&page.rendered_html, vault_base_url, &page.source_page_url);
        for (source, target) in pairs {
            current.push(OutboundMention {
                source_page_url: source.to_string(),
                target_url: target.to_string(),
                content_hash: hash.clone(),
            });
        }
    }
    idempotency_diff(&current, previous_log)
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SendStats {
    pub sent: u32,
    pub removed: u32,
    pub endpoint_not_found: u32,
    pub failed: u32,
}

/// Execute a send plan, recording successes to `sent.jsonl`. Failed
/// POSTs do NOT enter the idempotency log so they will be retried on the
/// next build.
pub fn execute_send_plan(
    plan: &IdempotencyDiff,
    transport: &dyn HttpTransport,
    vault_root: &Path,
) -> SendStats {
    let mut stats = SendStats::default();
    for mention in &plan.to_send {
        if try_send(mention, transport, vault_root, false) {
            stats.sent += 1;
        } else {
            stats.failed += 1;
        }
    }
    for mention in &plan.to_resend_for_removal {
        if try_send(mention, transport, vault_root, true) {
            stats.removed += 1;
        } else {
            stats.endpoint_not_found += 1;
        }
    }
    stats
}

fn try_send(
    mention: &OutboundMention,
    transport: &dyn HttpTransport,
    vault_root: &Path,
    removal: bool,
) -> bool {
    let target = match Url::parse(&mention.target_url) {
        Ok(u) => u,
        Err(_) => return false,
    };

    // Step 1: discover the target's endpoint (HEAD/GET against the target).
    let endpoint = match fetch_for_discovery(transport, &target) {
        Some(resp) => {
            let html = std::str::from_utf8(&resp.body).ok();
            discover_endpoint(&header_pairs(&resp), html, &resp.final_url)
        }
        None => None,
    };
    let Some(endpoint) = endpoint else {
        return false;
    };

    // Step 2: POST source/target as application/x-www-form-urlencoded.
    // Many transports in the codebase only model GET; we'll use a
    // minimal POST adapter via the same trait by passing an explicit
    // `auth_header` slot — but the existing trait doesn't model POSTs.
    // For v1 we model the POST as a separate transport extension so the
    // tests can mock it. Real-world transports embed reqwest/ureq.
    let posted = transport_post_form(
        transport,
        &endpoint,
        &mention.source_page_url,
        &mention.target_url,
    );
    let status = match posted {
        Some(s) => s,
        None => return false,
    };
    if !(200..300).contains(&status) {
        return false;
    }
    let _ = append_sent_record(
        vault_root,
        &SentRecord {
            source_page_url: mention.source_page_url.clone(),
            target_url: mention.target_url.clone(),
            content_hash: mention.content_hash.clone(),
            sent_at: now_epoch(),
            response_status: status,
            removal,
        },
    );
    true
}

fn fetch_for_discovery(transport: &dyn HttpTransport, target: &Url) -> Option<FetchResponse> {
    let req = FetchRequest {
        url: target.clone(),
        user_agent: crate::feed::fetch::user_agent(),
        conditional: Default::default(),
        auth_header: None,
        timeout: std::time::Duration::from_secs(crate::feed::fetch::DEFAULT_TIMEOUT_SECS),
    };
    transport.fetch(&req).ok()
}

fn header_pairs(resp: &FetchResponse) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(ct) = &resp.content_type {
        out.push(("Content-Type".to_string(), ct.clone()));
    }
    if let Some(et) = &resp.etag {
        out.push(("ETag".to_string(), et.clone()));
    }
    if let Some(lm) = &resp.last_modified {
        out.push(("Last-Modified".to_string(), lm.clone()));
    }
    out
}

/// POST `source` + `target` as application/x-www-form-urlencoded to
/// `endpoint`. Real transports plug in here; the trait surface is
/// extensible via a parallel `WebmentionPoster` trait below. Returns the
/// HTTP status on success, `None` on transport error.
fn transport_post_form(
    _transport: &dyn HttpTransport,
    _endpoint: &Url,
    _source: &str,
    _target: &str,
) -> Option<u16> {
    // The default `HttpTransport` only models GET. v1 ships with a
    // separate `WebmentionPoster` (defined elsewhere in the shell wiring)
    // that the build/serve hook plugs in. This helper is a stub for
    // wiring during the pure-core/test phase. The integration tests
    // override the transport via the WebmentionPoster trait.
    None
}

/// Trait an external HTTP transport implements to POST webmention pings.
/// Decoupled from the read-only `HttpTransport` so the existing
/// SPEC-038 transport doesn't have to grow a POST surface to compile.
pub trait WebmentionPoster {
    /// POST `source`/`target` form-encoded. Returns the response status
    /// or `Err` on transport-level failure.
    fn post_webmention(&self, endpoint: &Url, source: &str, target: &str) -> Result<u16, String>;
}

/// Outcome of one send attempt against a single target.
enum DispatchOutcome {
    Sent,
    NoEndpoint,
    PostFailed,
}

/// Higher-level send-plan executor that uses both an `HttpTransport`
/// (for endpoint discovery) and a `WebmentionPoster` (for the POST).
pub fn execute_send_plan_with_poster(
    plan: &IdempotencyDiff,
    transport: &dyn HttpTransport,
    poster: &dyn WebmentionPoster,
    vault_root: &Path,
) -> SendStats {
    let mut stats = SendStats::default();
    for mention in &plan.to_send {
        match dispatch(mention, transport, poster, vault_root, false) {
            DispatchOutcome::Sent => stats.sent += 1,
            DispatchOutcome::NoEndpoint => stats.endpoint_not_found += 1,
            DispatchOutcome::PostFailed => stats.failed += 1,
        }
    }
    for mention in &plan.to_resend_for_removal {
        match dispatch(mention, transport, poster, vault_root, true) {
            DispatchOutcome::Sent => stats.removed += 1,
            DispatchOutcome::NoEndpoint => stats.endpoint_not_found += 1,
            DispatchOutcome::PostFailed => stats.failed += 1,
        }
    }
    stats
}

fn dispatch(
    mention: &OutboundMention,
    transport: &dyn HttpTransport,
    poster: &dyn WebmentionPoster,
    vault_root: &Path,
    removal: bool,
) -> DispatchOutcome {
    let Ok(target) = Url::parse(&mention.target_url) else {
        return DispatchOutcome::PostFailed;
    };
    let Some(response) = fetch_for_discovery(transport, &target) else {
        return DispatchOutcome::PostFailed;
    };
    let html = std::str::from_utf8(&response.body).ok();
    let Some(endpoint) = discover_endpoint(&header_pairs(&response), html, &response.final_url)
    else {
        return DispatchOutcome::NoEndpoint;
    };
    let status =
        match poster.post_webmention(&endpoint, &mention.source_page_url, &mention.target_url) {
            Ok(s) => s,
            Err(_) => return DispatchOutcome::PostFailed,
        };
    if !(200..300).contains(&status) {
        return DispatchOutcome::PostFailed;
    }
    let _ = append_sent_record(
        vault_root,
        &SentRecord {
            source_page_url: mention.source_page_url.clone(),
            target_url: mention.target_url.clone(),
            content_hash: mention.content_hash.clone(),
            sent_at: now_epoch(),
            response_status: status,
            removal,
        },
    );
    let _ = status;
    DispatchOutcome::Sent
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::fetch::{FetchError, FetchResponse};
    use std::cell::RefCell;
    use tempfile::tempdir;

    struct StaticTransport {
        body: Vec<u8>,
        headers: Vec<(String, String)>,
    }

    impl HttpTransport for StaticTransport {
        fn fetch(&self, request: &FetchRequest) -> Result<FetchResponse, FetchError> {
            Ok(FetchResponse {
                status: 200,
                body: self.body.clone(),
                last_modified: None,
                etag: None,
                content_type: self.headers.iter().find_map(|(k, v)| {
                    if k.eq_ignore_ascii_case("content-type") {
                        Some(v.clone())
                    } else {
                        None
                    }
                }),
                final_url: request.url.clone(),
            })
        }
    }

    struct RecordingPoster {
        posted: RefCell<Vec<(Url, String, String)>>,
        status: u16,
    }

    impl WebmentionPoster for RecordingPoster {
        fn post_webmention(
            &self,
            endpoint: &Url,
            source: &str,
            target: &str,
        ) -> Result<u16, String> {
            self.posted.borrow_mut().push((
                endpoint.clone(),
                source.to_string(),
                target.to_string(),
            ));
            Ok(self.status)
        }
    }

    #[test]
    fn execute_records_sent_on_success() {
        let dir = tempdir().unwrap();
        let transport = StaticTransport {
            body: b"<head><link rel=\"webmention\" href=\"https://t.example/wm\"></head>".to_vec(),
            headers: vec![],
        };
        let poster = RecordingPoster {
            posted: RefCell::new(Vec::new()),
            status: 201,
        };
        let plan = IdempotencyDiff {
            to_send: vec![OutboundMention {
                source_page_url: "https://me.example/p".into(),
                target_url: "https://t.example/post".into(),
                content_hash: "h".into(),
            }],
            to_resend_for_removal: Vec::new(),
        };
        let stats = execute_send_plan_with_poster(&plan, &transport, &poster, dir.path());
        assert_eq!(stats.sent, 1);
        assert_eq!(poster.posted.borrow().len(), 1);
        let log = crate::webmention::persist::load_sent_log(dir.path()).unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].response_status, 201);
    }

    #[test]
    fn endpoint_not_found_does_not_record() {
        let dir = tempdir().unwrap();
        let transport = StaticTransport {
            body: b"<p>no endpoint</p>".to_vec(),
            headers: vec![],
        };
        let poster = RecordingPoster {
            posted: RefCell::new(Vec::new()),
            status: 201,
        };
        let plan = IdempotencyDiff {
            to_send: vec![OutboundMention {
                source_page_url: "https://me.example/p".into(),
                target_url: "https://t.example/post".into(),
                content_hash: "h".into(),
            }],
            to_resend_for_removal: Vec::new(),
        };
        let stats = execute_send_plan_with_poster(&plan, &transport, &poster, dir.path());
        assert_eq!(stats.sent, 0);
        assert_eq!(stats.endpoint_not_found, 1);
        assert!(crate::webmention::persist::load_sent_log(dir.path())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn rebuild_with_no_changes_sends_zero() {
        // Pure compute_send_plan property: building twice with the same
        // input yields an empty plan after the first build's records are
        // persisted.
        let pages = vec![RenderedPage {
            source_page_url: Url::parse("https://me.example/p").unwrap(),
            rendered_html: r#"<a href="https://other.example/post">x</a>"#.to_string(),
        }];
        let base = Url::parse("https://me.example/").unwrap();
        let first_plan = compute_send_plan(&pages, &[], &base);
        assert_eq!(first_plan.to_send.len(), 1);

        // Synthesize a successful sent log from the first plan.
        let log: Vec<SentRecord> = first_plan
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

        let second_plan = compute_send_plan(&pages, &log, &base);
        assert!(second_plan.to_send.is_empty());
        assert!(second_plan.to_resend_for_removal.is_empty());
    }
}
