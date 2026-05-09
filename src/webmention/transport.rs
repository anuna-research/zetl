//! Concrete HTTP transport implementations for webmention I/O.
//!
//! - [`UreqTransport`] implements [`crate::feed::fetch::HttpTransport`]
//!   (read-only GET) for source-fetch verification + endpoint discovery.
//! - [`UreqWebmentionPoster`] implements [`super::send::WebmentionPoster`]
//!   for outbound POST.
//!
//! Both honour the SSRF / scheme / size guards in
//! [`crate::feed::fetch`]: scheme is asserted before request, body is
//! capped at [`crate::feed::fetch::MAX_BODY_BYTES`], timeout is
//! [`crate::feed::fetch::DEFAULT_TIMEOUT_SECS`]. DNS-resolution +
//! per-redirect-hop SSRF check are delegated to ureq's stack — ureq
//! resolves at connect time and we set `redirects(0)` to avoid silent
//! cross-origin hops, requiring callers to handle redirect chains
//! explicitly via the `final_url` field (which here equals the request
//! URL since we don't follow redirects).

use std::io::Read;
use std::net::{IpAddr, ToSocketAddrs};

use url::{Host, Url};

use crate::feed::fetch::{
    assert_public_target, assert_safe_scheme, assert_under_size_cap, FetchError, FetchRequest,
    FetchResponse, HttpTransport, MAX_BODY_BYTES,
};
use crate::webmention::send::WebmentionPoster;

/// Resolve `url`'s host to one or more IPs and refuse if any is private,
/// link-local, loopback, multicast, RFC 6598, RFC 1918, or
/// IPv4-mapped-private-v6. Reuses [`crate::feed::fetch::is_public_ip`]
/// via [`assert_public_target`] for the canonical SSRF policy.
///
/// IP literal hosts (`http://127.0.0.1/`, `http://[::1]/`) are checked
/// directly; only domain hosts are resolved through DNS. This avoids
/// platform inconsistencies in `ToSocketAddrs`'s handling of bracketed
/// IPv6 host strings.
fn assert_public_url(url: &Url) -> Result<(), FetchError> {
    let host = url
        .host()
        .ok_or_else(|| FetchError::Transport("url has no host".to_string()))?;
    let addrs: Vec<IpAddr> = match host {
        Host::Ipv4(addr) => vec![IpAddr::V4(addr)],
        Host::Ipv6(addr) => vec![IpAddr::V6(addr)],
        Host::Domain(name) => {
            let port = url.port_or_known_default().unwrap_or(443);
            (name, port)
                .to_socket_addrs()
                .map_err(|e| FetchError::Transport(format!("dns resolve {name}: {e}")))?
                .map(|sa| sa.ip())
                .collect()
        }
    };
    assert_public_target(&addrs)
}

/// Synchronous HTTP transport backed by `ureq`. Used for source-fetch
/// (receive path) and target-fetch / endpoint-discovery (send path).
#[derive(Clone, Default)]
pub struct UreqTransport;

impl UreqTransport {
    pub fn new() -> Self {
        Self
    }
}

impl HttpTransport for UreqTransport {
    fn fetch(&self, request: &FetchRequest) -> Result<FetchResponse, FetchError> {
        assert_safe_scheme(&request.url)?;
        // T1 SSRF: resolve the host BEFORE handing the URL to ureq and
        // refuse if any resolved IP is non-public. We disable redirects
        // (`.redirects(0)`) so a cross-origin hop can't smuggle a
        // private-IP host past this guard.
        assert_public_url(&request.url)?;

        let agent = ureq::AgentBuilder::new()
            .timeout(request.timeout)
            .redirects(0)
            .user_agent(&request.user_agent)
            .build();

        let mut req = agent.request_url("GET", &request.url);
        if let Some(ims) = &request.conditional.if_modified_since {
            req = req.set("If-Modified-Since", ims);
        }
        if let Some(inm) = &request.conditional.if_none_match {
            req = req.set("If-None-Match", inm);
        }
        if let Some((name, value)) = &request.auth_header {
            req = req.set(name, value);
        }

        let response = match req.call() {
            Ok(r) => r,
            Err(ureq::Error::Status(_status, r)) => r,
            Err(ureq::Error::Transport(t)) => {
                return Err(FetchError::Transport(t.to_string()));
            }
        };

        let status = response.status();
        let last_modified = response.header("last-modified").map(str::to_string);
        let etag = response.header("etag").map(str::to_string);
        let content_type = response.header("content-type").map(str::to_string);
        // Capture every Link: response header for SPEC-039 endpoint
        // discovery. ureq's `headers_names()` returns lowercased names.
        let link_headers: Vec<String> = response
            .headers_names()
            .iter()
            .filter(|n| n.eq_ignore_ascii_case("link"))
            .filter_map(|n| response.header(n).map(str::to_string))
            .collect();

        // Cap body read at MAX_BODY_BYTES + 1 so we can detect overflow.
        let mut body = Vec::with_capacity(8192);
        let cap = MAX_BODY_BYTES + 1;
        let mut reader = response.into_reader().take(cap as u64);
        reader
            .read_to_end(&mut body)
            .map_err(|e| FetchError::Transport(e.to_string()))?;
        assert_under_size_cap(body.len())?;

        Ok(FetchResponse {
            status,
            body,
            last_modified,
            etag,
            content_type,
            link_headers,
            final_url: request.url.clone(),
        })
    }
}

/// Synchronous webmention POSTer backed by `ureq`.
#[derive(Clone, Default)]
pub struct UreqWebmentionPoster;

impl UreqWebmentionPoster {
    pub fn new() -> Self {
        Self
    }
}

impl WebmentionPoster for UreqWebmentionPoster {
    fn post_webmention(&self, endpoint: &Url, source: &str, target: &str) -> Result<u16, String> {
        assert_safe_scheme(endpoint).map_err(|e| e.to_string())?;
        // T1 SSRF: refuse to POST to a private endpoint. Redirects are
        // disabled (`.redirects(0)`) so a 3xx pointing at an internal
        // service cannot smuggle the request past this guard. Spec-wise
        // this is conservative — the W3C REC permits redirects on the
        // POST, but a malicious target advertising a private endpoint
        // would otherwise let an external page coerce zetl build into
        // POSTing to internal admin services.
        assert_public_url(endpoint).map_err(|e| e.to_string())?;
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(
                crate::feed::fetch::DEFAULT_TIMEOUT_SECS,
            ))
            .redirects(0)
            .user_agent(&crate::feed::fetch::user_agent())
            .build();
        let req = agent
            .request_url("POST", endpoint)
            .set("Content-Type", "application/x-www-form-urlencoded");
        let body = format!(
            "source={}&target={}",
            urlencoding::encode(source),
            urlencoding::encode(target),
        );
        match req.send_string(&body) {
            Ok(r) => Ok(r.status()),
            Err(ureq::Error::Status(s, _)) => Ok(s),
            Err(ureq::Error::Transport(t)) => Err(t.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assert_public_url_rejects_loopback_literal() {
        let err = assert_public_url(&Url::parse("http://127.0.0.1/").unwrap()).unwrap_err();
        assert!(matches!(err, FetchError::PrivateAddress(_)));
    }

    #[test]
    fn assert_public_url_rejects_private_v6_literal() {
        let err = assert_public_url(&Url::parse("http://[::1]/").unwrap()).unwrap_err();
        assert!(matches!(err, FetchError::PrivateAddress(_)));
    }

    #[test]
    fn assert_public_url_rejects_link_local() {
        let err = assert_public_url(&Url::parse("http://169.254.169.254/").unwrap()).unwrap_err();
        assert!(matches!(err, FetchError::PrivateAddress(_)));
    }

    #[test]
    fn poster_rejects_loopback_endpoint() {
        let poster = UreqWebmentionPoster::new();
        let err = poster
            .post_webmention(
                &Url::parse("http://127.0.0.1/wm").unwrap(),
                "https://me.example/p",
                "https://t.example/q",
            )
            .unwrap_err();
        assert!(err.to_lowercase().contains("non-public"));
    }

    #[test]
    fn poster_rejects_unsafe_scheme() {
        let poster = UreqWebmentionPoster::new();
        let err = poster
            .post_webmention(
                &Url::parse("javascript:alert(1)").unwrap(),
                "https://me.example/p",
                "https://t.example/q",
            )
            .unwrap_err();
        assert!(err.to_lowercase().contains("scheme"));
    }
}
